//! Shared terminal lifecycle helpers for inline TUIs.
//!
//! These guards are used by screens that need to draw an inline ratatui
//! viewport while preserving peanutbutter's shell-buffer stdout contract.

use crate::config::Theme;
use ansi_to_tui::IntoText;
use crossterm::cursor;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{self, ClearType, disable_raw_mode, enable_raw_mode};
use ratatui::backend::CrosstermBackend;
use ratatui::text::Text;
use ratatui::widgets::Paragraph;
use ratatui::{Terminal, TerminalOptions, Viewport};
use std::io::{self, IsTerminal, Write};
use std::time::Duration;

/// Show pre-rendered text in an inline, scrollable TUI until the user exits.
pub(crate) fn run_scrollable_text(
    mode: &str,
    title: &str,
    footer: &str,
    text: String,
    viewport_height: u16,
    theme: &Theme,
) -> io::Result<()> {
    let _stdout_guard = StdoutTtyGuard::enter()?;
    let tui_output = TuiOutputKind::detect();
    let _raw_mode = RawModeGuard::enter(tui_output)?;
    let mut terminal = build_terminal(viewport_height, tui_output)?;
    let mut viewport_top: Option<u16> = None;
    let mut scroll = 0u16;
    let text = text
        .as_str()
        .into_text()
        .unwrap_or_else(|_| Text::from(text));
    let line_count = text.lines.len() as u16;

    loop {
        terminal.draw(|frame| {
            let area = frame.area();
            viewport_top = viewport_top.or(Some(area.y));
            let content = crate::tui::chrome::Chrome {
                theme,
                mode,
                title,
                footer,
            }
            .render(area, frame.buffer_mut());
            let max_scroll = line_count.saturating_sub(content.height);
            scroll = scroll.min(max_scroll);
            frame.render_widget(Paragraph::new(text.clone()).scroll((scroll, 0)), content);
        })?;

        if !event::poll(Duration::from_millis(250))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c')) {
            break;
        }
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => break,
            KeyCode::Up | KeyCode::Char('k') => scroll = scroll.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => scroll = scroll.saturating_add(1),
            KeyCode::PageUp => scroll = scroll.saturating_sub(10),
            KeyCode::PageDown => scroll = scroll.saturating_add(10),
            KeyCode::Home => scroll = 0,
            KeyCode::End => scroll = u16::MAX,
            _ => {}
        }
    }

    while event::poll(Duration::ZERO).unwrap_or(false) {
        let _ = event::read();
    }
    cleanup_terminal(viewport_top, tui_output)
}

pub(crate) fn build_terminal(
    viewport_height: u16,
    output: TuiOutputKind,
) -> io::Result<Terminal<CrosstermBackend<TuiOutput>>> {
    let backend = CrosstermBackend::new(output.writer());
    let viewport_height = inline_viewport_height(viewport_height);
    match Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Inline(viewport_height),
        },
    ) {
        Ok(terminal) => Ok(terminal),
        Err(_) => Terminal::new(CrosstermBackend::new(output.writer())),
    }
}

fn inline_viewport_height(max_height: u16) -> u16 {
    compact_viewport_height(max_height)
}

pub(crate) fn compact_viewport_height(max_height: u16) -> u16 {
    max_height.max(1)
}

/// Where inline TUI output should be written for the current platform/stdout.
#[derive(Clone, Copy)]
pub(crate) enum TuiOutputKind {
    Stdout,
    #[cfg(not(unix))]
    Stderr,
}

impl TuiOutputKind {
    pub(crate) fn detect() -> Self {
        // On Unix, StdoutTtyGuard has already redirected fd 1 to the TTY when
        // stdout was piped, so we can safely draw to stdout. On Windows we
        // fall back to stderr because the dup2 trick isn't available.
        #[cfg(unix)]
        {
            Self::Stdout
        }
        #[cfg(not(unix))]
        {
            if io::stdout().is_terminal() {
                Self::Stdout
            } else {
                Self::Stderr
            }
        }
    }

    fn writer(self) -> TuiOutput {
        match self {
            Self::Stdout => TuiOutput::Stdout(io::stdout()),
            #[cfg(not(unix))]
            Self::Stderr => TuiOutput::Stderr(io::stderr()),
        }
    }
}

