use crate::domain::VariableSpec;
use crate::parser;
use std::collections::{BTreeMap, HashMap};
use tower_lsp::lsp_types::*;

use super::position::{byte_span_to_range, utf16_column_to_byte};
use super::{find_variable_declaration_line, frontmatter_end_line};

// ---------------------------------------------------------------------------
// Code actions
// ---------------------------------------------------------------------------

const FRONTMATTER_SCAFFOLD: &str =
    "---\nname: \"\"\ndescription: \"\"\ntags: []\nvariables:\n---\n";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InlineSourceKind {
    Default,
    Command,
    /// An inline `[...]` list; `value` keeps the raw source, see
    /// [`InlinePlaceholder::suggestion_items`].
    Suggestions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InlinePlaceholder {
    name: String,
    kind: InlineSourceKind,
    /// Raw source text after `:` (and after `?` for defaults).
    value: String,
    line: u32,
    start: usize,
    end: usize,
}

impl InlinePlaceholder {
    /// Parsed items of an inline list (empty for other kinds). Lists compare
    /// by items, so `[a,b]` and `[ a , b ]` collapse together.
    fn suggestion_items(&self) -> Vec<String> {
        inline_suggestion_items(self.kind, &self.value)
    }
}

fn inline_suggestion_items(kind: InlineSourceKind, value: &str) -> Vec<String> {
    match kind {
        InlineSourceKind::Suggestions => parser::parse_suggestion_list(value)
            .map(|list| list.items)
            .unwrap_or_default(),
        InlineSourceKind::Default | InlineSourceKind::Command => Vec::new(),
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn compute_code_actions(
    uri: &Url,
    content: &str,
    range: Range,
    config_vars: &BTreeMap<String, VariableSpec>,
) -> Option<CodeActionResponse> {
    compute_code_actions_filtered(uri, content, range, config_vars, None)
}

pub(super) fn compute_code_actions_filtered(
    uri: &Url,
    content: &str,
    range: Range,
    config_vars: &BTreeMap<String, VariableSpec>,
    only: Option<&[CodeActionKind]>,
) -> Option<CodeActionResponse> {
    let pos = range.start;
    let mut actions = Vec::new();
    if let Some(inline) = inline_placeholder_at(content, pos)
        && extraction_preserves_prompt(&inline, config_vars)
    {
        actions.push(extract_code_action(uri, content, &inline, config_vars));
    }
    if let Some(action) = inline_code_action(uri, content, pos, config_vars) {
        actions.push(action);
    }
    let lines: Vec<&str> = content.lines().collect();
    if should_offer_frontmatter_scaffold(&lines) {
        actions.push(frontmatter_scaffold_code_action(uri));
    }
    let actions: Vec<_> = actions
        .into_iter()
        .filter(|action| code_action_allowed(action, only))
        .collect();
    if actions.is_empty() {
        None
    } else {
        Some(
            actions
                .into_iter()
                .map(CodeActionOrCommand::CodeAction)
                .collect(),
        )
    }
}

/// Whether moving `inline` into a frontmatter spec keeps the prompt the same.
/// Inline commands and lists shadow every lower layer, but a file-local spec
/// only overlays config field by field: a config `default` would start
/// pre-filling `<@name>`, and an empty `suggestions: []` would let config or
/// built-in suggestions through. Decline rather than change what Enter
/// accepts. Inline `:?` extraction is unaffected: its `default_value` ghost
/// takes priority over any editable `default`.
fn extraction_preserves_prompt(
    inline: &InlinePlaceholder,
    config_vars: &BTreeMap<String, VariableSpec>,
) -> bool {
    match inline.kind {
        InlineSourceKind::Suggestions if inline.suggestion_items().is_empty() => false,
        InlineSourceKind::Suggestions | InlineSourceKind::Command => config_vars
            .get(&inline.name)
            .is_none_or(|spec| spec.default.is_none()),
        InlineSourceKind::Default => true,
    }
}

fn should_offer_frontmatter_scaffold(lines: &[&str]) -> bool {
    frontmatter_end_line(lines).is_none() && lines.first().map(|line| line.trim()) != Some("---")
}

fn frontmatter_scaffold_code_action(uri: &Url) -> CodeAction {
    CodeAction {
        title: "Add Peanutbutter frontmatter".to_string(),
        kind: Some(CodeActionKind::SOURCE),
        edit: Some(workspace_edit(
            uri,
            vec![TextEdit {
                range: empty_line_range(0, 0),
                new_text: FRONTMATTER_SCAFFOLD.to_string(),
            }],
        )),
        ..Default::default()
    }
}

fn code_action_allowed(action: &CodeAction, only: Option<&[CodeActionKind]>) -> bool {
    let Some(only) = only else {
        return true;
    };
    if only.is_empty() {
        return true;
    }
    let Some(kind) = action.kind.as_ref() else {
        return false;
    };
    only.iter()
        .any(|requested| code_action_kind_matches(kind, requested))
}

fn code_action_kind_matches(kind: &CodeActionKind, requested: &CodeActionKind) -> bool {
    let kind = kind.as_str();
    let requested = requested.as_str();
    kind == requested
        || (!requested.is_empty()
            && kind
                .strip_prefix(requested)
                .is_some_and(|suffix| suffix.starts_with('.')))
}

fn extract_code_action(
    uri: &Url,
    content: &str,
    inline: &InlinePlaceholder,
    config_vars: &BTreeMap<String, VariableSpec>,
) -> CodeAction {
    let lines: Vec<&str> = content.lines().collect();
    let parsed = parser::parse_file(std::path::Path::new(""), std::path::Path::new(""), content);
    let existing = parsed.frontmatter.variables.get(&inline.name);
    let spec_matches = existing.is_some_and(|spec| spec_matches_inline(spec, inline));
    let conflict = existing.is_some() && !spec_matches;
    // Collapse every placeholder sharing this name and inline source, not just the
    // one under the cursor: extracting a single value to a shared frontmatter spec
    // is only consistent if all identical duplicates lose their redundant source.
    let mut edits: Vec<TextEdit> = matching_inline_placeholders(content, inline)
        .into_iter()
        .map(|placeholder| {
            let line = lines.get(placeholder.line as usize).copied().unwrap_or("");
            TextEdit {
                range: byte_span_to_range(
                    placeholder.line,
                    line,
                    placeholder.start,
                    placeholder.end,
                ),
                new_text: format!("<@{}>", placeholder.name),
            }
        })
        .collect();
    let occurrences = edits.len();
    if !spec_matches {
        edits.push(if conflict {
            overwrite_variable_spec_edit(&lines, inline)
        } else {
            upsert_variable_spec_edit(&lines, inline)
        });
    }

    let mut title = if conflict {
        format!(
            "Extract and overwrite frontmatter spec for `{}`",
            inline.name
        )
    } else {
        format!("Extract `<@{}>` to frontmatter", inline.name)
    };
    if occurrences > 1 {
        title.push_str(&format!(" (collapses all {occurrences} occurrences)"));
    }
    if config_vars.contains_key(&inline.name) {
        title.push_str(" (overrides config-defined spec)");
    }
    CodeAction {
        title,
        kind: Some(CodeActionKind::REFACTOR_EXTRACT),
        edit: Some(workspace_edit(uri, edits)),
        ..Default::default()
    }
}

fn inline_code_action(
    uri: &Url,
    content: &str,
    pos: Position,
    config_vars: &BTreeMap<String, VariableSpec>,
) -> Option<CodeAction> {
    let lines: Vec<&str> = content.lines().collect();
    let name = frontmatter_variable_at(&lines, pos)?;
    if config_vars.contains_key(&name) {
        return None;
    }
    let parsed = parser::parse_file(std::path::Path::new(""), std::path::Path::new(""), content);
    let spec = parsed.frontmatter.variables.get(&name)?;
    // Inlining removes the whole spec, so only offer it when the spec is a
    // single non-deprecated source that has an inline equivalent.
    if spec.hint.is_some() {
        return None;
    }
    let source = match (
        &spec.default_value,
        &spec.command,
        &spec.default,
        spec.suggestions.as_slice(),
    ) {
        (Some(default), None, None, []) => format!("?{default}"),
        (None, Some(command), None, []) => command.clone(),
        // Skipped when an item can't be written inline (e.g. contains `>`).
        (None, None, None, items @ [_, ..]) => parser::format_suggestion_list(items)?,
        _ => return None,
    };
    let target = first_placeholder(content, &name)?;
    if target.has_source {
        return None;
    }
    let inline_text = format!("<@{name}:{source}>");
    let mut title = format!("Inline frontmatter variable `{name}`");
    let usages = placeholder_usage_count(content, &name);
    if usages > 1 {
        title.push_str(&format!(" (affects all {usages} usages)"));
    }
    let target_line = lines.get(target.line as usize).copied()?;
    Some(CodeAction {
        title,
        kind: Some(CodeActionKind::REFACTOR_INLINE),
        edit: Some(workspace_edit(
            uri,
            vec![
                TextEdit {
                    range: byte_span_to_range(target.line, target_line, target.start, target.end),
                    new_text: inline_text,
                },
                remove_variable_spec_edit(&lines, &name)?,
            ],
        )),
        ..Default::default()
    })
}

fn workspace_edit(uri: &Url, edits: Vec<TextEdit>) -> WorkspaceEdit {
    WorkspaceEdit {
        changes: Some(HashMap::from([(uri.clone(), edits)])),
        document_changes: None,
        change_annotations: None,
    }
}

fn inline_placeholder_at(content: &str, pos: Position) -> Option<InlinePlaceholder> {
    let line = content.lines().nth(pos.line as usize)?;
    let char_idx = utf16_column_to_byte(line, pos.character)?;
    let bytes = line.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'<' && bytes[i + 1] == b'@' {
            let start = i;
            let end = parser::find_placeholder_end(line, start)? + 1;
            if char_idx >= start && char_idx <= end {
                let inner = &line[start + 2..end - 1];
                let (name, rest) = inner.split_once(':')?;
                let name = name.trim();
                if !valid_variable_name(name) {
                    return None;
                }
                let Some((kind, value)) = classify_inline_source(rest) else {
                    i = end;
                    continue;
                };
                return Some(InlinePlaceholder {
                    name: name.to_string(),
                    kind,
                    value: value.to_string(),
                    line: pos.line,
                    start,
                    end,
                });
            }
            i = end;
        } else {
            i += 1;
        }
    }
    None
}

/// Classify the raw text after `<@name:` the same way the parser does, for
/// the source kinds code actions can move to and from frontmatter. Returns
/// `None` for deprecated hints: the parser keeps them reserved, but authoring
/// actions must not generate equivalent specs or treat their display text as
/// an executable command.
fn classify_inline_source(source: &str) -> Option<(InlineSourceKind, &str)> {
    if let Some(default) = source.strip_prefix('?') {
        Some((InlineSourceKind::Default, default))
    } else if source.starts_with('@') {
        None
    } else if parser::parse_suggestion_list(source).is_some() {
        Some((InlineSourceKind::Suggestions, source))
    } else {
        Some((InlineSourceKind::Command, source))
    }
}

/// Every placeholder in `content` whose name, source kind, and value match
/// `target`, including `target` itself. Differing inline overrides (same name,
/// different value) are excluded so they keep their own source after extraction.
fn matching_inline_placeholders(
    content: &str,
    target: &InlinePlaceholder,
) -> Vec<InlinePlaceholder> {
    let mut matches = Vec::new();
    for (line_idx, line) in content.lines().enumerate() {
        let bytes = line.as_bytes();
        let mut i = 0;
        while i + 1 < bytes.len() {
            if bytes[i] == b'<' && bytes[i + 1] == b'@' {
                let start = i;
                let Some(end) = parser::find_placeholder_end(line, start).map(|e| e + 1) else {
                    break;
                };
                let inner = &line[start + 2..end - 1];
                if let Some((name, rest)) = inner.split_once(':') {
                    let name = name.trim();
                    let Some((kind, value)) = classify_inline_source(rest) else {
                        i = end;
                        continue;
                    };
                    let same_source = match kind {
                        InlineSourceKind::Suggestions => {
                            inline_suggestion_items(kind, value) == target.suggestion_items()
                        }
                        InlineSourceKind::Default | InlineSourceKind::Command => {
                            value == target.value
                        }
                    };
                    if valid_variable_name(name)
                        && name == target.name
                        && kind == target.kind
                        && same_source
                    {
                        matches.push(InlinePlaceholder {
                            name: name.to_string(),
                            kind,
                            value: value.to_string(),
                            line: line_idx as u32,
                            start,
                            end,
                        });
                    }
                }
                i = end;
            } else {
                i += 1;
            }
        }
    }
    matches
}

#[derive(Debug, Clone, Copy)]
struct PlaceholderUse {
    line: u32,
    start: usize,
    end: usize,
    has_source: bool,
}

fn first_placeholder(content: &str, name: &str) -> Option<PlaceholderUse> {
    for (line_idx, line) in content.lines().enumerate() {
        let bytes = line.as_bytes();
        let mut i = 0;
        while i + 1 < bytes.len() {
            if bytes[i] == b'<' && bytes[i + 1] == b'@' {
                let start = i;
                let end = parser::find_placeholder_end(line, start)? + 1;
                let inner = &line[start + 2..end - 1];
                let found = inner.split(':').next().unwrap_or("").trim();
                if found == name {
                    return Some(PlaceholderUse {
                        line: line_idx as u32,
                        start,
                        end,
                        has_source: inner.contains(':'),
                    });
                }
                i = end;
            } else {
                i += 1;
            }
        }
    }
    None
}

fn spec_matches_inline(spec: &VariableSpec, inline: &InlinePlaceholder) -> bool {
    match inline.kind {
        InlineSourceKind::Default => {
            spec.default_value.as_deref() == Some(inline.value.as_str())
                && spec.default.is_none()
                && spec.command.is_none()
                && spec.hint.is_none()
                && spec.suggestions.is_empty()
        }
        InlineSourceKind::Command => {
            spec.command.as_deref() == Some(inline.value.as_str()) && spec.default.is_none()
        }
        InlineSourceKind::Suggestions => {
            spec.suggestions == inline.suggestion_items()
                && spec.command.is_none()
                && spec.default.is_none()
                && spec.default_value.is_none()
        }
    }
}

fn upsert_variable_spec_edit(lines: &[&str], inline: &InlinePlaceholder) -> TextEdit {
    let entry = variable_spec_text(inline);
    if let Some(fm_end) = frontmatter_end_line(lines) {
        if let Some(var_line) = variables_line(lines, fm_end) {
            let insert_line = variables_block_end_line(lines, var_line, fm_end);
            return TextEdit {
                range: empty_line_range(insert_line as u32, 0),
                new_text: entry,
            };
        }
        TextEdit {
            range: empty_line_range(fm_end as u32, 0),
            new_text: format!("variables:\n{entry}"),
        }
    } else {
        TextEdit {
            range: empty_line_range(0, 0),
            new_text: format!("---\nvariables:\n{entry}---\n"),
        }
    }
}

fn overwrite_variable_spec_edit(lines: &[&str], inline: &InlinePlaceholder) -> TextEdit {
    let Some(decl_line) = find_variable_declaration_line(lines, &inline.name) else {
        return upsert_variable_spec_edit(lines, inline);
    };
    let end_line = variable_entry_end_line(lines, decl_line);
    TextEdit {
        range: Range {
            start: Position {
                line: decl_line as u32,
                character: 0,
            },
            end: Position {
                line: end_line as u32,
                character: 0,
            },
        },
        new_text: variable_spec_text_preserving(lines, decl_line, end_line, inline),
    }
}

fn remove_variable_spec_edit(lines: &[&str], name: &str) -> Option<TextEdit> {
    let decl_line = find_variable_declaration_line(lines, name)?;
    let entry_end = variable_entry_end_line(lines, decl_line);
    let fm_end = frontmatter_end_line(lines)?;
    let var_line = variables_line(lines, fm_end)?;
    let block_end = variables_block_end_line(lines, var_line, fm_end);
    let only_entry = lines[var_line + 1..block_end]
        .iter()
        .enumerate()
        .filter(|(_, line)| {
            let trimmed = line.trim_start();
            let indent = line.len() - trimmed.len();
            indent == 2 && trimmed.ends_with(':')
        })
        .map(|(idx, _)| var_line + 1 + idx)
        .eq(std::iter::once(decl_line));
    let start_line = if only_entry { var_line } else { decl_line };
    let end_line = if only_entry { block_end } else { entry_end };
    Some(TextEdit {
        range: Range {
            start: Position {
                line: start_line as u32,
                character: 0,
            },
            end: Position {
                line: end_line as u32,
                character: 0,
            },
        },
        new_text: String::new(),
    })
}

fn variable_spec_text(inline: &InlinePlaceholder) -> String {
    format!("  {}:\n{}", inline.name, spec_source_lines(inline))
}

/// The YAML written for the extracted source under a `  <name>:` entry, one
/// level deeper (4 spaces) and newline-terminated.
fn spec_source_lines(inline: &InlinePlaceholder) -> String {
    let key = inline_source_key(inline.kind);
    match inline.kind {
        // Block form: `yaml_scalar` quotes for block context only, and an item
        // like `a,b` or `c]` would split or break a flow `[...]` sequence.
        InlineSourceKind::Suggestions => {
            let items = inline.suggestion_items();
            if items.is_empty() {
                return format!("    {key}: []\n");
            }
            let mut out = format!("    {key}:\n");
            for item in items {
                out.push_str(&format!("      - {}\n", yaml_scalar(&item)));
            }
            out
        }
        InlineSourceKind::Default | InlineSourceKind::Command => {
            format!("    {key}: {}\n", yaml_scalar(&inline.value))
        }
    }
}

fn variable_spec_text_preserving(
    lines: &[&str],
    decl_line: usize,
    end_line: usize,
    inline: &InlinePlaceholder,
) -> String {
    let replaced_keys = replaced_spec_keys(inline.kind);
    let mut out = format!("{}\n{}", lines[decl_line], spec_source_lines(inline));
    let mut skip_suggestions_list = false;
    for line in &lines[decl_line + 1..end_line] {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        if skip_suggestions_list {
            if indent > 4 && trimmed.starts_with('-') {
                continue;
            }
            skip_suggestions_list = false;
        }
        let key = trimmed.split_once(':').map(|(key, _)| key.trim());
        if indent == 4
            && key == Some("suggestions")
            && replaced_keys.contains(&"suggestions")
            && trimmed.ends_with(':')
        {
            skip_suggestions_list = true;
            continue;
        }
        if indent == 4 && key.is_some_and(|key| replaced_keys.contains(&key)) {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Spec keys an extraction overwrites, so the frontmatter entry behaves the
/// way the inline source did.
fn replaced_spec_keys(kind: InlineSourceKind) -> &'static [&'static str] {
    match kind {
        // `default_value` is a source, not a composable prompt option.
        // Extracting inline `:?` must remove every mutually-exclusive field,
        // including a block-form suggestions list.
        InlineSourceKind::Default => {
            &["default_value", "default", "command", "hint", "suggestions"]
        }
        // Retained hints can compose with commands, so extraction must not
        // silently discard their compatibility behavior.
        InlineSourceKind::Command => &["default", "command"],
        // An inline list shadows every other suggestion source and default,
        // so the extracted spec must not regain them.
        InlineSourceKind::Suggestions => &["suggestions", "command", "default", "default_value"],
    }
}

fn inline_source_key(kind: InlineSourceKind) -> &'static str {
    match kind {
        InlineSourceKind::Default => "default_value",
        InlineSourceKind::Command => "command",
        InlineSourceKind::Suggestions => "suggestions",
    }
}

fn yaml_scalar(value: &str) -> String {
    let needs_quote = value.is_empty()
        || value.trim() != value
        || value.contains(':')
        || value.contains('#')
        || value.starts_with([
            '-', '?', ':', ',', '[', ']', '{', '}', '&', '*', '!', '|', '>', '\'', '"', '%', '@',
            '`',
        ]);
    if needs_quote {
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        value.to_string()
    }
}

fn frontmatter_variable_at(lines: &[&str], pos: Position) -> Option<String> {
    let line_idx = pos.line as usize;
    let fm_end = frontmatter_end_line(lines)?;
    if line_idx == 0 || line_idx >= fm_end {
        return None;
    }
    let trimmed = lines.get(line_idx)?.trim_start();
    let indent = lines[line_idx].len() - trimmed.len();
    if indent != 2 || !trimmed.ends_with(':') {
        return None;
    }
    let name = trimmed.strip_suffix(':')?.trim();
    if find_variable_declaration_line(lines, name) == Some(line_idx) {
        Some(name.to_string())
    } else {
        None
    }
}

fn variables_line(lines: &[&str], fm_end: usize) -> Option<usize> {
    (1..fm_end).find(|&i| lines[i].trim() == "variables:")
}

fn variables_block_end_line(lines: &[&str], var_line: usize, fm_end: usize) -> usize {
    let mut i = var_line + 1;
    while i < fm_end {
        let trimmed = lines[i].trim_start();
        let indent = lines[i].len() - trimmed.len();
        if indent == 0 && !trimmed.is_empty() {
            break;
        }
        i += 1;
    }
    i
}

fn variable_entry_end_line(lines: &[&str], decl_line: usize) -> usize {
    let mut i = decl_line + 1;
    while i < lines.len() {
        let trimmed = lines[i].trim_start();
        let indent = lines[i].len() - trimmed.len();
        if indent <= 2 && !trimmed.is_empty() {
            break;
        }
        i += 1;
    }
    i
}

fn empty_line_range(line: u32, character: u32) -> Range {
    Range {
        start: Position { line, character },
        end: Position { line, character },
    }
}

fn valid_variable_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Number of `<@name...>` placeholders referencing `name`, regardless of source.
/// Inlining removes the shared frontmatter spec, so every usage is affected.
fn placeholder_usage_count(content: &str, name: &str) -> usize {
    let mut count = 0;
    for line in content.lines() {
        let bytes = line.as_bytes();
        let mut i = 0;
        while i + 1 < bytes.len() {
            if bytes[i] == b'<' && bytes[i + 1] == b'@' {
                let start = i;
                let Some(end) = parser::find_placeholder_end(line, start).map(|e| e + 1) else {
                    break;
                };
                let inner = &line[start + 2..end - 1];
                if inner.split(':').next().unwrap_or("").trim() == name {
                    count += 1;
                }
                i = end;
            } else {
                i += 1;
            }
        }
    }
    count
}

#[cfg(test)]
mod code_action_tests {
    use super::*;

    fn pos(line: u32, character: u32) -> Position {
        Position { line, character }
    }

    fn range(line: u32, character: u32) -> Range {
        let p = pos(line, character);
        Range { start: p, end: p }
    }

    fn uri() -> Url {
        Url::parse("file:///snippets.md").unwrap()
    }

    fn code_action(action: &CodeActionOrCommand) -> &CodeAction {
        let CodeActionOrCommand::CodeAction(action) = action else {
            panic!("expected code action");
        };
        action
    }

    fn edits(action: &CodeActionOrCommand) -> &[TextEdit] {
        let action = code_action(action);
        let changes = action.edit.as_ref().unwrap().changes.as_ref().unwrap();
        changes.get(&uri()).unwrap()
    }

    fn action_title(action: &CodeActionOrCommand) -> &str {
        &code_action(action).title
    }

    fn apply_edits(content: &str, edits: &[TextEdit]) -> String {
        let mut line_offsets = vec![0];
        for (byte_index, byte) in content.bytes().enumerate() {
            if byte == b'\n' {
                line_offsets.push(byte_index + 1);
            }
        }
        let lines: Vec<&str> = content.split('\n').collect();
        let to_offset = |p: Position| -> usize {
            line_offsets[p.line as usize]
                + super::super::position::utf16_column_to_byte(lines[p.line as usize], p.character)
                    .expect("generated LSP position must be valid")
        };
        let mut ordered = edits.to_vec();
        ordered.sort_by_key(|edit| std::cmp::Reverse(to_offset(edit.range.start)));
        let mut out = content.to_string();
        for edit in ordered {
            let start = to_offset(edit.range.start);
            let end = to_offset(edit.range.end);
            out.replace_range(start..end, &edit.new_text);
        }
        out
    }

    #[test]
    fn initialize_advertises_code_actions() {
        assert_eq!(
            super::super::server_capabilities().code_action_provider,
            Some(CodeActionProviderCapability::Simple(true))
        );
    }

    #[test]
    fn inline_placeholder_at_extracts_sources_and_ignores_plain() {
        let default = inline_placeholder_at("## D\n\n```bash\n<@p:?.>\n```\n", pos(3, 3)).unwrap();
        assert_eq!(default.name, "p");
        assert_eq!(default.kind, InlineSourceKind::Default);
        assert_eq!(default.value, ".");

        let command =
            inline_placeholder_at("## D\n\n```bash\n<@b:git log <#ref:raw>>\n```\n", pos(3, 5))
                .unwrap();
        assert_eq!(command.kind, InlineSourceKind::Command);
        assert_eq!(command.value, "git log <#ref:raw>");

        assert!(inline_placeholder_at("## D\n\n```bash\n<@x>\n```\n", pos(3, 2)).is_none());
        assert!(
            inline_placeholder_at("## D\n\n```bash\n<@x:unterminated\n```\n", pos(3, 2)).is_none()
        );
    }

    #[test]
    fn extract_default_to_existing_frontmatter_quotes_yaml() {
        let content =
            "---\nname: T\n---\n## D\n\n```bash\necho <@path:?<#a:raw>.out # keep>\n```\n";
        let actions = compute_code_actions(&uri(), content, range(6, 8), &BTreeMap::new()).unwrap();
        assert_eq!(actions.len(), 1);
        assert!(action_title(&actions[0]).contains("Extract `<@path>`"));
        let updated = apply_edits(content, edits(&actions[0]));
        assert!(
            updated
                .contains("variables:\n  path:\n    default_value: \"<#a:raw>.out # keep\"\n---")
        );
        assert!(updated.contains("echo <@path>"));
    }

    #[test]
    fn extract_creates_frontmatter_when_missing() {
        let content = "## D\n\n```bash\necho <@branch:git branch --show-current>\n```\n";
        let actions = compute_code_actions(&uri(), content, range(3, 8), &BTreeMap::new()).unwrap();
        let updated = apply_edits(content, edits(&actions[0]));
        assert!(updated.starts_with(
            "---\nvariables:\n  branch:\n    command: git branch --show-current\n---\n## D"
        ));
    }

    #[test]
    fn scaffold_frontmatter_action_inserts_canonical_block() {
        let content = "## D\n\n```bash\necho hi\n```\n";
        let actions = compute_code_actions(&uri(), content, range(0, 0), &BTreeMap::new()).unwrap();
        let action = actions
            .iter()
            .map(code_action)
            .find(|action| action.title == "Add Peanutbutter frontmatter")
            .unwrap();
        assert_eq!(action.kind, Some(CodeActionKind::SOURCE));
        let changes = action.edit.as_ref().unwrap().changes.as_ref().unwrap();
        let action_edits = changes.get(&uri()).unwrap();
        assert_eq!(action_edits.len(), 1);
        assert_eq!(action_edits[0].range.start, pos(0, 0));
        assert_eq!(action_edits[0].range.end, pos(0, 0));
        assert_eq!(action_edits[0].new_text, FRONTMATTER_SCAFFOLD);
        let updated = apply_edits(content, action_edits);
        assert_eq!(updated, format!("{FRONTMATTER_SCAFFOLD}{content}"));
    }

    #[test]
    fn scaffold_frontmatter_action_inserts_into_empty_document() {
        let content = "";
        let actions = compute_code_actions(&uri(), content, range(0, 0), &BTreeMap::new()).unwrap();
        let action = actions
            .iter()
            .find(|action| action_title(action) == "Add Peanutbutter frontmatter")
            .unwrap();
        let updated = apply_edits(content, edits(action));
        assert_eq!(updated, FRONTMATTER_SCAFFOLD);
    }

    #[test]
    fn scaffold_frontmatter_action_not_offered_when_frontmatter_exists() {
        for content in ["---\nname: T\n---\n## D\n", "---\n---\n## D\n"] {
            let actions = compute_code_actions(&uri(), content, range(3, 0), &BTreeMap::new());
            assert!(actions.is_none_or(|actions| {
                actions
                    .iter()
                    .all(|action| action_title(action) != "Add Peanutbutter frontmatter")
            }));
        }
    }

    #[test]
    fn scaffold_frontmatter_action_not_offered_for_malformed_frontmatter_start() {
        let content = "---\nname: T\n## D\n";
        let actions = compute_code_actions(&uri(), content, range(2, 0), &BTreeMap::new());
        assert!(actions.is_none_or(|actions| {
            actions
                .iter()
                .all(|action| action_title(action) != "Add Peanutbutter frontmatter")
        }));
    }

    #[test]
    fn scaffold_frontmatter_action_coexists_with_extract() {
        let content = "## D\n\n```bash\necho <@branch:git branch --show-current>\n```\n";
        let actions = compute_code_actions(&uri(), content, range(3, 8), &BTreeMap::new()).unwrap();
        assert!(action_title(&actions[0]).contains("Extract `<@branch>` to frontmatter"));
        assert!(
            actions
                .iter()
                .any(|action| action_title(action) == "Add Peanutbutter frontmatter")
        );
    }

    #[test]
    fn code_action_kind_filter_limits_source_and_refactor_actions() {
        let content = "## D\n\n```bash\necho <@branch:git branch --show-current>\n```\n";
        let source_actions = compute_code_actions_filtered(
            &uri(),
            content,
            range(3, 8),
            &BTreeMap::new(),
            Some(&[CodeActionKind::SOURCE]),
        )
        .unwrap();
        assert!(
            source_actions
                .iter()
                .any(|action| action_title(action) == "Add Peanutbutter frontmatter")
        );
        assert!(
            source_actions
                .iter()
                .all(|action| !action_title(action).contains("Extract `<@branch>`"))
        );

        let refactor_actions = compute_code_actions_filtered(
            &uri(),
            content,
            range(3, 8),
            &BTreeMap::new(),
            Some(&[CodeActionKind::REFACTOR]),
        )
        .unwrap();
        assert!(
            refactor_actions
                .iter()
                .any(|action| action_title(action).contains("Extract `<@branch>`"))
        );
        assert!(
            refactor_actions
                .iter()
                .all(|action| action_title(action) != "Add Peanutbutter frontmatter")
        );
    }

    #[test]
    fn extract_matching_spec_only_simplifies_placeholder() {
        let content =
            "---\nvariables:\n  p:\n    default_value: .\n---\n## D\n\n```bash\n<@p:?.>\n```\n";
        let actions = compute_code_actions(&uri(), content, range(8, 2), &BTreeMap::new()).unwrap();
        let action_edits = edits(&actions[0]);
        assert_eq!(action_edits.len(), 1);
        assert_eq!(action_edits[0].new_text, "<@p>");
    }

    #[test]
    fn extract_collapses_all_matching_duplicates_but_keeps_overrides() {
        let content = "## D\n\n```bash\n<@p:?.> <@p:?.>\n<@p:?/tmp>\n<@p:?.>\n```\n";
        let actions = compute_code_actions(&uri(), content, range(3, 2), &BTreeMap::new()).unwrap();
        // Title counts the placeholders rewritten, not the `/tmp` override.
        assert!(action_title(&actions[0]).contains("(collapses all 3 occurrences)"));
        let updated = apply_edits(content, edits(&actions[0]));
        // All `<@p:?.>` duplicates collapse to `<@p>`.
        assert_eq!(updated.matches("<@p>").count(), 3);
        assert!(!updated.contains("<@p:?.>"));
        // The differing override keeps its own inline value.
        assert!(updated.contains("<@p:?/tmp>"));
        assert!(updated.contains("variables:\n  p:\n    default_value: .\n"));
    }

    #[test]
    fn deprecated_hints_do_not_offer_refactor_actions() {
        let inline = "## D\n\n```bash\necho <@input:@hello>\n```\n";
        assert!(
            compute_code_actions_filtered(
                &uri(),
                inline,
                range(3, 8),
                &BTreeMap::new(),
                Some(&[CodeActionKind::REFACTOR]),
            )
            .is_none()
        );

        let adjacent = "## D\n\n```bash\n<@old:@h><@next:?value>\n```\n";
        let actions = compute_code_actions_filtered(
            &uri(),
            adjacent,
            range(3, 9),
            &BTreeMap::new(),
            Some(&[CodeActionKind::REFACTOR]),
        )
        .unwrap();
        assert!(action_title(&actions[0]).contains("`<@next>`"));

        let frontmatter = "---\nvariables:\n  input:\n    hint: hello\n---\n## D\n\n```bash\necho <@input>\n```\n";
        assert!(
            compute_code_actions_filtered(
                &uri(),
                frontmatter,
                range(2, 3),
                &BTreeMap::new(),
                Some(&[CodeActionKind::REFACTOR]),
            )
            .is_none()
        );
    }

    #[test]
    fn extract_default_removes_mutually_exclusive_fields() {
        let content = "---\nvariables:\n  p:\n    hint: guidance\n    default: old\n    suggestions:\n      - old\n---\n## D\n\n```bash\n<@p:?new>\n```\n";
        let actions =
            compute_code_actions(&uri(), content, range(11, 2), &BTreeMap::new()).unwrap();
        let updated = apply_edits(content, edits(&actions[0]));
        assert!(updated.contains("default_value: new"));
        assert!(!updated.contains("hint: guidance"));
        assert!(!updated.contains("default: old"));
        assert!(!updated.contains("- old"));
    }

    #[test]
    fn inline_skips_spec_with_both_hint_and_default() {
        let content =
            "---\nvariables:\n  p:\n    default: a\n    hint: b\n---\n## D\n\n```bash\n<@p>\n```\n";
        assert!(compute_code_actions(&uri(), content, range(2, 3), &BTreeMap::new()).is_none());
    }

    #[test]
    fn extract_conflict_offers_overwrite_action() {
        let content =
            "---\nvariables:\n  p:\n    default: old\n---\n## D\n\n```bash\n<@p:?new>\n```\n";
        let actions = compute_code_actions(&uri(), content, range(8, 2), &BTreeMap::new()).unwrap();
        assert_eq!(actions.len(), 1);
        assert!(action_title(&actions[0]).contains("overwrite"));
        let updated = apply_edits(content, edits(&actions[0]));
        assert!(updated.contains("  p:\n    default_value: new\n"));
        assert!(!updated.contains("default: old"));
    }

    #[test]
    fn inline_default_rewrites_first_plain_usage_and_removes_entry() {
        let content =
            "---\nvariables:\n  p:\n    default_value: .\n---\n## D\n\n```bash\n<@p> <@p>\n```\n";
        let actions = compute_code_actions(&uri(), content, range(2, 3), &BTreeMap::new()).unwrap();
        assert!(action_title(&actions[0]).contains("(affects all 2 usages)"));
        let updated = apply_edits(content, edits(&actions[0]));
        assert!(updated.contains("<@p:?.> <@p>"));
        assert!(!updated.contains("variables:"));
    }

    #[test]
    fn inline_skips_suggestions_ambiguous_config_and_inline_target() {
        // Suggestions only inline when they are the spec's sole source.
        for extra in ["default: a", "command: b", "default_value: c"] {
            let suggestions = format!(
                "---\nvariables:\n  p:\n    suggestions: [a]\n    {extra}\n---\n## D\n\n```bash\n<@p>\n```\n"
            );
            assert!(
                compute_code_actions(&uri(), &suggestions, range(2, 3), &BTreeMap::new()).is_none(),
                "{extra}"
            );
        }

        let ambiguous = "---\nvariables:\n  p:\n    default: a\n    command: b\n---\n## D\n\n```bash\n<@p>\n```\n";
        assert!(compute_code_actions(&uri(), ambiguous, range(2, 3), &BTreeMap::new()).is_none());

        let mut config = BTreeMap::new();
        config.insert("p".to_string(), VariableSpec::default());
        let default = "---\nvariables:\n  p:\n    default: a\n---\n## D\n\n```bash\n<@p>\n```\n";
        assert!(compute_code_actions(&uri(), default, range(2, 3), &config).is_none());

        let sourced =
            "---\nvariables:\n  p:\n    default: a\n---\n## D\n\n```bash\n<@p:already>\n```\n";
        assert!(compute_code_actions(&uri(), sourced, range(2, 3), &BTreeMap::new()).is_none());
    }

    #[test]
    fn no_action_on_bad_or_unrelated_positions() {
        assert!(
            compute_code_actions(
                &uri(),
                "---\nname: T\n---\n## D\n\n```bash\n<@>\n```\n",
                range(6, 2),
                &BTreeMap::new()
            )
            .is_none()
        );
        assert!(
            compute_code_actions(
                &uri(),
                "---\nname: T\n---\n## D\n\n```bash\nplain text\n```\n",
                range(6, 2),
                &BTreeMap::new()
            )
            .is_none()
        );
        assert!(
            compute_code_actions(
                &uri(),
                "---\nname: T\n---\n## D\n\n```bash\n<@p>\n```\n",
                range(1, 1),
                &BTreeMap::new()
            )
            .is_none()
        );
    }

    #[test]
    fn selection_start_inside_placeholder_is_eligible_and_outside_is_not() {
        let content = "## D\n\n```bash\n<@p:?.>\n```\n";
        let eligible = Range {
            start: pos(3, 1),
            end: pos(3, 7),
        };
        assert!(compute_code_actions(&uri(), content, eligible, &BTreeMap::new()).is_some());

        let with_frontmatter = "---\nname: T\n---\n## D\n\n```bash\n<@p:?.>\n```\n";
        assert!(
            compute_code_actions(&uri(), with_frontmatter, range(6, 8), &BTreeMap::new()).is_none()
        );
    }

    #[test]
    fn extract_uses_utf16_ranges_after_unicode_text() {
        let content = "## D\n\n```bash\né <@p:?.>\n```\n";
        let actions = compute_code_actions(&uri(), content, range(3, 4), &BTreeMap::new()).unwrap();
        let replacement = edits(&actions[0])
            .iter()
            .find(|edit| edit.new_text == "<@p>")
            .unwrap();

        assert_eq!(replacement.range.start, pos(3, 2));
        assert_eq!(replacement.range.end, pos(3, 9));
        let updated = apply_edits(content, edits(&actions[0]));
        assert!(updated.contains("é <@p>"), "{updated}");
    }

    #[test]
    fn extract_then_inline_preserves_body_bytes() {
        let body = "## D\n\n```bash\necho <@p:?<#a:raw>.out>\n```\n";
        let extracted = compute_code_actions(&uri(), body, range(3, 8), &BTreeMap::new()).unwrap();
        let with_frontmatter = apply_edits(body, edits(&extracted[0]));
        let inlined =
            compute_code_actions(&uri(), &with_frontmatter, range(2, 3), &BTreeMap::new()).unwrap();
        let round_tripped = apply_edits(&with_frontmatter, edits(&inlined[0]));
        let body_after = round_tripped.split("---\n").last().unwrap();
        assert_eq!(body_after, body);
    }

    fn frontmatter_suggestions(content: &str, name: &str) -> Vec<String> {
        parser::parse_file(std::path::Path::new(""), std::path::Path::new(""), content)
            .frontmatter
            .variables
            .get(name)
            .unwrap_or_else(|| panic!("no frontmatter spec for {name} in {content}"))
            .suggestions
            .clone()
    }

    #[test]
    fn inline_placeholder_at_classifies_suggestion_lists() {
        let list = inline_placeholder_at("## D\n\n```bash\n<@env:[dev, stage]>\n```\n", pos(3, 3))
            .unwrap();
        assert_eq!(list.kind, InlineSourceKind::Suggestions);
        assert_eq!(list.value, "[dev, stage]");

        // An unterminated list is still an inline command.
        let command =
            inline_placeholder_at("## D\n\n```bash\n<@env:[dev,stage>\n```\n", pos(3, 3)).unwrap();
        assert_eq!(command.kind, InlineSourceKind::Command);
    }

    #[test]
    fn extract_suggestion_list_writes_block_form_frontmatter() {
        let content = "## D\n\n```bash\ndeploy <@env:[dev, a\\,b, c\\], \\<#x>]>\n```\n";
        let actions = compute_code_actions(&uri(), content, range(3, 8), &BTreeMap::new()).unwrap();
        assert!(action_title(&actions[0]).contains("Extract `<@env>`"));
        let updated = apply_edits(content, edits(&actions[0]));
        assert!(
            updated.starts_with(
                "---\nvariables:\n  env:\n    suggestions:\n      - dev\n      - a,b\n      - c]\n      - \"<#x>\"\n---\n## D"
            ),
            "{updated}"
        );
        assert!(updated.contains("deploy <@env>\n"));
        assert_eq!(
            frontmatter_suggestions(&updated, "env"),
            vec!["dev", "a,b", "c]", "<#x>"]
        );
    }

    #[test]
    fn extract_suggestion_list_collapses_equivalent_lists_only() {
        let content =
            "## D\n\n```bash\n<@env:[dev,stage]> <@env:[ dev , stage ]>\n<@env:[prod]>\n```\n";
        let actions = compute_code_actions(&uri(), content, range(3, 2), &BTreeMap::new()).unwrap();
        assert!(action_title(&actions[0]).contains("(collapses all 2 occurrences)"));
        let updated = apply_edits(content, edits(&actions[0]));
        assert!(
            updated.contains("<@env> <@env>\n<@env:[prod]>"),
            "{updated}"
        );
    }

    #[test]
    fn extract_suggestion_list_matching_spec_only_simplifies_placeholder() {
        let content = "---\nvariables:\n  env:\n    suggestions: [dev, stage]\n---\n## D\n\n```bash\n<@env:[dev,stage]>\n```\n";
        let actions = compute_code_actions(&uri(), content, range(8, 2), &BTreeMap::new()).unwrap();
        let action_edits = edits(&actions[0]);
        assert_eq!(action_edits.len(), 1);
        assert_eq!(action_edits[0].new_text, "<@env>");
    }

    #[test]
    fn extract_suggestion_list_overwrites_conflicting_sources() {
        let content = "---\nvariables:\n  env:\n    default: local\n    command: ls envs\n    suggestions:\n      - old\n---\n## D\n\n```bash\n<@env:[dev,stage]>\n```\n";
        let actions =
            compute_code_actions(&uri(), content, range(11, 2), &BTreeMap::new()).unwrap();
        assert!(action_title(&actions[0]).contains("overwrite"));
        let updated = apply_edits(content, edits(&actions[0]));
        assert!(
            updated.starts_with(
                "---\nvariables:\n  env:\n    suggestions:\n      - dev\n      - stage\n---\n"
            ),
            "{updated}"
        );
    }

    #[test]
    fn inline_suggestions_spec_round_trips_through_inline_list() {
        let content = "---\nvariables:\n  env:\n    suggestions:\n      - dev\n      - a,b\n      - c]\n      - \"<#x>\"\n      - 'back\\slash'\n      - \" padded\"\n---\n## D\n\n```bash\ndeploy <@env> <@env>\n```\n";
        let actions = compute_code_actions(&uri(), content, range(2, 3), &BTreeMap::new()).unwrap();
        assert!(action_title(&actions[0]).contains("Inline frontmatter variable `env`"));
        let updated = apply_edits(content, edits(&actions[0]));
        assert!(
            updated
                .contains(r#"deploy <@env:[dev,"a,b","c]",\<#x>,back\\slash," padded"]> <@env>"#),
            "{updated}"
        );
        assert!(!updated.contains("variables:"), "{updated}");
        let body = updated.split("```bash\n").nth(1).unwrap();
        assert_eq!(
            parser::parse_variables(body)[0].source,
            crate::domain::VariableSource::Suggestions(
                ["dev", "a,b", "c]", "<#x>", r"back\slash", " padded"]
                    .map(String::from)
                    .to_vec()
            )
        );
    }

    #[test]
    fn inline_skips_suggestions_that_cannot_round_trip() {
        // Only `>` is unrepresentable from frontmatter: quoting handles commas,
        // brackets, and surrounding spaces; the frontmatter parser drops empty
        // items and can't produce multi-line ones (`format_suggestion_list`
        // unit tests cover those guards directly).
        for item in ["a>b", "\"quoted > too\""] {
            let content = format!(
                "---\nvariables:\n  env:\n    suggestions:\n      - ok\n      - {item}\n---\n## D\n\n```bash\n<@env>\n```\n"
            );
            assert!(
                compute_code_actions(&uri(), &content, range(2, 3), &BTreeMap::new()).is_none(),
                "{item}"
            );
        }
    }

    #[test]
    fn extract_then_inline_suggestion_list_preserves_body() {
        // The canonical inline form quotes items containing `,` or `]`.
        let body = "## D\n\n```bash\ndeploy <@env:[dev,\"a,b\",\\<#x>]>\n```\n";
        let extracted = compute_code_actions(&uri(), body, range(3, 8), &BTreeMap::new()).unwrap();
        let with_frontmatter = apply_edits(body, edits(&extracted[0]));
        let inlined =
            compute_code_actions(&uri(), &with_frontmatter, range(2, 3), &BTreeMap::new()).unwrap();
        let round_tripped = apply_edits(&with_frontmatter, edits(&inlined[0]));
        let body_after = round_tripped.split("---\n").last().unwrap();
        assert_eq!(body_after, body);
    }

    #[test]
    fn extract_suggestion_list_declines_when_config_default_would_apply() {
        // An inline list ignores config `default`, but a frontmatter spec only
        // overlays config field by field, so `<@x>` would start pre-filling it.
        let content = "## D\n\n```bash\necho <@x:[a,b]>\n```\n";
        let refactors = |config: &BTreeMap<String, VariableSpec>| {
            compute_code_actions_filtered(
                &uri(),
                content,
                range(3, 7),
                config,
                Some(&[CodeActionKind::REFACTOR]),
            )
        };
        let mut config = BTreeMap::new();
        config.insert(
            "x".to_string(),
            VariableSpec {
                default: Some("fallback".to_string()),
                ..Default::default()
            },
        );
        assert!(refactors(&config).is_none());

        // Config sources the extracted spec still shadows don't block it.
        config.insert(
            "x".to_string(),
            VariableSpec {
                suggestions: vec!["z".to_string()],
                default_value: Some("ghost".to_string()),
                ..Default::default()
            },
        );
        let actions = refactors(&config).unwrap();
        assert!(action_title(&actions[0]).contains("Extract `<@x>`"));
    }

    #[test]
    fn extract_declines_empty_suggestion_list() {
        // `suggestions: []` would let config or builtin suggestions through.
        let content = "## D\n\n```bash\necho <@file:[]>\n```\n";
        assert!(
            compute_code_actions_filtered(
                &uri(),
                content,
                range(3, 7),
                &BTreeMap::new(),
                Some(&[CodeActionKind::REFACTOR]),
            )
            .is_none()
        );
    }

    #[test]
    fn extract_command_declines_when_config_default_would_apply() {
        // Same exposure as lists: an inline command ignores config `default`,
        // but the extracted `<@x>` would start pre-filling it.
        let content = "## D\n\n```bash\necho <@x:git branch>\n```\n";
        let mut config = BTreeMap::new();
        config.insert(
            "x".to_string(),
            VariableSpec {
                default: Some("fallback".to_string()),
                ..Default::default()
            },
        );
        assert!(
            compute_code_actions_filtered(
                &uri(),
                content,
                range(3, 7),
                &config,
                Some(&[CodeActionKind::REFACTOR]),
            )
            .is_none()
        );
    }
}
