use crate::config::Paths;
use crate::domain::SnippetId;
use crate::execute::{self, ExecuteOptions, ExecutionOutcome};
use crate::frecency::FrecencyStore;
use crate::index::{IndexedSnippet, SnippetIndex};
use crate::parser::{SnippetLineRange, snippet_line_ranges};
use crate::{BASH_ALIAS_NAME, BINARY_NAME};
use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_ADD_PATH: &str = "snippets.md";
const DEFAULT_BASH_BINDING: &str = "C+b";

/// A parsed CLI invocation, ready to dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliCommand {
    /// Print usage and exit (no args, `-h`, `--help`, or `help`).
    Help,
    /// Run the interactive TUI and emit the selected command to stdout.
    Execute,
    /// Open `$EDITOR` on the given snippet file (or the default file).
    Add(Option<PathBuf>),
    /// Delete the named or id-matched snippet from its source file.
    Del(String),
    /// Emit shell integration code for the given readline binding.
    Bash { binding: String },
}

/// Result returned by [`run_execute_command`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecuteCommandResult {
    /// `true` if a command was written to the output writer.
    pub emitted: bool,
    /// Non-fatal warning shown when the frecency state could not be saved.
    pub persist_warning: Option<String>,
}

/// Identifies the snippet that was removed by [`run_del_command`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletedSnippet {
    /// Id of the removed snippet.
    pub id: SnippetId,
    /// Path of the file that was modified (it may now be empty).
    pub path: PathBuf,
}

/// Parse `args` (everything after the binary name) into a [`CliCommand`].
/// Returns `Err` with a human-readable message on unknown commands or
/// incorrect argument counts.
pub fn parse_args<I>(args: I) -> Result<CliCommand, String>
where
    I: IntoIterator<Item = OsString>,
{
    let mut args = args.into_iter();
    let Some(first) = args.next() else {
        return Ok(CliCommand::Help);
    };
    let first = first.to_string_lossy().into_owned();
    match first.as_str() {
        "-h" | "--help" | "help" => {
            if args.next().is_some() {
                Err("help does not accept extra arguments".to_string())
            } else {
                Ok(CliCommand::Help)
            }
        }
        "execute" => {
            if args.next().is_some() {
                Err("execute does not accept extra arguments".to_string())
            } else {
                Ok(CliCommand::Execute)
            }
        }
        "add" => {
            let path = args.next().map(PathBuf::from);
            if args.next().is_some() {
                Err("add accepts at most one path".to_string())
            } else {
                Ok(CliCommand::Add(path))
            }
        }
        "del" => match (args.next(), args.next()) {
            (Some(name), None) => Ok(CliCommand::Del(name.to_string_lossy().into_owned())),
            (None, _) => Err("del requires an exact snippet name or id".to_string()),
            (_, Some(_)) => Err("del accepts exactly one exact snippet name or id".to_string()),
        },
        "--bash" => match (args.next(), args.next()) {
            (Some(binding), None) => Ok(CliCommand::Bash {
                binding: binding.to_string_lossy().into_owned(),
            }),
            (None, None) => Ok(CliCommand::Bash {
                binding: DEFAULT_BASH_BINDING.to_string(),
            }),
            (_, Some(_)) => Err("--bash accepts at most one binding".to_string()),
        },
        other => Err(format!("unknown command or flag: {other}")),
    }
}

