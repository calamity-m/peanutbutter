//! Interactive configuration editor for `pb settings`.
//!
//! The v1 editor focuses on search ranking weights. It opens an inline TUI,
//! lets users tune frecency/fuzzy fields in memory, and saves only touched TOML
//! keys back to the resolved config file.

mod app;
mod persist;
mod render;

use crate::config::AppConfig;
use crate::tui::terminal::{
    RawModeGuard, StdoutTtyGuard, TuiOutputKind, build_terminal, cleanup_terminal,
};
use crossterm::event::{self, Event};
use std::io::{self, IsTerminal};
use std::time::Duration;

/// Run the interactive settings editor for the resolved application config.
pub fn run(config: &AppConfig) -> io::Result<()> {
    if !io::stdout().is_terminal() {
        return Err(io::Error::other(
            "settings requires an interactive terminal on stdout",
        ));
    }

    let _stdout_guard = StdoutTtyGuard::enter()?;
    let tui_output = TuiOutputKind::detect();
    let _raw_mode = RawModeGuard::enter(tui_output)?;
    let mut terminal = build_terminal(config.ui.height, tui_output)?;
    let mut viewport_top = None;
    let mut app = app::SettingsApp::new(config);

    loop {
        terminal.draw(|frame| {
            let area = frame.area();
            viewport_top = viewport_top.or(Some(area.y));
            let preview_theme = crate::config::Theme::named(app.theme_selected_name())
                .expect("theme_selected_name always names a built-in theme");
            render::draw(frame, &app, &preview_theme);
        })?;

        if app.should_quit() {
            break;
        }
        if !event::poll(Duration::from_millis(250))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if app.handle_key(key) {
            let mut saved = 0usize;
            let mut error = None;
            match persist::save_changed_fields(&config.paths.config_file, app.all_fields()) {
                Ok(n) => saved += n,
                Err(err) => error = Some(err),
            }
            if error.is_none()
                && let Some(name) = app.pending_theme_name()
            {
                match persist::save_theme_name(&config.paths.config_file, name) {
                    Ok(()) => saved += 1,
                    Err(err) => error = Some(err),
                }
            }
            match error {
                Some(err) => app.set_status(format!("save failed: {err}")),
                None if saved == 0 => app.set_status("no changes"),
                None => app.mark_saved(),
            }
        }
    }

    while event::poll(Duration::ZERO).unwrap_or(false) {
        let _ = event::read();
    }
    cleanup_terminal(viewport_top, tui_output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn temp_dir(prefix: &str) -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let path = std::env::temp_dir().join(format!(
            "pb-settings-flow-{prefix}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn navigate_edit_save_uses_resolved_config_file() {
        let root = temp_dir("resolved-config");
        let config_file = root.join("custom-config.toml");
        fs::write(&config_file, "[search.frecency]\nlocation_weight = 1.0\n").unwrap();
        let old_config = std::env::var_os("PB_CONFIG_FILE");
        let old_path = std::env::var_os("PEANUTBUTTER_PATH");
        unsafe {
            std::env::set_var("PB_CONFIG_FILE", &config_file);
            std::env::set_var("PEANUTBUTTER_PATH", root.join("snippets"));
        }

        let config = crate::config::load().unwrap();
        let mut app = app::SettingsApp::new(&config);
        app.handle_key(key(KeyCode::Enter));
        app.handle_key(key(KeyCode::Enter));
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Right));
        let changed =
            persist::save_changed_fields(&config.paths.config_file, app.all_fields()).unwrap();

        match old_config {
            Some(value) => unsafe { std::env::set_var("PB_CONFIG_FILE", value) },
            None => unsafe { std::env::remove_var("PB_CONFIG_FILE") },
        }
        match old_path {
            Some(value) => unsafe { std::env::set_var("PEANUTBUTTER_PATH", value) },
            None => unsafe { std::env::remove_var("PEANUTBUTTER_PATH") },
        }

        assert_eq!(changed, 1);
        assert!(
            fs::read_to_string(&config_file)
                .unwrap()
                .contains("location_weight = 1.1")
        );
    }
}
