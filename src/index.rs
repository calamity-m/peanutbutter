use crate::config::Paths;
use crate::discovery::discover_markdown_files;
use crate::domain::{Frontmatter, Snippet, SnippetFile, SnippetId};
use crate::parser::parse_file;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Key used for grouping snippets in the tags picker.
///
/// `Untagged` is a distinct variant so a user tag cannot collide with the
/// synthetic untagged bucket. `BTreeMap` ordering is used for display; render
/// code places the synthetic bucket after stored tags.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TagKey {
    /// A tag value parsed from snippet-file frontmatter.
    Tag(String),
    /// Synthetic bucket for snippets whose file has no tags.
    Untagged,
}

/// A snippet enriched with the file-level context needed for search and display.
///
/// Combines the parsed [`Snippet`] with its absolute path, relative path, and
/// the file's [`Frontmatter`] (tags, file-level description, etc.).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedSnippet {
    /// Absolute path to the source `.md` file.
    pub path: PathBuf,
    /// The parsed snippet (id, name, body, variables, …).
    pub snippet: Snippet,
    /// Path relative to the snippet root — used in search scoring and display.
    pub relative_path: PathBuf,
    /// File-level frontmatter shared by all snippets in this file.
    pub frontmatter: Frontmatter,
}

impl IndexedSnippet {
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn id(&self) -> &SnippetId {
        &self.snippet.id
    }
    pub fn name(&self) -> &str {
        &self.snippet.name
    }
    pub fn body(&self) -> &str {
        &self.snippet.body
    }
    pub fn description(&self) -> &str {
        &self.snippet.description
    }
    pub fn tags(&self) -> &[String] {
        &self.frontmatter.tags
    }
    pub fn language(&self) -> Option<&str> {
        self.snippet.language.as_deref()
    }
    pub fn relative_path_display(&self) -> String {
        self.relative_path.to_string_lossy().replace('\\', "/")
    }
}

/// In-memory collection of all parsed snippets, with O(1) lookup by id.
///
/// Insertion order is preserved; duplicate ids (same relative path + slug
/// across multiple roots) are silently dropped — the first one wins.
#[derive(Debug, Default, Clone)]
pub struct SnippetIndex {
    entries: Vec<IndexedSnippet>,
    by_id: HashMap<SnippetId, usize>,
}

