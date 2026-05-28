//! Capture TUI for `pb new` — history picker and token-confirm stages.
//!
//! Both stages are driven by a state machine that is independent of the
//! terminal. Tests drive these states directly; the [`run_capture`] entry
//! point wraps them with crossterm/ratatui plumbing.

use crate::config::Theme;
use crate::new::capture_heuristics::{Span, TokenCandidate};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use crossterm::{cursor, execute, terminal::ClearType};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span as RtSpan};
use ratatui::widgets::Paragraph;
use ratatui::{TerminalOptions, Viewport};
use std::io::{self, Write};
use std::time::Duration;

// =========================================================================
// History-pick stage
// =========================================================================

/// Outcome of the history-pick stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickOutcome {
    /// User cancelled (esc).
    Cancel,
    /// User picked an entry.
    Pick(String),
}

/// State machine for the history picker.
#[derive(Debug, Clone)]
pub struct HistoryPickerState {
    entries: Vec<String>,
    filter: String,
    filtered: Vec<usize>,
    cursor: usize,
}

impl HistoryPickerState {
    /// Build a new picker over `entries` (newest first).
    pub fn new(entries: Vec<String>) -> Self {
        let filtered: Vec<usize> = (0..entries.len()).collect();
        Self {
            entries,
            filter: String::new(),
            filtered,
            cursor: 0,
        }
    }

    /// Currently-visible entries in display order.
    pub fn visible(&self) -> Vec<&str> {
        self.filtered
            .iter()
            .map(|&i| self.entries[i].as_str())
            .collect()
    }

    /// Index into `visible()` of the highlighted row.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Current filter text.
    pub fn filter(&self) -> &str {
        &self.filter
    }

    /// Move the cursor by `delta`, clamping to the visible list.
    pub fn move_cursor(&mut self, delta: i32) {
        if self.filtered.is_empty() {
            self.cursor = 0;
            return;
        }
        let len = self.filtered.len() as i32;
        let new = (self.cursor as i32 + delta).rem_euclid(len);
        self.cursor = new as usize;
    }

    /// Append a character to the filter; selection resets to top.
    pub fn append_filter(&mut self, c: char) {
        self.filter.push(c);
        self.recompute_filter();
    }

    /// Remove the last filter character.
    pub fn pop_filter(&mut self) {
        self.filter.pop();
        self.recompute_filter();
    }

    /// Return the highlighted entry, if any.
    pub fn pick(&self) -> Option<String> {
        self.filtered
            .get(self.cursor)
            .map(|&i| self.entries[i].clone())
    }

    fn recompute_filter(&mut self) {
        let needle = self.filter.to_lowercase();
        self.filtered = (0..self.entries.len())
            .filter(|&i| needle.is_empty() || self.entries[i].to_lowercase().contains(&needle))
            .collect();
        self.cursor = 0;
    }
}

// =========================================================================
// Token-confirm stage
// =========================================================================

/// Outcome of the token-confirm stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmOutcome {
    /// User cancelled.
    Cancel,
    /// User pressed `b` to return to the history picker.
    Back,
    /// User accepted; commit the snippet.
    Accept(ConfirmAccept),
}

/// Acceptance payload — name + accepted spans + secret-warning flag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmAccept {
    pub name: String,
    pub accepted: Vec<(Span, String)>,
    /// `true` if any flagged secret remains literal (unselected) at accept time.
    pub has_unselected_secret: bool,
}

/// What part of the screen has keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Name,
    TokenList,
    TokenEdit,
}

/// Token-confirm state machine.
#[derive(Debug, Clone)]
pub struct TokenConfirmState {
    pub raw: String,
    pub candidates: Vec<TokenCandidate>,
    /// Per-candidate selection bit (parallel to `candidates`).
    pub selected: Vec<bool>,
    /// Optional per-candidate name override.
    pub names: Vec<String>,
    pub name: String,
    pub focus: Focus,
    pub cursor: usize,
    pub rename_buffer: String,
    /// One-shot hint shown at the bottom.
    pub hint: Option<String>,
    /// True when a "secret remains literal" warning is shown and acceptance is
    /// awaiting a second confirmation.
    pub awaiting_secret_confirm: bool,
}

