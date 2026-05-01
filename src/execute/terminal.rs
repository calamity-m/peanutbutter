use crate::config;
use crate::domain::SnippetId;
use crate::editor;
use crate::frecency::FrecencyStore;
use crate::index::IndexedSnippet;
use crate::index::SnippetIndex;
use crossterm::cursor;
use crossterm::event::{self, DisableBracketedPaste, EnableBracketedPaste, Event};
use crossterm::execute;
use crossterm::terminal::{self, ClearType, disable_raw_mode, enable_raw_mode};
use ratatui::backend::CrosstermBackend;
use ratatui::{Terminal, TerminalOptions, Viewport};
use std::fs;
use std::io;
use std::io::IsTerminal;
use std::io::Write;
use std::os::fd::{FromRawFd, OwnedFd};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::app::{AppEvent, ExecutionApp, SuggestionProvider, SystemSuggestionProvider};
use super::{ExecuteOptions, ExecutionOutcome};

/// Load config, index, and frecency from disk, then run the execute TUI.
/// Convenience entry point used by the `execute` CLI command.
pub fn execute_default() -> io::Result<Option<ExecutionOutcome>> {
    let app_config = config::load()?;
    let index = crate::index::load_from_roots(&app_config.paths.snippet_roots)?;
    let frecency = FrecencyStore::load(&app_config.paths.state_file)?;
    let options = ExecuteOptions {
        cwd: std::env::current_dir().unwrap_or_else(|_| ".".into()),
        viewport_height: app_config.ui.height,
        search: app_config.search.clone(),
        theme: app_config.theme.clone(),
        variables: app_config.variables.clone(),
        snippet_roots: app_config.paths.snippet_roots.clone(),
        ..ExecuteOptions::default()
    };
    run_execute(index, frecency, options)
}

/// Run the execute TUI with the default [`SystemSuggestionProvider`].
///
/// Thin wrapper around [`run_execute_with_provider`] for callers that don't
/// need to inject a custom provider.
pub fn run_execute(
    index: SnippetIndex,
    frecency: FrecencyStore,
    options: ExecuteOptions,
) -> io::Result<Option<ExecutionOutcome>> {
    let provider = SystemSuggestionProvider::new(options.variables.clone());
    run_execute_with_provider(index, frecency, options, provider)
}

/// Core TUI runner — sets up the terminal, runs the event loop, tears down.
///
/// Steps:
/// 1. Redirect stdout to the TTY if it was piped (needed so the shell can
///    capture the emitted command while we still draw to the terminal).
/// 2. Enter raw mode via [`RawModeGuard`].
/// 3. Build a ratatui inline viewport and enter the draw/poll loop.
/// 4. On exit, drain any buffered key-release events (kitty keyboard protocol)
///    so they don't leak into the shell's readline.
/// 5. Erase the viewport lines and restore the cursor.
///
/// Returns `None` if the user cancelled, or `Some(outcome)` on completion.
pub fn run_execute_with_provider<P: SuggestionProvider>(
    index: SnippetIndex,
    frecency: FrecencyStore,
    options: ExecuteOptions,
    provider: P,
) -> io::Result<Option<ExecutionOutcome>> {
    let mut app = ExecutionApp::new(
        index,
        frecency,
        options.cwd,
        options.now,
        options.search,
        options.theme,
        provider,
    );
    let _stdout_guard = StdoutTtyGuard::enter()?;
    let mut raw_mode = RawModeGuard::enter()?;
    let mut terminal = build_terminal(options.viewport_height)?;
    let mut viewport_top: Option<u16> = None;
    let outcome = loop {
        terminal.draw(|frame| {
            viewport_top = viewport_top.or(Some(frame.area().y));
            app.render(frame);
        })?;
        if !event::poll(Duration::from_millis(250))? {
            continue;
        }
        let event = event::read()?;
        let app_event = match event {
            Event::Key(key) => app.handle_key(key),
            Event::Paste(text) => {
                app.handle_paste(&text);
                continue;
            }
            _ => continue,
        };
        match app_event {
            AppEvent::Continue => {}
            AppEvent::EditSnippet(id) => {
                let Some(snippet) = app.index.get(&id).cloned() else {
                    app.status = Some(edit_status(
                        EditResult::MissingSnippet(id),
                        &mut app,
                        &options.snippet_roots,
                    ));
                    continue;
                };
                cleanup_terminal(viewport_top)?;
                raw_mode.suspend()?;
                let edit_result = edit_snippet(&snippet);
                raw_mode.resume()?;
                terminal = build_terminal(options.viewport_height)?;
                viewport_top = None;
                app.status = Some(edit_status(edit_result, &mut app, &options.snippet_roots));
            }
            AppEvent::Cancelled => break None,
            AppEvent::Completed(outcome) => break Some(outcome),
        }
    };
    // Drain any buffered events (e.g. key-release events from the kitty keyboard
    // protocol) so they don't leak into the shell's readline after we exit.
    while event::poll(Duration::ZERO).unwrap_or(false) {
        let _ = event::read();
    }
    cleanup_terminal(viewport_top)?;
    Ok(outcome)
}

