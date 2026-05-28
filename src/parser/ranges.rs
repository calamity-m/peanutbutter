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
