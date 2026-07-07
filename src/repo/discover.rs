//! Git repository discovery under snippet roots.

use crate::config::Paths;
use crate::discovery::IgnoreRules;
use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// A git repository discovered under a configured snippet root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnippetRepo {
    /// Absolute path of the repository working directory.
    pub path: PathBuf,
    /// The snippet root the repository was discovered under.
    pub root: PathBuf,
    /// Short label shown in the TUI: the path relative to its snippet root,
    /// or the root itself when the root is the repository.
    pub display: String,
    /// `true` when the repository is currently matched by `[paths] ignored`
    /// and therefore excluded from snippet discovery.
    pub hidden: bool,
    /// `true` for an actual git repository; `false` for a snippet root that has
    /// no git repository anywhere on or above it. Non-repo entries are surfaced
    /// only so they can be jumped into and hidden — sync/push/pull are inert.
    pub is_repo: bool,
}

impl SnippetRepo {
    /// The `ignored` entry `pb repo` writes to hide this repository: its path
    /// relative to the snippet root, or the absolute path when the repository
    /// *is* the root (a relative entry would be empty).
    pub fn ignore_entry(&self) -> String {
        match self.path.strip_prefix(&self.root) {
            Ok(rel) if !rel.as_os_str().is_empty() => rel.to_string_lossy().replace('\\', "/"),
            _ => self.path.to_string_lossy().replace('\\', "/"),
        }
    }
}

/// Find git repositories for the configured snippet roots: recursively below
/// each root (directories containing `.git`, including the root itself), plus
/// a best-effort upward scan to the nearest *enclosing* repository — a snippet
/// root is often a subdirectory of the repo that versions it, so the `.git`
/// marker sits above the root rather than under it.
///
/// Ignored/hidden paths are deliberately *not* skipped — hidden repositories
/// must stay visible in `pb repo` so they can be unhidden — but each repo is
/// flagged via [`SnippetRepo::hidden`]. Snippet roots with no git repository on
/// or above them are still surfaced as non-repo entries ([`SnippetRepo::is_repo`]
/// `= false`) so they can be jumped into. Results are deduplicated by canonical
/// path and sorted by display label.
pub fn discover_repos(paths: &Paths) -> io::Result<Vec<SnippetRepo>> {
    let ignored = IgnoreRules::new(paths.ignored.clone());
    let mut out = Vec::new();
    let mut visited = HashSet::new();
    let mut seen_repos = HashSet::new();
    for root in &paths.snippet_roots {
        if !root.is_dir() {
            continue;
        }
        let before = out.len();
        visit(
            root,
            root,
            &ignored,
            &mut out,
            &mut visited,
            &mut seen_repos,
        )?;
        scan_upward(root, &ignored, &mut out, &mut seen_repos);
        // No repository nested under, equal to, or enclosing this root: fall
        // back to the root itself so it stays reachable (jump only).
        if out.len() == before {
            let canonical = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
            if seen_repos.insert(canonical) {
                out.push(SnippetRepo {
                    path: root.to_path_buf(),
                    root: root.to_path_buf(),
                    display: root.to_string_lossy().replace('\\', "/"),
                    hidden: ignored.matches(root, root),
                    is_repo: false,
                });
            }
        }
    }
    out.sort_by(|a, b| a.display.cmp(&b.display));
    Ok(out)
}

/// Walk `root`'s ancestors and register the nearest one containing `.git`.
/// Only the closest enclosing repository is taken — repositories further up
/// govern the same snippet files through the same nested-repo boundary.
fn scan_upward(
    root: &Path,
    ignored: &IgnoreRules,
    out: &mut Vec<SnippetRepo>,
    seen_repos: &mut HashSet<PathBuf>,
) {
    // A root that is itself a repo is a nested-repo boundary: the enclosing
    // repository does not version the snippets, so don't surface it.
    if root.join(".git").exists() {
        return;
    }
    for ancestor in root.ancestors().skip(1) {
        if !ancestor.join(".git").exists() {
            continue;
        }
        let canonical = fs::canonicalize(ancestor).unwrap_or_else(|_| ancestor.to_path_buf());
        if seen_repos.insert(canonical) {
            out.push(SnippetRepo {
                path: ancestor.to_path_buf(),
                root: root.to_path_buf(),
                display: ancestor.to_string_lossy().replace('\\', "/"),
                hidden: ignored.matches(root, ancestor),
                is_repo: true,
            });
        }
        return;
    }
}

