use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Recursively find all `.md` / `.markdown` files under `root`, sorted
/// alphabetically. Symlinks to directories are followed; symlink cycles are
/// detected via canonical paths and skipped. Returns an empty vec if `root`
/// does not exist.
pub fn discover_markdown_files(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    if !root.exists() {
        return Ok(out);
    }
    let mut visited = HashSet::new();
    visit(root, &mut out, &mut visited)?;
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
        visit(root, &mut out, &mut visited)?;
        out[start..].sort();
    }
    Ok(out)
}

fn visit(path: &Path, out: &mut Vec<PathBuf>, visited: &mut HashSet<PathBuf>) -> io::Result<()> {
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
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            visit(&child, out, visited)?;
        } else if file_type.is_symlink() {
            // file_type() does not follow links; resolve via metadata() to
            // decide whether the target is a dir we should recurse into.
            match fs::metadata(&child) {
                Ok(target_meta) if target_meta.is_dir() => visit(&child, out, visited)?,
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

    #[test]
    fn discovers_examples_directory() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
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
        assert!(names.contains(&"simple/snippets.md".to_string()));
        assert!(names.contains(&"complex/complex.md".to_string()));
        assert!(names.contains(&"nested/root.md".to_string()));
        assert!(names.contains(&"nested/docker/docker.md".to_string()));
        assert!(names.contains(&"nested/docker/compose/snip.md".to_string()));
        assert!(names.contains(&"nested/docker/images/images.md".to_string()));
        assert!(names.contains(&"nested/grep/grep.md".to_string()));
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
