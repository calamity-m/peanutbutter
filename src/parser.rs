use crate::domain::{
    Frontmatter, Snippet, SnippetFile, SnippetId, Variable, VariableSource, VariableSpec,
};
use std::collections::BTreeMap;
use std::path::Path;

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

fn parse_frontmatter(lines: &[&str]) -> (Frontmatter, usize) {
    if lines.first().map(|l| l.trim()) != Some("---") {
        return (Frontmatter::default(), 0);
    }
    let mut end_idx = None;
    for (i, line) in lines.iter().enumerate().skip(1) {
        if line.trim() == "---" {
            end_idx = Some(i);
            break;
        }
    }
    let end = match end_idx {
        Some(i) => i,
        None => return (Frontmatter::default(), 0),
    };
    let fm = parse_yaml_frontmatter(&lines[1..end]);
    (fm, end + 1)
}

fn parse_yaml_frontmatter(lines: &[&str]) -> Frontmatter {
    let mut fm = Frontmatter::default();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            i += 1;
            continue;
        }
        let (key, value) = match trimmed.split_once(':') {
            Some(kv) => kv,
            None => {
                i += 1;
                continue;
            }
        };
        let key = key.trim();
        let value = value.trim();

        if value.starts_with('[') && value.ends_with(']') {
            if key == "tags" {
                fm.tags = parse_inline_list(value);
            }
            i += 1;
            continue;
        }

        if value.is_empty() {
            i += 1;
            match key {
                "tags" => {
                    while i < lines.len() {
                        let child = lines[i].trim_start();
                        let indent = lines[i].len() - child.len();
                        if indent == 0 || !child.starts_with('-') {
                            break;
                        }
                        fm.tags.push(strip_quotes(child[1..].trim()).to_string());
                        i += 1;
                    }
                }
                "variables" => {
                    let (vars, consumed) = parse_variable_block(&lines[i..]);
                    fm.variables = vars;
                    i += consumed;
                }
                _ => {}
            }
            continue;
        }

        match key {
            "name" => fm.name = Some(strip_quotes(value).to_string()),
            "description" => fm.description = Some(strip_quotes(value).to_string()),
            _ => {}
        }
        i += 1;
    }

    fm
}

/// Parse a `variables:` block into a map of name → [`VariableSpec`].
/// Returns the map and the number of lines consumed from `lines`.
fn parse_variable_block(lines: &[&str]) -> (BTreeMap<String, VariableSpec>, usize) {
    let mut out = BTreeMap::new();
    let mut j = 0;

    while j < lines.len() {
        let line = lines[j];
        let trimmed = line.trim_start();
        let var_indent = line.len() - trimmed.len();

        if var_indent == 0 || trimmed.is_empty() || trimmed.starts_with('#') {
            break;
        }

        let (name, rest) = match trimmed.split_once(':') {
            Some(kv) => kv,
            None => {
                j += 1;
                continue;
            }
        };
        let name = name.trim().to_string();
        let rest = rest.trim();

        // A valid variable entry is a block mapping (`varname:` with no inline value).
        if !rest.is_empty() {
            j += 1;
            continue;
        }

        j += 1;
        let mut spec = VariableSpec::default();
        while j < lines.len() {
            let field_line = lines[j];
            let field_trim = field_line.trim_start();
            let field_indent = field_line.len() - field_trim.len();
            if field_indent <= var_indent {
                break;
            }
            if let Some((fkey, fval)) = field_trim.split_once(':') {
                let fkey = fkey.trim();
                let fval = fval.trim();
                if !fval.is_empty() {
                    match fkey {
                        "default" => spec.default = Some(strip_quotes(fval).to_string()),
                        "suggestions" if fval.starts_with('[') && fval.ends_with(']') => {
                            spec.suggestions = parse_inline_list(fval);
                        }
                        "command" => spec.command = Some(fval.to_string()),
                        _ => {}
                    }
                }
            }
            j += 1;
        }
        out.insert(name, spec);
    }

    (out, j)
}

fn parse_inline_list(value: &str) -> Vec<String> {
    let inner = &value[1..value.len() - 1];
    inner
        .split(',')
        .map(|s| strip_quotes(s.trim()).to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn strip_quotes(s: &str) -> &str {
    let bytes = s.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return &s[1..s.len() - 1];
        }
    }
    s
}

enum State {
    Scanning,
    InSection {
        heading: String,
        description: Vec<String>,
    },
    InCode {
        heading: String,
        description: Vec<String>,
        fence: String,
        language: Option<String>,
        body: Vec<String>,
    },
}

