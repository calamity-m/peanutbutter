//! Snippet capture and creation for `pb new`.

pub mod capture;
pub mod capture_heuristics;

use crate::config::{AppConfig, Paths};
use crate::new::capture::TargetChoice;
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Capture a recently-run command and append it as a new snippet.
///
/// Interactive runs read `$PEANUTBUTTER_HISTORY`, let the user choose a command,
/// confirm placeholder candidates, choose a destination file, then append the
/// generated Markdown snippet.
pub fn run_new_command(
    config: &AppConfig,
    name_opt: Option<String>,
    explicit_argv: Vec<String>,
) -> io::Result<()> {
    let paths = &config.paths;
    let targets = new_target_choices(paths)?;

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
                 (e.g. eval \"$(peanutbutter completions bash C+b)\") and re-run, or bypass with: \
                 pb new <name> -- <command...>",
            ));
        }
        Some(entries)
    };

    let outcome = capture::run_capture(capture::CaptureRun {
        history,
        explicit_command,
        name_opt,
        theme: &config.theme,
        viewport_height: config.ui.height,
        targets,
        keymap: &config.keybinds.new,
        keybind_warnings: &config.keybinds.warnings,
    })?;

    let (name, raw, accepted, first_token, target) = match outcome {
        capture::CaptureOutcome::Cancelled => return Ok(()),
        capture::CaptureOutcome::Accepted {
            name,
            raw,
            accepted,
            first_token,
            target,
        } => (name, raw, accepted, first_token, target),
    };

    let accepted = bump_against_frontmatter(&target, accepted)?;
    let body = capture_heuristics::render_with_placeholders(&raw, &accepted);
    let lang = guess_language(first_token.as_deref());
    let final_name = bump_until_unique(&target, &name)?;
    let target_written = append_snippet(&target, &final_name, &body, &lang)?;
    println!(
        "wrote 1 snippet \"{final_name}\" to {}",
        target_written.display()
    );
    Ok(())
}

/// Build the list of destination files offered by the `new` target picker.
///
/// Returns every existing snippet file under the configured roots, labelled by
/// root-relative path (alias-prefixed when more than one root is configured).
/// When no files exist yet, falls back to a single default
/// `<first-root>/snippets.md` choice so the picker can be skipped.
fn new_target_choices(paths: &Paths) -> io::Result<Vec<TargetChoice>> {
    let files = crate::discovery::discover_all_ignoring(
        &paths.snippet_roots,
        &crate::discovery::IgnoreRules::new(paths.ignored.clone()),
    )?;
    if files.is_empty() {
        let root = paths
            .snippet_roots
            .first()
            .ok_or_else(|| io::Error::other("no snippet roots configured"))?;
        return Ok(vec![TargetChoice {
            label: crate::edit::DEFAULT_EDIT_PATH.to_string(),
            path: root.join(crate::edit::DEFAULT_EDIT_PATH),
        }]);
    }

    let aliases = crate::edit::edit_root_aliases(paths);
    let multi = paths.snippet_roots.len() > 1;
    Ok(files
        .into_iter()
        .map(|file| {
            let label = target_label(&file, &aliases, multi);
            TargetChoice { label, path: file }
        })
        .collect())
}

