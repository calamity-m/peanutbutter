//! UI state and key handling for the `pb repo` TUI.

use crate::config::{AppConfig, Theme};
use crate::edit::editor::{self, EditorTarget};
use crate::repo::discover::SnippetRepo;
use crate::repo::git::{self, GitOperation, GitSummary};
use crate::repo::persist;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::io;
use std::path::PathBuf;

/// What the event loop should do after a key press.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RepoEvent {
    Continue,
    Quit,
    /// Run a git operation against the selected repository.
    RunGit(GitOperation),
    /// Hide or unhide the selected repository.
    ToggleHide,
    /// Open `$VISUAL`/`$EDITOR` at the selected repository root.
    Jump,
}

/// Per-repo git state, populated by [`RepoApp::refresh_git_summaries`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RepoGitState {
    Unknown,
    Summary(GitSummary),
    Error(String),
    /// A snippet root that is not a git repository (jump/hide only).
    NotARepo,
}

pub(crate) struct RepoApp {
    pub(crate) theme: Theme,
    pub(crate) repos: Vec<SnippetRepo>,
    pub(crate) git_states: Vec<RepoGitState>,
    pub(crate) selected: usize,
    pub(crate) status: Option<String>,
    config_file: PathBuf,
}

impl RepoApp {
    pub(crate) fn new(config: &AppConfig, repos: Vec<SnippetRepo>) -> Self {
        let git_states = vec![RepoGitState::Unknown; repos.len()];
        Self {
            theme: config.theme.clone(),
            repos,
            git_states,
            selected: 0,
            status: None,
            config_file: config.paths.config_file.clone(),
        }
    }

    pub(crate) fn selected_repo(&self) -> Option<&SnippetRepo> {
        self.repos.get(self.selected)
    }

    pub(crate) fn set_status(&mut self, status: impl Into<String>) {
        self.status = Some(status.into());
    }

