use peanutbutter::config::Paths;
use peanutbutter::discovery::discover_markdown_files;
use peanutbutter::frecency::FrecencyStore;
use peanutbutter::parser::parse_file;
use peanutbutter::stats::{Output, Sort, StatsOptions};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

fn temp_dir_stats(prefix: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let path = std::env::temp_dir().join(format!(
        "pb-examples-stats-{prefix}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
}

fn stats_test_paths(root: &std::path::Path) -> Paths {
    Paths {
        snippet_roots: vec![root.to_path_buf()],
        xdg_snippets_dir: root.to_path_buf(),
        snippet_overrides_active: false,
        state_file: root.join("state.tsv"),
        config_file: root.join("config.toml"),
    }
}

const STATS_NOW: u64 = 1_715_600_000;

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
fn simple_file_yields_seven_snippets() {
    let by_file = parse_examples();
    let snippets = by_file
        .get("simple/snippets.md")
        .expect("simple/snippets.md parsed");
    assert_eq!(
        snippets,
        &vec![
            "List directory contents".to_string(),
            "Watch a file for new lines".to_string(),
            "Find files by name pattern".to_string(),
            "Read and decode base64 content from a file".to_string(),
            "Create a directory and navigate into it".to_string(),
            "Copy a file to a timestamped backup".to_string(),
            "Search for a running process".to_string(),
        ]
    );
}

#[test]
fn simple_file_frontmatter_is_parsed() {
    let root = examples_root();
    let path = root.join("simple/snippets.md");
    let content = fs::read_to_string(&path).unwrap();
    let parsed = parse_file(&path, &root, &content);
    assert_eq!(parsed.frontmatter.name.as_deref(), Some("Shell utilities"));
    assert_eq!(parsed.frontmatter.tags, vec!["shell", "files"]);
    assert_eq!(
        parsed.frontmatter.description.as_deref(),
        Some("Common shell and file operations.")
    );
}

#[test]
fn complex_file_has_five_snippets_with_variables() {
    let root = examples_root();
    let path = root.join("complex/complex.md");
    let content = fs::read_to_string(&path).unwrap();
    let parsed = parse_file(&path, &root, &content);
    assert_eq!(parsed.snippets.len(), 5);

    let env = &parsed.snippets[0];
    assert_eq!(env.name, "Write an .env file from variables");
    let var_names: Vec<&str> = env.variables.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(
        var_names,
        vec![
            "output",
            "environment",
            "port",
            "database_url",
            "secret_key"
        ]
    );

    let dockerfile = &parsed.snippets[1];
    assert_eq!(
        dockerfile.name,
        "Create a Dockerfile for serving static files"
    );
    assert!(dockerfile.body.contains("FROM nginx:alpine"));

    let curl = &parsed.snippets[4];
    assert_eq!(curl.name, "Curl with method, headers, and body");
    let var_names: Vec<&str> = curl.variables.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(
        var_names,
        vec!["http_method", "header_name", "header_value", "body", "url"]
    );
    let http_method = parsed.frontmatter.variables.get("http_method").unwrap();
    assert_eq!(http_method.default, None);
    assert_eq!(
        http_method.suggestions,
        vec!["GET", "POST", "PUT", "PATCH", "DELETE"]
    );
}

#[test]
fn nested_http_file_declares_url_default() {
    let root = examples_root();
    let path = root.join("nested/http/http.md");
    let content = fs::read_to_string(&path).unwrap();
    let parsed = parse_file(&path, &root, &content);
    let url = parsed.frontmatter.variables.get("url").unwrap();
    assert_eq!(url.default.as_deref(), Some("https://example.com"));
}

#[test]
fn nested_examples_all_parse() {
    let by_file = parse_examples();
    assert_eq!(
        by_file.get("nested/root.md"),
        Some(&vec![
            "Check external IP address".to_string(),
            "Generate a random UUID".to_string(),
            "Measure command execution time".to_string(),
        ])
    );
    assert_eq!(
        by_file.get("nested/docker/docker.md"),
        Some(&vec![
            "Run a container".to_string(),
            "Execute a shell in a running container".to_string(),
            "View container logs".to_string(),
            "Remove all stopped containers".to_string(),
        ])
    );
    assert_eq!(
        by_file.get("nested/docker/images/images.md"),
        Some(&vec![
            "List images".to_string(),
            "Build an image".to_string(),
            "Build with a specific Dockerfile".to_string(),
            "Push an image".to_string(),
            "Remove dangling images".to_string(),
        ])
    );
    assert_eq!(
        by_file.get("nested/docker/compose/snip.md"),
        Some(&vec![
            "Start services in the background".to_string(),
            "Stop and remove containers".to_string(),
            "View logs for a service".to_string(),
            "Rebuild and restart a service".to_string(),
            "Run a one-off command in a service container".to_string(),
            "Start with a specific compose file".to_string(),
        ])
    );
    assert_eq!(
        by_file.get("nested/grep/grep.md"),
        Some(&vec![
            "Search for a pattern in files".to_string(),
            "Search only files of a specific type".to_string(),
            "Search case-insensitively".to_string(),
            "Show only the matching portion of each line".to_string(),
            "Count matches per file".to_string(),
            "Search and replace (preview without writing)".to_string(),
        ])
    );
    assert_eq!(
        by_file.get("nested/git/git.md"),
        Some(&vec![
            "Commit staged changes".to_string(),
            "Stage path and commit".to_string(),
            "Create and switch to a new branch".to_string(),
            "Switch to an existing branch".to_string(),
            "Push branch and set upstream".to_string(),
            "Stash changes with a description".to_string(),
            "Cherry-pick a commit".to_string(),
            "Amend the last commit message".to_string(),
            "View log with graph".to_string(),
            "Soft reset to previous commit".to_string(),
        ])
    );
    assert_eq!(
        by_file.get("nested/http/http.md"),
        Some(&vec![
            "GET request".to_string(),
            "GET with bearer token".to_string(),
            "POST JSON body".to_string(),
            "POST JSON with bearer token".to_string(),
            "Check HTTP status code only".to_string(),
            "Download a file".to_string(),
            "Upload a file (multipart form)".to_string(),
        ])
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
fn stats_missing_state_file_prints_no_history_note() {
    let root = temp_dir_stats("no-state");
    fs::write(root.join("snip.md"), "## Echo\n\n```\necho\n```\n").unwrap();
    let paths = stats_test_paths(&root);
    // state file does not exist

    let mut out = Vec::new();
    peanutbutter::stats::run_with(
        &paths,
        StatsOptions {
            top_n: 10,
            sort: Sort::Stale,
            output: Output::Text,
            theme: Default::default(),
        },
        STATS_NOW,
        false,
        &mut out,
    )
    .unwrap();
    let s = String::from_utf8(out).unwrap();
    assert!(s.contains("No frecency history yet"), "got: {s}");
}

#[test]
fn stats_empty_state_file_shows_report() {
    let root = temp_dir_stats("empty-state");
    fs::write(root.join("snip.md"), "## Echo\n\n```\necho\n```\n").unwrap();
    let paths = stats_test_paths(&root);
    fs::write(&paths.state_file, "").unwrap();

    let mut out = Vec::new();
    peanutbutter::stats::run_with(
        &paths,
        StatsOptions {
            top_n: 10,
            sort: Sort::Stale,
            output: Output::Text,
            theme: Default::default(),
        },
        STATS_NOW,
        false,
        &mut out,
    )
    .unwrap();
    let s = String::from_utf8(out).unwrap();
    assert!(!s.contains("No frecency history yet"), "got: {s}");
    assert!(s.contains("Never Used") || s.contains("Echo"), "got: {s}");
}

#[test]
fn stats_with_events_shows_most_used() {
    let root = temp_dir_stats("with-events");
    // slugify("Echo") = "echo"
    fs::write(root.join("snip.md"), "## Echo\n\n```\necho\n```\n").unwrap();
    let paths = stats_test_paths(&root);
    let mut store = FrecencyStore::new();
    for i in 0..5u64 {
        store.record(
            peanutbutter::domain::SnippetId::new("snip.md", "echo"),
            std::path::PathBuf::from("/repo"),
            STATS_NOW - i * 3600,
        );
    }
    store.save(&paths.state_file).unwrap();

    let mut out = Vec::new();
    peanutbutter::stats::run_with(
        &paths,
        StatsOptions {
            top_n: 10,
            sort: Sort::Stale,
            output: Output::Text,
            theme: Default::default(),
        },
        STATS_NOW,
        false,
        &mut out,
    )
    .unwrap();
    let s = String::from_utf8(out).unwrap();
    assert!(s.contains("Most Used"), "got: {s}");
    assert!(s.contains("Echo"), "got: {s}");
}

#[test]
fn stats_json_produces_valid_json_with_all_keys() {
    let root = temp_dir_stats("json");
    fs::write(root.join("a.md"), "## A\n\n```\necho a\n```\n").unwrap();
    fs::write(root.join("b.md"), "## B\n\n```\necho b\n```\n").unwrap();
    let paths = stats_test_paths(&root);
    let mut store = FrecencyStore::new();
    store.record(
        peanutbutter::domain::SnippetId::new("a.md", "a"),
        std::path::PathBuf::from("/repo"),
        STATS_NOW,
    );
    store.save(&paths.state_file).unwrap();

    let mut out = Vec::new();
    peanutbutter::stats::run_with(
        &paths,
        StatsOptions {
            top_n: 10,
            sort: Sort::Stale,
            output: Output::Json,
            theme: Default::default(),
        },
        STATS_NOW,
        false,
        &mut out,
    )
    .unwrap();
    let s = String::from_utf8(out).unwrap();
    let v: serde_json::Value = serde_json::from_str(s.trim()).expect("valid JSON");
    assert!(v["most_used"].is_array());
    assert!(v["least_used"].is_array());
    assert!(v["never_used"].is_array());
    assert!(v["recency"].is_object());
    assert!(v["directory_affinity"].is_array());
    assert!(v["orphaned_event_count"].is_number());
    // b.md#b is never-used
    assert!(!v["never_used"].as_array().unwrap().is_empty());
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