/// Render a short, stable label for a discovered snippet file: its path
/// relative to the owning root, prefixed with the root alias when multiple
/// roots are configured.
fn target_label(file: &Path, aliases: &[crate::edit::EditRootAlias], multi: bool) -> String {
    let owner = aliases
        .iter()
        .filter(|entry| file.starts_with(&entry.root))
        .max_by_key(|entry| entry.root.as_os_str().len());
    match owner {
        Some(entry) => {
            let rel = file
                .strip_prefix(&entry.root)
                .unwrap_or(file)
                .to_string_lossy()
                .replace('\\', "/");
            let rel = if rel.is_empty() {
                file.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            } else {
                rel
            };
            if multi {
                format!("{}/{rel}", entry.alias)
            } else {
                rel
            }
        }
        None => file.to_string_lossy().replace('\\', "/"),
    }
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
    accepted: Vec<(capture_heuristics::Span, String)>,
) -> io::Result<Vec<(capture_heuristics::Span, String)>> {
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
        let final_name = capture_heuristics::bump_name(&candidate, &mut seen);
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
            xdg_snippets_dir: root.to_path_buf(),
            snippet_overrides_active: false,
            ignored: Vec::new(),
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
            ignored: Vec::new(),
            state_file: first.join("state.tsv"),
            config_file: first.join("config.toml"),
        }
    }

    #[test]
    fn target_label_uses_relative_path_for_single_root() {
        let paths = test_paths_with_roots(vec![PathBuf::from("/home/me/snippets")]);
        let aliases = crate::edit::edit_root_aliases(&paths);
        assert_eq!(
            target_label(Path::new("/home/me/snippets/work/db.md"), &aliases, false),
            "work/db.md"
        );
    }

    #[test]
    fn target_label_prefixes_alias_for_multiple_roots() {
        let paths = test_paths_with_roots(vec![
            PathBuf::from("/home/me/snippets"),
            PathBuf::from("/home/me/work"),
        ]);
        let aliases = crate::edit::edit_root_aliases(&paths);
        assert_eq!(
            target_label(Path::new("/home/me/work/db.md"), &aliases, true),
            "work/db.md"
        );
        assert_eq!(
            target_label(Path::new("/home/me/snippets/a.md"), &aliases, true),
            "snippets/a.md"
        );
    }

    #[test]
    fn new_target_choices_falls_back_to_default_when_empty() {
        let root = temp_dir("new-targets");
        let paths = test_paths(&root);
        let choices = new_target_choices(&paths).unwrap();
        assert_eq!(choices.len(), 1);
        assert_eq!(choices[0].label, crate::edit::DEFAULT_EDIT_PATH);
        assert_eq!(choices[0].path, root.join(crate::edit::DEFAULT_EDIT_PATH));
    }

    #[test]
    fn writer_creates_file_and_passes_lint() {
        let root = temp_dir("new-write-lint");
        let target = root.join("snippets.md");
        let written = append_snippet(&target, "demo", "echo <@name>", "bash").unwrap();
        assert_eq!(written, target);
        let content = fs::read_to_string(&target).unwrap();
        assert!(content.contains("## demo"));
        assert!(content.contains("```bash"));
        assert!(content.contains("echo <@name>"));

        let paths = test_paths(&root);
        let app_config = crate::config::AppConfig {
            paths,
            ui: Default::default(),
            search: crate::config::SearchConfig::default(),
            variables: Default::default(),
            theme: crate::config::Theme::default(),
            suggestion_commands: Default::default(),
            lint: Default::default(),
            keybinds: Default::default(),
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
    fn writer_collision_bumps_heading_suffix() {
        let root = temp_dir("new-write-collision");
        let target = root.join("snippets.md");
        fs::write(&target, "## demo\n\n```bash\necho a\n```\n").unwrap();
        let first = bump_until_unique(&target, "demo").unwrap();
        assert_eq!(first, "demo (2)");
        append_snippet(&target, &first, "echo b", "bash").unwrap();
        let second = bump_until_unique(&target, "demo").unwrap();
        assert_eq!(second, "demo (3)");
    }

    #[test]
    fn bumps_against_frontmatter_variable_keys() {
        let root = temp_dir("new-frontmatter");
        let target = root.join("snippets.md");
        fs::write(
            &target,
            "---\nvariables:\n  host:\n    default: localhost\n---\n",
        )
        .unwrap();
        let accepted = vec![(
            capture_heuristics::Span { start: 0, end: 3 },
            "host".to_string(),
        )];
        let bumped = bump_against_frontmatter(&target, accepted).unwrap();
        assert_eq!(bumped[0].1, "host2");
    }

    #[test]
    fn guess_language_falls_back_to_sh() {
        assert_eq!(guess_language(Some("ssh")), "bash");
        assert_eq!(guess_language(Some("/usr/bin/git")), "bash");
        assert_eq!(guess_language(Some("randomtool")), "sh");
        assert_eq!(guess_language(None), "sh");
    }

    #[test]
    fn shell_quote_argv_round_trips_simple_args() {
        let q = shell_quote_argv(&["echo".to_string(), "hello world".to_string()]);
        assert_eq!(q, "echo 'hello world'");
    }

    #[test]
    fn full_pipeline_writes_lint_clean_snippet() {
        let root = temp_dir("new-pipeline");
        let target = root.join("snippets.md");
        let raw = "ssh root@10.0.0.4 'systemctl restart nginx'";
        let cands = capture_heuristics::detect_variables(raw);
        let accepted: Vec<_> = cands
            .iter()
            .map(|c| (c.span, c.suggested_name.clone()))
            .collect();
        let body = capture_heuristics::render_with_placeholders(raw, &accepted);
        append_snippet(&target, "deploy", &body, "bash").unwrap();
        let paths = test_paths(&root);
        let app_config = crate::config::AppConfig {
            paths,
            ui: Default::default(),
            search: crate::config::SearchConfig::default(),
            variables: Default::default(),
            theme: crate::config::Theme::default(),
            suggestion_commands: Default::default(),
            lint: Default::default(),
            keybinds: Default::default(),
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
}
