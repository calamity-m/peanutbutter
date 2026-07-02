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
fn parses_frontmatter_variable_hint() {
    let content = r#"---
variables:
  input:
    hint: hello
  path:
    default: .
    hint: "where to copy"
---
"#;
    let lines: Vec<&str> = content.lines().collect();
    let (fm, _) = parse_frontmatter(&lines);
    assert_eq!(
        fm.variables.get("input").unwrap().hint.as_deref(),
        Some("hello")
    );
    let path = fm.variables.get("path").unwrap();
    assert_eq!(path.default.as_deref(), Some("."));
    assert_eq!(path.hint.as_deref(), Some("where to copy"));
}

#[test]
fn parses_frontmatter_variable_block_suggestions() {
    let content = r#"---
variables:
  pattern:
    suggestions:
      - "*.psd"
      - "*.png"
      - "*.pdf"
---
"#;
    let lines: Vec<&str> = content.lines().collect();
    let (fm, _) = parse_frontmatter(&lines);
    let pattern = fm.variables.get("pattern").unwrap();
    assert_eq!(pattern.suggestions, vec!["*.psd", "*.png", "*.pdf"]);
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
    assert_eq!(
        vars[1].source,
        VariableSource::Default(vec![crate::syntax::Fragment::Literal("hi".to_string())])
    );
    assert_eq!(vars[2].name, "cmd");
    assert_eq!(
        vars[2].source,
        VariableSource::Command("rg . --files".to_string())
    );
}

#[test]
fn parses_inline_hint_as_hint_not_command() {
    let vars = parse_variables("echo \"<@input:@hello> world\"");
    assert_eq!(vars.len(), 1);
    assert_eq!(vars[0].name, "input");
    assert_eq!(vars[0].source, VariableSource::Hint("hello".to_string()));
}

#[test]
fn inline_hint_preserves_spaces_and_at_signs() {
    let vars = parse_variables("git clone <@repo:@user@host: path>");
    assert_eq!(
        vars[0].source,
        VariableSource::Hint("user@host: path".to_string())
    );
}

#[test]
fn parses_dependent_default_template() {
    let vars = parse_variables("tee <@out:?<#a:raw>.<#b:raw>.out>");
    assert_eq!(vars.len(), 1);
    assert_eq!(vars[0].name, "out");
    assert_eq!(
        vars[0].source,
        VariableSource::Default(vec![
            crate::syntax::Fragment::Ref {
                name: "a".to_string(),
                raw: true,
            },
            crate::syntax::Fragment::Literal(".".to_string()),
            crate::syntax::Fragment::Ref {
                name: "b".to_string(),
                raw: true,
            },
            crate::syntax::Fragment::Literal(".out".to_string()),
        ])
    );
}

#[test]
fn parses_plain_default_as_literal_template() {
    let vars = parse_variables("echo <@p:?plain>");
    assert_eq!(
        vars[0].source,
        VariableSource::Default(vec![crate::syntax::Fragment::Literal("plain".to_string())])
    );
}

#[test]
fn placeholder_end_skips_nested_dependent_refs_in_defaults() {
    let body = "tee <@out:?<#a:raw>.<#b:raw>.out> done";
    let start = body.find("<@out:").unwrap() + 2;
    let end = find_placeholder_end(body, start).unwrap();
    assert_eq!(&body[end..=end], ">");
    assert_eq!(&body[start..end], "out:?<#a:raw>.<#b:raw>.out");
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
fn text_fences_are_description_not_body() {
    let content = "## Hello\n\n```text\nexample output\n```\n\n```bash\necho hi\n```\n";
    let snippets = parse_snippets(&content.lines().collect::<Vec<_>>(), &rel("x.md"));

    assert_eq!(snippets.len(), 1);
    assert_eq!(snippets[0].body, "echo hi");
    assert_eq!(snippets[0].language.as_deref(), Some("bash"));
    assert!(snippets[0].description.contains("```text"));
    assert!(snippets[0].description.contains("example output"));
}

#[test]
fn text_only_sections_are_not_snippets() {
    let content =
        "## Example only\n\n```text\nnot executable\n```\n\n## Real\n\n```bash\nrun\n```\n";
    let snippets = parse_snippets(&content.lines().collect::<Vec<_>>(), &rel("x.md"));

    assert_eq!(snippets.len(), 1);
    assert_eq!(snippets[0].name, "Real");
}

#[test]
fn text_fence_matching_is_case_insensitive() {
    let content = "## Hello\n\n```TEXT\nexample output\n```\n\n```bash\necho hi\n```\n";
    let snippets = parse_snippets(&content.lines().collect::<Vec<_>>(), &rel("x.md"));

    assert_eq!(snippets.len(), 1);
    assert_eq!(snippets[0].body, "echo hi");
}

#[test]
fn extended_text_info_string_is_executable() {
    let content = "## Hello\n\n```text linenums\nexample output\n```\n\n```bash\necho hi\n```\n";
    let snippets = parse_snippets(&content.lines().collect::<Vec<_>>(), &rel("x.md"));

    assert_eq!(snippets.len(), 1);
    assert_eq!(snippets[0].body, "example output");
    assert_eq!(snippets[0].language.as_deref(), Some("text linenums"));
}

#[test]
fn text_only_sections_do_not_consume_duplicate_slug_suffixes() {
    let content = "## Same\n\n```text\nexample only\n```\n\n## Same\n\n```bash\nrun\n```\n";
    let snippets = parse_snippets(&content.lines().collect::<Vec<_>>(), &rel("x.md"));

    assert_eq!(snippets.len(), 1);
    assert_eq!(snippets[0].id.as_str(), "x.md#same");
}

#[test]
fn unclosed_text_fence_yields_no_snippet() {
    let content = "## Hello\n\n```text\nno close here\n";
    let snippets = parse_snippets(&content.lines().collect::<Vec<_>>(), &rel("x.md"));

    assert!(snippets.is_empty());
}

#[test]
fn snippet_line_ranges_skip_text_fences() {
    let content = "## Hello\n\n```text\nexample output\n```\n\n```bash\necho hi\n```\n";
    let ranges = snippet_line_ranges(&rel("x.md"), content);

    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0].id.as_str(), "x.md#hello");
    assert_eq!(ranges[0].start_line, 0);
    assert_eq!(ranges[0].end_line, 9);
}

#[test]
fn snippet_line_ranges_do_not_include_text_only_sections() {
    let content = "## Same\n\n```text\nexample only\n```\n\n## Same\n\n```bash\nrun\n```\n";
    let ranges = snippet_line_ranges(&rel("x.md"), content);

    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0].id.as_str(), "x.md#same");
    assert_eq!(ranges[0].start_line, 6);
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
