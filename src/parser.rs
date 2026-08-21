use crate::domain::{SnippetFile, SnippetId};
use std::path::Path;

mod frontmatter;
mod ranges;
mod snippets;
mod variables;

use frontmatter::parse_frontmatter;
use ranges::{parse_snippet_line_ranges, section_end};
use snippets::parse_snippets;

pub(crate) use variables::find_placeholder_end;
pub use variables::parse_variables;

#[cfg(test)]
use crate::domain::{Frontmatter, VariableSource};
pub(crate) use snippets::slugify;

#[cfg(test)]
mod tests;

/// Half-open line range `[start_line, end_line)` over `content.lines()`,
/// identifying where a single snippet (heading + fenced code block) sits in
/// its source file. Indices are 0-based; `end_line == lines.len()` is valid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnippetLineRange {
    pub id: SnippetId,
    pub start_line: usize,
    pub end_line: usize,
}

/// Parse a Markdown file into a [`SnippetFile`].
///
/// `root` is used to compute the relative path stored on the returned value;
/// if `absolute_path` is not under `root` the full path is kept as-is.
pub fn parse_file(absolute_path: &Path, root: &Path, content: &str) -> SnippetFile {
    let relative_path = absolute_path
        .strip_prefix(root)
        .unwrap_or(absolute_path)
        .to_path_buf();
    let lines: Vec<&str> = content.lines().collect();
    let (frontmatter, body_start) = parse_frontmatter(&lines);
    let snippets = parse_snippets(&lines[body_start..], &relative_path);
    SnippetFile {
        path: absolute_path.to_path_buf(),
        relative_path,
        frontmatter,
        snippets,
    }
}

/// Return the source line ranges of every snippet in `content`, identified by
/// their [`SnippetId`].
pub fn snippet_line_ranges(relative_path: &Path, content: &str) -> Vec<SnippetLineRange> {
    let lines: Vec<&str> = content.lines().collect();
    let (_, body_start) = parse_frontmatter(&lines);
    parse_snippet_line_ranges(&lines[body_start..], relative_path, body_start)
}

/// Return the line range covering a snippet's entire `##` section: the heading
/// through the last line before the next level-one or level-two ATX heading (or
/// end of file), rather than stopping at the body's closing fence.
///
/// Returns `None` when `id` is not a parseable snippet in `content` — notably
/// when its code fence is unterminated, which leaves the parser with no range
/// at all. Callers that rewrite the file must treat that as "refuse", not as
/// "delete nothing".
pub fn snippet_section_range(
    relative_path: &Path,
    content: &str,
    id: &SnippetId,
) -> Option<SnippetLineRange> {
    let lines: Vec<&str> = content.lines().collect();
    let (_, body_start) = parse_frontmatter(&lines);
    let range = parse_snippet_line_ranges(&lines[body_start..], relative_path, body_start)
        .into_iter()
        .find(|range| range.id == *id)?;
    Some(SnippetLineRange {
        end_line: section_end(&lines, range.end_line),
        ..range
    })
}
