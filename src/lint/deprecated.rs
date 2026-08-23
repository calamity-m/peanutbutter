//! Deprecation diagnostics for syntax retained only for compatibility.

use std::collections::BTreeMap;
use std::path::Path;

use crate::config::VariableInputConfig;
use crate::domain::VariableSource;
use crate::parser;

use super::{CODE_DEPRECATED_HINT, FileContext, LintFinding, LintSeverity, finding, frontmatter};

const HINT_MIGRATION: &str = "use :?value or default_value when Enter should accept the displayed value; otherwise use a plain free-form placeholder";

/// Warn about deprecated hints in file-local specs and inline placeholders.
pub(super) fn lint_file_hints(file: &FileContext) -> Vec<LintFinding> {
    let mut out = Vec::new();
    let variable_lines = frontmatter::frontmatter_variable_lines(&file.content);

    for (name, spec) in &file.parsed.frontmatter.variables {
        if spec.hint.is_some() {
            let declaration_line = variable_lines.get(name).copied();
            out.push(deprecated_hint(
                file.path.clone(),
                declaration_line.and_then(|line| frontmatter_hint_line(&file.content, line)),
                None,
                format!("frontmatter hint for variable '{name}' is deprecated"),
            ));
        }
    }

    let ranges = parser::snippet_line_ranges(&file.parsed.relative_path, &file.content);
    for snippet in &file.parsed.snippets {
        let Some(range) = ranges.iter().find(|range| range.id == snippet.id) else {
            continue;
        };
        let body_line_count = if snippet.body.is_empty() {
            0
        } else {
            snippet.body.split('\n').count()
        };
        let body_start_line = range.end_line.saturating_sub(body_line_count + 1);
        for (name, line, col_start, col_end) in inline_hint_spans(&snippet.body, body_start_line) {
            out.push(
                deprecated_hint(
                    file.path.clone(),
                    Some(line),
                    Some(snippet.id.clone()),
                    format!("inline hint for variable '{name}' is deprecated"),
                )
                .with_span(line, col_start, col_end),
            );
        }
    }

    out
}

fn frontmatter_hint_line(content: &str, declaration_line: usize) -> Option<usize> {
    let lines: Vec<&str> = content.lines().collect();
    let declaration_idx = declaration_line.checked_sub(1)?;
    let declaration = *lines.get(declaration_idx)?;
    let declaration_indent = declaration.len() - declaration.trim_start().len();

    for (idx, line) in lines.iter().enumerate().skip(declaration_idx + 1) {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        if trimmed == "---" || (!trimmed.is_empty() && indent <= declaration_indent) {
            break;
        }
        if trimmed
            .split_once(':')
            .is_some_and(|(key, value)| key.trim() == "hint" && !value.trim().is_empty())
        {
            return Some(idx + 1);
        }
    }
    Some(declaration_line)
}

fn inline_hint_spans(body: &str, body_start_line: usize) -> Vec<(String, usize, usize, usize)> {
    let mut out = Vec::new();
    let bytes = body.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] != b'<' || bytes[i + 1] != b'@' {
            i += 1;
            continue;
        }
        let Some(end) = parser::find_placeholder_end(body, i + 2) else {
            break;
        };
        let raw = &body[i..=end];
        let parsed = parser::parse_variables(raw);
        if let [variable] = parsed.as_slice()
            && matches!(variable.source, VariableSource::Hint(_))
        {
            let prefix = &body[..i];
            let line_offset = prefix.bytes().filter(|byte| *byte == b'\n').count();
            let line_start = prefix.rfind('\n').map_or(0, |newline| newline + 1);
            let first_line_end = body[i..=end]
                .find('\n')
                .map_or(end + 1, |newline| i + newline);
            out.push((
                variable.name.clone(),
                body_start_line + line_offset + 1,
                i - line_start,
                first_line_end - line_start,
            ));
        }
        i = end + 1;
    }
    out
}

/// Warn once for every config variable that still defines a hint.
pub(super) fn lint_config_hints(
    globals: &BTreeMap<String, VariableInputConfig>,
    config_file: &Path,
) -> Vec<LintFinding> {
    globals
        .iter()
        .filter(|(_, spec)| spec.hint.is_some())
        .map(|(name, _)| {
            deprecated_hint(
                config_file.to_path_buf(),
                None,
                None,
                format!("config hint for variable '{name}' is deprecated"),
            )
        })
        .collect()
}

fn deprecated_hint(
    path: std::path::PathBuf,
    line: Option<usize>,
    snippet_id: Option<crate::domain::SnippetId>,
    message: String,
) -> LintFinding {
    finding(
        LintSeverity::Warning,
        CODE_DEPRECATED_HINT,
        path,
        line,
        snippet_id,
        message,
        Some(HINT_MIGRATION.to_string()),
    )
}