impl TokenConfirmState {
    /// Build a fresh confirm state.
    pub fn new(name_opt: Option<String>, raw: String, candidates: Vec<TokenCandidate>) -> Self {
        let selected: Vec<bool> = candidates.iter().map(|c| c.default_selected).collect();
        let names: Vec<String> = candidates
            .iter()
            .map(|c| c.suggested_name.clone())
            .collect();
        let focus = if name_opt.as_deref().unwrap_or("").is_empty() {
            Focus::Name
        } else {
            Focus::TokenList
        };
        Self {
            raw,
            candidates,
            selected,
            names,
            name: name_opt.unwrap_or_default(),
            focus,
            cursor: 0,
            rename_buffer: String::new(),
            hint: None,
            awaiting_secret_confirm: false,
        }
    }

    /// Live-rendered preview using current selection + names.
    pub fn preview(&self) -> String {
        let accepted: Vec<(Span, String)> = self
            .candidates
            .iter()
            .enumerate()
            .filter(|(i, _)| self.selected[*i])
            .map(|(i, c)| (c.span, self.names[i].clone()))
            .collect();
        crate::new::capture_heuristics::render_with_placeholders(&self.raw, &accepted)
    }

    /// Toggle the focused token's selection.
    pub fn toggle_focused(&mut self) {
        if let Some(slot) = self.selected.get_mut(self.cursor) {
            *slot = !*slot;
            self.awaiting_secret_confirm = false;
        }
    }

    /// Begin renaming the focused token.
    pub fn start_rename(&mut self) {
        if self.cursor < self.candidates.len() {
            self.rename_buffer = self.names[self.cursor].clone();
            self.focus = Focus::TokenEdit;
        }
    }

    /// Commit the rename buffer to the focused token.
    pub fn commit_rename(&mut self) {
        let trimmed: String = self
            .rename_buffer
            .trim()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .collect();
        if !trimmed.is_empty() && self.cursor < self.names.len() {
            self.names[self.cursor] = trimmed;
        }
        self.rename_buffer.clear();
        self.focus = Focus::TokenList;
    }

    /// Discard the rename buffer.
    pub fn cancel_rename(&mut self) {
        self.rename_buffer.clear();
        self.focus = Focus::TokenList;
    }

    /// Whether any secret-flagged candidate is currently unselected.
    pub fn has_unselected_secret(&self) -> bool {
        self.candidates.iter().enumerate().any(|(i, c)| {
            c.kind == crate::new::capture_heuristics::TokenKind::Secret && !self.selected[i]
        })
    }

    /// Try to accept. Returns `Some(ConfirmAccept)` on success, or `None` if
    /// state must remain on the screen (e.g. empty name → re-focus name input).
    pub fn try_accept(&mut self) -> Option<ConfirmAccept> {
        let name = self.name.trim().to_string();
        if name.is_empty() {
            self.focus = Focus::Name;
            self.hint = Some("name required".to_string());
            return None;
        }
        if self.has_unselected_secret() && !self.awaiting_secret_confirm {
            self.awaiting_secret_confirm = true;
            self.hint = Some(
                "warning: a likely secret is still literal — press enter again to write anyway"
                    .to_string(),
            );
            return None;
        }
        let accepted: Vec<(Span, String)> = self
            .candidates
            .iter()
            .enumerate()
            .filter(|(i, _)| self.selected[*i])
            .map(|(i, c)| (c.span, self.names[i].clone()))
            .collect();
        Some(ConfirmAccept {
            name,
            accepted,
            has_unselected_secret: self.has_unselected_secret(),
        })
    }
}