    /// Translate a key press into a [`RepoEvent`]. Pure — all side effects
    /// (git subprocesses, config writes, editor launch) live in the caller or
    /// in the dedicated methods below so this stays unit-testable.
    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> RepoEvent {
        if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c')) {
            return RepoEvent::Quit;
        }
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => RepoEvent::Quit,
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = self.selected.saturating_sub(1);
                RepoEvent::Continue
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.selected + 1 < self.repos.len() {
                    self.selected += 1;
                }
                RepoEvent::Continue
            }
            KeyCode::Char('r') => {
                self.refresh_git_summaries();
                self.set_status("refreshed");
                RepoEvent::Continue
            }
            _ if self.repos.is_empty() => RepoEvent::Continue,
            KeyCode::Char('s') => self.git_event(GitOperation::Sync),
            KeyCode::Char('p') => self.git_event(GitOperation::Push),
            KeyCode::Char('u') => self.git_event(GitOperation::Pull),
            KeyCode::Char('h') => RepoEvent::ToggleHide,
            KeyCode::Enter => RepoEvent::Jump,
            _ => RepoEvent::Continue,
        }
    }

    /// Map a git action to an event, refusing it on a non-repo entry (a snippet
    /// root with no git repository) where sync/push/pull are meaningless.
    fn git_event(&mut self, operation: GitOperation) -> RepoEvent {
        if self.selected_repo().is_some_and(|repo| repo.is_repo) {
            RepoEvent::RunGit(operation)
        } else {
            self.set_status(format!(
                "{} unavailable: not a git repository (jump and hide only)",
                operation.label()
            ));
            RepoEvent::Continue
        }
    }

    /// (Re)load branch/dirty/upstream summaries for every repository. Non-repo
    /// entries have no git state to read and are flagged as such.
    pub(crate) fn refresh_git_summaries(&mut self) {
        for (repo, state) in self.repos.iter().zip(self.git_states.iter_mut()) {
            if !repo.is_repo {
                *state = RepoGitState::NotARepo;
                continue;
            }
            *state = match git::summarize(&repo.path) {
                Ok(summary) => RepoGitState::Summary(summary),
                Err(err) => RepoGitState::Error(err),
            };
        }
    }

    /// Run `operation` against the selected repository and report the result
    /// in the status line. Blocking; git output is captured, never printed.
    ///
    /// `redraw` is called after each step's status is set so the caller can
    /// repaint the (otherwise frozen) TUI while the subprocess runs, surfacing
    /// e.g. `sync — pulling (rebase)` before the network round-trip.
    pub(crate) fn run_git_operation(
        &mut self,
        operation: GitOperation,
        redraw: &mut dyn FnMut(&RepoApp),
    ) {
        let Some(repo) = self.selected_repo() else {
            return;
        };
        let display = repo.display.clone();
        let path = repo.path.clone();
        let label = operation.label();
        let result = {
            let mut progress = |step: &str| {
                self.status = Some(format!("{display}: {label} — {step}"));
                redraw(self);
            };
            git::run_operation(&path, operation, &mut progress)
        };
        let status = match result {
            Ok(steps) => format!("{display}: {label} ok ({steps})"),
            Err(err) => format!("{display}: {label} failed: {err}"),
        };
        if let Some(state) = self.git_states.get_mut(self.selected) {
            *state = match git::summarize(&path) {
                Ok(summary) => RepoGitState::Summary(summary),
                Err(err) => RepoGitState::Error(err),
            };
        }
        self.set_status(status);
    }

    /// Hide the selected repository (append its entry to `[paths] ignored`) or
    /// unhide it (remove the entry). Updates the in-memory flag and status.
    pub(crate) fn toggle_hide_selected(&mut self) {
        let Some(repo) = self.repos.get(self.selected) else {
            return;
        };
        let entry = repo.ignore_entry();
        let display = repo.display.clone();
        let hide = !repo.hidden;
        let result: io::Result<String> = if hide {
            persist::add_ignored_entry(&self.config_file, &entry)
                .map(|()| format!("{display}: hidden ({entry} added to [paths] ignored)"))
        } else {
            persist::remove_ignored_entry(&self.config_file, &entry).map(|removed| {
                if removed {
                    format!("{display}: unhidden")
                } else {
                    format!(
                        "{display}: no verbatim `{entry}` entry; it may be hidden by a glob — edit [paths] ignored manually"
                    )
                }
            })
        };
        match result {
            Ok(status) => {
                // Only flip the flag when hiding, or when an entry was really
                // removed; a glob-hidden repo stays hidden.
                if hide || status.ends_with("unhidden") {
                    self.repos[self.selected].hidden = hide;
                }
                self.set_status(status);
            }
            Err(err) => self.set_status(format!("{display}: config update failed: {err}")),
        }
    }

    /// Open `$VISUAL` / `$EDITOR` at the selected repository root. The caller
    /// must suspend the TUI around this. Returns the repo path on success.
    pub(crate) fn jump_selected(&mut self) -> io::Result<PathBuf> {
        let repo = self
            .selected_repo()
            .ok_or_else(|| io::Error::other("no repository selected"))?;
        let path = repo.path.clone();
        editor::open(&EditorTarget::file(path.clone()), None)?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Paths;
    use std::fs;
    use std::path::Path;

    fn temp_dir(prefix: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};

        static NEXT: AtomicU64 = AtomicU64::new(1);
        let path = std::env::temp_dir().join(format!(
            "pb-repo-app-{prefix}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn test_config(root: &Path) -> AppConfig {
        AppConfig {
            paths: Paths {
                snippet_roots: vec![root.to_path_buf()],
                xdg_snippets_dir: root.to_path_buf(),
                snippet_overrides_active: false,
                ignored: Vec::new(),
                state_file: root.join("state.tsv"),
                config_file: root.join("config.toml"),
            },
            ui: crate::config::UiConfig::default(),
            search: crate::config::SearchConfig::default(),
            variables: Default::default(),
            theme: Theme::default(),
            suggestion_commands: Default::default(),
            lint: Default::default(),
            keybinds: crate::keybinds::Keymaps::default(),
        }
    }

    fn repo(root: &Path, display: &str, hidden: bool) -> SnippetRepo {
        SnippetRepo {
            path: root.join(display),
            root: root.to_path_buf(),
            display: display.to_string(),
            hidden,
            is_repo: true,
        }
    }

    fn non_repo(root: &Path, display: &str) -> SnippetRepo {
        SnippetRepo {
            path: root.join(display),
            root: root.to_path_buf(),
            display: display.to_string(),
            hidden: false,
            is_repo: false,
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn navigation_clamps_and_actions_map_to_events() {
        let root = temp_dir("nav");
        let config = test_config(&root);
        let mut app = RepoApp::new(
            &config,
            vec![repo(&root, "a", false), repo(&root, "b", false)],
        );

        assert_eq!(app.handle_key(key(KeyCode::Up)), RepoEvent::Continue);
        assert_eq!(app.selected, 0);
        app.handle_key(key(KeyCode::Down));
        assert_eq!(app.selected, 1);
        app.handle_key(key(KeyCode::Down));
        assert_eq!(app.selected, 1);

        assert_eq!(
            app.handle_key(key(KeyCode::Char('s'))),
            RepoEvent::RunGit(GitOperation::Sync)
        );
        assert_eq!(
            app.handle_key(key(KeyCode::Char('p'))),
            RepoEvent::RunGit(GitOperation::Push)
        );
        assert_eq!(
            app.handle_key(key(KeyCode::Char('u'))),
            RepoEvent::RunGit(GitOperation::Pull)
        );
        assert_eq!(
            app.handle_key(key(KeyCode::Char('h'))),
            RepoEvent::ToggleHide
        );
        assert_eq!(app.handle_key(key(KeyCode::Enter)), RepoEvent::Jump);
        assert_eq!(app.handle_key(key(KeyCode::Char('q'))), RepoEvent::Quit);
        assert_eq!(
            app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            RepoEvent::Quit
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn non_repo_entry_disables_git_but_keeps_jump_and_hide() {
        let root = temp_dir("non-repo");
        let config = test_config(&root);
        let mut app = RepoApp::new(&config, vec![non_repo(&root, "plain")]);

        // sync/push/pull are refused with an explanatory status.
        for code in [KeyCode::Char('s'), KeyCode::Char('p'), KeyCode::Char('u')] {
            assert_eq!(app.handle_key(key(code)), RepoEvent::Continue);
            assert!(
                app.status
                    .as_deref()
                    .unwrap()
                    .contains("not a git repository"),
                "unexpected status: {:?}",
                app.status
            );
        }

        // jump and hide remain available.
        assert_eq!(app.handle_key(key(KeyCode::Enter)), RepoEvent::Jump);
        assert_eq!(
            app.handle_key(key(KeyCode::Char('h'))),
            RepoEvent::ToggleHide
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn actions_are_inert_with_no_repos() {
        let root = temp_dir("empty");
        let config = test_config(&root);
        let mut app = RepoApp::new(&config, Vec::new());

        assert_eq!(app.handle_key(key(KeyCode::Char('s'))), RepoEvent::Continue);
        assert_eq!(app.handle_key(key(KeyCode::Enter)), RepoEvent::Continue);
        assert_eq!(app.handle_key(key(KeyCode::Char('q'))), RepoEvent::Quit);
        app.toggle_hide_selected();
        assert!(app.status.is_none());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn toggle_hide_writes_and_removes_config_entry() {
        let root = temp_dir("toggle");
        let config = test_config(&root);
        let mut app = RepoApp::new(&config, vec![repo(&root, "team", false)]);

        app.toggle_hide_selected();
        assert!(app.repos[0].hidden);
        let saved = fs::read_to_string(root.join("config.toml")).unwrap();
        assert!(saved.contains("ignored"));
        assert!(saved.contains("team"));

        app.toggle_hide_selected();
        assert!(!app.repos[0].hidden);
        let saved = fs::read_to_string(root.join("config.toml")).unwrap();
        assert!(!saved.contains("\"team\""));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn unhide_without_verbatim_entry_keeps_repo_hidden() {
        let root = temp_dir("glob-hidden");
        let config = test_config(&root);
        fs::write(root.join("config.toml"), "[paths]\nignored = [\"te*\"]\n").unwrap();
        let mut app = RepoApp::new(&config, vec![repo(&root, "team", true)]);

        app.toggle_hide_selected();

        assert!(app.repos[0].hidden, "glob-hidden repo must stay hidden");
        assert!(app.status.as_deref().unwrap().contains("glob"));

        let _ = fs::remove_dir_all(&root);
    }
}
