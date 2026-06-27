use crate::domain::VariableSpec;
use crate::parser;
use std::collections::{BTreeMap, HashMap};
use tower_lsp::lsp_types::*;

use super::{find_variable_declaration_line, frontmatter_end_line, line_range};

// ---------------------------------------------------------------------------
// Code actions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InlineSourceKind {
    Default,
    Command,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InlinePlaceholder {
    name: String,
    kind: InlineSourceKind,
    value: String,
    line: u32,
    start: usize,
    end: usize,
}

pub(super) fn compute_code_actions(
    uri: &Url,
    content: &str,
    range: Range,
    config_vars: &BTreeMap<String, VariableSpec>,
) -> Option<CodeActionResponse> {
    let pos = range.start;
    let mut actions = Vec::new();
    if let Some(inline) = inline_placeholder_at(content, pos) {
        actions.push(extract_code_action(uri, content, &inline, config_vars));
    }
    if let Some(action) = inline_code_action(uri, content, pos, config_vars) {
        actions.push(action);
    }
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
        .map(|placeholder| TextEdit {
            range: line_range(
                placeholder.line,
                placeholder.start as u32,
                placeholder.end as u32,
            ),
            new_text: format!("<@{}>", placeholder.name),
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
    if !spec.suggestions.is_empty() {
        return None;
    }
    let (kind, value) = match (&spec.default, &spec.command) {
        (Some(default), None) => (InlineSourceKind::Default, default.as_str()),
        (None, Some(command)) => (InlineSourceKind::Command, command.as_str()),
        _ => return None,
    };
    let target = first_placeholder(content, &name)?;
    if target.has_source {
        return None;
    }
    let inline_text = match kind {
        InlineSourceKind::Default => format!("<@{}:?{}>", name, value),
        InlineSourceKind::Command => format!("<@{}:{}>", name, value),
    };
    let mut title = format!("Inline frontmatter variable `{name}`");
    let usages = placeholder_usage_count(content, &name);
    if usages > 1 {
        title.push_str(&format!(" (affects all {usages} usages)"));
    }
    Some(CodeAction {
        title,
        kind: Some(CodeActionKind::REFACTOR_INLINE),
        edit: Some(workspace_edit(
            uri,
            vec![
                TextEdit {
                    range: line_range(target.line, target.start as u32, target.end as u32),
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
    let char_idx = pos.character as usize;
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
                let (kind, value) = if let Some(default) = rest.strip_prefix('?') {
                    (InlineSourceKind::Default, default)
                } else {
                    (InlineSourceKind::Command, rest)
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
                    let (kind, value) = if let Some(default) = rest.strip_prefix('?') {
                        (InlineSourceKind::Default, default)
                    } else {
                        (InlineSourceKind::Command, rest)
                    };
                    if valid_variable_name(name)
                        && name == target.name
                        && kind == target.kind
                        && value == target.value
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
            spec.default.as_deref() == Some(inline.value.as_str()) && spec.command.is_none()
        }
        InlineSourceKind::Command => {
            spec.command.as_deref() == Some(inline.value.as_str()) && spec.default.is_none()
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
    let key = inline_source_key(inline.kind);
    format!(
        "  {}:\n    {}: {}\n",
        inline.name,
        key,
        yaml_scalar(&inline.value)
    )
}

fn variable_spec_text_preserving(
    lines: &[&str],
    decl_line: usize,
    end_line: usize,
    inline: &InlinePlaceholder,
) -> String {
    let source_key = inline_source_key(inline.kind);
    let opposite_key = match inline.kind {
        InlineSourceKind::Default => "command",
        InlineSourceKind::Command => "default",
    };
    let mut out = format!(
        "{}\n    {}: {}\n",
        lines[decl_line],
        source_key,
        yaml_scalar(&inline.value)
    );
    let mut skip_suggestions_list = false;
    for line in &lines[decl_line + 1..end_line] {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        if skip_suggestions_list {
            if indent > 4 && trimmed.starts_with('-') {
                out.push_str(line);
                out.push('\n');
                continue;
            }
            skip_suggestions_list = false;
        }
        let key = trimmed.split_once(':').map(|(key, _)| key.trim());
        if indent == 4 && (key == Some(source_key) || key == Some(opposite_key)) {
            continue;
        }
        if indent == 4 && key == Some("suggestions") && trimmed.ends_with(':') {
            skip_suggestions_list = true;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn inline_source_key(kind: InlineSourceKind) -> &'static str {
    match kind {
        InlineSourceKind::Default => "default",
        InlineSourceKind::Command => "command",
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

    fn edits(action: &CodeActionOrCommand) -> &[TextEdit] {
        let CodeActionOrCommand::CodeAction(action) = action else {
            panic!("expected code action");
        };
        let changes = action.edit.as_ref().unwrap().changes.as_ref().unwrap();
        changes.get(&uri()).unwrap()
    }

    fn action_title(action: &CodeActionOrCommand) -> &str {
        let CodeActionOrCommand::CodeAction(action) = action else {
            panic!("expected code action");
        };
        &action.title
    }

    fn apply_edits(content: &str, edits: &[TextEdit]) -> String {
        let mut line_offsets = Vec::new();
        let mut offset = 0;
        for line in content.split_inclusive('\n') {
            line_offsets.push(offset);
            offset += line.len();
        }
        if line_offsets.is_empty() || !content.ends_with('\n') {
            line_offsets.push(offset);
        }
        let to_offset =
            |p: Position| -> usize { line_offsets[p.line as usize] + p.character as usize };
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
        assert!(updated.contains("variables:\n  path:\n    default: \"<#a:raw>.out # keep\"\n---"));
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
    fn extract_matching_spec_only_simplifies_placeholder() {
        let content = "---\nvariables:\n  p:\n    default: .\n---\n## D\n\n```bash\n<@p:?.>\n```\n";
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
        assert!(updated.contains("variables:\n  p:\n    default: .\n"));
    }

    #[test]
    fn extract_conflict_offers_overwrite_action() {
        let content =
            "---\nvariables:\n  p:\n    default: old\n---\n## D\n\n```bash\n<@p:?new>\n```\n";
        let actions = compute_code_actions(&uri(), content, range(8, 2), &BTreeMap::new()).unwrap();
        assert_eq!(actions.len(), 1);
        assert!(action_title(&actions[0]).contains("overwrite"));
        let updated = apply_edits(content, edits(&actions[0]));
        assert!(updated.contains("  p:\n    default: new\n"));
        assert!(!updated.contains("default: old"));
    }

    #[test]
    fn inline_default_rewrites_first_plain_usage_and_removes_entry() {
        let content =
            "---\nvariables:\n  p:\n    default: .\n---\n## D\n\n```bash\n<@p> <@p>\n```\n";
        let actions = compute_code_actions(&uri(), content, range(2, 3), &BTreeMap::new()).unwrap();
        assert!(action_title(&actions[0]).contains("(affects all 2 usages)"));
        let updated = apply_edits(content, edits(&actions[0]));
        assert!(updated.contains("<@p:?.> <@p>"));
        assert!(!updated.contains("variables:"));
    }

    #[test]
    fn inline_skips_suggestions_ambiguous_config_and_inline_target() {
        let suggestions =
            "---\nvariables:\n  p:\n    suggestions: [a]\n---\n## D\n\n```bash\n<@p>\n```\n";
        assert!(compute_code_actions(&uri(), suggestions, range(2, 3), &BTreeMap::new()).is_none());

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
                "## D\n\n```bash\n<@>\n```\n",
                range(3, 2),
                &BTreeMap::new()
            )
            .is_none()
        );
        assert!(
            compute_code_actions(
                &uri(),
                "## D\n\n```bash\nplain text\n```\n",
                range(3, 2),
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
        assert!(compute_code_actions(&uri(), content, range(3, 8), &BTreeMap::new()).is_none());
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
}