fn visit(
    root: &Path,
    dir: &Path,
    ignored: &IgnoreRules,
    out: &mut Vec<SnippetRepo>,
    visited: &mut HashSet<PathBuf>,
    seen_repos: &mut HashSet<PathBuf>,
) -> io::Result<()> {
    // Same symlink-cycle protection as snippet discovery.
    let canonical = fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    if !visited.insert(canonical.clone()) {
        return Ok(());
    }

    // `.git` may be a directory (normal repo) or a file (worktree/submodule).
    if dir.join(".git").exists() && seen_repos.insert(canonical) {
        let display = match dir.strip_prefix(root) {
            Ok(rel) if !rel.as_os_str().is_empty() => rel.to_string_lossy().replace('\\', "/"),
            _ => dir.to_string_lossy().replace('\\', "/"),
        };
        out.push(SnippetRepo {
            path: dir.to_path_buf(),
            root: root.to_path_buf(),
            display,
            hidden: ignored.matches(root, dir),
            is_repo: true,
        });
    }

    let mut entries: Vec<_> = fs::read_dir(dir)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let child = entry.path();
        if entry.file_name() == ".git" {
            continue;
        }
        let is_dir = entry
            .file_type()
            .map(|file_type| file_type.is_dir())
            .unwrap_or(false)
            || (fs::metadata(&child).map(|meta| meta.is_dir())).unwrap_or(false);
        if is_dir {
            visit(root, &child, ignored, out, visited, seen_repos)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(prefix: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};

        static NEXT: AtomicU64 = AtomicU64::new(1);
        let root = std::env::temp_dir().join(format!(
            "pb-repo-discover-{prefix}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn paths_for(root: &Path, ignored: Vec<String>) -> Paths {
        Paths {
            snippet_roots: vec![root.to_path_buf()],
            xdg_snippets_dir: root.to_path_buf(),
            snippet_overrides_active: false,
            ignored,
            state_file: root.join("state.tsv"),
            config_file: root.join("config.toml"),
        }
    }

    #[test]
    fn finds_nested_repos_and_root_repo() {
        let root = temp_root("nested");
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::create_dir_all(root.join("work/team-snippets/.git")).unwrap();
        fs::create_dir_all(root.join("plain-dir")).unwrap();
        // Worktree-style `.git` file.
        fs::create_dir_all(root.join("worktree")).unwrap();
        fs::write(root.join("worktree/.git"), "gitdir: elsewhere\n").unwrap();

        let repos = discover_repos(&paths_for(&root, Vec::new())).unwrap();
        let displays: Vec<&str> = repos.iter().map(|repo| repo.display.as_str()).collect();

        assert_eq!(repos.len(), 3);
        assert!(displays.contains(&"work/team-snippets"));
        assert!(displays.contains(&"worktree"));
        // The root repo displays as its own path.
        assert!(repos.iter().any(|repo| repo.path == root));
        assert!(repos.iter().all(|repo| !repo.hidden));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn hidden_repos_are_still_discovered_and_flagged() {
        let root = temp_root("hidden");
        fs::create_dir_all(root.join("visible/.git")).unwrap();
        fs::create_dir_all(root.join("secret/.git")).unwrap();

        // Keep the test hermetic: drop any enclosing repo the upward scan may
        // find above the temp dir (e.g. a stray /tmp/.git on the host).
        let repos: Vec<_> = discover_repos(&paths_for(&root, vec!["secret".to_string()]))
            .unwrap()
            .into_iter()
            .filter(|repo| repo.path.starts_with(&root))
            .collect();

        assert_eq!(repos.len(), 2);
        let secret = repos.iter().find(|repo| repo.display == "secret").unwrap();
        let visible = repos.iter().find(|repo| repo.display == "visible").unwrap();
        assert!(secret.hidden);
        assert!(!visible.hidden);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn ignore_entry_uses_relative_path_or_absolute_for_root_repo() {
        let root = temp_root("entry");
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::create_dir_all(root.join("sub/repo/.git")).unwrap();

        let repos = discover_repos(&paths_for(&root, Vec::new())).unwrap();
        let nested = repos
            .iter()
            .find(|repo| repo.display == "sub/repo")
            .unwrap();
        let root_repo = repos.iter().find(|repo| repo.path == root).unwrap();

        assert_eq!(nested.ignore_entry(), "sub/repo");
        assert_eq!(
            root_repo.ignore_entry(),
            root.to_string_lossy().replace('\\', "/")
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn upward_scan_finds_nearest_enclosing_repo() {
        let workspace = temp_root("upward");
        // outer/.git encloses outer/inner/snippets (the snippet root); the
        // nearer inner/.git must win over outer/.git.
        fs::create_dir_all(workspace.join("outer/.git")).unwrap();
        fs::create_dir_all(workspace.join("outer/inner/.git")).unwrap();
        let root = workspace.join("outer/inner/snippets");
        fs::create_dir_all(&root).unwrap();

        let repos = discover_repos(&paths_for(&root, Vec::new())).unwrap();

        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].path, workspace.join("outer/inner"));
        // Enclosing repos display and hide by absolute path.
        assert_eq!(
            repos[0].display,
            workspace
                .join("outer/inner")
                .to_string_lossy()
                .replace('\\', "/")
        );
        assert_eq!(repos[0].ignore_entry(), repos[0].display);

        let _ = fs::remove_dir_all(&workspace);
    }

    #[test]
    fn enclosing_repo_hidden_by_absolute_entry() {
        let workspace = temp_root("upward-hidden");
        fs::create_dir_all(workspace.join("outer/.git")).unwrap();
        let root = workspace.join("outer/snippets");
        fs::create_dir_all(&root).unwrap();
        let entry = workspace.join("outer").to_string_lossy().replace('\\', "/");

        let repos = discover_repos(&paths_for(&root, vec![entry])).unwrap();

        assert_eq!(repos.len(), 1);
        assert!(repos[0].hidden);

        let _ = fs::remove_dir_all(&workspace);
    }

    #[test]
    fn missing_roots_are_skipped() {
        let root = temp_root("missing");
        let mut paths = paths_for(&root, Vec::new());
        paths.snippet_roots.push(root.join("does-not-exist"));
        // The nonexistent root contributes nothing; the existing (repo-less)
        // root falls back to a single non-repo entry for itself.
        let under_root: Vec<_> = discover_repos(&paths)
            .unwrap()
            .into_iter()
            .filter(|repo| repo.path.starts_with(&root))
            .collect();
        assert_eq!(under_root.len(), 1);
        assert_eq!(under_root[0].path, root);
        assert!(!under_root[0].is_repo);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn repo_less_root_falls_back_to_non_repo_entry() {
        let workspace = temp_root("fallback");
        // A snippet root with no `.git` on it, under it, or above it (the
        // temp dir has no enclosing repo we control) — but guard anyway.
        let root = workspace.join("plain");
        fs::create_dir_all(root.join("sub")).unwrap();

        let repos: Vec<_> = discover_repos(&paths_for(&root, Vec::new()))
            .unwrap()
            .into_iter()
            .filter(|repo| repo.path.starts_with(&root))
            .collect();

        assert_eq!(repos.len(), 1);
        assert!(!repos[0].is_repo);
        assert_eq!(repos[0].path, root);
        assert_eq!(repos[0].display, root.to_string_lossy().replace('\\', "/"));

        let _ = fs::remove_dir_all(&workspace);
    }

    #[test]
    fn repo_bearing_root_has_no_non_repo_fallback() {
        let root = temp_root("no-fallback");
        fs::create_dir_all(root.join("has-repo/.git")).unwrap();

        let repos: Vec<_> = discover_repos(&paths_for(&root, Vec::new()))
            .unwrap()
            .into_iter()
            .filter(|repo| repo.path.starts_with(&root))
            .collect();

        // Only the nested repo; no synthetic non-repo entry for the root.
        assert_eq!(repos.len(), 1);
        assert!(repos.iter().all(|repo| repo.is_repo));

        let _ = fs::remove_dir_all(&root);
    }
}