fn build_terminal(viewport_height: u16) -> io::Result<Terminal<CrosstermBackend<io::Stdout>>> {
    let backend = CrosstermBackend::new(io::stdout());
    let viewport_height = inline_viewport_height(viewport_height)?;
    match Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Inline(viewport_height),
        },
    ) {
        Ok(terminal) => Ok(terminal),
        Err(_) => Terminal::new(CrosstermBackend::new(io::stdout())),
    }
}

fn inline_viewport_height(max_height: u16) -> io::Result<u16> {
    let _ = terminal::size()?;
    Ok(compact_viewport_height(max_height))
}

pub(crate) fn compact_viewport_height(max_height: u16) -> u16 {
    max_height.max(1)
}

#[derive(Debug)]
enum EditResult {
    Edited(String),
    Failed(io::Error),
    MissingSnippet(SnippetId),
}

fn edit_snippet(snippet: &IndexedSnippet) -> EditResult {
    match editor::open_snippet(snippet, None) {
        Ok(()) => EditResult::Edited(snippet.name().to_string()),
        Err(err) => EditResult::Failed(err),
    }
}

fn edit_status<P: SuggestionProvider>(
    result: EditResult,
    app: &mut ExecutionApp<P>,
    snippet_roots: &[std::path::PathBuf],
) -> String {
    match result {
        EditResult::Edited(name) => reload_after_edit(app, snippet_roots, name),
        EditResult::Failed(err) => format!("edit failed: {err}"),
        EditResult::MissingSnippet(id) => format!("snippet no longer exists: {id}"),
    }
}

fn reload_after_edit<P: SuggestionProvider>(
    app: &mut ExecutionApp<P>,
    snippet_roots: &[std::path::PathBuf],
    name: String,
) -> String {
    let previous_id = app.selected_snippet().map(|snippet| snippet.id().clone());
    if snippet_roots.is_empty() {
        return format!("edited {name}; reload skipped");
    }
    match crate::index::load_from_roots(snippet_roots) {
        Ok(index) => {
            let previous_found = app.replace_index(index, previous_id.as_ref());
            if previous_found {
                format!("edited {name}; reloaded")
            } else {
                format!("edited {name}; reloaded, previous snippet not found")
            }
        }
        Err(err) => format!("edited {name}; reload failed: {err}"),
    }
}

/// RAII guard that enables terminal raw mode on construction and disables it
/// on drop, even if the TUI exits via `?`.
struct RawModeGuard {
    active: bool,
}

impl RawModeGuard {
    fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        // Bracketed paste lets the terminal deliver pasted text as a single
        // Event::Paste(String) — preserving newlines — instead of a stream of
        // KeyCode::Char events that strip them. Best-effort: terminals that
        // don't support it silently ignore the escape.
        let _ = execute!(io::stdout(), EnableBracketedPaste);
        Ok(Self { active: true })
    }

    fn suspend(&mut self) -> io::Result<()> {
        if self.active {
            let _ = execute!(io::stdout(), DisableBracketedPaste);
            disable_raw_mode()?;
            self.active = false;
        }
        Ok(())
    }

    fn resume(&mut self) -> io::Result<()> {
        if !self.active {
            enable_raw_mode()?;
            let _ = execute!(io::stdout(), EnableBracketedPaste);
            self.active = true;
        }
        Ok(())
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        if self.active {
            let _ = execute!(io::stdout(), DisableBracketedPaste);
            let _ = disable_raw_mode();
        }
    }
}

/// RAII guard that redirects stdout to the TTY when stdout is not a terminal.
///
/// When peanutbutter is invoked via the shell hotkey (`pb`), bash captures
/// stdout to write the selected command into the readline buffer. That means
/// fd 1 is a pipe, not a terminal — we can't draw the TUI there. This guard
/// saves fd 1, points it at stderr (if that's a terminal) or `/dev/tty`, and
/// restores the original fd on drop so the caller can still print the command.
struct StdoutTtyGuard {
    saved_stdout: Option<OwnedFd>,
}

impl StdoutTtyGuard {
    fn enter() -> io::Result<Self> {
        if io::stdout().is_terminal() {
            return Ok(Self { saved_stdout: None });
        }

        io::stdout().flush()?;
        let saved = unsafe { libc::dup(libc::STDOUT_FILENO) };
        if saved < 0 {
            return Err(io::Error::last_os_error());
        }

        if io::stderr().is_terminal() {
            if unsafe { libc::dup2(libc::STDERR_FILENO, libc::STDOUT_FILENO) } < 0 {
                let _ = unsafe { libc::close(saved) };
                return Err(io::Error::last_os_error());
            }
            return Ok(Self {
                saved_stdout: Some(unsafe { OwnedFd::from_raw_fd(saved) }),
            });
        }

        let tty = match fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/tty")
        {
            Ok(tty) => tty,
            Err(err) => {
                let _ = unsafe { libc::close(saved) };
                return Err(err);
            }
        };
        if unsafe { libc::dup2(std::os::fd::AsRawFd::as_raw_fd(&tty), libc::STDOUT_FILENO) } < 0 {
            let _ = unsafe { libc::close(saved) };
            return Err(io::Error::last_os_error());
        }
        drop(tty);

        Ok(Self {
            saved_stdout: Some(unsafe { OwnedFd::from_raw_fd(saved) }),
        })
    }
}