// =========================================================================
// Terminal driver
// =========================================================================

/// A selectable destination file shown in the target-pick stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetChoice {
    /// Short label shown in the picker (e.g. root-relative path).
    pub label: String,
    /// Absolute path the snippet will be appended to.
    pub path: std::path::PathBuf,
}

/// Configuration for [`run_capture`].
pub struct CaptureRun<'a> {
    pub history: Option<Vec<String>>,
    pub explicit_command: Option<String>,
    pub name_opt: Option<String>,
    pub theme: &'a Theme,
    pub viewport_height: u16,
    /// Candidate destination files. When more than one is present the user
    /// picks one in a dedicated stage after accepting; a single entry is used
    /// directly with no extra prompt.
    pub targets: Vec<TargetChoice>,
}

/// Result of a full capture session.
pub enum CaptureOutcome {
    Cancelled,
    Accepted {
        name: String,
        raw: String,
        accepted: Vec<(Span, String)>,
        first_token: Option<String>,
        /// The destination file the user selected.
        target: std::path::PathBuf,
    },
}

/// RAII guard for raw mode used by the capture TUI.
struct RawModeGuard {
    active: bool,
}

impl RawModeGuard {
    fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        Ok(Self { active: true })
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        if self.active {
            let _ = disable_raw_mode();
        }
    }
}

/// Run the full capture flow. Returns either a cancellation or an accepted
/// snippet (name + rendered body + the first token for language guessing).
pub fn run_capture(run: CaptureRun<'_>) -> io::Result<CaptureOutcome> {
    let CaptureRun {
        history,
        explicit_command,
        name_opt,
        theme,
        viewport_height,
        targets,
    } = run;

    let _raw = RawModeGuard::enter()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Inline(viewport_height.max(14)),
        },
    )
    .or_else(|_| Terminal::new(CrosstermBackend::new(io::stdout())))?;

    let mut viewport_top: Option<u16> = None;

    'outer: loop {
        // Stage 1: history picker (skipped when explicit_command is set).
        let raw_command = if let Some(cmd) = &explicit_command {
            cmd.clone()
        } else {
            let entries = history.clone().unwrap_or_default();
            let mut state = HistoryPickerState::new(entries);
            match drive_picker(&mut terminal, &mut state, theme, &mut viewport_top)? {
                PickOutcome::Cancel => {
                    cleanup_terminal(viewport_top)?;
                    return Ok(CaptureOutcome::Cancelled);
                }
                PickOutcome::Pick(s) => s,
            }
        };

        let candidates = crate::new::capture_heuristics::detect_variables(&raw_command);
        let mut state = TokenConfirmState::new(name_opt.clone(), raw_command.clone(), candidates);

        // Stage 2 + 3 loop so the target picker can step back to confirm.
        loop {
            let accept = match drive_confirm(&mut terminal, &mut state, theme, &mut viewport_top)? {
                ConfirmOutcome::Cancel => {
                    cleanup_terminal(viewport_top)?;
                    return Ok(CaptureOutcome::Cancelled);
                }
                ConfirmOutcome::Back => continue 'outer,
                ConfirmOutcome::Accept(accept) => accept,
            };

            // Stage 3: target file picker. A single candidate is used directly.
            let target = if targets.len() == 1 {
                targets[0].path.clone()
            } else {
                let labels: Vec<String> = targets.iter().map(|t| t.label.clone()).collect();
                let mut picker = HistoryPickerState::new(labels);
                match drive_target_picker(&mut terminal, &mut picker, theme, &mut viewport_top)? {
                    TargetPickOutcome::Cancel => {
                        cleanup_terminal(viewport_top)?;
                        return Ok(CaptureOutcome::Cancelled);
                    }
                    TargetPickOutcome::Back => continue,
                    TargetPickOutcome::Pick(label) => targets
                        .iter()
                        .find(|t| t.label == label)
                        .map(|t| t.path.clone())
                        .expect("picked label maps to a known target"),
                }
            };

            let first_token = state.raw.split_whitespace().next().map(|s| s.to_string());
            cleanup_terminal(viewport_top)?;
            return Ok(CaptureOutcome::Accepted {
                name: accept.name,
                raw: state.raw.clone(),
                accepted: accept.accepted,
                first_token,
                target,
            });
        }
    }
}

