use crate::config;
use crate::frecency::FrecencyStore;
use crate::index::SnippetIndex;
use crossterm::cursor;
use crossterm::event::{self, Event};
use crossterm::terminal::{self, ClearType, disable_raw_mode, enable_raw_mode};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Position;
use ratatui::{Terminal, TerminalOptions, Viewport};
use std::fs;
use std::io;
use std::io::IsTerminal;
use std::io::Write;
use std::os::fd::{FromRawFd, OwnedFd};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::app::{AppEvent, ExecutionApp, SuggestionProvider, SystemSuggestionProvider};
use super::{ExecuteOptions, ExecutionOutcome};

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
        ..ExecuteOptions::default()
    };
    run_execute(index, frecency, options)
}

pub fn run_execute(
    index: SnippetIndex,
    frecency: FrecencyStore,
    options: ExecuteOptions,
) -> io::Result<Option<ExecutionOutcome>> {
    let provider = SystemSuggestionProvider::new(options.variables.clone());
    run_execute_with_provider(index, frecency, options, provider)
}

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
    let _raw_mode = RawModeGuard::enter()?;
    let restore_cursor = current_cursor_position();
    let mut terminal = build_terminal(options.viewport_height)?;
    let outcome = loop {
        terminal.draw(|frame| app.render(frame))?;
        if !event::poll(Duration::from_millis(250))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        match app.handle_key(key) {
            AppEvent::Continue => {}
            AppEvent::Cancelled => break None,
            AppEvent::Completed(outcome) => break Some(outcome),
        }
    };
    cleanup_terminal(restore_cursor)?;
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
    let (_, rows) = terminal::size()?;
    Ok(compact_viewport_height(rows, max_height))
}

pub(crate) fn compact_viewport_height(rows: u16, max_height: u16) -> u16 {
    let compact = (rows / 3).max(20);
    compact.min(max_height.max(1))
}

struct RawModeGuard;

impl RawModeGuard {
    fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
    }
}

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

fn current_cursor_position() -> Option<Position> {
    cursor::position().ok().map(|(x, y)| Position { x, y })
}

fn cleanup_terminal(restore_cursor: Option<Position>) -> io::Result<()> {
    let mut stdout = io::stdout();
    if let Some(position) = restore_cursor {
        crossterm::execute!(
            stdout,
            cursor::MoveTo(position.x, position.y),
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
