use crate::index::IndexedSnippet;
use crate::parser::snippet_section_range;
use std::fs;
use std::io;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// Delete a snippet's whole `##` section from its markdown file.
///
/// The section spans the heading through the last line before the next
/// top-level `##` (see [`snippet_section_range`]), so trailing prose written
/// under the body fence goes with it instead of dangling under the previous
/// section. The file itself is never removed: an emptied file may still carry
/// frontmatter or a title the author wants.
///
/// Frecency events for the removed id are deliberately left alone — they
/// become orphans that `pb gc` reattaches or purges.
///
/// The rewrite is atomic (temp file plus rename) so an interrupted write
/// cannot truncate the user's snippets. Files are normalized to `\n` line
/// endings with a single trailing newline.
pub fn remove_snippet(snippet: &IndexedSnippet) -> io::Result<()> {
    let path = snippet.path();
    let content = fs::read_to_string(path)?;
    let range = snippet_section_range(&snippet.relative_path, &content, snippet.id()).ok_or_else(
        || {
            io::Error::other(format!(
                "could not locate `{}` in {}; the file may have changed or its code fence may be unterminated",
                snippet.id(),
                path.display()
            ))
        },
    )?;

    let lines: Vec<&str> = content.lines().collect();
    let mut kept: Vec<&str> = Vec::with_capacity(lines.len());
    kept.extend_from_slice(&lines[..range.start_line]);
    kept.extend_from_slice(&lines[range.end_line.min(lines.len())..]);
    while kept.last().is_some_and(|line| line.trim().is_empty()) {
        kept.pop();
    }

    let mut updated = kept.join("\n");
    if !updated.is_empty() {
        updated.push('\n');
    }
    write_atomically(path, &updated)
}

/// Replace `path`'s contents via a sibling temp file and a rename, mirroring
/// how `cli::write_starter_snippets` writes the starter file.
fn write_atomically(path: &Path, contents: &str) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other(format!("{} has no parent directory", path.display())))?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("snippets.md");
    let tmp = parent.join(format!(
        ".{file_name}.tmp-{}-{}",
        std::process::id(),
        unique_suffix()
    ));
    fs::write(&tmp, contents)?;
    match fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(err) => {
            let _ = fs::remove_file(&tmp);
            Err(err)
        }
    }
}

/// Nanosecond-resolution suffix so concurrent removals in the same directory
/// cannot collide on the temp file name.
fn unique_suffix() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests;
