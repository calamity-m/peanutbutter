use crate::glob::glob_matches;
use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Config-driven ignore rules applied while walking snippet roots.
///
/// Each pattern is a glob (`*` / `?`, see [`crate::glob`]) matched against two
/// slash-normalized spellings of every directory and file the walk encounters:
/// its path relative to the snippet root being walked, and its absolute path.
/// A pattern that matches a directory prunes the entire subtree, which is how
/// `pb repo` hides whole snippet repositories.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IgnoreRules {
    patterns: Vec<String>,
}

impl IgnoreRules {
    /// Build rules from raw config patterns. Trailing slashes are trimmed so
    /// `dir/` and `dir` hide the same directory.
    pub fn new<I, S>(patterns: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            patterns: patterns
                .into_iter()
                .map(|pattern| {
                    let pattern: String = pattern.into();
                    pattern.trim_end_matches('/').to_string()
                })
                .filter(|pattern| !pattern.is_empty())
                .collect(),
        }
    }

    /// Returns `true` when no patterns are configured.
    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }

    /// Returns `true` if `path` (a directory or file under `root`) is matched
    /// by any pattern, either by its root-relative path or its absolute path.
    ///
    /// Only the path itself is tested — ancestors are not. Directory pruning
    /// happens naturally during the walk because every ancestor directory is
    /// itself visited and tested.
    pub fn matches(&self, root: &Path, path: &Path) -> bool {
        if self.patterns.is_empty() {
            return false;
        }
        let relative = path
            .strip_prefix(root)
            .ok()
            .filter(|rel| !rel.as_os_str().is_empty())
            .map(normalize_slashes);
        let absolute = normalize_slashes(path);
        self.patterns.iter().any(|pattern| {
            relative
                .as_deref()
                .is_some_and(|rel| glob_matches(pattern, rel))
                || glob_matches(pattern, &absolute)
        })
    }
}

fn normalize_slashes(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Recursively find all `.md` / `.markdown` files under `root`, sorted
/// alphabetically. Symlinks to directories are followed; symlink cycles are
/// detected via canonical paths and skipped. Returns an empty vec if `root`
/// does not exist.
pub fn discover_markdown_files(root: &Path) -> io::Result<Vec<PathBuf>> {
    discover_markdown_files_ignoring(root, &IgnoreRules::default())
}

/// Like [`discover_markdown_files`] but skipping directories and files matched
/// by `ignored`.
pub fn discover_markdown_files_ignoring(
    root: &Path,
    ignored: &IgnoreRules,
) -> io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    if !root.exists() {
        return Ok(out);
    }
    let mut visited = HashSet::new();
    visit(root, root, ignored, &mut out, &mut visited)?;
    out.sort();
    Ok(out)
}

/// Like [`discover_markdown_files`] but over multiple roots, deduplicating
/// directories shared between roots (e.g. via symlinks) so each file appears
/// at most once. Files from each root are sorted before appending.
pub fn discover_all<I, P>(roots: I) -> io::Result<Vec<PathBuf>>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    discover_all_ignoring(roots, &IgnoreRules::default())
}

/// Like [`discover_all`] but skipping directories and files matched by
/// `ignored`.
pub fn discover_all_ignoring<I, P>(roots: I, ignored: &IgnoreRules) -> io::Result<Vec<PathBuf>>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    // Single visited set across roots: if two configured roots both contain
    // (or symlink to) the same directory, walk it only once.
    let mut out = Vec::new();
    let mut visited = HashSet::new();
    for root in roots {
        let root = root.as_ref();
        if !root.exists() {
            continue;
        }
        let start = out.len();
        visit(root, root, ignored, &mut out, &mut visited)?;
        out[start..].sort();
    }
    Ok(out)
}

fn visit(
    root: &Path,
    path: &Path,
    ignored: &IgnoreRules,
    out: &mut Vec<PathBuf>,
    visited: &mut HashSet<PathBuf>,
) -> io::Result<()> {
    if ignored.matches(root, path) {
        return Ok(());
    }
    let meta = fs::metadata(path)?;
    if meta.is_file() {
        if is_markdown(path) {
            out.push(path.to_path_buf());
        }
        return Ok(());
    }
    if !meta.is_dir() {
        return Ok(());
    }
    // Track canonical directory paths so a symlink loop (e.g. dir/self -> dir)
    // can't recurse forever. Symlinks to *new* directories are still followed.
    let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if !visited.insert(canonical) {
        return Ok(());
    }
    let mut entries: Vec<_> = fs::read_dir(path)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let child = entry.path();
        if ignored.matches(root, &child) {
            continue;
        }
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            visit(root, &child, ignored, out, visited)?;
        } else if file_type.is_symlink() {
            // file_type() does not follow links; resolve via metadata() to
            // decide whether the target is a dir we should recurse into.
            match fs::metadata(&child) {
                Ok(target_meta) if target_meta.is_dir() => {
                    visit(root, &child, ignored, out, visited)?
                }
                Ok(target_meta) if target_meta.is_file() && is_markdown(&child) => {
                    out.push(child);
                }
                _ => {}
            }
        } else if file_type.is_file() && is_markdown(&child) {
            out.push(child);
        }
    }
    Ok(())
}

