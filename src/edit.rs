//! Snippet file target resolution and command handling for `pb edit`.

use crate::config::Paths;
use crate::edit::editor::EditorTarget;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

pub mod editor;

/// Default snippet file used by `pb init`, `pb edit`, and `pb new`.
pub(crate) const DEFAULT_EDIT_PATH: &str = "snippets.md";

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
/// - `None` -> `<first-root>/snippets.md`
/// - Relative path -> anchored under the first snippet root
/// - Absolute path -> used as-is after verifying it's inside a known root
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

/// Return shell completion candidates for the current `edit` argument.
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

/// A short alias for a configured snippet root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EditRootAlias {
    /// User-facing alias used for root-qualified edit paths.
    pub(crate) alias: String,
    /// Absolute snippet root path represented by the alias.
    pub(crate) root: PathBuf,
}

/// Build a short, unique alias for each snippet root. The alias is the leaf
/// path component by default; on collision (common because conventional setups
/// use `.../snippets/` for every root) we walk toward `/` and use the first
/// ancestor segment that disambiguates the colliding roots from each other.
/// A numeric suffix (`alias-N`) is the last-resort fallback when two roots are
/// indistinguishable even at every ancestor depth.
pub(crate) fn edit_root_aliases(paths: &Paths) -> Vec<EditRootAlias> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Paths;
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
}
