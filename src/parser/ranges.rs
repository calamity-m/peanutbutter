use crate::domain::SnippetId;
use std::path::Path;

use super::SnippetLineRange;
use super::snippets::{
    is_fence_close, is_ignored_body_language, next_slug, parse_fence_open, parse_snippet_heading,
};

enum RangeState {
    Scanning,
    InSection {
        heading: String,
        start_line: usize,
    },
    InIgnoredTextFence {
        heading: String,
        start_line: usize,
        fence: String,
    },
    InCode {
        heading: String,
        start_line: usize,
        fence: String,
    },
}
pub(super) fn parse_snippet_line_ranges(
    lines: &[&str],
    relative_path: &Path,
    base_line: usize,
) -> Vec<SnippetLineRange> {
    let mut out = Vec::new();
    let mut state = RangeState::Scanning;
    let mut seen_slugs: std::collections::HashMap<String, usize> = Default::default();

    for (idx, line) in lines.iter().enumerate() {
        let abs_idx = base_line + idx;
        match &mut state {
            RangeState::Scanning => {
                if let Some(heading) = parse_snippet_heading(line) {
                    state = RangeState::InSection {
                        heading,
                        start_line: abs_idx,
                    };
                }
            }
            RangeState::InSection {
                heading,
                start_line,
            } => {
                if let Some(next_heading) = parse_snippet_heading(line) {
                    *heading = next_heading;
                    *start_line = abs_idx;
                } else if let Some((fence, language)) = parse_fence_open(line) {
                    let heading = std::mem::take(heading);
                    let start_line = *start_line;
                    if is_ignored_body_language(language.as_deref()) {
                        state = RangeState::InIgnoredTextFence {
                            heading,
                            start_line,
                            fence,
                        };
                    } else {
                        state = RangeState::InCode {
                            heading,
                            start_line,
                            fence,
                        };
                    }
                }
            }
            RangeState::InIgnoredTextFence {
                heading,
                start_line,
                fence,
            } => {
                if is_fence_close(line, fence) {
                    let heading = std::mem::take(heading);
                    state = RangeState::InSection {
                        heading,
                        start_line: *start_line,
                    };
                }
            }
            RangeState::InCode {
                heading,
                start_line,
                fence,
            } => {
                if is_fence_close(line, fence) {
                    let slug = next_slug(heading, &mut seen_slugs);
                    let relative_display = relative_path.to_string_lossy().replace('\\', "/");
                    out.push(SnippetLineRange {
                        id: SnippetId::new(&relative_display, &slug),
                        start_line: *start_line,
                        end_line: abs_idx + 1,
                    });
                    state = RangeState::Scanning;
                }
            }
        }
    }

    out
}

/// Extend a snippet's range to cover its whole `##` section: from the heading
/// up to the next level-one or level-two ATX heading, or end of file.
///
/// [`parse_snippet_line_ranges`] stops at the body's closing fence, so any
/// prose or extra fences written after the body still belong to the section.
/// Removing a snippet has to take those too, or the file is left with text
/// dangling under the previous section.
///
/// The forward scan tracks fences so an H1 or H2 line inside a later code block
/// is not mistaken for the next heading.
pub(super) fn section_end(lines: &[&str], body_end: usize) -> usize {
    let mut open_fence: Option<String> = None;
    for (offset, line) in lines[body_end..].iter().enumerate() {
        match &open_fence {
            Some(fence) => {
                if is_fence_close(line, fence) {
                    open_fence = None;
                }
            }
            None => {
                if is_section_boundary(line) {
                    return body_end + offset;
                }
                if let Some((fence, _)) = parse_fence_open(line) {
                    open_fence = Some(fence);
                }
            }
        }
    }
    if open_fence.is_some() {
        // An unclosed trailing fence makes fence-aware boundaries unknowable.
        // Prefer leaving uncertain content behind over consuming a later H1/H2
        // section that the user did not choose to delete.
        return lines[body_end..]
            .iter()
            .position(|line| is_section_boundary(line))
            .map_or(lines.len(), |offset| body_end + offset);
    }
    lines.len()
}

fn is_section_boundary(line: &str) -> bool {
    // H2 recognition follows the snippet grammar, including its deliberately
    // permissive indentation and spacing rules.
    if parse_snippet_heading(line).is_some() {
        return true;
    }

    let trimmed = line.trim_start();
    if line.len() - trimmed.len() > 3 {
        return false;
    }
    let Some(rest) = trimmed.strip_prefix('#') else {
        return false;
    };
    let rest = rest.strip_prefix('#').unwrap_or(rest);
    !rest.starts_with('#') && rest.chars().next().is_none_or(char::is_whitespace)
}