pub(crate) enum TuiOutput {
    Stdout(io::Stdout),
    #[cfg(not(unix))]
    Stderr(io::Stderr),
}

impl Write for TuiOutput {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Self::Stdout(stdout) => stdout.write(buf),
            #[cfg(not(unix))]
            Self::Stderr(stderr) => stderr.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Stdout(stdout) => stdout.flush(),
            #[cfg(not(unix))]
            Self::Stderr(stderr) => stderr.flush(),
        }
    }
}

/// RAII guard that enables terminal raw mode on construction and disables it
/// on drop, even if the TUI exits via `?`.
pub(crate) struct RawModeGuard {
    active: bool,
    output: TuiOutputKind,
}

impl RawModeGuard {
    pub(crate) fn enter(output: TuiOutputKind) -> io::Result<Self> {
        enable_raw_mode()?;
        // Bracketed paste lets the terminal deliver pasted text as a single
        // Event::Paste(String) — preserving newlines — instead of a stream of
        // KeyCode::Char events that strip them. Best-effort: terminals that
        // don't support it silently ignore the escape.
        let _ = execute!(output.writer(), crossterm::event::EnableBracketedPaste);
        Ok(Self {
            active: true,
            output,
        })
    }

    pub(crate) fn suspend(&mut self) -> io::Result<()> {
        if self.active {
            let _ = execute!(
                self.output.writer(),
                crossterm::event::DisableBracketedPaste
            );
            disable_raw_mode()?;
            self.active = false;
        }
        Ok(())
    }

    pub(crate) fn resume(&mut self) -> io::Result<()> {
        if !self.active {
            enable_raw_mode()?;
            let _ = execute!(self.output.writer(), crossterm::event::EnableBracketedPaste);
            self.active = true;
        }
        Ok(())
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        if self.active {
            let _ = execute!(
                self.output.writer(),
                crossterm::event::DisableBracketedPaste
            );
            let _ = disable_raw_mode();
        }
    }
}

/// RAII guard that redirects stdout to the TTY when stdout is not a terminal.
///
/// When peanutbutter is invoked via the shell hotkey (`pb`), bash captures
/// stdout to write the selected command into the readline buffer. That means
/// fd 1 is a pipe, not a terminal — we can't draw the TUI there, and
/// crossterm's terminal queries (e.g. cursor-position DSR) would be written
/// into the pipe instead of reaching the terminal, causing multi-second
/// timeouts and stray escape sequences in the readline buffer. This guard
/// saves fd 1, points it at stderr (if that's a terminal) or `/dev/tty`, and
/// restores the original fd on drop so the caller can still print the command.
///
/// Unix-only — Windows uses a different mechanism (TUI writes to stderr).
#[cfg(unix)]
pub(crate) struct StdoutTtyGuard {
    saved_stdout: Option<std::os::fd::OwnedFd>,
}

#[cfg(unix)]
impl StdoutTtyGuard {
    pub(crate) fn enter() -> io::Result<Self> {
        use std::os::fd::FromRawFd;

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
                saved_stdout: Some(unsafe { std::os::fd::OwnedFd::from_raw_fd(saved) }),
            });
        }

        let tty = match std::fs::OpenOptions::new()
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
            saved_stdout: Some(unsafe { std::os::fd::OwnedFd::from_raw_fd(saved) }),
        })
    }
}

#[cfg(unix)]
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

#[cfg(not(unix))]
pub(crate) struct StdoutTtyGuard;

#[cfg(not(unix))]
impl StdoutTtyGuard {
    pub(crate) fn enter() -> io::Result<Self> {
        Ok(Self)
    }
}

pub(crate) fn cleanup_terminal(viewport_top: Option<u16>, output: TuiOutputKind) -> io::Result<()> {
    let mut writer = output.writer();
    if let Some(y) = viewport_top {
        crossterm::execute!(
            writer,
            cursor::MoveTo(0, y),
            terminal::Clear(ClearType::FromCursorDown),
            cursor::Show
        )?;
    } else {
        crossterm::execute!(writer, cursor::Show)?;
    }
    writer.flush()?;
    Ok(())
}
