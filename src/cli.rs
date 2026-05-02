use crate::config::Paths;
use crate::editor::{self, EditorTarget};
use crate::execute::{self, ExecuteOptions, ExecutionOutcome};
use crate::frecency::FrecencyStore;
use crate::index::SnippetIndex;
use crate::{BASH_ALIAS_NAME, BINARY_NAME};
use clap::{Parser, Subcommand};
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_EDIT_PATH: &str = "snippets.md";

/// Terminal snippet manager.
#[derive(Debug, Clone, Parser, PartialEq, Eq)]
#[command(name = BINARY_NAME, about = "terminal snippet manager")]
#[command(version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// A parsed CLI subcommand, ready to dispatch.
#[derive(Debug, Clone, Subcommand, PartialEq, Eq)]
pub enum Command {
    /// Run the interactive TUI and emit the selected command to stdout.
    Execute,
    /// Open `$EDITOR` on the given snippet file (or the default file).
    Edit { path: Option<PathBuf> },
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
    /// Internal bash completion helper for `edit`.
    #[command(hide = true)]
    CompleteEdit { current: Option<String> },
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
        "shell integration: `{BINARY_NAME} bash|zsh|fish` also defines `{BASH_ALIAS_NAME}`\n\n"
    ));
    out.push_str("snippet roots:\n");
    for root in &paths.snippet_roots {
        out.push_str(&format!("  {}\n", root.display()));
    }
    out.push_str(&format!("config file: {}\n", paths.config_file.display()));
    out.push_str(&format!("state file: {}\n", paths.state_file.display()));
    out
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
        snippet_roots: paths.snippet_roots.clone(),
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
    use std::process::Command as ProcessCommand;
    use std::sync::atomic::{AtomicU64, Ordering};

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
            state_file: root.join("state.tsv"),
            config_file: root.join("config.toml"),
        }
    }

    fn test_paths_with_roots(roots: Vec<PathBuf>) -> Paths {
        let first = roots.first().unwrap().clone();
        Paths {
            snippet_roots: roots,
            state_file: first.join("state.tsv"),
            config_file: first.join("config.toml"),
        }
    }

    #[test]
    fn clap_recognizes_expected_commands() {
        assert_eq!(
            Cli::try_parse_from(["peanutbutter"]).unwrap(),
            Cli { command: None }
        );
        assert_eq!(
            Cli::try_parse_from(["peanutbutter", "execute"])
                .unwrap()
                .command,
            Some(Command::Execute)
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
            Cli::try_parse_from(["peanutbutter", "complete-edit", "nested/"])
                .unwrap()
                .command,
            Some(Command::CompleteEdit {
                current: Some("nested/".to_string())
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
        assert!(help.contains("bash|zsh|fish` also defines `pb`"));
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
        let editor = root.join("fake-editor.sh");
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

        let target = run_edit_command_with_editor(
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
}
