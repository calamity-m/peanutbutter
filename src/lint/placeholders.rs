//! Locate inline `<@...>` placeholders in snippet bodies with file positions,
//! so per-placeholder rules can attach precise diagnostic spans.

use crate::domain::{SnippetId, Variable};
use crate::parser;

use super::FileContext;

/// One parsed `<@name[:source]>` placeholder and where it sits in the file.
pub(super) struct PlaceholderSpan<'a> {
    pub snippet_id: &'a SnippetId,
    pub variable: Variable,
    /// Raw, unparsed text after the first `:`, if the placeholder has a source.
    pub source: Option<&'a str>,
    /// 1-based file line of the opening `<@`.
    pub line: usize,
    /// 0-based byte column of the opening `<@` on `line`.
    pub col_start: usize,
    /// Exclusive byte column of the closing `>`, clipped to the end of `line`
    /// for placeholders that span several lines.
    pub col_end: usize,
}

/// Every well-formed placeholder in every snippet body of `file`, in
/// document order. Malformed placeholders the parser skips are skipped here.
pub(super) fn placeholder_spans(file: &FileContext) -> Vec<PlaceholderSpan<'_>> {
    let ranges = parser::snippet_line_ranges(&file.parsed.relative_path, &file.content);
    let mut out = Vec::new();
    for snippet in &file.parsed.snippets {
        let Some(range) = ranges.iter().find(|range| range.id == snippet.id) else {
            continue;
        };
        let body = snippet.body.as_str();
        let body_line_count = if body.is_empty() {
            0
        } else {
            body.split('\n').count()
        };
        // The body ends on the line before the closing fence.
        let body_start_line = range.end_line.saturating_sub(body_line_count + 1);
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
            if let [variable] = parser::parse_variables(raw).as_slice() {
                let prefix = &body[..i];
                let line_offset = prefix.bytes().filter(|byte| *byte == b'\n').count();
                let line_start = prefix.rfind('\n').map_or(0, |newline| newline + 1);
                let first_line_end = raw.find('\n').map_or(end + 1, |newline| i + newline);
                out.push(PlaceholderSpan {
                    snippet_id: &snippet.id,
                    variable: variable.clone(),
                    source: body[i + 2..end].split_once(':').map(|(_, source)| source),
                    line: body_start_line + line_offset + 1,
                    col_start: i - line_start,
                    col_end: first_line_end - line_start,
                });
            }
            i = end + 1;
        }
    }
    out
}
