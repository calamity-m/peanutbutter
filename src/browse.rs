use crate::domain::SnippetId;
use crate::index::SnippetIndex;
use std::collections::BTreeMap;
use std::path::{Component, Path};

/// A single snippet as displayed in browse mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowseSnippet {
    pub name: String,
    pub id: SnippetId,
}

/// A directory node in the browse tree. Sub-directories are kept in a
/// `BTreeMap` so iteration is alphabetical without an explicit sort step.
#[derive(Debug, Default, Clone)]
pub struct DirNode {
    pub children: BTreeMap<String, DirNode>,
    pub snippets: Vec<BrowseSnippet>,
}

/// The full browse tree: a root `DirNode` whose shape mirrors the directory
/// layout of snippet files. Files themselves are hidden — each snippet is
/// attached directly to the directory its source file lives in, matching
/// the README's tree example.
#[derive(Debug, Clone)]
pub struct BrowseTree {
    root: DirNode,
}

impl BrowseTree {
    pub fn from_index(index: &SnippetIndex) -> Self {
        let mut root = DirNode::default();
        for entry in index.iter() {
            let components = file_path_components(&entry.relative_path);
            let dir_node = ensure_dir(&mut root, &components);
            dir_node.snippets.push(BrowseSnippet {
                name: entry.name().to_string(),
                id: entry.id().clone(),
            });
        }
        sort_snippets(&mut root);
        BrowseTree { root }
    }

    pub fn root(&self) -> &DirNode {
        &self.root
    }

    pub fn get(&self, path: &[String]) -> Option<&DirNode> {
        let mut cur = &self.root;
        for p in path {
            cur = cur.children.get(p)?;
        }
        Some(cur)
    }
}

fn file_path_components(rel: &Path) -> Vec<String> {
    rel.components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect()
}

fn ensure_dir<'a>(root: &'a mut DirNode, path: &[String]) -> &'a mut DirNode {
    let mut cur = root;
    for p in path {
        cur = cur.children.entry(p.clone()).or_default();
    }
    cur
}

fn sort_snippets(node: &mut DirNode) {
    node.snippets.sort_by(|a, b| a.name.cmp(&b.name));
    for child in node.children.values_mut() {
        sort_snippets(child);
    }
}

/// What we render inside a directory: sub-dirs first (alphabetical), then
/// the snippets that live directly under that directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowseEntry {
    Directory(String),
    Snippet(BrowseSnippet),
}

impl BrowseEntry {
    pub fn display_name(&self) -> &str {
        match self {
            BrowseEntry::Directory(name) => name,
            BrowseEntry::Snippet(s) => &s.name,
        }
    }
}

/// Browse mode state: the path from the root to the current directory, the
/// typed input used for filtering and tab-completion, and the selected list
/// index. `path` is analogous to `cwd`: typing narrows visible children,
/// `tab` descends into a unique matching directory, and `backspace` walks back
/// out.
#[derive(Debug, Default)]
pub struct BrowseState {
    pub path: Vec<String>,
    pub input: String,
    pub selection: Option<usize>,
}

impl BrowseState {
    pub fn new() -> Self {
        Self {
            path: Vec::new(),
            input: String::new(),
            selection: Some(0),
        }
    }

    pub fn path_display(&self) -> String {
        if self.path.is_empty() {
            "/".to_string()
        } else {
            format!("/{}/", self.path.join("/"))
        }
    }

    pub fn type_char(&mut self, c: char) {
        self.input.push(c);
        self.selection = Some(0);
    }

    /// Backspace behaviour: first delete the typed input character by
    /// character; once input is empty, pop a directory off the path. This
    /// means holding backspace walks back out of nested directories in a
    /// single predictable motion.
    pub fn backspace(&mut self) -> bool {
        if self.input.pop().is_some() {
            self.selection = Some(0);
            return true;
        }
        if self.path.pop().is_some() {
            self.selection = Some(0);
            return true;
        }
        false
    }

    pub fn reset(&mut self) {
        self.path.clear();
        self.input.clear();
        self.selection = Some(0);
    }

    /// Entries visible at the current cursor, filtered by `input`. Directory
    /// names match by case-insensitive prefix (matching tab-completion
    /// semantics); snippet names match by case-insensitive substring.
    pub fn visible(&self, tree: &BrowseTree) -> Vec<BrowseEntry> {
        let Some(node) = tree.get(&self.path) else {
            return Vec::new();
        };
        let input_lc = self.input.to_lowercase();
        let mut out = Vec::new();
        for name in node.children.keys() {
            if input_lc.is_empty() || name.to_lowercase().starts_with(&input_lc) {
                out.push(BrowseEntry::Directory(name.clone()));
            }
        }
        for s in &node.snippets {
            if input_lc.is_empty() || s.name.to_lowercase().contains(&input_lc) {
                out.push(BrowseEntry::Snippet(s.clone()));
            }
        }
        out
    }

    /// Tab-complete against directory names at the current level. Returns
    /// `true` if state changed.
    ///
    /// - Zero matches: no-op.
    /// - One match: descend into it, clear input.
    /// - Many matches: extend input to the longest common prefix.
    pub fn tab_complete(&mut self, tree: &BrowseTree) -> bool {
        let Some(node) = tree.get(&self.path) else {
            return false;
        };
        let input_lc = self.input.to_lowercase();
        let matches: Vec<&String> = node
            .children
            .keys()
            .filter(|n| n.to_lowercase().starts_with(&input_lc))
            .collect();
        match matches.len() {
            0 => false,
            1 => {
                let only = matches[0].clone();
                self.path.push(only);
                self.input.clear();
                self.selection = Some(0);
                true
            }
            _ => {
                let lcp = longest_common_prefix(matches.iter().map(|s| s.as_str()));
                if lcp.len() > self.input.len() {
                    self.input = lcp;
                    self.selection = Some(0);
                    true
                } else {
                    false
                }
            }
        }
    }

