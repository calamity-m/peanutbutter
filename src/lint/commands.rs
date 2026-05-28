//! Inline command lint rules — static suggestion lists and duplicate commands
//! repeated across multiple snippets in the same file.

use std::collections::HashMap;

use crate::domain::{SnippetId, VariableSource};

use super::{
    CODE_DUPLICATE_INLINE_COMMAND, CODE_STATIC_INLINE_COMMAND, FileContext, LintFinding,
    LintSeverity, finding,
};

/// Warn when an inline command looks like a static list (`echo …` / `printf …`).
pub(super) fn lint_static_inline_commands(file: &FileContext) -> Vec<LintFinding> {
    let mut out = Vec::new();
    for snippet in &file.parsed.snippets {
        for variable in &snippet.variables {
            let VariableSource::Command(command) = &variable.source else {
                continue;
            };
            if looks_static_command(command) {
                out.push(
                    finding(
                        LintSeverity::Warning,
                        CODE_STATIC_INLINE_COMMAND,
                        file.path.clone(),
                        None,
                        Some(snippet.id.clone()),
                        format!(
                            "inline command for variable '{}' looks like a static suggestion list",
                            variable.name
                        ),
                        Some(
                            "prefer frontmatter variables.<name>.suggestions for static lists"
                                .to_string(),
                        ),
                    )
                    .with_suppress_command(command.clone()),
                );
            }
        }
    }
    out
}

/// Warn when the same (variable name, command) pair appears inline in multiple snippets.
pub(super) fn lint_duplicate_inline_commands(file: &FileContext) -> Vec<LintFinding> {
    // Collect (variable_name, command) -> first snippet id for each inline command.
    let mut seen: HashMap<(String, String), SnippetId> = HashMap::new();
    let mut duplicates: Vec<(String, String, SnippetId)> = Vec::new();
    for snippet in &file.parsed.snippets {
        for variable in &snippet.variables {
            let VariableSource::Command(command) = &variable.source else {
                continue;
            };
            let key = (variable.name.clone(), command.clone());
            match seen.get(&key) {
                None => {
                    seen.insert(key, snippet.id.clone());
                }
                Some(first_id)
                    if !duplicates
                        .iter()
                        .any(|(n, c, _)| n == &key.0 && c == &key.1) =>
                {
                    duplicates.push((variable.name.clone(), command.clone(), first_id.clone()));
                }
                _ => {}
            }
        }
    }
    duplicates
        .into_iter()
        .map(|(name, command, first_id)| {
            finding(
                LintSeverity::Warning,
                CODE_DUPLICATE_INLINE_COMMAND,
                file.path.clone(),
                None,
                Some(first_id),
                format!(
                    "inline command for variable '{name}' is repeated across multiple snippets"
                ),
                Some(format!(
                    "move to frontmatter: variables:\n  {name}:\n    command: {command}"
                )),
            )
        })
        .collect()
}

fn looks_static_command(command: &str) -> bool {
    let trimmed = command.trim();
    trimmed.starts_with("echo ") || trimmed.starts_with("printf ")
}
