//! Markdown structure lint rules — duplicate slugs, unclosed fences, sections
//! without executable bodies, and missing code-fence language tags.

use std::collections::HashMap;
use std::path::Path;

use crate::parser;

use super::{
    CODE_DUPLICATE_SLUG, CODE_MARKDOWN_STRUCTURE, CODE_MISSING_CODE_LANGUAGE,
    CODE_TEXT_ONLY_SECTION, FileContext, LintFinding, LintSeverity, finding,
};

/// Warn when two executable snippets in the same file slugify to the same base slug.
pub(super) fn lint_duplicate_slugs(file: &FileContext) -> Vec<LintFinding> {
    let ranges = parser::snippet_line_ranges(&file.parsed.relative_path, &file.content);
    debug_assert_eq!(file.parsed.snippets.len(), ranges.len());
    let lines: Vec<_> = file.content.lines().collect();
    let mut first: HashMap<String, usize> = HashMap::new();
    let mut out = Vec::new();

    for (snippet, range) in file.parsed.snippets.iter().zip(ranges) {
        let line_idx = range.start_line;
        let Some(line) = lines.get(line_idx) else {
            continue;
        };
        let slug = parser::slugify(&snippet.name);
        if let Some(first_line) = first.get(&slug) {
            // Byte offsets: skip leading whitespace and the "##" prefix, then
            // any spaces before the heading text, so editors underline just
            // the heading rather than the whole line.
            let leading = line.len() - line.trim_start().len();
            let after_hashes = leading + 2;
            let rest = &line[after_hashes..];
            let col_start = after_hashes + (rest.len() - rest.trim_start().len());
            let col_end = col_start + snippet.name.len();
            out.push(
                finding(
                    LintSeverity::Warning,
                    CODE_DUPLICATE_SLUG,
                    file.path.clone(),
                    None,
                    None,
                    format!("snippet heading duplicates base slug '{slug}'"),
                    Some(format!("first occurrence is on line {first_line}")),
                )
                .with_span(line_idx + 1, col_start, col_end),
            );
        } else {
            first.insert(slug, line_idx + 1);
        }
    }
    out
}

/// Check for unclosed fences and snippet sections missing an executable code fence.
/// Strict-mode only.
pub(super) fn lint_markdown_structure(path: &Path, content: &str) -> Vec<LintFinding> {
    let mut out = Vec::new();
    // `in_fence` carries `(fence, line_no, is_text)` for the currently-open fence.
    let mut in_fence: Option<(String, usize, bool)> = None;
    // `open_heading` carries `(heading, line_no, has_executable, has_text)`.
    let mut open_heading: Option<(String, usize, bool, bool)> = None;
    for (idx, line) in content.lines().enumerate() {
        let line_no = idx + 1;
        if let Some((fence, _, is_text)) = &in_fence {
            if is_fence_close(line, fence) {
                let was_text = *is_text;
                in_fence = None;
                if let Some((_, _, has_executable, has_text)) = &mut open_heading {
                    if was_text {
                        *has_text = true;
                    } else {
                        *has_executable = true;
                    }
                }
            }
            continue;
        }
        if let Some(heading) = snippet_heading(line) {
            if let Some(previous) = open_heading.replace((heading, line_no, false, false)) {
                emit_section_structure_finding(&mut out, path, previous);
            }
        } else if let Some((fence, language)) = fence_open(line) {
            let is_text = is_ignored_body_language(language.as_deref());
            in_fence = Some((fence, line_no, is_text));
        }
    }
    if let Some((_, start, _)) = in_fence {
        out.push(finding(
            LintSeverity::Warning,
            CODE_MARKDOWN_STRUCTURE,
            path.to_path_buf(),
            Some(start),
            None,
            "code fence is not closed".to_string(),
            None,
        ));
    }
    if let Some(previous) = open_heading {
        emit_section_structure_finding(&mut out, path, previous);
    }
    out
}

/// Warn for every snippet whose code fence has no language tag. Strict-mode only.
pub(super) fn lint_missing_code_languages(file: &FileContext) -> Vec<LintFinding> {
    let ranges = parser::snippet_line_ranges(&file.parsed.relative_path, &file.content);
    let mut line_by_id = HashMap::new();
    for range in ranges {
        line_by_id.insert(range.id, range.start_line + 1);
    }
    file.parsed
        .snippets
        .iter()
        .filter(|snippet| snippet.language.is_none())
        .map(|snippet| {
            finding(
                LintSeverity::Warning,
                CODE_MISSING_CODE_LANGUAGE,
                file.path.clone(),
                line_by_id.get(&snippet.id).copied(),
                Some(snippet.id.clone()),
                "snippet code fence has no language tag".to_string(),
                None,
            )
        })
        .collect()
}