fn cleanup_terminal(viewport_top: Option<u16>) -> io::Result<()> {
    let mut stdout = io::stdout();
    if let Some(y) = viewport_top {
        execute!(
            stdout,
            cursor::MoveTo(0, y),
            crossterm::terminal::Clear(ClearType::FromCursorDown),
            cursor::Show
        )?;
    } else {
        execute!(stdout, cursor::Show)?;
    }
    stdout.flush()
}

fn drive_picker(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut HistoryPickerState,
    theme: &Theme,
    viewport_top: &mut Option<u16>,
) -> io::Result<PickOutcome> {
    loop {
        terminal.draw(|frame| {
            *viewport_top = viewport_top.or(Some(frame.area().y));
            render_picker(
                frame.area(),
                frame.buffer_mut(),
                state,
                theme,
                "pick a command",
                "↑↓/jk move   enter pick   type to filter   esc cancel",
            );
        })?;
        if !event::poll(Duration::from_millis(250))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        match picker_key(state, key) {
            PickerAction::Continue => {}
            PickerAction::Cancel => return Ok(PickOutcome::Cancel),
            PickerAction::Pick(s) => return Ok(PickOutcome::Pick(s)),
        }
    }
}

/// Outcome of the target-file pick stage.
enum TargetPickOutcome {
    /// User cancelled the whole capture (Ctrl+C).
    Cancel,
    /// User stepped back to the confirm screen (Esc).
    Back,
    /// User picked a destination (carries the chosen label).
    Pick(String),
}

enum TargetPickAction {
    Continue,
    Cancel,
    Back,
    Pick(String),
}

fn target_picker_key(state: &mut HistoryPickerState, key: KeyEvent) -> TargetPickAction {
    match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            TargetPickAction::Cancel
        }
        KeyCode::Esc => TargetPickAction::Back,
        KeyCode::Enter => match state.pick() {
            Some(s) if !s.trim().is_empty() => TargetPickAction::Pick(s),
            _ => TargetPickAction::Continue,
        },
        KeyCode::Up => {
            state.move_cursor(-1);
            TargetPickAction::Continue
        }
        KeyCode::Down => {
            state.move_cursor(1);
            TargetPickAction::Continue
        }
        KeyCode::Char('k') if key.modifiers.is_empty() && state.filter().is_empty() => {
            state.move_cursor(-1);
            TargetPickAction::Continue
        }
        KeyCode::Char('j') if key.modifiers.is_empty() && state.filter().is_empty() => {
            state.move_cursor(1);
            TargetPickAction::Continue
        }
        KeyCode::Backspace => {
            state.pop_filter();
            TargetPickAction::Continue
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.append_filter(c);
            TargetPickAction::Continue
        }
        _ => TargetPickAction::Continue,
    }
}

fn drive_target_picker(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut HistoryPickerState,
    theme: &Theme,
    viewport_top: &mut Option<u16>,
) -> io::Result<TargetPickOutcome> {
    loop {
        terminal.draw(|frame| {
            *viewport_top = viewport_top.or(Some(frame.area().y));
            render_picker(
                frame.area(),
                frame.buffer_mut(),
                state,
                theme,
                "pick a destination file",
                "↑↓/jk move   enter select   type to filter   esc back",
            );
        })?;
        if !event::poll(Duration::from_millis(250))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        match target_picker_key(state, key) {
            TargetPickAction::Continue => {}
            TargetPickAction::Cancel => return Ok(TargetPickOutcome::Cancel),
            TargetPickAction::Back => return Ok(TargetPickOutcome::Back),
            TargetPickAction::Pick(s) => return Ok(TargetPickOutcome::Pick(s)),
        }
    }
}

