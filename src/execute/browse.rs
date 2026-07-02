use super::tree_picker::TreePicker;
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
    /// Total snippets contained at or beneath this node. Precomputed once at
    /// tree construction so render-time lookups stay O(1).
    pub recursive_count: usize,
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
        compute_recursive_counts(&mut root);
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

fn compute_recursive_counts(node: &mut DirNode) -> usize {
    let mut total = node.snippets.len();
    for child in node.children.values_mut() {
        total += compute_recursive_counts(child);
    }
    node.recursive_count = total;
    total
}

fn sort_snippets(node: &mut DirNode) {
    node.snippets.sort_by(|a, b| a.name.cmp(&b.name));
    for child in node.children.values_mut() {
        sort_snippets(child);
    }
}

/// A single row in the browse list. Sub-directories are listed first
/// (alphabetical), followed by snippets in the current node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowseEntry {
    /// A child directory that can be descended into.
    Directory(String),
    /// A snippet that can be selected.
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

/// Browse mode state: a [`TreePicker`] keyed by directory name. Each level
/// has its own filter buffer and remembered child, so going back up (Esc /
/// Backspace-at-empty) restores the cursor onto the directory you came from
/// without losing the parent's typed filter.
#[derive(Debug, Default)]
pub struct BrowseState {
    pub picker: TreePicker<String>,
}

impl BrowseState {
    pub fn new() -> Self {
        Self {
            picker: TreePicker::new(),
        }
    }

    pub fn path(&self) -> &[String] {
        self.picker.path()
    }

    pub fn input(&self) -> &str {
        self.picker.filter()
    }

    pub fn selection(&self) -> Option<usize> {
        self.picker.selection()
    }

    pub fn set_selection(&mut self, selection: Option<usize>) {
        self.picker.set_selection(selection);
    }

    pub fn path_display(&self) -> String {
        let path = self.picker.path();
        if path.is_empty() {
            "/".to_string()
        } else {
            format!("/{}/", path.join("/"))
        }
    }

    pub fn type_char(&mut self, c: char) {
        self.picker.type_char(c);
    }

    /// Backspace behaviour: first delete the typed input character by
    /// character; once input is empty, ascend one level and restore the
    /// cursor onto the directory we descended into. Returns `true` if
    /// anything changed.
    pub fn backspace(&mut self, tree: &BrowseTree) -> bool {
        if self.picker.input_backspace() {
            return true;
        }
        self.ascend(tree)
    }

    /// Pop one level off the path and restore the cursor onto the directory
    /// we just came from in the parent listing. Used by both Backspace (when
    /// the filter is empty) and Esc. Returns `false` at the root.
    pub fn ascend(&mut self, tree: &BrowseTree) -> bool {
        if self.picker.ascend().is_none() {
            return false;
        }
        let visible = self.visible(tree);
        self.picker
            .restore_selection(&visible, |entry| match entry {
                BrowseEntry::Directory(name) => Some(name.clone()),
                BrowseEntry::Snippet(_) => None,
            });
        true
    }

    pub fn reset(&mut self) {
        self.picker.reset();
    }

    /// Truncate `path` to the deepest still-existing prefix in `tree`. Used
    /// after an index rebuild that may have removed directories.
    pub fn trim_missing_path(&mut self, tree: &BrowseTree) {
        while !self.picker.path().is_empty() && tree.get(self.picker.path()).is_none() {
            self.picker.ascend();
        }
    }