/// Produce the human-readable help/usage string shown on `--help` or bare
/// invocation. Includes resolved snippet roots and config/state paths so the
/// user can immediately see where the tool is looking for files.
pub fn help_text(paths: &Paths) -> String {
    let mut out = String::new();
    out.push_str(&format!("{BINARY_NAME}: snippet manager\n"));
    out.push_str("usage:\n");
    out.push_str(&format!("  {BINARY_NAME}\n"));
    out.push_str(&format!("  {BINARY_NAME} execute\n"));
    out.push_str(&format!("  {BINARY_NAME} --bash [C+b]\n"));
    out.push_str(&format!("  {BINARY_NAME} add [path]\n"));
    out.push_str(&format!("  {BINARY_NAME} del <name-or-id>\n"));
    out.push('\n');
    out.push_str(&format!(
        "bash shorthand: `{BINARY_NAME} --bash` also defines `{BASH_ALIAS_NAME}`\n\n"
    ));
    out.push_str("snippet roots:\n");
    for root in &paths.snippet_roots {
        out.push_str(&format!("  {}\n", root.display()));
    }
    out.push_str(&format!("config file: {}\n", paths.config_file.display()));
    out.push_str(&format!("state file: {}\n", paths.state_file.display()));
    out
}

/// Emit the bash integration script using the path of the currently running
/// executable. Intended for `peanutbutter --bash`; the caller should `eval`
/// the output in their shell init file.
pub fn bash_integration_for_current_exe(binding: &str) -> io::Result<String> {
    let exe = env::current_exe()?;
    bash_integration_script(binding, &exe)
}

