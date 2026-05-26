use crate::config::AppConfig;
use crate::parser;
use std::path::Path;
use tower_lsp::lsp_types::*;

use super::{
    dependent_ref_at, find_variable_declaration_line, frontmatter_end_line, line_range,
    placeholder_at,
};

// ---------------------------------------------------------------------------
// Go-to-definition
// ---------------------------------------------------------------------------

pub(super) fn compute_definition(
    uri: &Url,
    content: &str,
    pos: Position,
    config: &AppConfig,
) -> Option<GotoDefinitionResponse> {
    let lines: Vec<&str> = content.lines().collect();
    let line_idx = pos.line as usize;
    let char_idx = pos.character as usize;
    let current_line = lines.get(line_idx).copied()?;

    // Cursor may be on either `<@name>` or `<#name>`. Prefer the inner
    // `<#name>` token because `placeholder_at` will match an enclosing
    // `<@key:cmd-with-<#name>>` even when the cursor is on the nested ref.
    let name = dependent_ref_at(current_line, char_idx)
        .map(|(n, ..)| n)
        .or_else(|| placeholder_at(current_line, char_idx).map(|(n, ..)| n))?;

    // Prefer frontmatter declaration in the current file.
    if let Some(def_line) = find_variable_declaration_line(&lines, &name) {
        let target_range = line_range(def_line as u32, 0, lines[def_line].len() as u32);
        return Some(GotoDefinitionResponse::Scalar(Location {
            uri: uri.clone(),
            range: target_range,
        }));
    }

    // Fall back to the config file if the variable is declared there.
    if config.variables.contains_key(&name)
        && let Some(loc) = find_config_variable_location(&config.paths.config_file, &name)
    {
        return Some(GotoDefinitionResponse::Scalar(loc));
    }

    None
}

/// Find the location of `[variables.<name>]` in the config TOML file.
fn find_config_variable_location(config_file: &Path, name: &str) -> Option<Location> {
    let content = std::fs::read_to_string(config_file).ok()?;
    let target = format!("[variables.{name}]");
    let (line_idx, _) = content
        .lines()
        .enumerate()
        .find(|(_, line)| line.trim() == target)?;
    let uri = Url::from_file_path(config_file).ok()?;
    let range = line_range(line_idx as u32, 0, target.len() as u32);
    Some(Location { uri, range })
}

// ---------------------------------------------------------------------------
// Find references
// ---------------------------------------------------------------------------

pub(super) fn compute_references(uri: &Url, content: &str, pos: Position) -> Option<Vec<Location>> {
    let lines: Vec<&str> = content.lines().collect();
    let line_idx = pos.line as usize;
    let current_line = lines.get(line_idx).copied()?;

    // If cursor is on the variable declaration in frontmatter, find all `<@name>` usages.
    let fm_end = frontmatter_end_line(&lines)?;
    let in_frontmatter = line_idx > 0 && line_idx < fm_end;

    let var_name: String;
    if in_frontmatter {
        // Check if this line is a variable declaration line (inside `variables:` block).
        let trimmed = current_line.trim_start();
        var_name = trimmed.split(':').next()?.trim().to_string();
        // Verify it is actually declared in frontmatter variables.
        let parsed =
            parser::parse_file(std::path::Path::new(""), std::path::Path::new(""), content);
        if !parsed.frontmatter.variables.contains_key(&var_name) {
            return None;
        }
    } else {
        // Cursor might be on a `<#name>` ref or a `<@name>` placeholder.
        // Prefer the inner `<#name>` so nested refs inside `<@key:...>` work.
        let char_idx = pos.character as usize;
        var_name = dependent_ref_at(current_line, char_idx)
            .map(|(n, ..)| n)
            .or_else(|| placeholder_at(current_line, char_idx).map(|(n, ..)| n))?;
    }

    let mut locations = Vec::new();
    // Find all `<@var_name>` and `<@var_name:...>` placeholder occurrences.
    let at_pattern = format!("<@{var_name}");
    // Find all `<#var_name>` and `<#var_name:raw>` dependent-ref occurrences.
    let hash_pattern = format!("<#{var_name}");
    for (i, line) in lines.iter().enumerate() {
        for pattern in [&at_pattern, &hash_pattern] {
            let mut search_from = 0;
            while let Some(col) = line[search_from..].find(pattern.as_str()) {
                let abs_col = search_from + col;
                let end_col = abs_col + pattern.len();
                // Boundary check: the next char must be `>` or `:` to avoid
                // matching `<@foo` inside `<@foobar>`.
                let next = line[end_col..].chars().next();
                if matches!(next, Some('>') | Some(':')) {
                    locations.push(Location {
                        uri: uri.clone(),
                        range: line_range(i as u32, abs_col as u32, end_col as u32),
                    });
                }
                search_from = abs_col + 1;
            }
        }
    }
    Some(locations)
}
