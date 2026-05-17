use crate::config::Paths;
use crate::editor::{self, EditorTarget};
use crate::execute::{self, ExecuteOptions, ExecutionOutcome};
use crate::frecency::FrecencyStore;
use crate::index::SnippetIndex;
use crate::stats;
use crate::{BASH_ALIAS_NAME, BINARY_NAME};
use clap::{Parser, Subcommand};
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_EDIT_PATH: &str = "snippets.md";
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
    /// Emit shell integration code for the given readline binding.
    Bash {
        #[arg(default_value = "C+b")]
        binding: String,
    },
    /// Emit zsh integration code for the given ZLE binding.
    Zsh {
        #[arg(default_value = "C+b")]
        binding: String,
    },
    /// Emit fish integration code for the given key binding.
    Fish {
        #[arg(default_value = "C+b")]
        binding: String,
    },
    /// Emit PowerShell integration code for the given PSReadLine binding.
    Powershell {
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
        /// Emit JSON instead of human-readable text.
        #[arg(long)]
        json: bool,
    },
    /// Internal shell completion helper for `edit`.
    #[command(hide = true)]
    CompleteEdit { current: Option<String> },
    /// Internal shell completion helper for `--theme`.
    #[command(hide = true)]
    CompleteTheme { current: Option<String> },
    /// Start a Language Server Protocol server over stdio for snippet authoring.
    Lsp,
}

/// Result returned by [`run_execute_command`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecuteCommandResult {
    /// `true` if a command was written to the output writer.
    pub emitted: bool,
    /// Non-fatal warning shown when the frecency state could not be saved.
    pub persist_warning: Option<String>,
}

