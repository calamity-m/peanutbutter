use crate::BINARY_NAME;
use crate::index::IndexedSnippet;
use crate::parser::snippet_line_ranges;
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

/// A file target to open in the user's editor, optionally at a one-based line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorTarget {
    /// Source file to open.
    pub path: PathBuf,
    /// One-based line number to jump to when the editor command supports it.
    pub line: Option<usize>,
}

impl EditorTarget {
    /// Create a file-only editor target.
    pub fn file(path: PathBuf) -> Self {
        Self { path, line: None }
    }

    /// Create an editor target with a one-based line number.
    pub fn line(path: PathBuf, line: usize) -> Self {
        Self {
            path,
            line: Some(line),
        }
    }
}

/// Build an editor target for a parsed snippet by locating its heading line in
/// the source file. If the snippet id cannot be found, falls back to file-only.
pub fn target_for_snippet(snippet: &IndexedSnippet) -> io::Result<EditorTarget> {
    let content = fs::read_to_string(snippet.path())?;
    let line = snippet_line_ranges(&snippet.relative_path, &content)
        .into_iter()
        .find(|range| range.id == *snippet.id())
        .map(|range| range.start_line + 1);
    Ok(EditorTarget {
        path: snippet.path().to_path_buf(),
        line,
    })
}

/// Open a snippet in `$VISUAL` / `$EDITOR`, targeting its heading line when the
/// editor command is a known bare executable that supports line arguments.
pub fn open_snippet(snippet: &IndexedSnippet, editor_override: Option<&str>) -> io::Result<()> {
    open(&target_for_snippet(snippet)?, editor_override)
}

/// Open an editor target in `$VISUAL` / `$EDITOR`.
///
/// Compound editor commands are shell-evaluated to preserve existing `edit`
/// command behavior. Line targeting is deliberately conservative: known bare
/// editor commands get line arguments; compound or unknown commands receive
/// only the file path.
pub fn open(target: &EditorTarget, editor_override: Option<&str>) -> io::Result<()> {
    let editor = resolve_editor(editor_override)?;
    let args = editor_args(&editor, target);

    let status = ProcessCommand::new("bash")
        .arg("-lc")
        .arg("eval \"$PB_EDITOR\" $PB_EDITOR_ARGS")
        .env("PB_EDITOR", editor)
        .env("PB_EDITOR_ARGS", args)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "editor exited unsuccessfully for {}",
            target.path.display()
        )))
    }
}

fn resolve_editor(editor_override: Option<&str>) -> io::Result<String> {
    editor_override
        .map(ToOwned::to_owned)
        .or_else(|| {
            env::var("VISUAL")
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
        .or_else(|| {
            env::var("EDITOR")
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
        .ok_or_else(|| {
            io::Error::other(format!(
                "set $VISUAL or $EDITOR before using {BINARY_NAME} edit"
            ))
        })
}

fn editor_args(editor: &str, target: &EditorTarget) -> String {
    line_args(editor, target)
        .unwrap_or_else(|| vec![target.path.to_string_lossy().into_owned()])
        .into_iter()
        .map(|arg| shell_quote(&arg))
        .collect::<Vec<_>>()
        .join(" ")
}

fn line_args(editor: &str, target: &EditorTarget) -> Option<Vec<String>> {
    let line = target.line?;
    let command = bare_editor_command(editor)?;
    let file = target.path.to_string_lossy().into_owned();
    match command.as_str() {
        "vi" | "vim" | "nvim" | "view" | "nano" | "micro" | "emacs" | "emacsclient" => {
            Some(vec![format!("+{line}"), file])
        }
        "code" | "codium" | "code-insiders" => {
            Some(vec!["-g".to_string(), format!("{file}:{line}")])
        }
        "hx" | "helix" => Some(vec![format!("{file}:{line}")]),
        _ => None,
    }
}

fn bare_editor_command(editor: &str) -> Option<String> {
    let editor = editor.trim();
    if editor.is_empty()
        || editor
            .chars()
            .any(|c| c.is_whitespace() || "'\"\\$;&|<>(){}[]*?!~`".contains(c))
    {
        return None;
    }
    Path::new(editor)
        .file_name()
        .and_then(|name| name.to_str())
        .map(ToOwned::to_owned)
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Frontmatter, Snippet, SnippetFile, SnippetId};
    use crate::index::SnippetIndex;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn temp_dir(prefix: &str) -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let path = std::env::temp_dir().join(format!(
            "pb-editor-{prefix}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn editor_args_add_line_for_known_bare_editors() {
        let target = EditorTarget::line(PathBuf::from("/tmp/demo.md"), 12);

        assert_eq!(editor_args("nvim", &target), "'+12' '/tmp/demo.md'");
        assert_eq!(
            editor_args("/usr/bin/nano", &target),
            "'+12' '/tmp/demo.md'"
        );
        assert_eq!(editor_args("code", &target), "'-g' '/tmp/demo.md:12'");
        assert_eq!(editor_args("hx", &target), "'/tmp/demo.md:12'");
    }

    #[test]
    fn editor_args_fall_back_to_file_for_compound_or_unknown_editors() {
        let target = EditorTarget::line(PathBuf::from("/tmp/demo.md"), 12);

        assert_eq!(editor_args("nvim -f", &target), "'/tmp/demo.md'");
        assert_eq!(editor_args("custom-editor", &target), "'/tmp/demo.md'");
    }

    #[test]
    fn editor_args_shell_quote_file_paths() {
        let target = EditorTarget::file(PathBuf::from("/tmp/it's here.md"));

        assert_eq!(editor_args("nvim", &target), "'/tmp/it'\\''s here.md'");
    }

    #[test]
    fn target_for_snippet_returns_one_based_heading_line() {
        let root = temp_dir("source-line");
        let path = root.join("snippets.md");
        fs::write(
            &path,
            "---\nname: demo\n---\n\n## First\n\n```\na\n```\n\n## Second\n\n```\nb\n```\n",
        )
        .unwrap();
        let file = SnippetFile {
            path: path.clone(),
            relative_path: PathBuf::from("snippets.md"),
            frontmatter: Frontmatter::default(),
            snippets: vec![Snippet {
                id: SnippetId::new("snippets.md", "second"),
                name: "Second".to_string(),
                description: String::new(),
                body: "b".to_string(),
                variables: vec![],
            }],
        };
        let index = SnippetIndex::from_files([file]);
        let snippet = index.iter().next().unwrap();

        let target = target_for_snippet(snippet).unwrap();

        assert_eq!(target, EditorTarget::line(path, 11));
        let _ = fs::remove_dir_all(root);
    }
}
