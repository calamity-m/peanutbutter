use crate::completions;
use crate::config::Paths;
use crate::execute::{self, ExecuteOptions, ExecutionOutcome};
use crate::frecency::FrecencyStore;
use crate::index::SnippetIndex;
use crate::stats;
use crate::{BASH_ALIAS_NAME, BINARY_NAME};
use clap::{Parser, Subcommand};
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const STARTER_SNIPPETS_MD: &str = include_str!("../assets/starter_snippets.md");

/// Terminal snippet manager.
#[derive(Debug, Clone, Parser, PartialEq, Eq)]
#[command(name = BINARY_NAME, about = "terminal snippet manager")]
#[command(version)]
pub struct Cli {
    /// Built-in or custom theme name to use for the interactive TUI.
    #[arg(long, global = true, value_name = "NAME")]
    pub theme: Option<String>,
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// A parsed CLI subcommand, ready to dispatch.
#[derive(Debug, Clone, Subcommand, PartialEq, Eq)]
pub enum Command {
    /// Run the interactive TUI and emit the selected command to stdout.
    Execute,
    /// Scaffold starter snippets at the XDG default snippets directory.
    ///
    /// This deliberately writes to the XDG default path, not the first resolved
    /// snippet root. `--force` overwrites `snippets.md` unconditionally and does
    /// not create a backup.
    Init {
        /// Overwrite an existing starter file unconditionally, without backup.
        #[arg(long)]
        force: bool,
    },
    /// Open `$EDITOR` on the given snippet file (or the default file).
    Edit { path: Option<PathBuf> },
    /// Capture a recently-run shell command and append it as a new snippet.
    ///
    /// Reads `$PEANUTBUTTER_HISTORY` (populated by the shell integration) and
    /// shows a TUI picker, then a token-confirmation screen. Pass `-- <cmd...>`
    /// to skip the picker and supply the command directly.
    New {
        /// Optional snippet name. If omitted, the TUI prompts for one.
        name: Option<String>,
        /// Explicit command to capture, after `--`. Bypasses the history picker.
        #[arg(last = true)]
        command: Vec<String>,
    },
    /// Emit shell integration code for the given shell and key binding.
    Completions {
        /// Which shell to emit integration code for.
        shell: completions::Shell,
        #[arg(default_value = "C+b")]
        binding: String,
    },
    /// Check snippet files for authoring problems.
    Lint {
        /// Include stricter style and structure checks.
        #[arg(long)]
        strict: bool,
        /// Emit JSON for scripts instead of human-readable text.
        #[arg(long)]
        json: bool,
    },
    /// Garbage collect orphaned frecency events.
    Gc {
        /// Report changes without modifying the frecency store.
        #[arg(long)]
        dry_run: bool,
        /// Remove orphaned events that are not reattached.
        #[arg(long)]
        purge: bool,
        /// Use compact output suitable for scripts.
        #[arg(short, long)]
        quiet: bool,
    },
    /// Show usage statistics from frecency history.
    Stats {
        /// How many snippets to show in the most-used and least-used lists.
        #[arg(long, default_value_t = 10)]
        top: usize,
        /// Sort order for the least-used list.
        #[arg(long, value_enum, default_value_t = stats::Sort::Stale)]
        sort: stats::Sort,
        /// Output mode.
        #[arg(long, value_enum, default_value_t = stats::Output::Tui)]
        output: stats::Output,
    },
    /// Open the interactive config tuning TUI.
    Settings,
    /// Internal shell completion helper for `edit`.
    #[command(hide = true)]
    CompleteEdit { current: Option<String> },
    /// Internal shell completion helper for `--theme`.
    #[command(hide = true)]
    CompleteTheme { current: Option<String> },
    /// Start a Language Server Protocol server over stdio for snippet authoring.
    Lsp,
    /// Print embedded reference documentation verbatim to stdout.
    Docs {
        /// Which reference document to print. Omit to list available topics.
        topic: Option<crate::docs::Topic>,
    },
}

/// Result returned by [`run_execute_command`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecuteCommandResult {
    /// `true` if a command was written to the output writer.
    pub emitted: bool,
    /// Non-fatal warning shown when the frecency state could not be saved.
    pub persist_warning: Option<String>,
    /// `true` if the emitted command consumed the shell buffer into its first
    /// variable; the caller should signal the shell to replace the whole line.
    pub replace_buffer: bool,
}

