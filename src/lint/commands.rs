//! Inline source lint rules — static suggestion lists written as commands,
//! malformed inline `[...]` lists, and duplicate commands repeated across
//! multiple snippets in the same file.

use std::collections::HashMap;

use crate::domain::{SnippetId, VariableSource};
use crate::parser::{self, SuggestionListError};

use super::{
    CODE_DUPLICATE_INLINE_COMMAND, CODE_INVALID_SUGGESTION_LIST, CODE_STATIC_INLINE_COMMAND,
    FileContext, LintFinding, LintSeverity, finding, placeholders,
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
                        Some(format!(
                            "use an inline list like <@{name}:[a,b]> for this snippet, or frontmatter variables.{name}.suggestions to share it",
                            name = variable.name
                        )),
                    )
                    .with_suppress_command(command.clone()),
                );
            }
        }
    }
    out
}

/// Report malformed inline `<@name:[...]>` lists (error) and inline commands
/// that look like an unterminated list (warning). The parser stays lenient,
/// so neither stops the snippet from running.
pub(super) fn lint_inline_suggestion_lists(file: &FileContext) -> Vec<LintFinding> {
    let mut out = Vec::new();
    for span in placeholders::placeholder_spans(file) {
        let Some(source) = span.source else {
            continue;
        };
        let name = &span.variable.name;
        let (severity, message, help) = match &span.variable.source {
            VariableSource::Suggestions(_) => {
                let Some(error) = parser::parse_suggestion_list(source).and_then(|list| list.error)
                else {
                    continue;
                };
                (
                    LintSeverity::Error,
                    format!("inline suggestion list for variable '{name}' is invalid: {error}"),
                    suggestion_list_help(name, error),
                )
            }
            VariableSource::Command(command) if looks_like_unterminated_list(command) => (
                LintSeverity::Warning,
                format!(
                    "inline command for variable '{name}' looks like an unterminated suggestion list"
                ),
                format!(
                    "close the list with an unescaped ']' (e.g. <@{name}:[a,b]>); until then it runs as a shell command"
                ),
            ),
            _ => continue,
        };
        out.push(
            finding(
                severity,
                CODE_INVALID_SUGGESTION_LIST,
                file.path.clone(),
                Some(span.line),
                Some(span.snippet_id.clone()),
                message,
                Some(help),
            )
            .with_suppress_command(source.to_string())
            .with_span(span.line, span.col_start, span.col_end),
        );
    }
    out
}

fn suggestion_list_help(name: &str, error: SuggestionListError) -> String {
    match error {
        SuggestionListError::Empty => format!(
            "list at least one value, e.g. <@{name}:[a,b]>, or drop the source for free-form input"
        ),
        SuggestionListError::EmptyItem => {
            "remove the extra comma; for a literal comma, quote the item (\"a,b\") or write a\\,b"
                .to_string()
        }
        SuggestionListError::StrayBracket => {
            "for a literal ']', quote the item (\"a]b\") or write a\\]b".to_string()
        }
        SuggestionListError::UnterminatedQuote => {
            "close the quoted item with '\"', or write \\\" for a literal quote".to_string()
        }
        SuggestionListError::TextAfterQuote => {
            "put the whole item inside the quotes, or separate items with ','".to_string()
        }
        SuggestionListError::ReservedReference => format!(
            "use <@{name}:?<#ref>...> for one derived value or a suggestion command for several; write \\<# for a literal"
        ),
    }
}

/// An inline command that opens with `[` but has no unescaped `]` outside
/// double quotes — most likely `<@name:[a,b>` or `<@name:[a, "b]"` missing its
/// closing bracket. Deliberately narrow so real test commands such as
/// `[ -d .git ] && git branch` or `[ "$x" = y ]` never match.
fn looks_like_unterminated_list(command: &str) -> bool {
    let Some(rest) = command.trim().strip_prefix('[') else {
        return false;
    };
    let bytes = rest.as_bytes();
    let mut in_quotes = false;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => {
                in_quotes = !in_quotes;
                i += 1;
            }
            b']' if !in_quotes => return false,
            _ => i += 1,
        }
    }
    true
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