fn emit_section_structure_finding(
    out: &mut Vec<LintFinding>,
    path: &Path,
    section: (String, usize, bool, bool),
) {
    let (heading, line, has_executable, has_text) = section;
    if has_executable {
        return;
    }
    if has_text {
        out.push(finding(
            LintSeverity::Warning,
            CODE_TEXT_ONLY_SECTION,
            path.to_path_buf(),
            Some(line),
            None,
            format!(
                "snippet section '{heading}' has only `text` fences; \
                 `text` is reserved for preview examples and is not executable"
            ),
            None,
        ));
    } else {
        out.push(finding(
            LintSeverity::Warning,
            CODE_MARKDOWN_STRUCTURE,
            path.to_path_buf(),
            Some(line),
            None,
            format!("snippet section '{heading}' has no code fence"),
            None,
        ));
    }
}

fn snippet_heading(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix("##")?;
    if rest.starts_with('#') {
        return None;
    }
    let text = rest.trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn fence_open(line: &str) -> Option<(String, Option<String>)> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with("```") {
        return None;
    }
    let ticks: String = trimmed.chars().take_while(|c| *c == '`').collect();
    if ticks.len() < 3 {
        return None;
    }
    let lang = trimmed[ticks.len()..].trim();
    let language = (!lang.is_empty()).then(|| lang.to_string());
    Some((ticks, language))
}

fn is_fence_close(line: &str, fence: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with(fence) && trimmed.chars().all(|c| c == '`') && trimmed.len() >= fence.len()
}

fn is_ignored_body_language(language: Option<&str>) -> bool {
    language.is_some_and(|language| language.eq_ignore_ascii_case("text"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn file_context(content: &str) -> FileContext {
        let path = Path::new("test.md");
        FileContext {
            path: path.to_path_buf(),
            content: content.to_string(),
            parsed: parser::parse_file(path, Path::new("."), content),
        }
    }

    #[test]
    fn duplicate_slug_finding_has_column_span() {
        let content =
            "## Deploy App\n\n```bash\necho hi\n```\n\n## Deploy App!\n\n```bash\necho bye\n```\n";
        let findings = lint_duplicate_slugs(&file_context(content));
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        assert_eq!(f.line, Some(7));
        // "## Deploy App!" → heading text starts at byte 3
        assert_eq!(f.col_start, Some(3));
        assert_eq!(f.col_end, Some(3 + "Deploy App!".len()));
    }

    #[test]
    fn duplicate_slug_finding_column_span_with_leading_whitespace() {
        // Headings shouldn't normally have leading whitespace but the span
        // computation should still be correct if they do.
        let content = "## Foo\n\n```bash\necho a\n```\n\n  ## Foo!\n\n```bash\necho b\n```\n";
        let findings = lint_duplicate_slugs(&file_context(content));
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        // "  ## Foo!" → 2 leading spaces + ## + space = col 5
        assert_eq!(f.col_start, Some(5));
        assert_eq!(f.col_end, Some(5 + "Foo!".len()));
    }

    #[test]
    fn duplicate_slug_source_span_does_not_depend_on_unique_snippet_ids() {
        let content = "## Foo\n\n```bash\na\n```\n\n## Foo\n\n```bash\nb\n```\n\n## Foo-1\n\n```bash\nc\n```\n";
        let findings = lint_duplicate_slugs(&file_context(content));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, Some(7));
        assert_eq!(findings[0].col_start, Some(3));
        assert_eq!(findings[0].col_end, Some(6));
    }

    #[test]
    fn duplicate_slug_ignores_non_executable_sections() {
        let content = "## Deploy\n\nNotes only.\n\n## Deploy!\n\n```bash\necho deploy\n```\n";
        assert!(lint_duplicate_slugs(&file_context(content)).is_empty());
    }

    #[test]
    fn duplicate_slug_ignores_text_only_sections() {
        let content =
            "## Deploy\n\n```text\nexample\n```\n\n## Deploy!\n\n```bash\necho deploy\n```\n";
        assert!(lint_duplicate_slugs(&file_context(content)).is_empty());
    }
}
