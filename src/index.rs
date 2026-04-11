use crate::config::Paths;
use crate::discovery::discover_markdown_files;
use crate::domain::{Frontmatter, Snippet, SnippetFile, SnippetId};
use crate::parser::parse_file;
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedSnippet {
    pub path: PathBuf,
    pub snippet: Snippet,
    pub relative_path: PathBuf,
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
    pub fn relative_path_display(&self) -> String {
        self.relative_path.to_string_lossy().replace('\\', "/")
    }
}

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
}

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
}