/// Produce dynamic help content for resolved snippet roots and config/state
/// paths. This is attached to clap's generated help at runtime.
pub fn after_help(paths: &Paths) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "shell integration: `{BINARY_NAME} bash|zsh|fish|powershell` also defines `{BASH_ALIAS_NAME}`\n"
    ));
    out.push_str(&format!(
        "new here? run `{BASH_ALIAS_NAME} init` to scaffold starter snippets.\n\n"
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
    let target = paths.xdg_snippets_dir.join(DEFAULT_EDIT_PATH);
    if target.exists() && !force {
        return Ok(InitOutcome::Skipped(display_path(&target)));
    }

    let tmp = paths.xdg_snippets_dir.join(format!(
        ".{DEFAULT_EDIT_PATH}.tmp-{}-{}",
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

/// Capture a recently-run command and append it as a new snippet to
/// `<first-root>/snippets.md`. See [`Command::New`].
pub fn run_new_command(
    paths: &Paths,
    theme: &crate::config::Theme,
    viewport_height: u16,
    name_opt: Option<String>,
    explicit_argv: Vec<String>,
) -> io::Result<()> {
    let target_root = paths
        .snippet_roots
        .first()
        .ok_or_else(|| io::Error::other("no snippet roots configured"))?;
    let target = target_root.join("snippets.md");

    let explicit_command = if explicit_argv.is_empty() {
        None
    } else {
        Some(shell_quote_argv(&explicit_argv))
    };

    let history = if explicit_command.is_some() {
        None
    } else {
        let raw = env::var("PEANUTBUTTER_HISTORY").unwrap_or_default();
        let entries: Vec<String> = raw
            .split('\u{1F}')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if entries.is_empty() {
            return Err(io::Error::other(
                "pb new: no shell history available. Source the shell integration \
                 (e.g. eval \"$(peanutbutter bash C+b)\") and re-run, or bypass with: \
                 pb new <name> -- <command...>",
            ));
        }
        Some(entries)
    };

    let outcome = crate::capture::run_capture(crate::capture::CaptureRun {
        history,
        explicit_command,
        name_opt,
        theme,
        viewport_height,
        target_display: target.display().to_string(),
    })?;

    let (name, raw, accepted, first_token) = match outcome {
        crate::capture::CaptureOutcome::Cancelled => return Ok(()),
        crate::capture::CaptureOutcome::Accepted {
            name,
            raw,
            accepted,
            first_token,
        } => (name, raw, accepted, first_token),
    };

    let accepted = bump_against_frontmatter(&target, accepted)?;
    let body = crate::capture_heuristics::render_with_placeholders(&raw, &accepted);
    let lang = guess_language(first_token.as_deref());
    let final_name = bump_until_unique(&target, &name)?;
    let target_written = append_snippet(&target, &final_name, &body, &lang)?;
    println!(
        "wrote 1 snippet \"{final_name}\" to {}",
        target_written.display()
    );
    Ok(())
}

#[cfg(test)]
pub(crate) fn test_shell_quote_argv(argv: &[String]) -> String {
    shell_quote_argv(argv)
}

#[cfg(test)]
pub(crate) fn test_guess_language(first_token: Option<&str>) -> String {
    guess_language(first_token)
}

#[cfg(test)]
pub(crate) fn test_bump_until_unique(target: &Path, base: &str) -> io::Result<String> {
    bump_until_unique(target, base)
}

#[cfg(test)]
pub(crate) fn test_append_snippet(
    target: &Path,
    name: &str,
    body: &str,
    lang: &str,
) -> io::Result<PathBuf> {
    append_snippet(target, name, body, lang)
}

#[cfg(test)]
pub(crate) fn test_bump_against_frontmatter(
    target: &Path,
    accepted: Vec<(crate::capture_heuristics::Span, String)>,
) -> io::Result<Vec<(crate::capture_heuristics::Span, String)>> {
    bump_against_frontmatter(target, accepted)
}

fn shell_quote_argv(argv: &[String]) -> String {
    argv.iter()
        .map(|a| {
            if a.chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_./=:@,".contains(c))
                && !a.is_empty()
            {
                a.clone()
            } else {
                format!("'{}'", a.replace('\'', "'\\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn guess_language(first_token: Option<&str>) -> String {
    let known_shells = [
        "bash",
        "sh",
        "zsh",
        "fish",
        "ssh",
        "docker",
        "kubectl",
        "git",
        "curl",
        "wget",
        "make",
        "npm",
        "yarn",
        "pnpm",
        "cargo",
        "go",
        "rustc",
        "python",
        "python3",
        "node",
        "deno",
        "rg",
        "ls",
        "cat",
        "echo",
        "mv",
        "cp",
        "rm",
        "find",
        "grep",
        "sed",
        "awk",
        "tar",
        "ssh-copy-id",
        "scp",
        "rsync",
    ];
    let token = match first_token {
        Some(t) => Path::new(t)
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or(t),
        None => return "sh".to_string(),
    };
    if known_shells.contains(&token) {
        "bash".to_string()
    } else {
        "sh".to_string()
    }
}

fn bump_against_frontmatter(
    target: &Path,
    accepted: Vec<(crate::capture_heuristics::Span, String)>,
) -> io::Result<Vec<(crate::capture_heuristics::Span, String)>> {
    if !target.exists() {
        return Ok(accepted);
    }
    let content = fs::read_to_string(target)?;
    let parsed =
        crate::parser::parse_file(target, target.parent().unwrap_or(Path::new(".")), &content);
    let reserved: std::collections::HashSet<String> =
        parsed.frontmatter.variables.keys().cloned().collect();
    if reserved.is_empty() {
        return Ok(accepted);
    }
    let mut seen: std::collections::HashMap<String, usize> = Default::default();
    let mut out = Vec::with_capacity(accepted.len());
    for (span, name) in accepted {
        let mut candidate = name.clone();
        if reserved.contains(&candidate) {
            let mut n = 2;
            loop {
                let attempt = format!("{name}{n}");
                if !reserved.contains(&attempt) {
                    candidate = attempt;
                    break;
                }
                n += 1;
            }
        }
        let final_name = crate::capture_heuristics::bump_name(&candidate, &mut seen);
        out.push((span, final_name));
    }
    Ok(out)
}

fn bump_until_unique(target: &Path, base: &str) -> io::Result<String> {
    if !target.exists() {
        return Ok(base.to_string());
    }
    let content = fs::read_to_string(target)?;
    let existing_names: std::collections::HashSet<String> =
        crate::parser::parse_file(target, target.parent().unwrap_or(Path::new(".")), &content)
            .snippets
            .into_iter()
            .map(|s| s.name)
            .collect();

    if !existing_names.contains(base) {
        return Ok(base.to_string());
    }
    let mut n = 2;
    loop {
        let candidate = format!("{base} ({n})");
        if !existing_names.contains(&candidate) {
            return Ok(candidate);
        }
        n += 1;
    }
}

fn append_snippet(target: &Path, name: &str, body: &str, lang: &str) -> io::Result<PathBuf> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    let existing = if target.exists() {
        fs::read_to_string(target)?
    } else {
        String::new()
    };

    let mut next = existing.clone();
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    if !next.is_empty() && !next.ends_with("\n\n") {
        next.push('\n');
    }
    next.push_str("## ");
    next.push_str(name);
    next.push_str("\n\n```");
    next.push_str(lang);
    next.push('\n');
    next.push_str(body);
    if !body.ends_with('\n') {
        next.push('\n');
    }
    next.push_str("```\n");

    let tmp = target.with_extension("md.tmp");
    fs::write(&tmp, next)?;
    fs::rename(&tmp, target)?;
    Ok(target.to_path_buf())
}

/// Resolve the target snippet file and open it in `$EDITOR` / `$VISUAL`.
/// Creates the file (and any parent directories) if it doesn't exist yet.
/// Returns the path of the file that was opened.
pub fn run_edit_command(paths: &Paths, requested: Option<&Path>) -> io::Result<PathBuf> {
    run_edit_command_with_editor(paths, requested, None)
}

fn run_edit_command_with_editor(
    paths: &Paths,
    requested: Option<&Path>,
    editor_override: Option<&str>,
) -> io::Result<PathBuf> {
    let target = resolve_edit_target(paths, requested)?;
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    if !target.exists() {
        fs::write(&target, "")?;
    }
    editor::open(&EditorTarget::file(target.clone()), editor_override)?;
    Ok(target)
}

/// Determine the absolute path of the file to edit for `edit`.
///
/// - `None` → `<first-root>/snippets.md`
/// - Relative path → anchored under the first snippet root
/// - Absolute path → used as-is after verifying it's inside a known root
pub fn resolve_edit_target(paths: &Paths, requested: Option<&Path>) -> io::Result<PathBuf> {
    let default_root = paths
        .snippet_roots
        .first()
        .ok_or_else(|| io::Error::other("no snippet roots configured"))?;

    let aliases = edit_root_aliases(paths);
    let target = match requested {
        None => default_root.join(DEFAULT_EDIT_PATH),
        Some(path) if path.is_absolute() => {
            let target = normalize_edit_path(path.to_path_buf());
            if paths
                .snippet_roots
                .iter()
                .any(|root| target.starts_with(root))
            {
                target
            } else {
                return Err(io::Error::other(format!(
                    "absolute edit target must live under a configured snippet root: {}",
                    path.display()
                )));
            }
        }
        Some(path) if starts_with_current_dir(path) => {
            default_root.join(normalize_edit_path(strip_current_dir(path)))
        }
        Some(path) => {
            if let Some((root, child)) = resolve_root_qualified_path(path, &aliases) {
                if child.as_os_str().is_empty() {
                    root.join(DEFAULT_EDIT_PATH)
                } else {
                    root.join(normalize_edit_path(child))
                }
            } else {
                default_root.join(normalize_edit_path(path.to_path_buf()))
            }
        }
    };
    Ok(target)
}

/// Return bash completion candidates for the current `edit` argument.
pub fn complete_edit(paths: &Paths, current: &str) -> io::Result<Vec<String>> {
    let aliases = edit_root_aliases(paths);
    let mut candidates = Vec::new();

    if current.starts_with('/') {
        return Ok(candidates);
    } else if let Some((alias, rest)) = current.split_once('/') {
        if let Some(root) = aliases
            .iter()
            .find(|entry| entry.alias == alias)
            .map(|entry| entry.root.as_path())
        {
            candidates.extend(complete_under_root(root, rest, Some(alias))?);
        }
    } else {
        if let Some(root) = paths.snippet_roots.first() {
            candidates.extend(complete_under_root(root, current, None)?);
        }
        candidates.extend(
            aliases
                .iter()
                .filter(|entry| entry.alias.starts_with(current))
                .map(|entry| format!("{}/", entry.alias)),
        );
    }

    candidates.sort();
    candidates.dedup();
    Ok(candidates)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EditRootAlias {
    alias: String,
    root: PathBuf,
}

/// Build a short, unique alias for each snippet root. The alias is the leaf
/// path component by default; on collision (common because conventional setups
/// use `…/snippets/` for every root) we walk toward `/` and use the first
/// ancestor segment that disambiguates the colliding roots from each other.
/// A numeric suffix (`alias-N`) is the last-resort fallback when two roots are
/// indistinguishable even at every ancestor depth.
fn edit_root_aliases(paths: &Paths) -> Vec<EditRootAlias> {
    let component_lists: Vec<Vec<String>> = paths
        .snippet_roots
        .iter()
        .map(|root| named_components(root))
        .collect();
    let mut depths: Vec<usize> = vec![0; paths.snippet_roots.len()];
    let mut aliases: Vec<String> = component_lists
        .iter()
        .map(|components| component_at_depth(components, 0))
        .collect();

    loop {
        let colliding: Vec<usize> = (0..aliases.len())
            .filter(|&i| {
                aliases
                    .iter()
                    .enumerate()
                    .any(|(j, other)| j != i && other == &aliases[i])
            })
            .collect();
        if colliding.is_empty() {
            break;
        }
        let mut progressed = false;
        for &idx in &colliding {
            let next_depth = depths[idx] + 1;
            if next_depth < component_lists[idx].len() {
                depths[idx] = next_depth;
                aliases[idx] = component_at_depth(&component_lists[idx], next_depth);
                progressed = true;
            }
        }
        if !progressed {
            break;
        }
    }

    let mut final_aliases: Vec<String> = Vec::with_capacity(aliases.len());
    for alias in aliases {
        let mut candidate = alias;
        if final_aliases.contains(&candidate) {
            let base = candidate.clone();
            let mut next = 2;
            loop {
                let attempt = format!("{base}-{next}");
                if !final_aliases.contains(&attempt) {
                    candidate = attempt;
                    break;
                }
                next += 1;
            }
        }
        final_aliases.push(candidate);
    }

    paths
        .snippet_roots
        .iter()
        .zip(final_aliases)
        .map(|(root, alias)| EditRootAlias {
            alias,
            root: root.clone(),
        })
        .collect()
}

fn named_components(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect()
}

fn component_at_depth(components: &[String], depth: usize) -> String {
    if components.is_empty() {
        return "root".to_string();
    }
    let idx = components.len() - 1 - depth.min(components.len() - 1);
    components[idx].clone()
}

fn starts_with_current_dir(path: &Path) -> bool {
    matches!(path.components().next(), Some(Component::CurDir))
}

fn strip_current_dir(path: &Path) -> PathBuf {
    path.components()
        .filter(|component| !matches!(component, Component::CurDir))
        .collect()
}

fn resolve_root_qualified_path(
    path: &Path,
    aliases: &[EditRootAlias],
) -> Option<(PathBuf, PathBuf)> {
    let mut components = path.components();
    let Some(Component::Normal(first)) = components.next() else {
        return None;
    };
    let alias = first.to_str()?;
    let root = aliases
        .iter()
        .find(|entry| entry.alias == alias)
        .map(|entry| entry.root.clone())?;
    let child: PathBuf = components.collect();
    Some((root, child))
}

fn complete_under_root(root: &Path, current: &str, alias: Option<&str>) -> io::Result<Vec<String>> {
    let (dir_part, name_prefix) = split_completion_path(current);
    let dir = root.join(&dir_part);
    let mut out = Vec::new();
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(out),
        Err(err) => return Err(err),
    };

    for entry in entries {
        let entry = entry?;
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        if !file_name.starts_with(name_prefix) {
            continue;
        }
        let file_type = entry.file_type()?;
        let candidate_path = dir_part.join(file_name);
        if file_type.is_dir() {
            out.push(format_completion_candidate(&candidate_path, alias, true));
        } else if file_type.is_file() && is_markdown_path(Path::new(file_name)) {
            out.push(format_completion_candidate(&candidate_path, alias, false));
        }
    }

    Ok(out)
}

fn split_completion_path(current: &str) -> (PathBuf, &str) {
    if let Some((dir, name)) = current.rsplit_once('/') {
        (PathBuf::from(dir), name)
    } else {
        (PathBuf::new(), current)
    }
}

fn format_completion_candidate(path: &Path, alias: Option<&str>, is_dir: bool) -> String {
    let mut value = path.to_string_lossy().replace('\\', "/");
    if let Some(alias) = alias {
        value = format!("{alias}/{value}");
    }
    if is_dir {
        value.push('/');
    }
    value
}

fn is_markdown_path(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("md") | Some("markdown")
    )
}

fn normalize_edit_path(mut path: PathBuf) -> PathBuf {
    if path.extension().is_none() {
        path.set_extension("md");
    }
    path
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
    #[cfg(not(windows))]
    use std::process::Command as ProcessCommand;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[cfg(not(windows))]
    fn shell_quote(value: &str) -> String {
        format!("'{}'", value.replace('\'', "'\\''"))
    }

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

    fn test_paths_with_roots(roots: Vec<PathBuf>) -> Paths {
        let first = roots.first().unwrap().clone();
        Paths {
            snippet_roots: roots,
            xdg_snippets_dir: first.clone(),
            snippet_overrides_active: false,
            state_file: first.join("state.tsv"),
            config_file: first.join("config.toml"),
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
            Cli::try_parse_from(["peanutbutter", "bash"])
                .unwrap()
                .command,
            Some(Command::Bash {
                binding: "C+b".to_string()
            })
        );
        assert_eq!(
            Cli::try_parse_from(["peanutbutter", "bash", "C+f"])
                .unwrap()
                .command,
            Some(Command::Bash {
                binding: "C+f".to_string()
            })
        );
        assert_eq!(
            Cli::try_parse_from(["peanutbutter", "powershell"])
                .unwrap()
                .command,
            Some(Command::Powershell {
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
    fn after_help_mentions_all_shells_and_pb_alias() {
        let paths = test_paths(Path::new("/tmp/snippets"));
        let help = after_help(&paths);
        assert!(help.contains("bash|zsh|fish|powershell` also defines `pb`"));
        assert!(help.contains("pb init"));
        assert!(help.contains("snippet roots:"));
        assert!(help.contains("/tmp/snippets"));
    }

    #[test]
    fn clap_help_mentions_bash_subcommand() {
        let mut command = <Cli as clap::CommandFactory>::command();
        let mut help = Vec::new();
        command.write_long_help(&mut help).unwrap();
        let help = String::from_utf8(help).unwrap();
        assert!(help.contains("bash"));
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
    fn resolve_edit_target_defaults_and_appends_markdown_extension() {
        let root = temp_dir("edit-target");
        let paths = test_paths(&root);
        assert_eq!(
            resolve_edit_target(&paths, None).unwrap(),
            root.join("snippets.md")
        );
        assert_eq!(
            resolve_edit_target(&paths, Some(Path::new("docker/compose"))).unwrap(),
            root.join("docker/compose.md")
        );
    }

    #[test]
    fn edit_root_aliases_use_basename_when_unique() {
        let paths = test_paths_with_roots(vec![
            PathBuf::from("/home/me/snippets"),
            PathBuf::from("/home/me/work"),
            PathBuf::from("/home/me/personal"),
        ]);

        let aliases: Vec<_> = edit_root_aliases(&paths)
            .into_iter()
            .map(|entry| entry.alias)
            .collect();

        assert_eq!(aliases, vec!["snippets", "work", "personal"]);
    }

    #[test]
    fn edit_root_aliases_walk_up_to_disambiguate_basename_collisions() {
        let paths = test_paths_with_roots(vec![
            PathBuf::from("/home/me/.config/peanutbutter/snippets"),
            PathBuf::from("/home/me/.config/peanutbutter-private/snippets"),
        ]);

        let aliases: Vec<_> = edit_root_aliases(&paths)
            .into_iter()
            .map(|entry| entry.alias)
            .collect();

        assert_eq!(aliases, vec!["peanutbutter", "peanutbutter-private"]);
    }

    #[test]
    fn edit_root_aliases_walk_only_colliding_roots() {
        let paths = test_paths_with_roots(vec![
            PathBuf::from("/home/me/snippets"),
            PathBuf::from("/home/me/work"),
            PathBuf::from("/home/other/work"),
        ]);

        let aliases: Vec<_> = edit_root_aliases(&paths)
            .into_iter()
            .map(|entry| entry.alias)
            .collect();

        assert_eq!(aliases, vec!["snippets", "me", "other"]);
    }

    #[test]
    fn edit_root_aliases_walk_past_shared_ancestors() {
        let paths = test_paths_with_roots(vec![
            PathBuf::from("/home/work/team/snippets"),
            PathBuf::from("/home/personal/team/snippets"),
        ]);

        let aliases: Vec<_> = edit_root_aliases(&paths)
            .into_iter()
            .map(|entry| entry.alias)
            .collect();

        assert_eq!(aliases, vec!["work", "personal"]);
    }

    #[test]
    fn edit_root_aliases_fall_back_to_numeric_suffix_when_indistinguishable() {
        let paths =
            test_paths_with_roots(vec![PathBuf::from("/snippets"), PathBuf::from("/snippets")]);

        let aliases: Vec<_> = edit_root_aliases(&paths)
            .into_iter()
            .map(|entry| entry.alias)
            .collect();

        assert_eq!(aliases, vec!["snippets", "snippets-2"]);
    }

    #[test]
    fn resolve_edit_target_supports_root_qualified_paths_and_current_dir_escape() {
        let workspace = temp_dir("edit-qualified");
        let first_root = workspace.join("first");
        let work_root = workspace.join("work");
        fs::create_dir_all(&first_root).unwrap();
        fs::create_dir_all(&work_root).unwrap();
        let paths = test_paths_with_roots(vec![first_root.clone(), work_root.clone()]);

        assert_eq!(
            resolve_edit_target(&paths, Some(Path::new("work/docker.md"))).unwrap(),
            work_root.join("docker.md")
        );
        assert_eq!(
            resolve_edit_target(&paths, Some(Path::new("work"))).unwrap(),
            work_root.join("snippets.md")
        );
        assert_eq!(
            resolve_edit_target(&paths, Some(Path::new("./work/docker.md"))).unwrap(),
            first_root.join("work/docker.md")
        );
    }

    #[test]
    fn resolve_edit_target_keeps_absolute_paths_inside_any_root() {
        let workspace = temp_dir("edit-absolute");
        let first_root = workspace.join("first");
        let work_root = workspace.join("work");
        fs::create_dir_all(&first_root).unwrap();
        fs::create_dir_all(&work_root).unwrap();
        let paths = test_paths_with_roots(vec![first_root, work_root.clone()]);
        let target = work_root.join("docker/compose");

        assert_eq!(
            resolve_edit_target(&paths, Some(&target)).unwrap(),
            work_root.join("docker/compose.md")
        );
    }

    #[test]
    fn complete_edit_lists_first_root_entries_and_root_aliases() {
        let workspace = temp_dir("edit-complete-top");
        let first_root = workspace.join("snippets");
        let work_root = workspace.join("work");
        fs::create_dir_all(first_root.join("nested")).unwrap();
        fs::create_dir_all(&work_root).unwrap();
        fs::write(first_root.join("snippets.md"), "").unwrap();
        fs::write(first_root.join("readme.txt"), "").unwrap();
        fs::write(first_root.join("notes.markdown"), "").unwrap();
        let paths = test_paths_with_roots(vec![first_root, work_root]);

        let candidates = complete_edit(&paths, "").unwrap();

        assert_eq!(
            candidates,
            vec![
                "nested/".to_string(),
                "notes.markdown".to_string(),
                "snippets.md".to_string(),
                "snippets/".to_string(),
                "work/".to_string(),
            ]
        );
    }

    #[test]
    fn complete_edit_lists_nested_root_qualified_entries() {
        let workspace = temp_dir("edit-complete-root");
        let first_root = workspace.join("snippets");
        let work_root = workspace.join("work");
        fs::create_dir_all(&first_root).unwrap();
        fs::create_dir_all(work_root.join("docker/compose")).unwrap();
        fs::write(work_root.join("docker.md"), "").unwrap();
        fs::write(work_root.join("docker/compose/snip.md"), "").unwrap();
        let paths = test_paths_with_roots(vec![first_root, work_root]);

        assert_eq!(
            complete_edit(&paths, "work/do").unwrap(),
            vec!["work/docker.md".to_string(), "work/docker/".to_string()]
        );
        assert_eq!(
            complete_edit(&paths, "work/docker/compose/s").unwrap(),
            vec!["work/docker/compose/snip.md".to_string()]
        );
    }

    #[test]
    fn complete_edit_treats_missing_roots_as_empty() {
        let root = temp_dir("edit-complete-missing").join("missing");
        let paths = test_paths(&root);

        assert_eq!(
            complete_edit(&paths, "anything").unwrap(),
            Vec::<String>::new()
        );
    }

    #[test]
    fn run_edit_command_creates_file_and_invokes_editor() {
        let root = temp_dir("edit-command");
        let paths = test_paths(&root);
        let editor_log = root.join("editor.log");
        #[cfg(windows)]
        let editor = root.join("fake-editor.cmd");
        #[cfg(not(windows))]
        let editor = root.join("fake-editor.sh");

        #[cfg(windows)]
        fs::write(
            &editor,
            format!(
                "@echo off\r\n> \"{}\" echo %~1\r\nexit /b 0\r\n",
                editor_log.display()
            ),
        )
        .unwrap();
        #[cfg(not(windows))]
        {
            fs::write(
                &editor,
                format!(
                    "#!/usr/bin/env bash\nprintf '%s' \"$1\" > {}\n",
                    shell_quote(&editor_log.to_string_lossy())
                ),
            )
            .unwrap();
            let status = ProcessCommand::new("chmod")
                .arg("+x")
                .arg(&editor)
                .status()
                .unwrap();
            assert!(status.success());
        }

        let target = run_edit_command_with_editor(
            &paths,
            Some(Path::new("git/log")),
            Some(&editor.to_string_lossy()),
        )
        .unwrap();

        assert_eq!(target, root.join("git/log.md"));
        assert!(target.exists());
        let logged_path = fs::read_to_string(editor_log).unwrap();
        assert_eq!(
            logged_path.trim_end_matches(['\r', '\n']),
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
        let result =
            run_execute_command_with(&paths, &mut out, None, |_index, _store, _options| {
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
    fn new_command_writer_creates_file_and_passes_lint() {
        let root = temp_dir("new-write-lint");
        let target = root.join("snippets.md");
        let written = super::test_append_snippet(&target, "demo", "echo <@name>", "bash").unwrap();
        assert_eq!(written, target);
        let content = fs::read_to_string(&target).unwrap();
        assert!(content.contains("## demo"));
        assert!(content.contains("```bash"));
        assert!(content.contains("echo <@name>"));

        // Run lint over the file and assert no findings.
        let paths = test_paths(&root);
        let app_config = crate::config::AppConfig {
            paths,
            ui: Default::default(),
            search: crate::config::SearchConfig::default(),
            variables: Default::default(),
            theme: crate::config::Theme::default(),
            suggestion_commands: Default::default(),
            lint: Default::default(),
        };
        let mut buf = Vec::new();
        let result = crate::lint::run(
            &app_config,
            crate::lint::LintOptions {
                strict: false,
                json: false,
            },
            &mut buf,
        )
        .unwrap();
        assert!(
            !result.has_findings(),
            "lint findings: {}",
            String::from_utf8_lossy(&buf)
        );
    }

    #[test]
    fn new_command_writer_collision_bumps_heading_suffix() {
        let root = temp_dir("new-write-collision");
        let target = root.join("snippets.md");
        fs::write(&target, "## demo\n\n```bash\necho a\n```\n").unwrap();
        let first = super::test_bump_until_unique(&target, "demo").unwrap();
        assert_eq!(first, "demo (2)");
        super::test_append_snippet(&target, &first, "echo b", "bash").unwrap();
        let second = super::test_bump_until_unique(&target, "demo").unwrap();
        assert_eq!(second, "demo (3)");
    }

    #[test]
    fn new_command_bumps_against_frontmatter_variable_keys() {
        let root = temp_dir("new-frontmatter");
        let target = root.join("snippets.md");
        fs::write(
            &target,
            "---\nvariables:\n  host:\n    default: localhost\n---\n",
        )
        .unwrap();
        let accepted = vec![(
            crate::capture_heuristics::Span { start: 0, end: 3 },
            "host".to_string(),
        )];
        let bumped = super::test_bump_against_frontmatter(&target, accepted).unwrap();
        assert_eq!(bumped[0].1, "host2");
    }

    #[test]
    fn new_command_guess_language_falls_back_to_sh() {
        assert_eq!(super::test_guess_language(Some("ssh")), "bash");
        assert_eq!(super::test_guess_language(Some("/usr/bin/git")), "bash");
        assert_eq!(super::test_guess_language(Some("randomtool")), "sh");
        assert_eq!(super::test_guess_language(None), "sh");
    }

    #[test]
    fn new_command_shell_quote_argv_round_trips_simple_args() {
        let q = super::test_shell_quote_argv(&["echo".to_string(), "hello world".to_string()]);
        assert_eq!(q, "echo 'hello world'");
    }

    #[test]
    fn new_command_full_pipeline_writes_lint_clean_snippet() {
        let root = temp_dir("new-pipeline");
        let target = root.join("snippets.md");
        let raw = "ssh root@10.0.0.4 'systemctl restart nginx'";
        let cands = crate::capture_heuristics::detect_variables(raw);
        let accepted: Vec<_> = cands
            .iter()
            .map(|c| (c.span, c.suggested_name.clone()))
            .collect();
        let body = crate::capture_heuristics::render_with_placeholders(raw, &accepted);
        super::test_append_snippet(&target, "deploy", &body, "bash").unwrap();
        let paths = test_paths(&root);
        let app_config = crate::config::AppConfig {
            paths,
            ui: Default::default(),
            search: crate::config::SearchConfig::default(),
            variables: Default::default(),
            theme: crate::config::Theme::default(),
            suggestion_commands: Default::default(),
            lint: Default::default(),
        };
        let mut buf = Vec::new();
        let result = crate::lint::run(
            &app_config,
            crate::lint::LintOptions {
                strict: false,
                json: false,
            },
            &mut buf,
        )
        .unwrap();
        assert!(
            !result.has_findings(),
            "lint findings: {}",
            String::from_utf8_lossy(&buf)
        );
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
                }))
            })
            .unwrap_err();
        assert_eq!(err.to_string(), "nope");
        let saved = FrecencyStore::load(&paths.state_file).unwrap();
        assert!(saved.events().is_empty());
    }
}
