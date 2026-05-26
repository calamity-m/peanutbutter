use crate::domain::VariableSpec;
use crate::parser;
use std::collections::BTreeMap;
use tower_lsp::lsp_types::*;

use super::{enclosing_placeholder_owner, frontmatter_end_line, snippet_at_line};

// ---------------------------------------------------------------------------
// Completions
// ---------------------------------------------------------------------------

/// Known top-level frontmatter keys with documentation.
pub(super) const FRONTMATTER_KEYS: &[(&str, &str)] = &[
    ("name", "Human-readable title for this snippet file"),
    (
        "description",
        "Short prose description of the file contents",
    ),
    ("tags", "Searchable tags (e.g. `[git, shell]`)"),
    ("variables", "File-local variable input specifications"),
];

/// Known sub-keys under `variables.<name>:` with documentation.
const VARIABLE_SPEC_KEYS: &[(&str, &str)] = &[
    (
        "default",
        "Pre-populated value shown in the prompt input box",
    ),
    (
        "suggestions",
        "Fixed suggestion values shown in the suggestion list",
    ),
    (
        "command",
        "Shell command whose stdout lines are used as suggestions",
    ),
];

pub(super) fn compute_completions(
    content: &str,
    pos: Position,
    config_vars: &BTreeMap<String, VariableSpec>,
) -> Option<CompletionResponse> {
    let lines: Vec<&str> = content.lines().collect();
    let line_idx = pos.line as usize;
    let char_idx = pos.character as usize;
    let current_line = lines.get(line_idx).copied().unwrap_or("");

    // Detect context
    let fm_end = frontmatter_end_line(&lines);
    let in_frontmatter = fm_end
        .map(|end| line_idx > 0 && line_idx < end)
        .unwrap_or(false);

    if in_frontmatter {
        // Are we inside a `variables:` block (indented)?
        let indent = current_line.len() - current_line.trim_start().len();
        if indent >= 2 {
            // Could be inside a variable spec; check if parent block is `variables:`.
            let in_var_block = lines[..line_idx]
                .iter()
                .rev()
                .any(|l| l.trim() == "variables:");
            if in_var_block {
                // Completing variable spec sub-keys or an inner list item
                let trimmed = current_line.trim_start();
                let prefix = trimmed.split(':').next().unwrap_or("").trim();
                let items = VARIABLE_SPEC_KEYS
                    .iter()
                    .filter(|(k, _)| k.starts_with(prefix))
                    .map(|(k, doc)| completion_item(k, doc, CompletionItemKind::FIELD))
                    .collect();
                return Some(CompletionResponse::Array(items));
            }
        }
        // Top-level frontmatter key completion
        let prefix = current_line
            .trim_start()
            .split(':')
            .next()
            .unwrap_or("")
            .trim();
        let items = FRONTMATTER_KEYS
            .iter()
            .filter(|(k, _)| k.starts_with(prefix))
            .map(|(k, doc)| completion_item(k, doc, CompletionItemKind::FIELD))
            .collect();
        return Some(CompletionResponse::Array(items));
    }

    let before_cursor = &current_line[..char_idx.min(current_line.len())];

    // Offer `<#variable>` dependent-ref completions when user typed `<#`.
    // These are valid inside a suggestion command source (which we cannot
    // easily detect from raw text), so we offer them whenever the cursor
    // follows a `<#` token.
    if let Some(at_pos) = before_cursor.rfind("<#") {
        let var_prefix = &before_cursor[at_pos + 2..];
        let parsed =
            parser::parse_file(std::path::Path::new(""), std::path::Path::new(""), content);
        let mut names: Vec<String> = parsed.frontmatter.variables.keys().cloned().collect();
        // Add config-defined names not already in frontmatter.
        for name in config_vars.keys() {
            if !names.contains(name) {
                names.push(name.clone());
            }
        }
        // Add body placeholder names too, in document order.
        for snippet in &parsed.snippets {
            for v in &snippet.variables {
                if !names.contains(&v.name) {
                    names.push(v.name.clone());
                }
            }
        }
        if let Some(owner) = enclosing_placeholder_owner(before_cursor, at_pos)
            && let Some(snippet) = snippet_at_line(content, line_idx)
        {
            let mut earlier = Vec::new();
            for v in &snippet.variables {
                if v.name == owner {
                    break;
                }
                if !earlier.contains(&v.name) {
                    earlier.push(v.name.clone());
                }
            }
            names.retain(|name| earlier.contains(name));
        }
        let items: Vec<CompletionItem> = names
            .into_iter()
            .filter(|name| name.starts_with(var_prefix))
            .map(|name| {
                let detail = parsed
                    .frontmatter
                    .variables
                    .get(&name)
                    .or_else(|| config_vars.get(&name))
                    .map(variable_spec_summary)
                    .unwrap_or_else(|| "dependent reference".to_string());
                let mut item = completion_item(&name, &detail, CompletionItemKind::VARIABLE);
                item.insert_text = Some(format!("{name}>"));
                item.insert_text_format = Some(InsertTextFormat::PLAIN_TEXT);
                item
            })
            .collect();
        return Some(CompletionResponse::Array(items));
    }

    // Inside a code block — offer `<@variable>` completions when user typed `<@`
    if before_cursor.ends_with("<@") || before_cursor.contains("<@") {
        // Extract the prefix after `<@`
        let at_pos = before_cursor.rfind("<@").map(|i| i + 2).unwrap_or(0);
        let var_prefix = &before_cursor[at_pos..];
        let parsed =
            parser::parse_file(std::path::Path::new(""), std::path::Path::new(""), content);
        // Merge frontmatter variables with config-defined variables; frontmatter takes priority.
        let mut all_vars: BTreeMap<&str, &VariableSpec> =
            config_vars.iter().map(|(k, v)| (k.as_str(), v)).collect();
        for (k, v) in &parsed.frontmatter.variables {
            all_vars.insert(k.as_str(), v);
        }
        let items: Vec<CompletionItem> = all_vars
            .into_iter()
            .filter(|(name, _)| name.starts_with(var_prefix))
            .map(|(name, spec)| {
                let detail = variable_spec_summary(spec);
                let mut item = completion_item(name, &detail, CompletionItemKind::VARIABLE);
                item.insert_text = Some(format!("{name}>"));
                item.insert_text_format = Some(InsertTextFormat::PLAIN_TEXT);
                item
            })
            .collect();
        return Some(CompletionResponse::Array(items));
    }

    None
}

fn completion_item(label: &str, documentation: &str, kind: CompletionItemKind) -> CompletionItem {
    CompletionItem {
        label: label.to_string(),
        kind: Some(kind),
        documentation: Some(Documentation::String(documentation.to_string())),
        ..Default::default()
    }
}

pub(super) fn variable_spec_summary(spec: &crate::domain::VariableSpec) -> String {
    let mut parts = Vec::new();
    if let Some(d) = &spec.default {
        parts.push(format!("default: `{d}`"));
    }
    if !spec.suggestions.is_empty() {
        parts.push(format!("suggestions: {}", spec.suggestions.join(", ")));
    }
    if let Some(cmd) = &spec.command {
        parts.push(format!("command: `{cmd}`"));
    }
    if parts.is_empty() {
        "free-form input".to_string()
    } else {
        parts.join("\n")
    }
}