    /// Activate the currently selected entry: directories are descended
    /// into, snippets return their id for the caller to handle.
    pub fn activate(&mut self, tree: &BrowseTree) -> Option<SnippetId> {
        let entries = self.visible(tree);
        let selected = self.selection.unwrap_or(0);
        let entry = entries.get(selected)?;
        match entry {
            BrowseEntry::Directory(name) => {
                self.path.push(name.clone());
                self.input.clear();
                self.selection = Some(0);
                None
            }
            BrowseEntry::Snippet(s) => Some(s.id.clone()),
        }
    }

    pub fn move_cursor(&mut self, delta: i32, visible_len: usize) {
        if visible_len == 0 {
            self.selection = None;
            return;
        }
        let current = self.selection.unwrap_or(0) as i32;
        let next = (current + delta).clamp(0, visible_len as i32 - 1);
        self.selection = Some(next as usize);
    }
}

fn longest_common_prefix<'a, I: Iterator<Item = &'a str>>(mut iter: I) -> String {
    let Some(first) = iter.next() else {
        return String::new();
    };
    let mut prefix: String = first.to_string();
    for s in iter {
        let match_chars = prefix
            .chars()
            .zip(s.chars())
            .take_while(|(a, b)| a.to_lowercase().eq(b.to_lowercase()))
            .count();
        let byte_len: usize = prefix.chars().take(match_chars).map(char::len_utf8).sum();
        prefix.truncate(byte_len);
        if prefix.is_empty() {
            break;
        }
    }
    prefix
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::load_from_roots;
    use std::path::PathBuf;

    fn examples_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples")
    }

    fn example_tree() -> BrowseTree {
        let index = load_from_roots(&[examples_root()]).unwrap();
        BrowseTree::from_index(&index)
    }

    #[test]
    fn tree_mirrors_example_directories() {
        let tree = example_tree();
        let root = tree.root();
        let dirs: Vec<&String> = root.children.keys().collect();
        assert!(dirs.iter().any(|s| s.as_str() == "nested"));
        assert!(dirs.iter().any(|s| s.as_str() == "simple"));
        assert!(dirs.iter().any(|s| s.as_str() == "complex"));
    }

    #[test]
    fn nested_docker_has_compose_images_and_file_subdirs() {
        let tree = example_tree();
        let docker = tree
            .get(&["nested".into(), "docker".into()])
            .expect("docker dir");
        let subdirs: Vec<&String> = docker.children.keys().collect();
        assert!(subdirs.iter().any(|s| s.as_str() == "compose"));
        assert!(subdirs.iter().any(|s| s.as_str() == "images"));
        assert!(subdirs.iter().any(|s| s.as_str() == "docker.md"));
        assert!(
            docker.snippets.is_empty(),
            "snippets now live under the docker.md file node, not the directory"
        );
    }

    #[test]
    fn file_node_carries_its_snippets() {
        let tree = example_tree();
        let file = tree
            .get(&["nested".into(), "docker".into(), "docker.md".into()])
            .expect("docker.md file node");
        assert!(
            file.children.is_empty(),
            "a file node has no nested children"
        );
        assert!(
            !file.snippets.is_empty(),
            "docker.md's snippets attach to its file node"
        );
    }

    #[test]
    fn tab_unique_prefix_descends_into_directory() {
        let tree = example_tree();
        let mut state = BrowseState::new();
        state.type_char('s');
        let changed = state.tab_complete(&tree);
        assert!(changed);
        assert_eq!(state.path, vec!["simple".to_string()]);
        assert!(state.input.is_empty());
    }

    #[test]
    fn tab_extends_to_longest_common_prefix_when_ambiguous() {
        let mut root = DirNode::default();
        root.children.insert("gitlog".into(), DirNode::default());
        root.children.insert("gitfoo".into(), DirNode::default());
        let tree = BrowseTree { root };
        let mut state = BrowseState::new();
        state.type_char('g');
        let changed = state.tab_complete(&tree);
        assert!(changed);
        assert_eq!(state.input, "git");
        assert!(state.path.is_empty());
    }

    #[test]
    fn backspace_empties_input_then_climbs_path() {
        let tree = example_tree();
        let mut state = BrowseState::new();
        state.path.push("nested".into());
        state.path.push("docker".into());
        state.type_char('c');
        state.type_char('o');
        assert!(state.backspace());
        assert_eq!(state.input, "c");
        assert!(state.backspace());
        assert_eq!(state.input, "");
        assert!(state.backspace());
        assert_eq!(state.path, vec!["nested".to_string()]);
        assert!(state.backspace());
        assert!(state.path.is_empty());
        assert!(!state.backspace());
        let _ = tree; // silence unused in this branch
    }

    #[test]
    fn visible_filters_directories_by_prefix() {
        let tree = example_tree();
        let mut state = BrowseState::new();
        state.path.push("nested".into());
        state.type_char('d');
        let names: Vec<String> = state
            .visible(&tree)
            .into_iter()
            .map(|e| e.display_name().to_string())
            .collect();
        assert!(names.contains(&"docker".to_string()));
        assert!(!names.contains(&"grep".to_string()));
    }

    #[test]
    fn activate_descends_into_directory_entries() {
        let tree = example_tree();
        let mut state = BrowseState::new();
        // First child at root should be a directory entry given example corpus.
        state.selection = Some(0);
        let id = state.activate(&tree);
        assert!(id.is_none(), "activating a directory returns no snippet id");
        assert_eq!(state.path.len(), 1);
    }
}