    /// Entries visible at the current cursor, filtered by the typed input.
    /// Directory names match by case-insensitive prefix (matching
    /// tab-completion semantics); snippet names match by case-insensitive
    /// substring.
    pub fn visible(&self, tree: &BrowseTree) -> Vec<BrowseEntry> {
        let Some(node) = tree.get(self.picker.path()) else {
            return Vec::new();
        };
        let input_lc = self.picker.filter().to_lowercase();
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
    /// - One match: descend into it (filter is consumed by the new frame).
    /// - Many matches: extend input to the longest common prefix.
    pub fn tab_complete(&mut self, tree: &BrowseTree) -> bool {
        let Some(node) = tree.get(self.picker.path()) else {
            return false;
        };
        let input_lc = self.picker.filter().to_lowercase();
        let matches: Vec<&String> = node
            .children
            .keys()
            .filter(|n| n.to_lowercase().starts_with(&input_lc))
            .collect();
        match matches.len() {
            0 => false,
            1 => {
                let only = matches[0].clone();
                self.picker.descend(only);
                true
            }
            _ => {
                let lcp = longest_common_prefix(matches.iter().map(|s| s.as_str()));
                if lcp.len() > self.picker.filter().len() {
                    let frame = self.picker.current_mut();
                    frame.filter = lcp;
                    frame.cursor = frame.filter.len();
                    frame.selection = Some(0);
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
        let selected = self.picker.selection().unwrap_or(0);
        let entry = entries.get(selected)?;
        match entry {
            BrowseEntry::Directory(name) => {
                self.picker.descend(name.clone());
                None
            }
            BrowseEntry::Snippet(s) => Some(s.id.clone()),
        }
    }

    pub fn move_cursor(&mut self, delta: i32, visible_len: usize) {
        self.picker.move_selection(delta, visible_len);
    }

    /// Test/setup helper: forcibly set the path by descending into each
    /// segment in turn (no remembered-child markers). Resets the picker
    /// first so the resulting state is `path = segments`, empty filter at
    /// each level.
    pub fn set_path(&mut self, segments: Vec<String>) {
        self.picker.reset();
        for segment in segments {
            self.picker.descend(segment);
        }
        // descend leaves a `descended_from` on each non-root frame; clear
        // them so an ascent from this synthetic state falls back to 0
        // rather than chasing a phantom child.
        for frame in self.picker.frames_mut() {
            frame.descended_from = None;
        }
    }

    /// Test/setup helper: overwrite the typed filter at the current level.
    pub fn set_input(&mut self, input: String) {
        let frame = self.picker.current_mut();
        frame.cursor = input.len();
        frame.filter = input;
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
    use crate::domain::{Frontmatter, Snippet, SnippetFile};
    use crate::index::SnippetIndex;
    use std::path::PathBuf;

    fn snippet_file(rel: &str, names: &[&str]) -> SnippetFile {
        SnippetFile {
            path: PathBuf::from("/fixtures").join(rel),
            relative_path: PathBuf::from(rel),
            frontmatter: Frontmatter::default(),
            snippets: names
                .iter()
                .map(|name| Snippet {
                    id: SnippetId::new(rel, &name.to_lowercase().replace(' ', "-")),
                    name: (*name).to_string(),
                    description: String::new(),
                    body: format!("echo {name}"),
                    variables: Vec::new(),
                    language: Some("bash".to_string()),
                })
                .collect(),
        }
    }

    fn fixture_tree() -> BrowseTree {
        let index = SnippetIndex::from_files([
            snippet_file("simple/snippets.md", &["Simple fixture"]),
            snippet_file("complex/complex.md", &["Complex fixture"]),
            snippet_file("nested/root.md", &["Nested root fixture"]),
            snippet_file(
                "nested/docker/docker.md",
                &["Run fixture container", "View fixture logs"],
            ),
            snippet_file("nested/docker/compose/snip.md", &["Start fixture services"]),
            snippet_file("nested/docker/images/images.md", &["List fixture images"]),
            snippet_file("nested/grep/grep.md", &["Search fixture pattern"]),
        ]);
        BrowseTree::from_index(&index)
    }

    #[test]
    fn tree_mirrors_fixture_directories() {
        let tree = fixture_tree();
        let root = tree.root();
        let dirs: Vec<&String> = root.children.keys().collect();
        assert!(dirs.iter().any(|s| s.as_str() == "nested"));
        assert!(dirs.iter().any(|s| s.as_str() == "simple"));
        assert!(dirs.iter().any(|s| s.as_str() == "complex"));
    }

    #[test]
    fn nested_docker_has_compose_images_and_file_subdirs() {
        let tree = fixture_tree();
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
    fn recursive_count_sums_descendants_and_matches_file_snippet_count() {
        let tree = fixture_tree();
        // File node count equals its snippet count.
        let file = tree
            .get(&["nested".into(), "docker".into(), "docker.md".into()])
            .expect("docker.md");
        assert_eq!(file.recursive_count, file.snippets.len());
        assert!(file.recursive_count > 0);
        // Directory count equals sum of descendants.
        let docker = tree.get(&["nested".into(), "docker".into()]).unwrap();
        let descendant_total: usize = docker
            .children
            .values()
            .map(|c| c.recursive_count)
            .sum::<usize>()
            + docker.snippets.len();
        assert_eq!(docker.recursive_count, descendant_total);
        assert!(docker.recursive_count >= file.recursive_count);
        // Root totals all snippets in tree.
        let mut walk_total = 0usize;
        fn walk(n: &DirNode, total: &mut usize) {
            *total += n.snippets.len();
            for c in n.children.values() {
                walk(c, total);
            }
        }
        walk(tree.root(), &mut walk_total);
        assert_eq!(tree.root().recursive_count, walk_total);
    }

    #[test]
    fn file_node_carries_its_snippets() {
        let tree = fixture_tree();
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
        let tree = fixture_tree();
        let mut state = BrowseState::new();
        state.type_char('s');
        let changed = state.tab_complete(&tree);
        assert!(changed);
        assert_eq!(state.path(), &["simple".to_string()][..]);
        assert!(state.input().is_empty());
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
        assert_eq!(state.input(), "git");
        assert!(state.path().is_empty());
    }

    #[test]
    fn backspace_empties_input_then_climbs_path() {
        let tree = fixture_tree();
        let mut state = BrowseState::new();
        state.set_path(vec!["nested".into(), "docker".into()]);
        state.type_char('c');
        state.type_char('o');
        assert!(state.backspace(&tree));
        assert_eq!(state.input(), "c");
        assert!(state.backspace(&tree));
        assert_eq!(state.input(), "");
        assert!(state.backspace(&tree));
        assert_eq!(state.path(), &["nested".to_string()][..]);
        assert!(state.backspace(&tree));
        assert!(state.path().is_empty());
        assert!(!state.backspace(&tree));
    }

    #[test]
    fn visible_filters_directories_by_prefix() {
        let tree = fixture_tree();
        let mut state = BrowseState::new();
        state.set_path(vec!["nested".into()]);
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
        let tree = fixture_tree();
        let mut state = BrowseState::new();
        state.set_selection(Some(0));
        let id = state.activate(&tree);
        assert!(id.is_none(), "activating a directory returns no snippet id");
        assert_eq!(state.path().len(), 1);
    }

    #[test]
    fn ascend_restores_cursor_onto_descended_child() {
        // Build a tree with several siblings so "first/last by accident"
        // can't pass the test.
        let mut root = DirNode::default();
        root.children.insert("alpha".into(), DirNode::default());
        root.children.insert("docker".into(), DirNode::default());
        root.children.insert("nested".into(), DirNode::default());
        let tree = BrowseTree { root };

        let mut state = BrowseState::new();
        // descend into "docker" via activate, after positioning the cursor
        let visible = state.visible(&tree);
        let docker_idx = visible
            .iter()
            .position(|e| matches!(e, BrowseEntry::Directory(n) if n == "docker"))
            .unwrap();
        state.set_selection(Some(docker_idx));
        assert!(state.activate(&tree).is_none());
        assert_eq!(state.path(), &["docker".to_string()][..]);

        // Now ascend (Backspace at empty filter); cursor must land on "docker".
        assert!(state.backspace(&tree));
        let visible_after = state.visible(&tree);
        let landed = state.selection().expect("selection present");
        assert!(matches!(&visible_after[landed], BrowseEntry::Directory(n) if n == "docker"));
    }

    #[test]
    fn ascend_preserves_parent_filter_text() {
        let mut root = DirNode::default();
        root.children.insert("alpha".into(), DirNode::default());
        root.children.insert("docker".into(), DirNode::default());
        let tree = BrowseTree { root };

        let mut state = BrowseState::new();
        state.type_char('d');
        // Tab-complete descends into "docker" since it is the unique match.
        assert!(state.tab_complete(&tree));
        assert_eq!(state.path(), &["docker".to_string()][..]);

        // Ascend; the parent's "d" must come back, and cursor must be on "docker".
        assert!(state.backspace(&tree));
        assert_eq!(state.input(), "d");
        let visible = state.visible(&tree);
        let landed = state.selection().expect("selection present");
        assert!(matches!(&visible[landed], BrowseEntry::Directory(n) if n == "docker"));
    }

    #[test]
    fn ascend_falls_back_to_zero_when_child_missing() {
        let mut root = DirNode::default();
        root.children.insert("alpha".into(), DirNode::default());
        root.children.insert("gone".into(), DirNode::default());
        let mut tree = BrowseTree { root };

        let mut state = BrowseState::new();
        let idx = state
            .visible(&tree)
            .iter()
            .position(|e| matches!(e, BrowseEntry::Directory(n) if n == "gone"))
            .unwrap();
        state.set_selection(Some(idx));
        assert!(state.activate(&tree).is_none());

        // Remove the child between descend and ascend.
        tree.root.children.remove("gone");

        assert!(state.backspace(&tree));
        assert_eq!(state.selection(), Some(0));
    }
}
