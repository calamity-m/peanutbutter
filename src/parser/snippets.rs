use crate::domain::{Snippet, SnippetId};
use std::path::Path;

use super::variables::parse_variables;

enum State {
    Scanning,
    InSection {
        heading: String,
        description: Vec<String>,
    },
    InIgnoredTextFence {
        heading: String,
        description: Vec<String>,
        fence: String,
    },
    InCode {
        heading: String,
        description: Vec<String>,
        fence: String,
        language: Option<String>,
        body: Vec<String>,
    },
}
pub(super) fn parse_snippets(lines: &[&str], relative_path: &Path) -> Vec<Snippet> {
    let mut out = Vec::new();
    let mut state = State::Scanning;
    let mut seen_slugs: std::collections::HashMap<String, usize> = Default::default();

    for line in lines {
        match &mut state {
            State::Scanning => {
                if let Some(h) = parse_snippet_heading(line) {
                    state = State::InSection {
                        heading: h,
                        description: Vec::new(),
                    };
                }
            }
            State::InSection {
                heading,
                description,
            } => {
                if let Some(h) = parse_snippet_heading(line) {
                    state = State::InSection {
                        heading: h,
                        description: Vec::new(),
                    };
                } else if let Some((fence, language)) = parse_fence_open(line) {
                    let heading = std::mem::take(heading);
                    let mut description = std::mem::take(description);
                    if is_ignored_body_language(language.as_deref()) {
                        description.push((*line).to_string());
                        state = State::InIgnoredTextFence {
                            heading,
                            description,
                            fence,
                        };
                    } else {
                        state = State::InCode {
                            heading,
                            description,
                            fence,
                            language,
                            body: Vec::new(),
                        };
                    }
                } else {
                    description.push((*line).to_string());
                }
            }
            State::InIgnoredTextFence {
                heading,
                description,
                fence,
            } => {
                description.push((*line).to_string());
                if is_fence_close(line, fence) {
                    let heading = std::mem::take(heading);
                    let description = std::mem::take(description);
                    state = State::InSection {
                        heading,
                        description,
                    };
                }
            }
            State::InCode {
                heading,
                description,
                fence,
                language,
                body,
            } => {
                if is_fence_close(line, fence) {
                    let heading = std::mem::take(heading);
                    let description = std::mem::take(description);
                    let language = std::mem::take(language);
                    let body = std::mem::take(body);
                    out.push(build_snippet(
                        relative_path,
                        heading,
                        description,
                        body,
                        language,
                        &mut seen_slugs,
                    ));
                    state = State::Scanning;
                } else {
                    body.push((*line).to_string());
                }
            }
        }
    }

    out
}

pub(super) fn parse_snippet_heading(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix("##")?;
    if rest.starts_with('#') {
        return None;
    }
    let text = rest.trim();
    if text.is_empty() {
        return None;
    }
    Some(text.to_string())
}

/// Returns `(fence, language)` where `language` is the text after the backticks, if any.
pub(super) fn parse_fence_open(line: &str) -> Option<(String, Option<String>)> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with("```") {
        return None;
    }
    let ticks: String = trimmed.chars().take_while(|c| *c == '`').collect();
    if ticks.len() < 3 {
        return None;
    }
    let lang = trimmed[ticks.len()..].trim();
    let language = if lang.is_empty() {
        None
    } else {
        Some(lang.to_string())
    };
    Some((ticks, language))
}

pub(super) fn is_fence_close(line: &str, fence: &str) -> bool {
    let trimmed = line.trim();
    if !trimmed.starts_with(fence) {
        return false;
    }
    trimmed.chars().all(|c| c == '`') && trimmed.len() >= fence.len()
}

/// Returns true for fenced code languages reserved for preview-only examples.
pub(super) fn is_ignored_body_language(language: Option<&str>) -> bool {
    language.is_some_and(|language| language.eq_ignore_ascii_case("text"))
}

fn build_snippet(
    relative_path: &Path,
    heading: String,
    description: Vec<String>,
    body: Vec<String>,
    language: Option<String>,
    seen_slugs: &mut std::collections::HashMap<String, usize>,
) -> Snippet {
    let description = description.join("\n").trim().to_string();
    let body = body.join("\n");
    let variables = parse_variables(&body);

    let slug = next_slug(&heading, seen_slugs);

    let relative_display = relative_path.to_string_lossy().replace('\\', "/");
    let id = SnippetId::new(&relative_display, &slug);
    Snippet {
        id,
        name: heading,
        description,
        body,
        variables,
        language,
    }
}

pub(super) fn next_slug(
    heading: &str,
    seen_slugs: &mut std::collections::HashMap<String, usize>,
) -> String {
    let base_slug = slugify(heading);
    match seen_slugs.get_mut(&base_slug) {
        Some(count) => {
            *count += 1;
            format!("{base_slug}-{count}")
        }
        None => {
            seen_slugs.insert(base_slug.clone(), 0);
            base_slug
        }
    }
}

pub(super) fn slugify(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut last_dash = true;
    for c in input.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "snippet".to_string()
    } else {
        out
    }
}
