use crate::domain::VariableSpec;
use crate::parser;
use std::collections::BTreeMap;
use tower_lsp::lsp_types::*;

use super::completions::FRONTMATTER_KEYS;
use super::{dependent_ref_at, frontmatter_end_line, line_range, placeholder_at};

// ---------------------------------------------------------------------------
// Hover
// ---------------------------------------------------------------------------

pub(super) fn compute_hover(
    content: &str,
    pos: Position,
    config_vars: &BTreeMap<String, VariableSpec>,
) -> Option<Hover> {
    let lines: Vec<&str> = content.lines().collect();
    let line_idx = pos.line as usize;
    let char_idx = pos.character as usize;
    let current_line = lines.get(line_idx).copied()?;

    let fm_end = frontmatter_end_line(&lines);
    let in_frontmatter = fm_end
        .map(|end| line_idx > 0 && line_idx < end)
        .unwrap_or(false);

    if in_frontmatter {
        return hover_frontmatter_key(current_line, pos);
    }

    // Prefer `<#name>` (inner) over `<@name>` (potentially enclosing).
    if let Some(h) = hover_dependent_ref(content, current_line, char_idx, pos, config_vars) {
        return Some(h);
    }
    hover_variable_placeholder(content, current_line, char_idx, pos, config_vars)
}

fn hover_dependent_ref(
    content: &str,
    line: &str,
    char_idx: usize,
    pos: Position,
    config_vars: &BTreeMap<String, VariableSpec>,
) -> Option<Hover> {
    let (name, start, end, raw) = dependent_ref_at(line, char_idx)?;
    let parsed = parser::parse_file(std::path::Path::new(""), std::path::Path::new(""), content);
    let mut md = if raw {
        format!("**`<#{name}:raw>`** — dependent reference (raw splice, **not quoted**)\n\n")
    } else {
        format!("**`<#{name}>`** — dependent reference (shell-quoted)\n\n")
    };
    let spec = parsed
        .frontmatter
        .variables
        .get(&name)
        .or_else(|| config_vars.get(&name));
    if let Some(spec) = spec {
        if let Some(d) = &spec.default {
            md.push_str(&format!("- **default** (editable pre-fill): `{d}`\n"));
        }
        if let Some(d) = &spec.default_value {
            md.push_str(&format!("- **default_value** (accepted ghost): `{d}`\n"));
        }
        if !spec.suggestions.is_empty() {
            md.push_str(&format!(
                "- **suggestions**: {}\n",
                spec.suggestions.join(", ")
            ));
        }
        if let Some(cmd) = &spec.command {
            md.push_str(&format!("- **command**: `{cmd}`\n"));
        }
        if let Some(hint) = &spec.hint {
            md.push_str(&format!("- **hint**: `{hint}`\n"));
        }
    } else {
        md.push_str("_no variable spec; declared inline_\n");
    }
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: md,
        }),
        range: Some(line_range(pos.line, start as u32, end as u32)),
    })
}

fn hover_frontmatter_key(line: &str, pos: Position) -> Option<Hover> {
    let key = line.trim_start().split(':').next()?.trim();
    let doc = FRONTMATTER_KEYS
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, doc)| *doc)?;
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: format!("**`{key}`** — {doc}"),
        }),
        range: Some(line_range(pos.line, 0, key.len() as u32)),
    })
}

fn hover_variable_placeholder(
    content: &str,
    line: &str,
    char_idx: usize,
    pos: Position,
    config_vars: &BTreeMap<String, VariableSpec>,
) -> Option<Hover> {
    let (name, start, end) = placeholder_at(line, char_idx)?;
    let parsed = parser::parse_file(std::path::Path::new(""), std::path::Path::new(""), content);
    let spec = parsed
        .frontmatter
        .variables
        .get(&name)
        .or_else(|| config_vars.get(&name))?;
    let mut md = format!("**`<@{name}>`**\n\n");
    if let Some(d) = &spec.default {
        md.push_str(&format!("- **default** (editable pre-fill): `{d}`\n"));
    }
    if let Some(d) = &spec.default_value {
        md.push_str(&format!("- **default_value** (accepted ghost): `{d}`\n"));
    }
    if !spec.suggestions.is_empty() {
        md.push_str(&format!(
            "- **suggestions**: {}\n",
            spec.suggestions.join(", ")
        ));
    }
    if let Some(cmd) = &spec.command {
        md.push_str(&format!("- **command**: `{cmd}`\n"));
    }
    if let Some(hint) = &spec.hint {
        md.push_str(&format!("- **hint**: `{hint}`\n"));
    }
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: md,
        }),
        range: Some(line_range(pos.line, start as u32, end as u32)),
    })
}