impl SnippetIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_files<I: IntoIterator<Item = SnippetFile>>(files: I) -> Self {
        let mut out = Self::new();
        for file in files {
            out.insert_file(file);
        }
        out
    }

    pub fn insert_file(&mut self, file: SnippetFile) {
        let SnippetFile {
            path,
            relative_path,
            frontmatter,
            snippets,
        } = file;
        for snippet in snippets {
            if self.by_id.contains_key(&snippet.id) {
                continue;
            }
            let entry = IndexedSnippet {
                path: path.clone(),
                snippet,
                relative_path: relative_path.clone(),
                frontmatter: frontmatter.clone(),
            };
            let pos = self.entries.len();
            self.by_id.insert(entry.snippet.id.clone(), pos);
            self.entries.push(entry);
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &IndexedSnippet> {
        self.entries.iter()
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn get(&self, id: &SnippetId) -> Option<&IndexedSnippet> {
        self.by_id.get(id).map(|i| &self.entries[*i])
    }
    pub fn as_slice(&self) -> &[IndexedSnippet] {
        &self.entries
    }
    /// Group snippets by their file-level frontmatter tags.
    ///
    /// Snippets with multiple tags are included once in each tag bucket.
    /// Snippets with no tags are included in the synthetic `Untagged` bucket.
    pub fn tag_index(&self) -> BTreeMap<TagKey, Vec<SnippetId>> {
        let mut tags = BTreeMap::new();
        for entry in &self.entries {
            if entry.tags().is_empty() {
                tags.entry(TagKey::Untagged)
                    .or_insert_with(Vec::new)
                    .push(entry.id().clone());
                continue;
            }
            for tag in entry.tags() {
                tags.entry(TagKey::Tag(tag.clone()))
                    .or_insert_with(Vec::new)
                    .push(entry.id().clone());
            }
        }
        tags
    }
}

/// Walk each root directory, parse every `.md` file found, and return a
/// populated [`SnippetIndex`]. Missing roots are silently skipped.
pub fn load_from_roots(roots: &[PathBuf]) -> io::Result<SnippetIndex> {
    let mut index = SnippetIndex::new();
    for root in roots {
        for file_path in discover_markdown_files(root)? {
            let content = fs::read_to_string(&file_path)?;
            let parsed = parse_file(&file_path, root, &content);
            index.insert_file(parsed);
        }
    }
    Ok(index)
}

/// Load a [`SnippetIndex`] from the paths resolved by [`crate::config::default_paths`].
pub fn load_default() -> io::Result<SnippetIndex> {
    let paths: Paths = crate::config::default_paths();
    load_from_roots(&paths.snippet_roots)
}

#[allow(dead_code)]
fn _path_is_absolute(p: &Path) -> bool {
    p.is_absolute()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn examples_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples")
    }

    #[test]
    fn loads_examples_with_expected_snippet_count() {
        let index = load_from_roots(&[examples_root()]).expect("load");
        assert!(!index.is_empty());
        assert!(index.len() >= 10, "got only {} snippets", index.len());
    }

    #[test]
    fn duplicate_ids_are_deduped_first_wins() {
        use crate::domain::{Snippet, SnippetFile};
        let mut index = SnippetIndex::new();
        let file_a = SnippetFile {
            path: PathBuf::from("/a/x.md"),
            relative_path: PathBuf::from("x.md"),
            frontmatter: Frontmatter::default(),
            snippets: vec![Snippet {
                id: SnippetId::new("x.md", "echo"),
                name: "Echo".into(),
                description: "first".into(),
                body: "echo a".into(),
                variables: vec![],
                language: None,
            }],
        };
        let file_b = SnippetFile {
            path: PathBuf::from("/b/x.md"),
            relative_path: PathBuf::from("x.md"),
            frontmatter: Frontmatter::default(),
            snippets: vec![Snippet {
                id: SnippetId::new("x.md", "echo"),
                name: "Echo".into(),
                description: "second".into(),
                body: "echo b".into(),
                variables: vec![],
                language: None,
            }],
        };
        index.insert_file(file_a);
        index.insert_file(file_b);
        assert_eq!(index.len(), 1);
        assert_eq!(index.iter().next().unwrap().description(), "first");
    }

    #[test]
    fn get_by_id_round_trips() {
        let index = load_from_roots(&[examples_root()]).unwrap();
        let first = index.iter().next().unwrap();
        let id = first.id().clone();
        assert_eq!(index.get(&id).unwrap().name(), first.name());
    }

    #[test]
    fn tag_index_groups_by_tag_and_untagged_bucket() {
        use crate::domain::{Snippet, SnippetFile};

        fn file(rel: &str, tags: &[&str], slug: &str) -> SnippetFile {
            SnippetFile {
                path: PathBuf::from(rel),
                relative_path: PathBuf::from(rel),
                frontmatter: Frontmatter {
                    tags: tags.iter().map(|tag| tag.to_string()).collect(),
                    ..Default::default()
                },
                snippets: vec![Snippet {
                    id: SnippetId::new(rel, slug),
                    name: slug.to_string(),
                    description: String::new(),
                    body: "echo hi".to_string(),
                    variables: vec![],
                    language: None,
                }],
            }
        }

        let index = SnippetIndex::from_files([
            file("git.md", &["git", "shell"], "one"),
            file("literal.md", &["__pb_untagged__"], "two"),
            file("none.md", &[], "three"),
        ]);
        let tags = index.tag_index();

        assert_eq!(tags[&TagKey::Tag("git".to_string())].len(), 1);
        assert_eq!(tags[&TagKey::Tag("shell".to_string())].len(), 1);
        assert_eq!(tags[&TagKey::Tag("__pb_untagged__".to_string())].len(), 1);
        assert_eq!(tags[&TagKey::Untagged].len(), 1);
    }
}
