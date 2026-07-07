//! Snippet repository manager for `pb repo`.
//!
//! Discovers git repositories under the configured snippet roots and drives an
//! inline TUI that can sync, push, pull, and jump into them.

mod app;
mod discover;
mod git;
mod render;

pub use discover::{SnippetRepo, discover_repos};

use crate::config::AppConfig;
use crate::tui::terminal::{
    RawModeGuard, StdoutTtyGuard, TuiOutputKind, build_terminal, cleanup_terminal,
};
use crossterm::event::{self, Event};
use std::io::{self, IsTerminal};
use std::time::Duration;

/// Run the interactive repository manager over the resolved snippet roots.
pub fn run(config: &AppConfig) -> io::Result<()> {
    if !io::stdout().is_terminal() {
        return Err(io::Error::other(
            "repo requires an interactive terminal on stdout",
        ));
    }

    let repos = discover_repos(&config.paths)?;
    let _stdout_guard = StdoutTtyGuard::enter()?;
    let tui_output = TuiOutputKind::detect();
    let mut raw_mode = RawModeGuard::enter(tui_output)?;
    let mut terminal = build_terminal(config.ui.height, tui_output)?;
    let mut viewport_top: Option<u16> = None;
    let mut app = app::RepoApp::new(config, repos);
    app.refresh_git_summaries();

    loop {
        terminal.draw(|frame| {
            let area = frame.area();
            viewport_top = viewport_top.or(Some(area.y));
            render::draw(frame, &app);
        })?;

        if !event::poll(Duration::from_millis(250))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        match app.handle_key(key) {
            app::RepoEvent::Continue => {}
            app::RepoEvent::Quit => break,
            app::RepoEvent::RunGit(operation) => {
                // Runs synchronously with output captured; nothing is written
                // to the terminal or stdout by the git subprocesses. The
                // redraw hook repaints per-step progress while git blocks.
                app.run_git_operation(operation, &mut |app| {
                    let _ = terminal.draw(|frame| render::draw(frame, app));
                });
            }
            app::RepoEvent::Jump => {
                // The editor owns the terminal while it runs; tear the
                // viewport down first and rebuild it afterwards, mirroring the
                // execute TUI's edit flow.
                cleanup_terminal(viewport_top, tui_output)?;
                raw_mode.suspend()?;
                let result = app.jump_selected();
                raw_mode.resume()?;
                terminal = build_terminal(config.ui.height, tui_output)?;
                viewport_top = None;
                app.set_status(match result {
                    Ok(path) => format!("opened editor at {}", path.display()),
                    Err(err) => format!("jump failed: {err}"),
                });
            }
        }
    }

    while event::poll(Duration::ZERO).unwrap_or(false) {
        let _ = event::read();
    }
    cleanup_terminal(viewport_top, tui_output)
}