/// Produce dynamic help content for resolved snippet roots and config/state
/// paths. This is attached to clap's generated help at runtime.
pub fn after_help(paths: &Paths) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "shell integration: `{BINARY_NAME} completions bash|zsh|fish|powershell` also defines `{BASH_ALIAS_NAME}`\n"
    ));
    out.push_str(&format!(
        "new here? run `{BASH_ALIAS_NAME} init` to scaffold starter snippets.\n"
    ));
    out.push_str(&format!(
        "reference: `{BASH_ALIAS_NAME} docs syntax` or `{BASH_ALIAS_NAME} docs config` prints the embedded spec to stdout.\n\n"
    ));
    out.push_str("snippet roots:\n");
    for root in &paths.snippet_roots {
        out.push_str(&format!("  {}\n", root.display()));
    }
    out.push_str(&format!("config file: {}\n", paths.config_file.display()));
    out.push_str(&format!("state file: {}\n", paths.state_file.display()));
    out
}

/// Scaffold the starter snippet collection at the XDG default snippets path.
///
/// This command intentionally targets [`Paths::xdg_snippets_dir`] even when
/// `$PEANUTBUTTER_PATH` or config roots are set, so first-run docs and auto-init
/// always agree on a single default location. Passing `force` overwrites the
/// existing `snippets.md` without creating a backup.
pub fn run_init_command<W: Write>(paths: &Paths, force: bool, writer: &mut W) -> io::Result<()> {
    match write_starter_snippets(paths, force)? {
        InitOutcome::Written(path) => {
            writeln!(writer, "wrote {}", path.display())?;
            writeln!(
                writer,
                "next: pb new   |   pb edit   |   docs/SNIPPET_SYNTAX.md"
            )?;
        }
        InitOutcome::Skipped(path) => {
            writeln!(
                writer,
                "snippets.md already exists at {} (use --force to overwrite)",
                path.display()
            )?;
        }
    }
    if overrides_active(paths) {
        eprintln!(
            "{BINARY_NAME}: note: snippet root overrides are set; init writes to the XDG default by design"
        );
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum InitOutcome {
    Written(PathBuf),
    Skipped(PathBuf),
}

fn auto_init_if_needed(paths: &Paths) {
    if paths.xdg_snippets_dir.exists() {
        return;
    }
    match write_starter_snippets(paths, false) {
        Ok(InitOutcome::Written(path)) => {
            if overrides_active(paths) {
                eprintln!(
                    "{BINARY_NAME}: note: seeded starter snippets at {}; snippet root overrides are active",
                    path.display()
                );
            }
        }
        Ok(InitOutcome::Skipped(_)) => {}
        Err(err) => eprintln!("{BINARY_NAME}: warning: could not seed starter snippets: {err}"),
    }
}

fn write_starter_snippets(paths: &Paths, force: bool) -> io::Result<InitOutcome> {
    fs::create_dir_all(&paths.xdg_snippets_dir)?;
    let target = paths.xdg_snippets_dir.join(crate::edit::DEFAULT_EDIT_PATH);
    if target.exists() && !force {
        return Ok(InitOutcome::Skipped(display_path(&target)));
    }

    let tmp = paths.xdg_snippets_dir.join(format!(
        ".{}.tmp-{}-{}",
        crate::edit::DEFAULT_EDIT_PATH,
        std::process::id(),
        unique_tmp_suffix()
    ));
    fs::write(&tmp, STARTER_SNIPPETS_MD)?;
    match fs::rename(&tmp, &target) {
        Ok(()) => Ok(InitOutcome::Written(display_path(&target))),
        Err(err) => {
            let _ = fs::remove_file(&tmp);
            Err(err)
        }
    }
}

fn unique_tmp_suffix() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default()
}

fn display_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

fn overrides_active(paths: &Paths) -> bool {
    env::var_os("PEANUTBUTTER_PATH").is_some() || paths.snippet_overrides_active
}

/// Run the execute TUI, write the selected command to `writer`, and record a
/// frecency event. Returns an [`ExecuteCommandResult`] indicating whether a
/// command was emitted and whether persisting the frecency state failed.
pub fn run_execute_command<W: Write>(
    paths: &Paths,
    writer: &mut W,
    theme_name: Option<&str>,
) -> io::Result<ExecuteCommandResult> {
    run_execute_command_with(paths, writer, theme_name, execute::run_execute)
}

/// Testable variant of [`run_execute_command`] that accepts a custom `runner`
/// function instead of calling [`execute::run_execute`] directly.
pub fn run_execute_command_with<W, F>(
    paths: &Paths,
    writer: &mut W,
    theme_name: Option<&str>,
    runner: F,
) -> io::Result<ExecuteCommandResult>
where
    W: Write,
    F: FnOnce(SnippetIndex, FrecencyStore, ExecuteOptions) -> io::Result<Option<ExecutionOutcome>>,
{
    auto_init_if_needed(paths);
    let index = crate::index::load_from_roots(&paths.snippet_roots)?;
    let mut store = FrecencyStore::load(&paths.state_file)?;
    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let app_config = crate::config::load_with_theme_override(theme_name)?;
    let options = ExecuteOptions {
        cwd: cwd.clone(),
        viewport_height: app_config.ui.height,
        search: app_config.search.clone(),
        theme: app_config.theme.clone(),
        variables: app_config.variables.clone(),
        snippet_roots: paths.snippet_roots.clone(),
        suggestion_commands: app_config.suggestion_commands.clone(),
        initial_buffer: env::var("PEANUTBUTTER_BUFFER")
            .ok()
            .filter(|s| !s.is_empty()),
        ..ExecuteOptions::default()
    };
    let outcome = runner(index, store.clone(), options)?;
    let Some(outcome) = outcome else {
        return Ok(ExecuteCommandResult {
            emitted: false,
            persist_warning: None,
            replace_buffer: false,
        });
    };
    let replace_buffer = outcome.consumed_buffer;

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
        replace_buffer,
    })
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
    use crate::domain::SnippetId;
    use std::path::Path;
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
            xdg_snippets_dir: root.to_path_buf(),
            snippet_overrides_active: false,
            state_file: root.join("state.tsv"),
            config_file: root.join("config.toml"),
        }
    }

    #[test]
    fn clap_recognizes_expected_commands() {
        assert_eq!(
            Cli::try_parse_from(["peanutbutter"]).unwrap(),
            Cli {
                theme: None,
                command: None,
            }
        );
        assert_eq!(
            Cli::try_parse_from(["peanutbutter", "--theme", "nord"])
                .unwrap()
                .theme,
            Some("nord".to_string())
        );
        assert_eq!(
            Cli::try_parse_from(["peanutbutter", "execute", "--theme", "gruvbox"])
                .unwrap()
                .theme,
            Some("gruvbox".to_string())
        );
        assert_eq!(
            Cli::try_parse_from(["peanutbutter", "execute"])
                .unwrap()
                .command,
            Some(Command::Execute)
        );
        assert_eq!(
            Cli::try_parse_from(["peanutbutter", "init", "--force"])
                .unwrap()
                .command,
            Some(Command::Init { force: true })
        );
        assert_eq!(
            Cli::try_parse_from(["peanutbutter", "edit", "nested/demo"])
                .unwrap()
                .command,
            Some(Command::Edit {
                path: Some(PathBuf::from("nested/demo"))
            })
        );
        assert_eq!(
            Cli::try_parse_from(["peanutbutter", "completions", "bash"])
                .unwrap()
                .command,
            Some(Command::Completions {
                shell: completions::Shell::Bash,
                binding: "C+b".to_string()
            })
        );
        assert_eq!(
            Cli::try_parse_from(["peanutbutter", "completions", "bash", "C+f"])
                .unwrap()
                .command,
            Some(Command::Completions {
                shell: completions::Shell::Bash,
                binding: "C+f".to_string()
            })
        );
        assert_eq!(
            Cli::try_parse_from(["peanutbutter", "completions", "powershell"])
                .unwrap()
                .command,
            Some(Command::Completions {
                shell: completions::Shell::Powershell,
                binding: "C+b".to_string()
            })
        );
        assert_eq!(
            Cli::try_parse_from(["peanutbutter", "complete-edit", "nested/"])
                .unwrap()
                .command,
            Some(Command::CompleteEdit {
                current: Some("nested/".to_string())
            })
        );
        assert_eq!(
            Cli::try_parse_from(["peanutbutter", "gc", "--dry-run", "--purge", "-q"])
                .unwrap()
                .command,
            Some(Command::Gc {
                dry_run: true,
                purge: true,
                quiet: true,
            })
        );
        assert_eq!(
            Cli::try_parse_from(["peanutbutter", "lint", "--strict", "--json"])
                .unwrap()
                .command,
            Some(Command::Lint {
                strict: true,
                json: true,
            })
        );
        assert_eq!(
            Cli::try_parse_from(["peanutbutter", "stats", "--output", "text"])
                .unwrap()
                .command,
            Some(Command::Stats {
                top: 10,
                sort: stats::Sort::Stale,
                output: stats::Output::Text,
            })
        );
        assert_eq!(
            Cli::try_parse_from(["peanutbutter", "stats", "--output", "json"])
                .unwrap()
                .command,
            Some(Command::Stats {
                top: 10,
                sort: stats::Sort::Stale,
                output: stats::Output::Json,
            })
        );
        assert_eq!(
            Cli::try_parse_from(["peanutbutter", "settings"])
                .unwrap()
                .command,
            Some(Command::Settings)
        );
        assert_eq!(
            Cli::try_parse_from(["peanutbutter", "docs"])
                .unwrap()
                .command,
            Some(Command::Docs { topic: None })
        );
        assert_eq!(
            Cli::try_parse_from(["peanutbutter", "docs", "syntax"])
                .unwrap()
                .command,
            Some(Command::Docs {
                topic: Some(crate::docs::Topic::Syntax)
            })
        );
        assert_eq!(
            Cli::try_parse_from(["peanutbutter", "docs", "config"])
                .unwrap()
                .command,
            Some(Command::Docs {
                topic: Some(crate::docs::Topic::Config)
            })
        );
    }

    #[test]
    fn clap_rejects_unknown_docs_topic() {
        let err = Cli::try_parse_from(["peanutbutter", "docs", "bogus"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::InvalidValue);
    }

    #[test]
    fn clap_rejects_old_bash_flag() {
        let err = Cli::try_parse_from(["peanutbutter", "--bash"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::UnknownArgument);
    }

    #[test]
    fn clap_rejects_removed_del_subcommand() {
        let err = Cli::try_parse_from(["peanutbutter", "del", "Echo"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::InvalidSubcommand);
    }

    #[test]
    fn clap_rejects_old_add_subcommand() {
        let err = Cli::try_parse_from(["peanutbutter", "add", "nested/demo"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::InvalidSubcommand);
    }

    #[test]
    fn clap_rejects_removed_stats_json_flag() {
        let err = Cli::try_parse_from(["peanutbutter", "stats", "--json"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::UnknownArgument);
    }

    #[test]
    fn after_help_mentions_all_shells_and_pb_alias() {
        let paths = test_paths(Path::new("/tmp/snippets"));
        let help = after_help(&paths);
        assert!(help.contains("completions bash|zsh|fish|powershell` also defines `pb`"));
        assert!(help.contains("pb init"));
        assert!(help.contains("snippet roots:"));
        assert!(help.contains("/tmp/snippets"));
    }

    #[test]
    fn clap_help_mentions_completions_subcommand() {
        let mut command = <Cli as clap::CommandFactory>::command();
        let mut help = Vec::new();
        command.write_long_help(&mut help).unwrap();
        let help = String::from_utf8(help).unwrap();
        assert!(help.contains("completions"));
        assert!(!help.contains("--bash"));
    }

    #[test]
    fn run_init_command_writes_skips_and_forces_starter_file() {
        let root = temp_dir("init");
        let paths = test_paths(&root);
        let target = root.join("snippets.md");

        let mut out = Vec::new();
        run_init_command(&paths, false, &mut out).unwrap();
        assert!(target.exists());
        assert!(
            String::from_utf8(out)
                .unwrap()
                .contains("docs/SNIPPET_SYNTAX.md")
        );
        assert!(fs::read_to_string(&target).unwrap().contains("<@branch>"));

        fs::write(&target, "custom").unwrap();
        let mut out = Vec::new();
        run_init_command(&paths, false, &mut out).unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "custom");
        assert!(String::from_utf8(out).unwrap().contains("already exists"));

        let mut out = Vec::new();
        run_init_command(&paths, true, &mut out).unwrap();
        assert!(
            fs::read_to_string(&target)
                .unwrap()
                .contains("Conventional commit")
        );
    }

    #[test]
    fn execute_auto_init_seeds_before_loading_index() {
        let root = temp_dir("auto-init").join("snippets");
        let paths = test_paths(&root);
        let mut saw_starter = false;

        run_execute_command_with(&paths, &mut Vec::new(), None, |index, _store, _options| {
            saw_starter = index
                .iter()
                .any(|snippet| snippet.name() == "Conventional commit");
            Ok(None)
        })
        .unwrap();

        assert!(saw_starter);
        assert!(root.join("snippets.md").exists());
    }

    #[test]
    fn execute_auto_init_does_not_retrigger_when_dir_exists() {
        let root = temp_dir("auto-init-once").join("snippets");
        let paths = test_paths(&root);

        run_execute_command_with(&paths, &mut Vec::new(), None, |_index, _store, _options| {
            Ok(None)
        })
        .unwrap();
        fs::write(root.join("snippets.md"), "custom").unwrap();

        run_execute_command_with(&paths, &mut Vec::new(), None, |_index, _store, _options| {
            Ok(None)
        })
        .unwrap();

        assert_eq!(
            fs::read_to_string(root.join("snippets.md")).unwrap(),
            "custom"
        );
    }

    #[test]
    fn execute_auto_init_failure_does_not_abort_empty_index() {
        let root = temp_dir("auto-init-failure");
        let blocker = root.join("blocker");
        fs::write(&blocker, "not a directory").unwrap();
        let paths = Paths {
            snippet_roots: vec![root.join("empty")],
            xdg_snippets_dir: blocker.join("snippets"),
            snippet_overrides_active: false,
            state_file: root.join("state.tsv"),
            config_file: root.join("config.toml"),
        };
        let mut out = Vec::new();
        let mut saw_empty_index = false;

        run_execute_command_with(&paths, &mut out, None, |index, _store, _options| {
            saw_empty_index = index.is_empty();
            Ok(None)
        })
        .unwrap();

        assert!(saw_empty_index);
        assert!(out.is_empty());
    }

    #[test]
    fn concurrent_init_writes_parseable_starter_file() {
        let root = temp_dir("init-concurrent");
        let paths = test_paths(&root);
        let first = paths.clone();
        let second = paths.clone();

        let a = std::thread::spawn(move || write_starter_snippets(&first, false));
        let b = std::thread::spawn(move || write_starter_snippets(&second, false));
        a.join().unwrap().unwrap();
        b.join().unwrap().unwrap();

        let index = crate::index::load_from_roots(&paths.snippet_roots).unwrap();
        assert!(
            index
                .iter()
                .any(|snippet| snippet.name() == "Conventional commit")
        );
    }

    #[test]
    fn overrides_active_matches_env_and_paths_flags() {
        let root = temp_dir("overrides");
        let mut paths = test_paths(&root);
        let old = env::var_os("PEANUTBUTTER_PATH");

        unsafe { env::remove_var("PEANUTBUTTER_PATH") };
        paths.snippet_overrides_active = false;
        assert!(!overrides_active(&paths));

        paths.snippet_overrides_active = true;
        assert!(overrides_active(&paths));

        paths.snippet_overrides_active = false;
        unsafe { env::set_var("PEANUTBUTTER_PATH", "") };
        assert!(overrides_active(&paths));

        match old {
            Some(value) => unsafe { env::set_var("PEANUTBUTTER_PATH", value) },
            None => unsafe { env::remove_var("PEANUTBUTTER_PATH") },
        }
    }

    #[test]
    fn run_execute_command_persists_usage_after_write() {
        let root = temp_dir("execute-record");
        let snippet_file = root.join("snippets.md");
        fs::write(&snippet_file, "## Echo\n\n```\necho hi\n```\n").unwrap();
        let paths = test_paths(&root);

        let mut out = Vec::new();
        let result =
            run_execute_command_with(&paths, &mut out, None, |_index, _store, _options| {
                Ok(Some(ExecutionOutcome {
                    snippet_id: SnippetId::new("snippets.md", "echo"),
                    command: "echo hi".to_string(),
                    consumed_buffer: false,
                }))
            })
            .unwrap();

        assert!(result.emitted);
        assert!(!result.replace_buffer);
        assert!(result.persist_warning.is_none());
        assert_eq!(String::from_utf8(out).unwrap(), "echo hi\n");
        let saved = FrecencyStore::load(&paths.state_file).unwrap();
        assert_eq!(saved.events().len(), 1);
        assert_eq!(saved.events()[0].id.as_str(), "snippets.md#echo");
    }

    #[test]
    fn run_execute_command_signals_replace_when_buffer_consumed() {
        let root = temp_dir("execute-replace");
        fs::write(root.join("snippets.md"), "## Echo\n\n```\necho hi\n```\n").unwrap();
        let paths = test_paths(&root);

        let mut out = Vec::new();
        let result =
            run_execute_command_with(&paths, &mut out, None, |_index, _store, _options| {
                Ok(Some(ExecutionOutcome {
                    snippet_id: SnippetId::new("snippets.md", "echo"),
                    command: "echo hi | xclip".to_string(),
                    consumed_buffer: true,
                }))
            })
            .unwrap();

        assert!(result.emitted);
        assert!(result.replace_buffer);
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
        let err =
            run_execute_command_with(&paths, &mut writer, None, |_index, _store, _options| {
                Ok(Some(ExecutionOutcome {
                    snippet_id: SnippetId::new("snippets.md", "echo"),
                    command: "echo hi".to_string(),
                    consumed_buffer: false,
                }))
            })
            .unwrap_err();
        assert_eq!(err.to_string(), "nope");
        let saved = FrecencyStore::load(&paths.state_file).unwrap();
        assert!(saved.events().is_empty());
    }
}
