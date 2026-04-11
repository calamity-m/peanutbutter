use peanutbutter::discovery::discover_markdown_files;
use peanutbutter::parser::parse_file;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

fn examples_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples")
}

fn parse_examples() -> BTreeMap<String, Vec<String>> {
    let root = examples_root();
    let files = discover_markdown_files(&root).expect("discover");
    let mut by_file: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for file in files {
        let content = fs::read_to_string(&file).expect("read");
        let parsed = parse_file(&file, &root, &content);
        let rel = parsed.relative_path.to_string_lossy().replace('\\', "/");
        let names: Vec<String> = parsed.snippets.iter().map(|s| s.name.clone()).collect();
        by_file.insert(rel, names);
    }
    by_file
}

#[test]
fn simple_file_yields_three_snippets() {
    let by_file = parse_examples();
    let snippets = by_file
        .get("simple/snippets.md")
        .expect("simple/snippets.md parsed");
    assert_eq!(
        snippets,
        &vec![
            "Echo something without newline".to_string(),
            "List all files, including hidden".to_string(),
            "Cat file and base64 contents, with no newline".to_string(),
        ]
    );
}

#[test]
fn simple_file_frontmatter_is_parsed() {
    let root = examples_root();
    let path = root.join("simple/snippets.md");
    let content = fs::read_to_string(&path).unwrap();
    let parsed = parse_file(&path, &root, &content);
    assert_eq!(parsed.frontmatter.name.as_deref(), Some("snippet name"));
    assert_eq!(parsed.frontmatter.tags, vec!["a", "b"]);
    assert_eq!(
        parsed.frontmatter.description.as_deref(),
        Some("frontmatter metadata")
    );
}

#[test]
fn complex_file_has_two_snippets_with_variables() {
    let root = examples_root();
    let path = root.join("complex/complex.md");
    let content = fs::read_to_string(&path).unwrap();
    let parsed = parse_file(&path, &root, &content);
    assert_eq!(parsed.snippets.len(), 2);

    let dockerfile = &parsed.snippets[0];
    assert_eq!(dockerfile.name, "Create a dockerfile for nginx");
    assert!(dockerfile.body.contains("FROM nginx:alpine"));
    let var_names: Vec<&str> = dockerfile
        .variables
        .iter()
        .map(|v| v.name.as_str())
        .collect();
    assert_eq!(var_names, vec!["dockerfile_name"]);

    let curl = &parsed.snippets[1];
    assert_eq!(curl.name, "Curl with headers");
    let var_names: Vec<&str> = curl.variables.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(var_names, vec!["http_method", "header", "value", "url"]);
}

#[test]
fn nested_examples_all_parse() {
    let by_file = parse_examples();
    assert_eq!(
        by_file.get("nested/root.md"),
        Some(&vec!["Curl something".to_string()])
    );
    assert_eq!(
        by_file.get("nested/docker/docker.md"),
        Some(&vec!["Run docker".to_string()])
    );
    assert_eq!(
        by_file.get("nested/docker/images/images.md"),
        Some(&vec!["List docker images".to_string()])
    );
    assert_eq!(
        by_file.get("nested/docker/compose/snip.md"),
        Some(&vec![
            "Start docker compose".to_string(),
            "start specific docker compose".to_string(),
        ])
    );
    assert_eq!(
        by_file.get("nested/grep/grep.md"),
        Some(&vec!["Grep contents".to_string()])
    );
}

#[test]
fn every_example_file_produces_at_least_one_snippet() {
    let by_file = parse_examples();
    assert!(!by_file.is_empty(), "no example files discovered");
    for (path, names) in &by_file {
        assert!(!names.is_empty(), "expected at least one snippet in {path}");
    }
}

#[test]
fn snippet_ids_are_stable_and_unique_across_tree() {
    let root = examples_root();
    let files = discover_markdown_files(&root).unwrap();
    let mut ids = std::collections::HashSet::new();
    for file in files {
        let content = fs::read_to_string(&file).unwrap();
        let parsed = parse_file(&file, &root, &content);
        for snip in parsed.snippets {
            assert!(
                ids.insert(snip.id.as_str().to_string()),
                "duplicate id: {}",
                snip.id
            );
        }
    }
}
