//! EXPERIMENTAL: Ctrl+R shell-history search trial (`pb history`).
//!
//! Throwaway prototype for using peanutbutter as a ctrl-r replacement: a
//! minimal fuzzy picker over shell history (full history piped on stdin by
//! the widget scripts, or `$PEANUTBUTTER_HISTORY` as a fallback) with a one-key
//! fallthrough (Ctrl+T) into the full snippet execute TUI. History picks are
//! plain strings — no placeholder variables, no frecency events, no snippet
//! preview; the "preview" strip just shows the selected command in full.
//!
//! Deliberately not wired into keybind config, `pb completions`, or docs.
//! Delete this module (plus the `history` CLI arm and `scripts/ctrl-r-trial.*`)
//! to remove the trial.

use crate::config::Theme;
use crate::fuzzy::{FuzzyScorer, build_pattern};
use crate::tui::Chrome;
use crate::tui::terminal::{
    RawModeGuard, StdoutTtyGuard, TuiOutputKind, build_terminal, cleanup_terminal,
};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget, Wrap};
use std::io;
use std::time::Duration;

/// How the user left the history picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryOutcome {
    /// Esc / Ctrl+C: emit nothing.
    Cancelled,
    /// Enter: replace the shell buffer with this history entry verbatim.
    Picked(String),
    /// Ctrl+T: fall through to the normal snippet execute TUI.
    Snippets,
}

