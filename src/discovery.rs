use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub fn discover_markdown_files(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    if !root.exists() {
        return Ok(out);
    }
    visit(root, &mut out)?;
    out.sort();
    Ok(out)
}

pub fn discover_all<I, P>(roots: I) -> io::Result<Vec<PathBuf>>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    let mut out = Vec::new();
    for root in roots {
        let mut found = discover_markdown_files(root.as_ref())?;
        out.append(&mut found);
    }
    Ok(out)
}

fn visit(path: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
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
    let mut entries: Vec<_> = fs::read_dir(path)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let child = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            visit(&child, out)?;
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
}