impl Drop for StdoutTtyGuard {
    fn drop(&mut self) {
        let Some(saved_stdout) = &self.saved_stdout else {
            return;
        };
        let _ = io::stdout().flush();
        let _ = unsafe {
            libc::dup2(
                std::os::fd::AsRawFd::as_raw_fd(saved_stdout),
                libc::STDOUT_FILENO,
            )
        };
        let _ = io::stdout().flush();
    }
}

fn cleanup_terminal(viewport_top: Option<u16>) -> io::Result<()> {
    let mut stdout = io::stdout();
    if let Some(y) = viewport_top {
        crossterm::execute!(
            stdout,
            cursor::MoveTo(0, y),
            terminal::Clear(ClearType::FromCursorDown),
            cursor::Show
        )?;
    } else {
        crossterm::execute!(stdout, cursor::Show)?;
    }
    stdout.flush()?;
    Ok(())
}

pub(crate) fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Frontmatter, Snippet, SnippetFile};
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn edit_status_reports_success() {
        let mut app = test_app();
        assert_eq!(
            edit_status(EditResult::Edited("Demo".to_string()), &mut app, &[]),
            "edited Demo; reload skipped"
        );
    }

    #[test]
    fn edit_status_reports_failure_without_panicking() {
        let mut app = test_app();
        let status = edit_status(
            EditResult::Failed(io::Error::other("no editor")),
            &mut app,
            &[],
        );

        assert_eq!(status, "edit failed: no editor");
    }

    #[test]
    fn edit_status_reports_missing_snippet() {
        let mut app = test_app();
        let id = SnippetId::new("snippets.md", "demo");

        assert_eq!(
            edit_status(EditResult::MissingSnippet(id), &mut app, &[]),
            "snippet no longer exists: snippets.md#demo"
        );
    }

    #[test]
    fn reload_after_edit_replaces_index_from_roots() {
        let root = temp_dir("reload-success");
        let path = root.join("snippets.md");
        fs::write(&path, "## Demo\n\n```\necho old\n```\n").unwrap();
        let mut app = test_app_with_file(snippet_file(&path, "echo old"));
        fs::write(&path, "## Demo\n\n```\necho new\n```\n").unwrap();

        let status = reload_after_edit(&mut app, &[root.clone()], "Demo".to_string());

        assert_eq!(status, "edited Demo; reloaded");
        assert_eq!(
            app.selected_snippet().map(|snippet| snippet.body()),
            Some("echo new")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn reload_after_edit_failure_keeps_previous_index() {
        let root = temp_dir("reload-failure");
        let path = root.join("snippets.md");
        fs::write(&path, "## Demo\n\n```\necho old\n```\n").unwrap();
        let mut app = test_app_with_file(snippet_file(&path, "echo old"));
        fs::write(&path, b"\xff").unwrap();

        let status = reload_after_edit(&mut app, &[root.clone()], "Demo".to_string());

        assert!(status.starts_with("edited Demo; reload failed:"));
        assert_eq!(
            app.selected_snippet().map(|snippet| snippet.body()),
            Some("echo old")
        );
        let _ = fs::remove_dir_all(root);
    }

    fn test_app() -> ExecutionApp {
        ExecutionApp::new(
            SnippetIndex::new(),
            FrecencyStore::new(),
            std::path::PathBuf::from("."),
            0,
            config::SearchConfig::default(),
            config::Theme::default(),
            SystemSuggestionProvider::new(Default::default()),
        )
    }

    fn test_app_with_file(file: SnippetFile) -> ExecutionApp {
        ExecutionApp::new(
            SnippetIndex::from_files([file]),
            FrecencyStore::new(),
            std::path::PathBuf::from("."),
            0,
            config::SearchConfig::default(),
            config::Theme::default(),
            SystemSuggestionProvider::new(Default::default()),
        )
    }

    fn snippet_file(path: &std::path::Path, body: &str) -> SnippetFile {
        SnippetFile {
            path: path.to_path_buf(),
            relative_path: std::path::PathBuf::from("snippets.md"),
            frontmatter: Frontmatter::default(),
            snippets: vec![Snippet {
                id: SnippetId::new("snippets.md", "demo"),
                name: "Demo".to_string(),
                description: String::new(),
                body: body.to_string(),
                variables: vec![],
            }],
        }
    }

    fn temp_dir(prefix: &str) -> std::path::PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let path = std::env::temp_dir().join(format!(
            "pb-terminal-{prefix}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }
}