/// Build the bash integration script for a given `executable` path and
/// readline `binding` (e.g. `"C+b"`). Separated from
/// [`bash_integration_for_current_exe`] so tests can supply a controlled path.
pub fn bash_integration_script(binding: &str, executable: &Path) -> io::Result<String> {
    let binding = readline_binding(binding)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;
    let executable = shell_quote(&executable.to_string_lossy());
    Ok(format!(
        r#"\builtin unalias {BASH_ALIAS_NAME} &>/dev/null || \builtin true
\builtin alias {BASH_ALIAS_NAME}='{BINARY_NAME}'
__pb_insert_command() {{
  local __pb_cmd
  __pb_cmd=$({executable} execute)
  local __pb_status=$?
  if [[ $__pb_status -ne 0 ]]; then
    return $__pb_status
  fi
  if [[ -z $__pb_cmd ]]; then
    READLINE_LINE="${{READLINE_LINE}}"
    READLINE_POINT=${{READLINE_POINT}}
    return 0
  fi
  READLINE_LINE="${{READLINE_LINE:0:$READLINE_POINT}}${{__pb_cmd}}${{READLINE_LINE:$READLINE_POINT}}"
  READLINE_POINT=$(( READLINE_POINT + ${{#__pb_cmd}} ))
}}
bind -x '"{binding}":__pb_insert_command'
"#
    ))
}

/// Run the execute TUI, write the selected command to `writer`, and record a
/// frecency event. Returns an [`ExecuteCommandResult`] indicating whether a
/// command was emitted and whether persisting the frecency state failed.
pub fn run_execute_command<W: Write>(
    paths: &Paths,
    writer: &mut W,
) -> io::Result<ExecuteCommandResult> {
    run_execute_command_with(paths, writer, execute::run_execute)
}

/// Testable variant of [`run_execute_command`] that accepts a custom `runner`
/// function instead of calling [`execute::run_execute`] directly.
pub fn run_execute_command_with<W, F>(
    paths: &Paths,
    writer: &mut W,
    runner: F,
) -> io::Result<ExecuteCommandResult>
where
    W: Write,
    F: FnOnce(SnippetIndex, FrecencyStore, ExecuteOptions) -> io::Result<Option<ExecutionOutcome>>,
{
    let index = crate::index::load_from_roots(&paths.snippet_roots)?;
    let mut store = FrecencyStore::load(&paths.state_file)?;
    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let app_config = crate::config::load()?;
    let options = ExecuteOptions {
        cwd: cwd.clone(),
        viewport_height: app_config.ui.height,
        search: app_config.search.clone(),
        theme: app_config.theme.clone(),
        variables: app_config.variables.clone(),
        ..ExecuteOptions::default()
    };
    let outcome = runner(index, store.clone(), options)?;
    let Some(outcome) = outcome else {
        return Ok(ExecuteCommandResult {
            emitted: false,
            persist_warning: None,
        });
    };

    writer.write_all(outcome.command.as_bytes())?;
    writer.write_all(b"\n")?;
    writer.flush()?;

    store.record(outcome.snippet_id, cwd, unix_now());
    let persist_warning = store
        .save(&paths.state_file)
        .err()
        .map(|err| err.to_string());
    Ok(ExecuteCommandResult {
        emitted: true,
        persist_warning,
    })
}

/// Resolve the target snippet file and open it in `$EDITOR` / `$VISUAL`.
/// Creates the file (and any parent directories) if it doesn't exist yet.
/// Returns the path of the file that was opened.
pub fn run_add_command(paths: &Paths, requested: Option<&Path>) -> io::Result<PathBuf> {
    run_add_command_with_editor(paths, requested, None)
}

fn run_add_command_with_editor(
    paths: &Paths,
    requested: Option<&Path>,
    editor_override: Option<&str>,
) -> io::Result<PathBuf> {
    let target = resolve_add_target(paths, requested)?;
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    if !target.exists() {
        fs::write(&target, "")?;
    }
    open_in_editor(&target, editor_override)?;
    Ok(target)
}

/// Determine the absolute path of the file to edit for `add`.
///
/// - `None` → `<first-root>/snippets.md`
/// - Relative path → anchored under the first snippet root
/// - Absolute path → used as-is after verifying it's inside a known root
pub fn resolve_add_target(paths: &Paths, requested: Option<&Path>) -> io::Result<PathBuf> {
    let default_root = paths
        .snippet_roots
        .first()
        .ok_or_else(|| io::Error::other("no snippet roots configured"))?;

    let target = match requested {
        None => default_root.join(DEFAULT_ADD_PATH),
        Some(path) if path.is_absolute() => {
            let target = normalize_add_path(path.to_path_buf());
            if paths
                .snippet_roots
                .iter()
                .any(|root| target.starts_with(root))
            {
                target
            } else {
                return Err(io::Error::other(format!(
                    "absolute add target must live under a configured snippet root: {}",
                    path.display()
                )));
            }
        }
        Some(path) => default_root.join(normalize_add_path(path.to_path_buf())),
    };
    Ok(target)
}

/// Delete the snippet identified by `query` (exact name or `file#slug` id).
/// Removes the `##` section and its code block from the source file. If the
/// file becomes empty after removal, the file itself is deleted.
pub fn run_del_command(paths: &Paths, query: &str) -> io::Result<DeletedSnippet> {
    let index = crate::index::load_from_roots(&paths.snippet_roots)?;
    let target = resolve_delete_target(&index, query)?;
    let content = fs::read_to_string(target.path())?;
    let rendered = remove_snippet_from_content(&target.relative_path, &content, target.id())?;
    if rendered.trim().is_empty() {
        fs::remove_file(target.path())?;
    } else {
        fs::write(target.path(), rendered)?;
    }
    Ok(DeletedSnippet {
        id: target.id().clone(),
        path: target.path().to_path_buf(),
    })
}

fn resolve_delete_target<'a>(
    index: &'a SnippetIndex,
    query: &str,
) -> io::Result<&'a IndexedSnippet> {
    if let Some(entry) = index.iter().find(|entry| entry.id().as_str() == query) {
        return Ok(entry);
    }

    let matches: Vec<&IndexedSnippet> =
        index.iter().filter(|entry| entry.name() == query).collect();
    match matches.as_slice() {
        [entry] => Ok(*entry),
        [] => Err(io::Error::other(format!(
            "no snippet matched exactly: {query}"
        ))),
        many => {
            let ids = many
                .iter()
                .map(|entry| entry.id().as_str().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            Err(io::Error::other(format!(
                "snippet name is ambiguous; use an exact id instead: {ids}"
            )))
        }
    }
}

fn remove_snippet_from_content(
    relative_path: &Path,
    content: &str,
    snippet_id: &SnippetId,
) -> io::Result<String> {
    let ranges = snippet_line_ranges(relative_path, content);
    let target = ranges
        .iter()
        .find(|range| &range.id == snippet_id)
        .ok_or_else(|| io::Error::other(format!("snippet not found in file: {snippet_id}")))?;
    remove_lines_in_range(content, target)
}

fn remove_lines_in_range(content: &str, target: &SnippetLineRange) -> io::Result<String> {
    let lines: Vec<&str> = content.lines().collect();
    if target.start_line > lines.len()
        || target.end_line > lines.len()
        || target.start_line > target.end_line
    {
        return Err(io::Error::other(format!(
            "snippet line range out of bounds for {}: {}..{} (file has {} lines)",
            target.id,
            target.start_line,
            target.end_line,
            lines.len()
        )));
    }

    let had_trailing_newline = content.ends_with('\n');
    let mut prefix: Vec<&str> = lines[..target.start_line].to_vec();
    let mut suffix: Vec<&str> = lines[target.end_line..].to_vec();
    while prefix
        .last()
        .is_some_and(|line: &&str| line.trim().is_empty())
    {
        prefix.pop();
    }
    while suffix
        .first()
        .is_some_and(|line: &&str| line.trim().is_empty())
    {
        suffix.remove(0);
    }

    let mut out = Vec::new();
    out.extend(prefix);
    if !out.is_empty() && !suffix.is_empty() {
        out.push("");
    }
    out.extend(suffix);

    let mut rendered = out.join("\n");
    if had_trailing_newline && !rendered.is_empty() {
        rendered.push('\n');
    }
    Ok(rendered)
}

fn normalize_add_path(mut path: PathBuf) -> PathBuf {
    if path.extension().is_none() {
        path.set_extension("md");
    }
    path
}

fn open_in_editor(target: &Path, editor_override: Option<&str>) -> io::Result<()> {
    let editor = editor_override
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
                "set $VISUAL or $EDITOR before using {BINARY_NAME} add"
            ))
        })?;

    let status = Command::new("bash")
        .arg("-lc")
        .arg("eval \"$PB_EDITOR\" \"$PB_TARGET_QUOTED\"")
        .env("PB_EDITOR", editor)
        .env("PB_TARGET_QUOTED", shell_quote(&target.to_string_lossy()))
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "editor exited unsuccessfully for {}",
            target.display()
        )))
    }
}

