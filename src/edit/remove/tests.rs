use super::*;
use crate::domain::SnippetId;
use crate::index::SnippetIndex;
use crate::parser::parse_file;
use std::path::PathBuf;

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "pb-remove-{label}-{}-{}",
        std::process::id(),
        unique_suffix()
    ));
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// Write `content` to `snippets.md` under a fresh root and index it, so tests
/// exercise the same `IndexedSnippet` shape the picker passes in.
fn indexed(label: &str, content: &str) -> (PathBuf, SnippetIndex) {
    let root = temp_dir(label);
    let path = root.join("snippets.md");
    fs::write(&path, content).expect("write fixture");
    let parsed = parse_file(&path, &root, content);
    (root, SnippetIndex::from_files([parsed]))
}

fn remove_by_slug(index: &SnippetIndex, slug: &str) -> io::Result<()> {
    let id = SnippetId::new("snippets.md", slug);
    let snippet = index.get(&id).expect("snippet in index");
    remove_snippet(snippet)
}

fn read(root: &Path) -> String {
    fs::read_to_string(root.join("snippets.md")).expect("read back")
}

#[test]
fn removes_the_first_snippet_and_keeps_the_rest() {
    let (root, index) = indexed(
        "first",
        "# Title\n\n## Alpha\n\n```sh\nalpha\n```\n\n## Beta\n\n```sh\nbeta\n```\n",
    );
    remove_by_slug(&index, "alpha").expect("remove");
    assert_eq!(read(&root), "# Title\n\n## Beta\n\n```sh\nbeta\n```\n");
}

#[test]
fn removes_a_middle_snippet_without_disturbing_its_neighbours() {
    let (root, index) = indexed(
        "middle",
        "## Alpha\n\n```sh\nalpha\n```\n\n## Beta\n\n```sh\nbeta\n```\n\n## Gamma\n\n```sh\ngamma\n```\n",
    );
    remove_by_slug(&index, "beta").expect("remove");
    assert_eq!(
        read(&root),
        "## Alpha\n\n```sh\nalpha\n```\n\n## Gamma\n\n```sh\ngamma\n```\n"
    );
}

#[test]
fn removes_the_last_snippet_and_trims_trailing_blank_lines() {
    let (root, index) = indexed(
        "last",
        "## Alpha\n\n```sh\nalpha\n```\n\n## Beta\n\n```sh\nbeta\n```\n\n\n",
    );
    remove_by_slug(&index, "beta").expect("remove");
    assert_eq!(read(&root), "## Alpha\n\n```sh\nalpha\n```\n");
}

#[test]
fn removes_prose_and_extra_fences_written_after_the_body() {
    // The parsed range stops at the body's closing fence; everything up to the
    // next `##` still belongs to the section and must go with it.
    let (root, index) = indexed(
        "trailing",
        "## Alpha\n\n```sh\nalpha\n```\n\nA note about alpha.\n\n```text\n## not a heading\n```\n\n## Beta\n\n```sh\nbeta\n```\n",
    );
    remove_by_slug(&index, "alpha").expect("remove");
    assert_eq!(read(&root), "## Beta\n\n```sh\nbeta\n```\n");
}

#[test]
fn keeps_frontmatter_when_removing_a_snippet_below_it() {
    let (root, index) = indexed(
        "frontmatter",
        "---\nname: Demo\ntags: [demo]\n---\n\n## Alpha\n\n```sh\nalpha\n```\n\n## Beta\n\n```sh\nbeta\n```\n",
    );
    remove_by_slug(&index, "alpha").expect("remove");
    assert_eq!(
        read(&root),
        "---\nname: Demo\ntags: [demo]\n---\n\n## Beta\n\n```sh\nbeta\n```\n"
    );
}

#[test]
fn keeps_the_file_when_its_only_snippet_is_removed() {
    // An emptied file may still carry frontmatter or a title worth keeping, so
    // removal never deletes the file itself.
    let (root, index) = indexed(
        "only",
        "---\nname: Demo\n---\n\n# Title\n\n## Alpha\n\n```sh\nalpha\n```\n",
    );
    remove_by_slug(&index, "alpha").expect("remove");
    assert_eq!(read(&root), "---\nname: Demo\n---\n\n# Title\n");
}

#[test]
fn keeps_a_following_non_snippet_section() {
    // A `##` section holding only a `text` fence is not an executable snippet;
    // removing the snippet above it must not swallow it.
    let (root, index) = indexed(
        "non-snippet",
        "## Alpha\n\n```sh\nalpha\n```\n\n## Notes\n\n```text\njust an example\n```\n",
    );
    remove_by_slug(&index, "alpha").expect("remove");
    assert_eq!(read(&root), "## Notes\n\n```text\njust an example\n```\n");
}

#[test]
fn refuses_when_the_snippet_is_no_longer_in_the_file() {
    let (root, index) = indexed("stale", "## Alpha\n\n```sh\nalpha\n```\n");
    let id = SnippetId::new("snippets.md", "alpha");
    let snippet = index.get(&id).expect("snippet in index").clone();
    let replacement = "## Renamed\n\n```sh\nalpha\n```\n";
    fs::write(root.join("snippets.md"), replacement).expect("rewrite fixture");

    let err = remove_snippet(&snippet).expect_err("stale id should not be removed");
    assert!(err.to_string().contains("could not locate"), "{err}");
    assert_eq!(read(&root), replacement, "file must be left untouched");
}

#[test]
fn refuses_when_the_code_fence_is_unterminated() {
    // An unterminated fence yields no parsed range at all; refusing is the only
    // safe answer, since any guessed span could eat the rest of the file.
    let (root, index) = indexed("unterminated", "## Alpha\n\n```sh\nalpha\n```\n");
    let id = SnippetId::new("snippets.md", "alpha");
    let snippet = index.get(&id).expect("snippet in index").clone();
    let broken = "## Alpha\n\n```sh\nalpha\n";
    fs::write(root.join("snippets.md"), broken).expect("rewrite fixture");

    remove_snippet(&snippet).expect_err("unterminated fence should not be removed");
    assert_eq!(read(&root), broken, "file must be left untouched");
}

#[test]
fn leaves_no_temp_file_behind() {
    let (root, index) = indexed(
        "tempfile",
        "## Alpha\n\n```sh\nalpha\n```\n\n## Beta\n\n```sh\nbeta\n```\n",
    );
    remove_by_slug(&index, "alpha").expect("remove");
    let leftovers: Vec<_> = fs::read_dir(&root)
        .expect("read dir")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name != "snippets.md")
        .collect();
    assert!(leftovers.is_empty(), "unexpected files: {leftovers:?}");
}