/// Parse a history payload (newest first) into deduplicated entries, keeping
/// the most recent occurrence.
///
/// Accepts both wire formats: unit-separator-joined (`$PEANUTBUTTER_HISTORY`,
/// as fed to `pb new`) and newline-separated (full history piped on stdin by
/// the ctrl-r widget — an env var can't carry a full history because Linux
/// caps a single env string at ~128KiB).
pub fn parse_history_payload(raw: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    raw.split(['\u{1F}', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter(|s| seen.insert(s.to_string()))
        .map(str::to_string)
        .collect()
}

/// Picker state: fuzzy filter over history entries (newest first).
///
/// Unlike the snippet picker there is no frecency or field weighting — matches
/// are ranked by fuzzy score, ties broken by recency; an empty query shows
/// everything in recency order.
pub struct HistoryTrialState {
    entries: Vec<String>,
    filter: String,
    filtered: Vec<usize>,
    cursor: usize,
    scorer: FuzzyScorer,
}

impl HistoryTrialState {
    pub fn new(entries: Vec<String>, seed_query: &str) -> Self {
        let mut state = Self {
            entries,
            filter: seed_query.to_string(),
            filtered: Vec::new(),
            cursor: 0,
            scorer: FuzzyScorer::new(),
        };
        state.recompute();
        // A seeded query (shell buffer contents) that matches nothing would
        // start the picker on an empty list; drop it instead.
        if !state.filter.is_empty() && state.filtered.is_empty() {
            state.filter.clear();
            state.recompute();
        }
        state
    }

    pub fn filter(&self) -> &str {
        &self.filter
    }

    pub fn visible(&self) -> Vec<&str> {
        self.filtered
            .iter()
            .map(|&i| self.entries[i].as_str())
            .collect()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn selected(&self) -> Option<&str> {
        self.filtered.get(self.cursor).map(|&i| &*self.entries[i])
    }

    pub fn move_cursor(&mut self, delta: i32) {
        if self.filtered.is_empty() {
            self.cursor = 0;
            return;
        }
        let len = self.filtered.len() as i32;
        self.cursor = (self.cursor as i32 + delta).rem_euclid(len) as usize;
    }

    pub fn type_char(&mut self, c: char) {
        self.filter.push(c);
        self.recompute();
    }

    pub fn backspace(&mut self) {
        self.filter.pop();
        self.recompute();
    }

    fn recompute(&mut self) {
        if self.filter.is_empty() {
            self.filtered = (0..self.entries.len()).collect();
        } else {
            let pattern = build_pattern(&self.filter);
            let mut scored: Vec<(u32, usize)> = self
                .entries
                .iter()
                .enumerate()
                .filter_map(|(i, entry)| self.scorer.score(&pattern, entry).map(|s| (s, i)))
                .collect();
            // Highest score first; equal scores keep recency (lower index) order.
            scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
            self.filtered = scored.into_iter().map(|(_, i)| i).collect();
        }
        self.cursor = 0;
    }
}

/// Translate one key event; `None` means keep looping.
pub fn handle_key(state: &mut HistoryTrialState, key: &KeyEvent) -> Option<HistoryOutcome> {
    if key.kind == KeyEventKind::Release {
        return None;
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return match key.code {
            KeyCode::Char('c') => Some(HistoryOutcome::Cancelled),
            KeyCode::Char('t') => Some(HistoryOutcome::Snippets),
            KeyCode::Char('p') => {
                state.move_cursor(-1);
                None
            }
            KeyCode::Char('n') => {
                state.move_cursor(1);
                None
            }
            _ => None,
        };
    }
    match key.code {
        KeyCode::Esc => Some(HistoryOutcome::Cancelled),
        KeyCode::Enter => state.selected().map(|s| HistoryOutcome::Picked(s.into())),
        KeyCode::Up => {
            state.move_cursor(-1);
            None
        }
        KeyCode::Down => {
            state.move_cursor(1);
            None
        }
        KeyCode::Backspace => {
            state.backspace();
            None
        }
        KeyCode::Char(c) => {
            state.type_char(c);
            None
        }
        _ => None,
    }
}

/// Run the inline history picker TUI and return how the user left it.
///
/// Owns a full terminal session (raw mode, TTY-redirected stdout, event
/// drain); by the time this returns the terminal is restored, so the caller
/// may start another TUI session (the snippet execute fallthrough) safely.
pub fn run_history_picker(
    entries: Vec<String>,
    seed_query: &str,
    viewport_height: u16,
    theme: &Theme,
) -> io::Result<HistoryOutcome> {
    let mut state = HistoryTrialState::new(entries, seed_query);

    let _stdout_guard = StdoutTtyGuard::enter()?;
    let tui_output = TuiOutputKind::detect();
    let _raw_mode = RawModeGuard::enter(tui_output)?;
    let mut terminal = build_terminal(viewport_height.max(10), tui_output)?;
    let mut viewport_top: Option<u16> = None;

    let outcome = loop {
        terminal.draw(|frame| {
            viewport_top = viewport_top.or(Some(frame.area().y));
            render(frame.area(), frame.buffer_mut(), &state, theme);
        })?;
        if !event::poll(Duration::from_millis(250))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if let Some(outcome) = handle_key(&mut state, &key) {
            break outcome;
        }
    };

    while event::poll(Duration::ZERO).unwrap_or(false) {
        let _ = event::read();
    }
    cleanup_terminal(viewport_top, tui_output)?;
    Ok(outcome)
}

fn render(area: Rect, buf: &mut ratatui::buffer::Buffer, state: &HistoryTrialState, theme: &Theme) {
    let content = Chrome {
        theme,
        mode: "pb history (trial)",
        title: "shell history",
        footer: "enter replace line · ctrl+t snippets · esc cancel",
    }
    .render(area, buf);
    if content.height == 0 {
        return;
    }

    // Layout: filter row, divider, list, divider, 2-line full-command strip.
    let filter_h = 1u16;
    let preview_h = if content.height >= 7 { 2u16 } else { 0 };
    let preview_divider = u16::from(preview_h > 0);
    let list_area = Rect {
        x: content.x,
        y: content.y + filter_h + 1,
        width: content.width,
        height: content
            .height
            .saturating_sub(filter_h + 1 + preview_h + preview_divider),
    };

    Paragraph::new(Line::from(vec![
        Span::styled("History: ", theme.chrome),
        Span::styled(state.filter().to_string(), theme.emphasis),
    ]))
    .render(
        Rect {
            x: content.x,
            y: content.y,
            width: content.width,
            height: 1,
        },
        buf,
    );
    crate::tui::chrome::draw_divider(area, content.y + filter_h, buf, theme);

    let visible = state.visible();
    let max = list_area.height as usize;
    if max > 0 {
        let total = visible.len();
        let start = if total <= max {
            0
        } else {
            state.cursor().saturating_sub(max / 2).min(total - max)
        };
        let mut lines: Vec<Line> = Vec::new();
        for (i, entry) in visible.iter().enumerate().skip(start).take(max) {
            let selected = i == state.cursor();
            let marker = if selected { "▌ " } else { "  " };
            let marker_style = if selected {
                theme.fuzzy_highlight
            } else {
                theme.chrome
            };
            let row_style = if selected {
                theme.selected_item
            } else {
                Style::default()
            };
            lines.push(Line::from(vec![
                Span::styled(marker.to_string(), marker_style),
                Span::styled((*entry).to_string(), row_style),
            ]));
        }
        Paragraph::new(lines).render(list_area, buf);
    }

    // History entries have no snippet preview; show the selected command in
    // full (wrapped) so long lines aren't lost to list truncation.
    if preview_h > 0 {
        let divider_y = list_area.y + list_area.height;
        crate::tui::chrome::draw_divider(area, divider_y, buf, theme);
        Paragraph::new(Span::styled(
            state.selected().unwrap_or("").to_string(),
            theme.chrome,
        ))
        .wrap(Wrap { trim: false })
        .render(
            Rect {
                x: content.x,
                y: divider_y + 1,
                width: content.width,
                height: preview_h,
            },
            buf,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    #[test]
    fn parse_history_payload_splits_trims_and_dedups_keeping_newest() {
        let raw = "git status\u{1F}  cargo test \u{1F}\u{1F}git status\u{1F}ls";
        assert_eq!(
            parse_history_payload(raw),
            vec!["git status", "cargo test", "ls"]
        );
        assert!(parse_history_payload("").is_empty());
    }

    #[test]
    fn parse_history_payload_accepts_newline_separated_stdin_format() {
        let raw = "git status\n  cargo test \n\ngit status\nls\n";
        assert_eq!(
            parse_history_payload(raw),
            vec!["git status", "cargo test", "ls"]
        );
    }

    #[test]
    fn empty_query_lists_all_entries_in_recency_order() {
        let state =
            HistoryTrialState::new(vec!["newest".into(), "middle".into(), "oldest".into()], "");
        assert_eq!(state.visible(), vec!["newest", "middle", "oldest"]);
        assert_eq!(state.selected(), Some("newest"));
    }

    #[test]
    fn typing_filters_fuzzily_and_ranks_by_score() {
        let mut state = HistoryTrialState::new(
            vec![
                "cargo test".into(),
                "git commit".into(),
                "cat notes.txt".into(),
            ],
            "",
        );
        for c in "cargo".chars() {
            state.type_char(c);
        }
        assert_eq!(state.selected(), Some("cargo test"));
        assert!(!state.visible().contains(&"git commit"));
    }

    #[test]
    fn equal_scores_keep_recency_order() {
        let mut state = HistoryTrialState::new(vec!["echo one".into(), "echo two".into()], "");
        for c in "echo".chars() {
            state.type_char(c);
        }
        assert_eq!(state.visible(), vec!["echo one", "echo two"]);
    }

    #[test]
    fn seed_query_prefills_filter_but_is_dropped_when_nothing_matches() {
        let entries = vec!["cargo test".into(), "git push".into()];
        let seeded = HistoryTrialState::new(entries.clone(), "cargo");
        assert_eq!(seeded.filter(), "cargo");
        assert_eq!(seeded.visible(), vec!["cargo test"]);

        let unmatched = HistoryTrialState::new(entries, "zzzznope");
        assert_eq!(unmatched.filter(), "");
        assert_eq!(unmatched.visible().len(), 2);
    }

    #[test]
    fn enter_picks_the_selected_entry() {
        let mut state = HistoryTrialState::new(vec!["one".into(), "two".into()], "");
        state.move_cursor(1);
        assert_eq!(
            handle_key(&mut state, &key(KeyCode::Enter)),
            Some(HistoryOutcome::Picked("two".into()))
        );
    }

    #[test]
    fn enter_on_empty_list_keeps_looping() {
        let mut state = HistoryTrialState::new(vec!["one".into()], "");
        for c in "nomatch".chars() {
            state.type_char(c);
        }
        assert_eq!(handle_key(&mut state, &key(KeyCode::Enter)), None);
    }

    #[test]
    fn ctrl_t_falls_through_to_snippets_and_esc_cancels() {
        let mut state = HistoryTrialState::new(vec!["one".into()], "");
        assert_eq!(
            handle_key(&mut state, &ctrl('t')),
            Some(HistoryOutcome::Snippets)
        );
        assert_eq!(
            handle_key(&mut state, &key(KeyCode::Esc)),
            Some(HistoryOutcome::Cancelled)
        );
        assert_eq!(
            handle_key(&mut state, &ctrl('c')),
            Some(HistoryOutcome::Cancelled)
        );
    }

    #[test]
    fn cursor_wraps_and_ctrl_p_n_navigate() {
        let mut state = HistoryTrialState::new(vec!["a".into(), "b".into()], "");
        handle_key(&mut state, &ctrl('n'));
        assert_eq!(state.selected(), Some("b"));
        handle_key(&mut state, &ctrl('n'));
        assert_eq!(state.selected(), Some("a"));
        handle_key(&mut state, &ctrl('p'));
        assert_eq!(state.selected(), Some("b"));
    }
}
