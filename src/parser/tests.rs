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
fn parses_frontmatter_default_value_scalars() {
    let content = "---\nvariables:\n  output:\n    default_value: \"<#name:raw>.out\"\n  empty:\n    default_value: ''\n---\n";
    let lines: Vec<&str> = content.lines().collect();
    let (fm, _) = parse_frontmatter(&lines);
    assert_eq!(
        fm.variables["output"].default_value.as_deref(),
        Some("<#name:raw>.out")
    );
    assert_eq!(fm.variables["empty"].default_value.as_deref(), Some(""));
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

fn suggestions(items: &[&str]) -> VariableSource {
    VariableSource::Suggestions(items.iter().map(|item| item.to_string()).collect())
}

fn source_of(body: &str) -> VariableSource {
    let vars = parse_variables(body);
    assert_eq!(vars.len(), 1, "expected one variable in {body:?}");
    vars[0].source.clone()
}

#[test]
fn parses_inline_suggestion_list() {
    let vars = parse_variables("deploy --env <@env:[dev,stage,prod]>");
    assert_eq!(vars.len(), 1);
    assert_eq!(vars[0].name, "env");
    assert_eq!(vars[0].source, suggestions(&["dev", "stage", "prod"]));
}

#[test]
fn inline_suggestion_list_trims_source_and_items() {
    assert_eq!(
        source_of("<@env: [ dev , stage prod ] >"),
        suggestions(&["dev", "stage prod"])
    );
}

#[test]
fn inline_suggestion_list_unescapes_commas_brackets_and_backslashes() {
    assert_eq!(
        source_of(r"<@v:[a\,b, c\]d, e\\f, g\nh]>"),
        suggestions(&["a,b", "c]d", r"e\f", r"g\nh"])
    );
}

#[test]
fn inline_suggestion_list_escaped_reference_opener_is_literal() {
    assert_eq!(source_of(r"<@v:[\<#r>a, b]>"), suggestions(&["<#r>a", "b"]));
}

#[test]
fn inline_suggestion_list_keeps_reserved_reference_literal() {
    assert_eq!(source_of("<@v:[<#r>a, b]>"), suggestions(&["<#r>a", "b"]));
}

#[test]
fn inline_suggestion_list_drops_empty_items() {
    assert_eq!(source_of("<@v:[]>"), suggestions(&[]));
    assert_eq!(source_of("<@v:[a,,b, ]>"), suggestions(&["a", "b"]));
}

#[test]
fn inline_suggestion_list_keeps_bare_inner_bracket_literal() {
    assert_eq!(source_of("<@v:[a]b,c]>"), suggestions(&["a]b", "c"]));
}

#[test]
fn inline_suggestion_list_requires_unescaped_closing_bracket() {
    assert_eq!(source_of(r"<@v:[a\\]>"), suggestions(&[r"a\"]));
    assert_eq!(
        source_of(r"<@v:[a,b\]>"),
        VariableSource::Command(r"[a,b\]".to_string())
    );
    assert_eq!(
        source_of("<@v:[a,b>"),
        VariableSource::Command("[a,b".to_string())
    );
}

#[test]
fn bracketed_test_command_is_reinterpreted_only_when_it_spans_the_source() {
    assert_eq!(source_of("<@v:[ -d x ]>"), suggestions(&["-d x"]));
    assert_eq!(
        source_of("<@b:[ -d .git ] && git branch>"),
        VariableSource::Command("[ -d .git ] && git branch".to_string())
    );
}

#[test]
fn inline_suggestion_list_items_may_contain_colons() {
    assert_eq!(
        source_of("<@url:[http://a,http://b]>"),
        suggestions(&["http://a", "http://b"])
    );
}

#[test]
fn default_prefix_wins_over_suggestion_list() {
    assert_eq!(
        source_of("<@v:?[a]>"),
        VariableSource::Default(vec![crate::syntax::Fragment::Literal("[a]".to_string())])
    );
}

#[test]
fn suggestion_lists_mix_with_other_sources() {
    let vars = parse_variables(
        "kubectl --context <@ctx:[dev,prod]> get <@kind> <@name:kubectl get <#kind> -o name> <@ctx>",
    );
    assert_eq!(vars.len(), 4);
    assert_eq!(vars[0].source, suggestions(&["dev", "prod"]));
    assert_eq!(vars[1].source, VariableSource::Free);
    assert_eq!(
        vars[2].source,
        VariableSource::Command("kubectl get <#kind> -o name".to_string())
    );
    assert_eq!(vars[3].name, "ctx");
    assert_eq!(vars[3].source, VariableSource::Free);
}

#[test]
fn suggestion_list_reports_first_problem() {
    use crate::parser::{SuggestionListError, parse_suggestion_list};
    let error = |source: &str| parse_suggestion_list(source).unwrap().error;
    assert!(parse_suggestion_list("rg . --files").is_none());
    assert!(parse_suggestion_list("[a,b").is_none());
    assert!(parse_suggestion_list(r"[a,b\]").is_none());
    assert_eq!(error("[a, b]"), None);
    assert_eq!(error(r"[a\,b, \<#r>]"), None);
    assert_eq!(error("[]"), Some(SuggestionListError::Empty));
    assert_eq!(error("[  ]"), Some(SuggestionListError::Empty));
    assert_eq!(error("[a,,b]"), Some(SuggestionListError::EmptyItem));
    assert_eq!(error("[a,]"), Some(SuggestionListError::EmptyItem));
    assert_eq!(error("[a]b]"), Some(SuggestionListError::StrayBracket));
    assert_eq!(
        error("[<#r>a]"),
        Some(SuggestionListError::ReservedReference)
    );
    assert_eq!(error(r#"[a, "b,c"]"#), None);
    assert_eq!(error(r#"[""]"#), Some(SuggestionListError::EmptyItem));
    assert_eq!(
        error(r#"[a, "b,c]"#),
        Some(SuggestionListError::UnterminatedQuote)
    );
    assert_eq!(
        error(r#"["b"c]"#),
        Some(SuggestionListError::TextAfterQuote)
    );
    assert_eq!(
        error(r#"["<#r>"]"#),
        Some(SuggestionListError::ReservedReference)
    );
}

#[test]
fn quoted_items_keep_commas_brackets_and_spaces() {
    assert_eq!(source_of(r#"<@v:[a, "b,c"]>"#), suggestions(&["a", "b,c"]));
    assert_eq!(
        source_of(r#"<@v:[ "  spaced  " , "x]y"]>"#),
        suggestions(&["  spaced  ", "x]y"])
    );
}

#[test]
fn quoted_and_escaped_forms_are_equivalent() {
    assert_eq!(
        source_of(r#"<@v:[a, "b,c"]>"#),
        source_of(r"<@v:[a, b\,c]>")
    );
}

#[test]
fn quoted_items_unescape_quotes_and_backslashes() {
    assert_eq!(
        source_of(r#"<@v:["say \"hi\"", "c\\d", "e\,f"]>"#),
        suggestions(&[r#"say "hi""#, r"c\d", "e,f"])
    );
}

#[test]
fn quotes_are_literal_unless_they_open_an_item() {
    assert_eq!(
        source_of(r#"<@v:[\"lit\", a"b]>"#),
        suggestions(&[r#""lit""#, r#"a"b"#])
    );
    // Single quotes never group an item.
    assert_eq!(source_of("<@v:['b,c']>"), suggestions(&["'b", "c'"]));
}

#[test]
fn malformed_quoted_items_stay_usable() {
    // An unterminated quote runs to the end of the list.
    assert_eq!(source_of(r#"<@v:[a, "b,c]>"#), suggestions(&["a", "b,c"]));
    // Text after a closing quote is kept, joined to the quoted text.
    assert_eq!(source_of(r#"<@v:["b"c, d]>"#), suggestions(&["bc", "d"]));
    // An empty quoted item is dropped like any other empty item.
    assert_eq!(source_of(r#"<@v:["", d]>"#), suggestions(&["d"]));
}

#[test]
fn format_suggestion_list_escapes_and_round_trips() {
    use crate::parser::format_suggestion_list;
    let items: Vec<String> = [
        "dev",
        "a,b",
        "c]",
        r"d\e",
        "<#x>",
        "a<b",
        " padded",
        r#""quoted"#,
        r#"in"side"#,
    ]
    .map(String::from)
    .to_vec();
    let source = format_suggestion_list(&items).unwrap();
    assert_eq!(
        source,
        r#"[dev,"a,b","c]",d\\e,\<#x>,a<b," padded","\"quoted",in"side]"#
    );
    assert_eq!(
        source_of(&format!("<@v:{source}>")),
        VariableSource::Suggestions(items)
    );
}

#[test]
fn format_suggestion_list_rejects_items_that_cannot_be_inline() {
    use crate::parser::format_suggestion_list;
    for items in [
        vec![],
        vec![String::new()],
        vec!["a>b".to_string()],
        vec!["<#x>y>z".to_string()],
        vec!["two\nlines".to_string()],
    ] {
        assert_eq!(format_suggestion_list(&items), None, "{items:?}");
    }
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
