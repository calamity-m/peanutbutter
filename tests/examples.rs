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