enum PickerAction {
    Continue,
    Cancel,
    Pick(String),
}

fn picker_key(state: &mut HistoryPickerState, key: KeyEvent) -> PickerAction {
    match key.code {
        KeyCode::Esc => PickerAction::Cancel,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => PickerAction::Cancel,
        KeyCode::Enter => match state.pick() {
            Some(s) if !s.trim().is_empty() => PickerAction::Pick(s),
            _ => PickerAction::Continue,
        },
        KeyCode::Up => {
            state.move_cursor(-1);
            PickerAction::Continue
        }
        KeyCode::Down => {
            state.move_cursor(1);
            PickerAction::Continue
        }
        KeyCode::Char('k') if key.modifiers.is_empty() && state.filter().is_empty() => {
            state.move_cursor(-1);
            PickerAction::Continue
        }
        KeyCode::Char('j') if key.modifiers.is_empty() && state.filter().is_empty() => {
            state.move_cursor(1);
            PickerAction::Continue
        }
        KeyCode::Backspace => {
            state.pop_filter();
            PickerAction::Continue
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.append_filter(c);
            PickerAction::Continue
        }
        _ => PickerAction::Continue,
    }
}

fn render_picker(
    area: Rect,
    buf: &mut ratatui::buffer::Buffer,
    state: &HistoryPickerState,
    theme: &Theme,
    title: &str,
    footer: &str,
) {
    use ratatui::widgets::Widget;
    let content = crate::tui_chrome::Chrome {
        theme,
        mode: "pb new",
        title,
        footer,
    }
    .render(area, buf);
    if content.height == 0 {
        return;
    }

    // Layout: filter row, divider, list.
    let filter_h = 1u16;
    let divider_y = content.y + filter_h;
    let list_area = Rect {
        x: content.x,
        y: divider_y + 1,
        width: content.width,
        height: content.height.saturating_sub(filter_h + 1),
    };

    Paragraph::new(Line::from(vec![
        RtSpan::styled("Filter: ", theme.chrome),
        RtSpan::styled(state.filter().to_string(), theme.emphasis),
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
    crate::tui_chrome::draw_divider(area, divider_y, buf, theme);

    let visible = state.visible();
    let max = list_area.height as usize;
    if max == 0 {
        return;
    }
    let total = visible.len();
    let start = if total <= max {
        0
    } else {
        state.cursor().saturating_sub(max / 2).min(total - max)
    };
    let mut lines: Vec<Line> = Vec::new();
    let total = visible.len();
    let idx_width = total.to_string().len().max(1);
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
        let index_label = format!("{:>width$}  ", i + 1, width = idx_width);
        lines.push(Line::from(vec![
            RtSpan::styled(marker.to_string(), marker_style),
            RtSpan::styled(index_label, theme.chrome),
            RtSpan::styled((*entry).to_string(), row_style),
        ]));
    }
    Paragraph::new(lines).render(list_area, buf);
}

fn drive_confirm(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut TokenConfirmState,
    theme: &Theme,
    viewport_top: &mut Option<u16>,
) -> io::Result<ConfirmOutcome> {
    loop {
        terminal.draw(|frame| {
            *viewport_top = viewport_top.or(Some(frame.area().y));
            render_confirm(frame.area(), frame.buffer_mut(), state, theme);
        })?;
        if !event::poll(Duration::from_millis(250))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        match confirm_key(state, key) {
            ConfirmAction::Continue => {}
            ConfirmAction::Cancel => return Ok(ConfirmOutcome::Cancel),
            ConfirmAction::Back => return Ok(ConfirmOutcome::Back),
            ConfirmAction::Accept(payload) => return Ok(ConfirmOutcome::Accept(payload)),
        }
    }
}

enum ConfirmAction {
    Continue,
    Cancel,
    Back,
    Accept(ConfirmAccept),
}

fn confirm_key(state: &mut TokenConfirmState, key: KeyEvent) -> ConfirmAction {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return ConfirmAction::Cancel;
    }
    match state.focus {
        Focus::Name => match key.code {
            KeyCode::Esc => ConfirmAction::Cancel,
            KeyCode::Enter => {
                if state.name.trim().is_empty() {
                    state.hint = Some("name required".to_string());
                    ConfirmAction::Continue
                } else {
                    state.focus = Focus::TokenList;
                    state.hint = None;
                    ConfirmAction::Continue
                }
            }
            KeyCode::Backspace => {
                state.name.pop();
                ConfirmAction::Continue
            }
            KeyCode::Tab | KeyCode::Down => {
                state.focus = Focus::TokenList;
                ConfirmAction::Continue
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                state.name.push(c);
                ConfirmAction::Continue
            }
            _ => ConfirmAction::Continue,
        },
        Focus::TokenList => match key.code {
            KeyCode::Esc => ConfirmAction::Cancel,
            KeyCode::Char('b') => ConfirmAction::Back,
            KeyCode::Up | KeyCode::Char('k') => {
                if state.cursor > 0 {
                    state.cursor -= 1;
                }
                ConfirmAction::Continue
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if state.cursor + 1 < state.candidates.len() {
                    state.cursor += 1;
                }
                ConfirmAction::Continue
            }
            KeyCode::Char(' ') => {
                state.toggle_focused();
                ConfirmAction::Continue
            }
            KeyCode::Char('e') => {
                state.start_rename();
                ConfirmAction::Continue
            }
            KeyCode::Char('n') => {
                state.focus = Focus::Name;
                ConfirmAction::Continue
            }
            KeyCode::Enter => match state.try_accept() {
                Some(payload) => ConfirmAction::Accept(payload),
                None => ConfirmAction::Continue,
            },
            _ => ConfirmAction::Continue,
        },
        Focus::TokenEdit => match key.code {
            KeyCode::Esc => {
                state.cancel_rename();
                ConfirmAction::Continue
            }
            KeyCode::Enter => {
                state.commit_rename();
                ConfirmAction::Continue
            }
            KeyCode::Backspace => {
                state.rename_buffer.pop();
                ConfirmAction::Continue
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                state.rename_buffer.push(c);
                ConfirmAction::Continue
            }
            _ => ConfirmAction::Continue,
        },
    }
}

fn render_confirm(
    area: Rect,
    buf: &mut ratatui::buffer::Buffer,
    state: &TokenConfirmState,
    theme: &Theme,
) {
    use ratatui::widgets::Widget;

    let footer = match state.focus {
        Focus::Name => "type name   enter confirm   tab/↓ tokens   esc cancel".to_string(),
        Focus::TokenList => {
            let mut base = "space toggle   e rename   n name   ↑↓/jk move   enter accept   b back   esc cancel".to_string();
            if let Some(hint) = &state.hint {
                base.push_str("   — ");
                base.push_str(hint);
            }
            base
        }
        Focus::TokenEdit => "type new name   enter commit   esc cancel".to_string(),
    };
    let content = crate::tui_chrome::Chrome {
        theme,
        mode: "pb new",
        title: "",
        footer: &footer,
    }
    .render(area, buf);
    if content.height == 0 {
        return;
    }

    // Section heights — Header (Name) and Preview (label + value) are fixed;
    // Tokens takes the remainder. Each section has a divider row above it
    // (except the first), drawn directly into the outer border.
    let header_h: u16 = 1;
    let preview_h: u16 = 2;
    let mut y = content.y;
    let render_section = |area: Rect, buf: &mut ratatui::buffer::Buffer, lines: Vec<Line>| {
        Paragraph::new(lines).render(area, buf);
    };

    // 1. Header: Name.
    let header_area = Rect {
        x: content.x,
        y,
        width: content.width,
        height: header_h.min(content.height),
    };
    let name_style = if state.focus == Focus::Name {
        theme.active_prompt
    } else {
        theme.emphasis
    };
    let name_cursor = if state.focus == Focus::Name { "_" } else { "" };
    let header_lines = vec![Line::from(vec![
        RtSpan::styled("Name:   ", theme.chrome),
        RtSpan::styled(format!("{}{name_cursor}", state.name), name_style),
    ])];
    render_section(header_area, buf, header_lines);
    y += header_area.height;

    // 2. Preview section.
    if y + 1 < content.y + content.height {
        crate::tui_chrome::draw_divider(area, y, buf, theme);
        y += 1;
        let preview_area = Rect {
            x: content.x,
            y,
            width: content.width,
            height: preview_h.min(content.y + content.height - y),
        };
        let preview_lines = vec![
            Line::from(RtSpan::styled("Preview:", theme.chrome)),
            Line::from(state.preview()),
        ];
        render_section(preview_area, buf, preview_lines);
        y += preview_area.height;
    }

    // 3. Tokens section.
    if y + 1 < content.y + content.height {
        crate::tui_chrome::draw_divider(area, y, buf, theme);
        y += 1;
        let tokens_area = Rect {
            x: content.x,
            y,
            width: content.width,
            height: (content.y + content.height).saturating_sub(y),
        };
        let mut rows: Vec<Line> = Vec::with_capacity(state.candidates.len() + 1);
        rows.push(Line::from(RtSpan::styled("Tokens:", theme.chrome)));
        for (i, cand) in state.candidates.iter().enumerate() {
            let mark = if state.selected[i] { "[x]" } else { "[ ]" };
            let name = if state.focus == Focus::TokenEdit && state.cursor == i {
                state.rename_buffer.as_str()
            } else {
                state.names[i].as_str()
            };
            let label = match cand.kind {
                crate::new::capture_heuristics::TokenKind::Secret => " (secret)",
                _ => "",
            };
            let prefix = if state.focus == Focus::TokenList && state.cursor == i {
                "  ▸ "
            } else if state.focus == Focus::TokenEdit && state.cursor == i {
                "  ✎ "
            } else {
                "    "
            };
            let mut name_style = Style::default();
            if cand.kind == crate::new::capture_heuristics::TokenKind::Secret {
                name_style = theme.error;
            }
            if state.cursor == i
                && (state.focus == Focus::TokenList || state.focus == Focus::TokenEdit)
            {
                name_style = name_style.add_modifier(Modifier::BOLD);
            }
            rows.push(Line::from(vec![
                RtSpan::styled(prefix.to_string(), theme.chrome),
                RtSpan::styled(format!("{mark} "), name_style),
                RtSpan::styled(
                    format!("{:<20}", truncate(&cand.original, 20)),
                    theme.chrome,
                ),
                RtSpan::styled(" → ".to_string(), theme.chrome),
                RtSpan::styled(name.to_string(), name_style),
                RtSpan::styled(label.to_string(), theme.chrome),
            ]));
        }
        render_section(tokens_area, buf, rows);
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picker_filter_narrows_visible_entries() {
        let entries = vec![
            "ssh root@host".to_string(),
            "docker run image".to_string(),
            "ssh other".to_string(),
        ];
        let mut state = HistoryPickerState::new(entries);
        state.append_filter('s');
        state.append_filter('s');
        state.append_filter('h');
        assert_eq!(state.visible().len(), 2);
        state.move_cursor(1);
        assert_eq!(state.pick(), Some("ssh other".to_string()));
    }

    #[test]
    fn picker_cursor_wraps() {
        let mut state =
            HistoryPickerState::new(vec!["a".to_string(), "b".to_string(), "c".to_string()]);
        state.move_cursor(-1);
        assert_eq!(state.pick(), Some("c".to_string()));
    }

    #[test]
    fn picker_pop_filter_restores_entries() {
        let mut state = HistoryPickerState::new(vec!["abc".to_string(), "def".to_string()]);
        state.append_filter('a');
        assert_eq!(state.visible().len(), 1);
        state.pop_filter();
        assert_eq!(state.visible().len(), 2);
    }

    fn ssh_state() -> TokenConfirmState {
        let raw = "ssh root@10.0.0.4 'systemctl restart nginx'".to_string();
        let candidates = crate::new::capture_heuristics::detect_variables(&raw);
        TokenConfirmState::new(Some("deploy".to_string()), raw, candidates)
    }

    #[test]
    fn confirm_empty_name_re_focuses_name() {
        let raw = "echo hi".to_string();
        let cands = crate::new::capture_heuristics::detect_variables(&raw);
        let mut state = TokenConfirmState::new(None, raw, cands);
        state.focus = Focus::TokenList;
        let r = state.try_accept();
        assert!(r.is_none());
        assert_eq!(state.focus, Focus::Name);
    }

    #[test]
    fn confirm_accepts_with_selected_tokens() {
        let mut state = ssh_state();
        for s in state.selected.iter_mut() {
            *s = true;
        }
        let r = state.try_accept().expect("accept");
        assert_eq!(r.name, "deploy");
        assert!(!r.accepted.is_empty());
    }

    #[test]
    fn confirm_warns_then_writes_when_secret_left_literal() {
        let raw = "curl --token=abc123XYZdef456ghijk http://x".to_string();
        let cands = crate::new::capture_heuristics::detect_variables(&raw);
        let mut state = TokenConfirmState::new(Some("call".to_string()), raw, cands);
        // Deselect the secret.
        for (i, c) in state.candidates.iter().enumerate() {
            if c.kind == crate::new::capture_heuristics::TokenKind::Secret {
                state.selected[i] = false;
            }
        }
        assert!(state.try_accept().is_none());
        assert!(state.awaiting_secret_confirm);
        // Second enter accepts despite the warning.
        assert!(state.try_accept().is_some());
    }

    #[test]
    fn confirm_preview_reflects_selection() {
        let raw = "echo 'hello world'".to_string();
        let cands = crate::new::capture_heuristics::detect_variables(&raw);
        let mut state = TokenConfirmState::new(Some("greet".to_string()), raw, cands);
        for s in state.selected.iter_mut() {
            *s = true;
        }
        assert_eq!(state.preview(), "echo '<@value>'");
    }

    #[test]
    fn confirm_rename_changes_token_name() {
        let mut state = ssh_state();
        state.cursor = 0;
        state.start_rename();
        state.rename_buffer = "service".to_string();
        state.commit_rename();
        assert_eq!(state.names[0], "service");
    }

    #[test]
    fn confirm_rename_strips_disallowed_chars() {
        let mut state = ssh_state();
        state.cursor = 0;
        state.start_rename();
        state.rename_buffer = "bad name!".to_string();
        state.commit_rename();
        assert_eq!(state.names[0], "badname");
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn target_picker_enter_picks_filtered_entry() {
        let mut state = HistoryPickerState::new(vec![
            "work/db.md".to_string(),
            "personal/notes.md".to_string(),
        ]);
        for c in "notes".chars() {
            state.append_filter(c);
        }
        match target_picker_key(&mut state, key(KeyCode::Enter)) {
            TargetPickAction::Pick(s) => assert_eq!(s, "personal/notes.md"),
            _ => panic!("expected pick"),
        }
    }

    #[test]
    fn target_picker_esc_steps_back_ctrl_c_cancels() {
        let mut state = HistoryPickerState::new(vec!["a.md".to_string()]);
        assert!(matches!(
            target_picker_key(&mut state, key(KeyCode::Esc)),
            TargetPickAction::Back
        ));
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(matches!(
            target_picker_key(&mut state, ctrl_c),
            TargetPickAction::Cancel
        ));
    }
}