fn readline_binding(binding: &str) -> Result<String, String> {
    let binding = binding.trim();
    for prefix in ["C+", "C-", "Ctrl+", "Ctrl-", "ctrl+", "ctrl-"] {
        if let Some(rest) = binding.strip_prefix(prefix) {
            let mut chars = rest.chars();
            let ch = chars
                .next()
                .ok_or_else(|| "binding is missing a key after the control prefix".to_string())?;
            if chars.next().is_some() {
                return Err("only single-key control bindings are supported in v1".to_string());
            }
            return Ok(format!("\\C-{}", ch.to_ascii_lowercase()));
        }
    }
    Err("expected a control binding like C+b".to_string())
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Paths;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn temp_dir(prefix: &str) -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let path = std::env::temp_dir().join(format!(
            "pb-{prefix}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn test_paths(root: &Path) -> Paths {
        Paths {
            snippet_roots: vec![root.to_path_buf()],
            state_file: root.join("state.tsv"),
            config_file: root.join("config.toml"),
        }
    }

    #[test]
    fn parse_args_recognizes_expected_commands() {
        assert_eq!(
            parse_args(Vec::<OsString>::new()).unwrap(),
            CliCommand::Help
        );
        assert_eq!(
            parse_args(vec![OsString::from("execute")]).unwrap(),
            CliCommand::Execute
        );
        assert_eq!(
            parse_args(vec![OsString::from("add"), OsString::from("nested/demo")]).unwrap(),
            CliCommand::Add(Some(PathBuf::from("nested/demo")))
        );
        assert_eq!(
            parse_args(vec![OsString::from("del"), OsString::from("Echo")]).unwrap(),
            CliCommand::Del("Echo".to_string())
        );
        assert_eq!(
            parse_args(vec![OsString::from("--bash")]).unwrap(),
            CliCommand::Bash {
                binding: "C+b".to_string()
            }
        );
        assert_eq!(
            parse_args(vec![OsString::from("--bash"), OsString::from("C+b")]).unwrap(),
            CliCommand::Bash {
                binding: "C+b".to_string()
            }
        );
    }

    #[test]
    fn bash_script_uses_readline_bind_and_executable_path() {
        let script = bash_integration_script("C+b", Path::new("/tmp/peanutbutter")).unwrap();
        assert!(script.contains("\\builtin unalias pb &>/dev/null || \\builtin true"));
        assert!(script.contains("\\builtin alias pb='peanutbutter'"));
        assert!(script.contains("bind -x '\"\\C-b\":__pb_insert_command'"));
        assert!(script.contains("'/tmp/peanutbutter' execute"));
        assert!(script.contains("READLINE_LINE=\"${READLINE_LINE}\""));
        assert!(script.contains("READLINE_POINT=${READLINE_POINT}"));
    }

    #[test]
    fn help_text_prefers_peanutbutter_and_mentions_pb_alias() {
        let paths = test_paths(Path::new("/tmp/snippets"));
        let help = help_text(&paths);
        assert!(help.contains("peanutbutter: snippet manager"));
        assert!(help.contains("  peanutbutter --bash [C+b]"));
        assert!(help.contains("defines `pb`"));
        assert!(!help.contains("  pb execute"));
    }

    #[test]
    fn default_bash_binding_script_emits_pb_alias() {
        let CliCommand::Bash { binding } = parse_args(vec![OsString::from("--bash")]).unwrap()
        else {
            panic!("expected bash command");
        };
        let script = bash_integration_script(&binding, Path::new("/tmp/peanutbutter")).unwrap();
        assert!(script.contains("\\builtin alias pb='peanutbutter'"));
        assert!(script.contains("bind -x '\"\\C-b\":__pb_insert_command'"));
    }

    #[test]
    fn resolve_add_target_defaults_and_appends_markdown_extension() {
        let root = temp_dir("add-target");
        let paths = test_paths(&root);
        assert_eq!(
            resolve_add_target(&paths, None).unwrap(),
            root.join("snippets.md")
        );
        assert_eq!(
            resolve_add_target(&paths, Some(Path::new("docker/compose"))).unwrap(),
            root.join("docker/compose.md")
        );
    }

    #[test]
    fn run_add_command_creates_file_and_invokes_editor() {
        let root = temp_dir("add-command");
        let paths = test_paths(&root);
        let editor_log = root.join("editor.log");
        let editor = root.join("fake-editor.sh");
        fs::write(
            &editor,
            format!(
                "#!/usr/bin/env bash\nprintf '%s' \"$1\" > {}\n",
                shell_quote(&editor_log.to_string_lossy())
            ),
        )
        .unwrap();
        let status = Command::new("chmod")
            .arg("+x")
            .arg(&editor)
            .status()
            .unwrap();
        assert!(status.success());

        let target = run_add_command_with_editor(
            &paths,
            Some(Path::new("git/log")),
            Some(&editor.to_string_lossy()),
        )
        .unwrap();

        assert_eq!(target, root.join("git/log.md"));
        assert!(target.exists());
        assert_eq!(
            fs::read_to_string(editor_log).unwrap(),
            target.to_string_lossy()
        );
    }

    #[test]
    fn run_execute_command_persists_usage_after_write() {
        let root = temp_dir("execute-record");
        let snippet_file = root.join("snippets.md");
        fs::write(&snippet_file, "## Echo\n\n```\necho hi\n```\n").unwrap();
        let paths = test_paths(&root);

        let mut out = Vec::new();
        let result = run_execute_command_with(&paths, &mut out, |_index, _store, _options| {
            Ok(Some(ExecutionOutcome {
                snippet_id: SnippetId::new("snippets.md", "echo"),
                command: "echo hi".to_string(),
            }))
        })
        .unwrap();

        assert!(result.emitted);
        assert!(result.persist_warning.is_none());
        assert_eq!(String::from_utf8(out).unwrap(), "echo hi\n");
        let saved = FrecencyStore::load(&paths.state_file).unwrap();
        assert_eq!(saved.events().len(), 1);
        assert_eq!(saved.events()[0].id.as_str(), "snippets.md#echo");
    }

    #[test]
    fn execute_does_not_persist_when_output_write_fails() {
        struct FailingWriter;

        impl Write for FailingWriter {
            fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("nope"))
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let root = temp_dir("execute-no-record");
        fs::write(root.join("snippets.md"), "## Echo\n\n```\necho hi\n```\n").unwrap();
        let paths = test_paths(&root);
        let mut writer = FailingWriter;
        let err = run_execute_command_with(&paths, &mut writer, |_index, _store, _options| {
            Ok(Some(ExecutionOutcome {
                snippet_id: SnippetId::new("snippets.md", "echo"),
                command: "echo hi".to_string(),
            }))
        })
        .unwrap_err();
        assert_eq!(err.to_string(), "nope");
        let saved = FrecencyStore::load(&paths.state_file).unwrap();
        assert!(saved.events().is_empty());
    }

    #[test]
    fn delete_removes_only_target_snippet_block() {
        let root = temp_dir("delete");
        let content = "\
---\n\
name: demo\n\
---\n\
\n\
# Title\n\
\n\
## One\n\
\n\
```\n\
echo one\n\
```\n\
\n\
## Two\n\
\n\
```\n\
echo two\n\
```\n";
        fs::write(root.join("snippets.md"), content).unwrap();
        let paths = test_paths(&root);
        let deleted = run_del_command(&paths, "Two").unwrap();
        assert_eq!(deleted.id.as_str(), "snippets.md#two");
        let rewritten = fs::read_to_string(root.join("snippets.md")).unwrap();
        assert!(rewritten.contains("## One"));
        assert!(!rewritten.contains("## Two"));
        assert!(rewritten.contains("# Title"));
    }

    #[test]
    fn remove_lines_in_range_errors_when_out_of_bounds() {
        let content = "one\ntwo\nthree\n";
        let bogus = SnippetLineRange {
            id: SnippetId::new("snippets.md", "ghost"),
            start_line: 10,
            end_line: 20,
        };
        let err = remove_lines_in_range(content, &bogus).unwrap_err();
        assert!(
            err.to_string().contains("out of bounds"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn remove_lines_in_range_errors_when_start_after_end() {
        let content = "one\ntwo\nthree\n";
        let bogus = SnippetLineRange {
            id: SnippetId::new("snippets.md", "ghost"),
            start_line: 2,
            end_line: 1,
        };
        let err = remove_lines_in_range(content, &bogus).unwrap_err();
        assert!(err.to_string().contains("out of bounds"));
    }

    #[test]
    fn delete_reports_ambiguous_exact_name_matches() {
        let root = temp_dir("delete-ambiguous");
        fs::create_dir_all(root.join("a")).unwrap();
        fs::create_dir_all(root.join("b")).unwrap();
        fs::write(root.join("a/one.md"), "## Echo\n\n```\na\n```\n").unwrap();
        fs::write(root.join("b/two.md"), "## Echo\n\n```\nb\n```\n").unwrap();
        let paths = test_paths(&root);
        let err = run_del_command(&paths, "Echo").unwrap_err();
        assert!(err.to_string().contains("ambiguous"));
        assert!(err.to_string().contains("a/one.md#echo"));
        assert!(err.to_string().contains("b/two.md#echo"));
    }
}