enum RangeState {
    Scanning,
    InSection {
        heading: String,
        start_line: usize,
    },
    InCode {
        heading: String,
        start_line: usize,
        fence: String,
    },
}

fn parse_snippets(lines: &[&str], relative_path: &Path) -> Vec<Snippet> {
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
                    let description = std::mem::take(description);
                    state = State::InCode {
                        heading,
                        description,
                        fence,
                        language,
                        body: Vec::new(),
                    };
                } else {
                    description.push((*line).to_string());
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

fn parse_snippet_line_ranges(
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
                } else if let Some((fence, _)) = parse_fence_open(line) {
                    let heading = std::mem::take(heading);
                    let start_line = *start_line;
                    state = RangeState::InCode {
                        heading,
                        start_line,
                        fence,
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

fn parse_snippet_heading(line: &str) -> Option<String> {
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
fn parse_fence_open(line: &str) -> Option<(String, Option<String>)> {
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

fn is_fence_close(line: &str, fence: &str) -> bool {
    let trimmed = line.trim();
    if !trimmed.starts_with(fence) {
        return false;
    }
    trimmed.chars().all(|c| c == '`') && trimmed.len() >= fence.len()
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

fn next_slug(heading: &str, seen_slugs: &mut std::collections::HashMap<String, usize>) -> String {
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

fn slugify(input: &str) -> String {
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

/// Extract all `<@name[:source]>` placeholders from a snippet body in order
/// of first appearance. Malformed or unterminated placeholders are silently
/// skipped. Duplicates are preserved here; callers that need unique variables
/// should use [`crate::execute::prompt::unique_variables`].
pub fn parse_variables(body: &str) -> Vec<Variable> {
    let mut out = Vec::new();
    let bytes = body.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'<' && bytes[i + 1] == b'@' {
            let start = i + 2;
            if let Some(offset) = body[start..].find('>') {
                let inner = &body[start..start + offset];
                if let Some(var) = parse_variable_inner(inner) {
                    out.push(var);
                }
                i = start + offset + 1;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn parse_variable_inner(inner: &str) -> Option<Variable> {
    let (name, source) = match inner.split_once(':') {
        Some((name, rest)) => {
            let source = if let Some(default) = rest.strip_prefix('?') {
                VariableSource::Default(default.to_string())
            } else {
                VariableSource::Command(rest.to_string())
            };
            (name.trim(), source)
        }
        None => (inner.trim(), VariableSource::Free),
    };
    if name.is_empty() || !name.chars().all(is_name_char) {
        return None;
    }
    Some(Variable {
        name: name.to_string(),
        source,
    })
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn rel(p: &str) -> PathBuf {
        PathBuf::from(p)
    }

    #[test]
    fn parses_frontmatter_and_snippets() {
        let content = "---\nname: demo\ntags: [a, b]\n---\n\n## First\n\n```\necho hi\n```\n";
        let lines: Vec<&str> = content.lines().collect();
        let (fm, start) = parse_frontmatter(&lines);
        assert_eq!(fm.name.as_deref(), Some("demo"));
        assert_eq!(fm.tags, vec!["a".to_string(), "b".to_string()]);
        let snippets = parse_snippets(&lines[start..], &rel("demo.md"));
        assert_eq!(snippets.len(), 1);
        assert_eq!(snippets[0].name, "First");
        assert_eq!(snippets[0].body, "echo hi");
    }

    #[test]
    fn parses_block_style_tags_list() {
        let content = "---\ntags:\n    - a\n    - b\ndescription: meta\n---\n";
        let lines: Vec<&str> = content.lines().collect();
        let (fm, _) = parse_frontmatter(&lines);
        assert_eq!(fm.tags, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(fm.description.as_deref(), Some("meta"));
    }

    #[test]
    fn parses_frontmatter_variable_specs() {
        let content = r#"---
name: HTTP
variables:
  http_method:
    default: GET
    suggestions: [GET, POST]
  git_branch:
    command: git branch --format='%(refname:short)'
  bad_spec: nope
---
"#;
        let lines: Vec<&str> = content.lines().collect();
        let (fm, _) = parse_frontmatter(&lines);
        let method = fm.variables.get("http_method").unwrap();
        assert_eq!(method.default.as_deref(), Some("GET"));
        assert_eq!(method.suggestions, vec!["GET", "POST"]);
        assert_eq!(
            fm.variables.get("git_branch").unwrap().command.as_deref(),
            Some("git branch --format='%(refname:short)'")
        );
        assert!(!fm.variables.contains_key("bad_spec"));
    }

    #[test]
    fn malformed_frontmatter_is_ignored() {
        let content = "---\nvariables: [\n---\n\n## Demo\n\n```\necho hi\n```\n";
        let parsed = parse_file(Path::new("demo.md"), Path::new("."), content);
        assert_eq!(parsed.frontmatter, Frontmatter::default());
        assert_eq!(parsed.snippets.len(), 1);
    }

    #[test]
    fn first_code_block_wins() {
        let content = "## Title\n\ndesc\n\n```\nfirst\n```\n\n```\nsecond\n```\n";
        let lines: Vec<&str> = content.lines().collect();
        let snippets = parse_snippets(&lines, &rel("x.md"));
        assert_eq!(snippets.len(), 1);
        assert_eq!(snippets[0].body, "first");
    }

    #[test]
    fn sections_without_code_are_discarded() {
        let content = "## Empty\n\njust description, no code\n\n## Real\n\n```\nrun\n```\n";
        let lines: Vec<&str> = content.lines().collect();
        let snippets = parse_snippets(&lines, &rel("x.md"));
        assert_eq!(snippets.len(), 1);
        assert_eq!(snippets[0].name, "Real");
    }

    #[test]
    fn ignores_h1_and_h3_headings() {
        let content = "# Big\n\n## Real\n\n### Nested\n\n```\ncmd\n```\n";
        let lines: Vec<&str> = content.lines().collect();
        let snippets = parse_snippets(&lines, &rel("x.md"));
        assert_eq!(snippets.len(), 1);
        assert_eq!(snippets[0].name, "Real");
        assert!(snippets[0].description.contains("### Nested"));
    }

    #[test]
    fn parses_all_variable_forms() {
        let body = "cat <@file> | grep <@pattern:?hi> | xargs <@cmd:rg . --files>";
        let vars = parse_variables(body);
        assert_eq!(vars.len(), 3);
        assert_eq!(vars[0].name, "file");
        assert_eq!(vars[0].source, VariableSource::Free);
        assert_eq!(vars[1].name, "pattern");
        assert_eq!(vars[1].source, VariableSource::Default("hi".to_string()));
        assert_eq!(vars[2].name, "cmd");
        assert_eq!(
            vars[2].source,
            VariableSource::Command("rg . --files".to_string())
        );
    }

    #[test]
    fn skips_unterminated_variables() {
        let body = "echo <@unterminated and more";
        assert!(parse_variables(body).is_empty());
    }

    #[test]
    fn slugify_handles_punctuation() {
        assert_eq!(
            slugify("Echo something without newline"),
            "echo-something-without-newline"
        );
        assert_eq!(slugify("  weird!! chars??  "), "weird-chars");
    }

    #[test]
    fn snippet_ids_are_unique_within_file() {
        let content = "## Same\n\n```\na\n```\n\n## Same\n\n```\nb\n```\n";
        let lines: Vec<&str> = content.lines().collect();
        let snippets = parse_snippets(&lines, &rel("x.md"));
        assert_eq!(snippets.len(), 2);
        assert_ne!(snippets[0].id, snippets[1].id);
    }

    #[test]
    fn language_tag_is_parsed_from_fence() {
        let content = "## Hello\n\n```python\nprint('hi')\n```\n";
        let snippets = parse_snippets(&content.lines().collect::<Vec<_>>(), &rel("x.md"));
        assert_eq!(snippets[0].language.as_deref(), Some("python"));
    }

    #[test]
    fn no_language_tag_produces_none() {
        let content = "## Hello\n\n```\necho hi\n```\n";
        let snippets = parse_snippets(&content.lines().collect::<Vec<_>>(), &rel("x.md"));
        assert_eq!(snippets[0].language, None);
    }

    #[test]
    fn snippet_line_ranges_follow_parser_order() {
        let content =
            "---\nname: demo\n---\n# Title\n\n## One\n\n```\na\n```\n\n## Two\n\n```\nb\n```\n";
        let ranges = snippet_line_ranges(&rel("x.md"), content);
        assert_eq!(ranges.len(), 2);
        assert_eq!(ranges[0].id.as_str(), "x.md#one");
        assert_eq!(ranges[0].start_line, 5);
        assert_eq!(ranges[0].end_line, 10);
        assert_eq!(ranges[1].id.as_str(), "x.md#two");
    }
}