fn is_markdown(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("md") | Some("markdown")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(prefix: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};

        static NEXT: AtomicU64 = AtomicU64::new(1);
        let root = std::env::temp_dir().join(format!(
            "pb-discovery-{prefix}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn discovers_markdown_files_in_nested_tree() {
        let root = temp_root("nested");
        fs::create_dir_all(root.join("alpha/deep")).unwrap();
        fs::create_dir_all(root.join("beta")).unwrap();
        fs::write(root.join("root.md"), "# root\n").unwrap();
        fs::write(root.join("alpha/one.markdown"), "# one\n").unwrap();
        fs::write(root.join("alpha/deep/two.md"), "# two\n").unwrap();
        fs::write(root.join("beta/ignored.txt"), "not markdown\n").unwrap();

        let files = discover_markdown_files(&root).expect("discovery");
        let names: Vec<String> = files
            .iter()
            .map(|p| {
                p.strip_prefix(&root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        assert_eq!(
            names,
            vec![
                "alpha/deep/two.md".to_string(),
                "alpha/one.markdown".to_string(),
                "root.md".to_string(),
            ]
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn ignored_patterns_skip_files_directories_and_globs() {
        let root = temp_root("ignored");
        fs::create_dir_all(root.join("keep")).unwrap();
        fs::create_dir_all(root.join("hidden-repo/nested")).unwrap();
        fs::write(root.join("keep/a.md"), "# a\n").unwrap();
        fs::write(root.join("keep/generated.md"), "# gen\n").unwrap();
        fs::write(root.join("hidden-repo/b.md"), "# b\n").unwrap();
        fs::write(root.join("hidden-repo/nested/c.md"), "# c\n").unwrap();

        let names = |ignored: &IgnoreRules| -> Vec<String> {
            discover_markdown_files_ignoring(&root, ignored)
                .unwrap()
                .iter()
                .map(|p| {
                    p.strip_prefix(&root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/")
                })
                .collect()
        };

        // Directory entry (with trailing slash) prunes the whole subtree.
        assert_eq!(
            names(&IgnoreRules::new(["hidden-repo/"])),
            vec!["keep/a.md".to_string(), "keep/generated.md".to_string()]
        );
        // Glob against relative file paths.
        assert_eq!(
            names(&IgnoreRules::new(["*generated*"])),
            vec![
                "hidden-repo/b.md".to_string(),
                "hidden-repo/nested/c.md".to_string(),
                "keep/a.md".to_string(),
            ]
        );
        // Absolute path entry hides the directory too.
        let absolute = root
            .join("hidden-repo")
            .to_string_lossy()
            .replace('\\', "/");
        assert_eq!(
            names(&IgnoreRules::new([absolute])),
            vec!["keep/a.md".to_string(), "keep/generated.md".to_string()]
        );
        // Empty rules keep everything.
        assert_eq!(names(&IgnoreRules::default()).len(), 4);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn returns_empty_when_root_missing() {
        let files = discover_markdown_files(Path::new("/nonexistent/pb/path")).unwrap();
        assert!(files.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn does_not_loop_on_self_referential_symlink() {
        use std::os::unix::fs::symlink;
        use std::sync::atomic::{AtomicU64, Ordering};

        static NEXT: AtomicU64 = AtomicU64::new(1);
        let root = std::env::temp_dir().join(format!(
            "pb-discovery-loop-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("real.md"), "# real\n").unwrap();
        // dir/loop -> dir creates a cycle when followed.
        symlink(&root, root.join("loop")).unwrap();

        let files = discover_markdown_files(&root).expect("discovery must terminate");
        let names: Vec<String> = files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"real.md".to_string()));

        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn follows_symlink_to_sibling_directory() {
        use std::os::unix::fs::symlink;
        use std::sync::atomic::{AtomicU64, Ordering};

        static NEXT: AtomicU64 = AtomicU64::new(1);
        let workspace = std::env::temp_dir().join(format!(
            "pb-discovery-symlink-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&workspace);
        let target_dir = workspace.join("target");
        let root = workspace.join("root");
        fs::create_dir_all(&target_dir).unwrap();
        fs::create_dir_all(&root).unwrap();
        fs::write(target_dir.join("linked.md"), "# linked\n").unwrap();
        symlink(&target_dir, root.join("via-link")).unwrap();

        let files = discover_markdown_files(&root).unwrap();
        let names: Vec<String> = files
            .iter()
            .map(|p| {
                p.strip_prefix(&root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        assert!(
            names.contains(&"via-link/linked.md".to_string()),
            "expected symlinked file to be discovered, got: {names:?}"
        );

        let _ = fs::remove_dir_all(&workspace);
    }
}
