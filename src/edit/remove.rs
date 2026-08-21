use crate::edit::write_atomically;
use crate::index::IndexedSnippet;
use crate::parser::{parse_file, snippet_section_range};
use std::fs;
use std::io;

/// Delete a snippet's whole `##` section from its markdown file.
///
/// The section spans the heading through the last line before the next H1 or
/// H2 boundary (see [`snippet_section_range`]), so trailing prose written under
/// the body fence goes with it instead of dangling under the previous section.
/// The file itself is never removed: an emptied file may still carry
/// frontmatter or a title the author wants.
///
/// Frecency events for the removed id are deliberately left alone — they
/// become orphans that `pb gc` reattaches or purges.
///
/// The rewrite keeps the original file intact until an atomic rename. Existing
/// symlinks and permissions are preserved, as are the exact bytes of every
/// surviving line.
pub fn remove_snippet(snippet: &IndexedSnippet) -> io::Result<()> {
    let path = snippet.path();
    let content = fs::read_to_string(path)?;
    let current_file = parse_file(path, snippet.root_dir(), &content);
    if !current_file
        .snippets
        .iter()
        .any(|current| current == &snippet.snippet)
    {
        return Err(stale_snippet_error(snippet));
    }
    let range = snippet_section_range(&snippet.relative_path, &content, snippet.id())
        .ok_or_else(|| stale_snippet_error(snippet))?;

    let lines: Vec<&str> = content.split_inclusive('\n').collect();
    let mut kept: Vec<&str> = Vec::with_capacity(lines.len());
    kept.extend_from_slice(&lines[..range.start_line]);
    kept.extend_from_slice(&lines[range.end_line.min(lines.len())..]);
    while kept.last().is_some_and(|line| line.trim().is_empty()) {
        kept.pop();
    }

    write_atomically(path, &kept.concat())
}

fn stale_snippet_error(snippet: &IndexedSnippet) -> io::Error {
    io::Error::other(format!(
        "could not locate `{}` in {}; the file may have changed or its code fence may be unterminated",
        snippet.id(),
        snippet.path().display()
    ))
}

#[cfg(test)]
mod tests;
