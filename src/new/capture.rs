//! Capture TUI for `pb new` — history picker and token-confirm stages.
//!
//! Both stages are driven by a state machine that is independent of the
//! terminal. Tests drive these states directly; the [`run_capture`] entry
//! point wraps them with crossterm/ratatui plumbing.

use crate::config::Theme;
use crate::keybinds::{
    ContextBindings, NewConfirmNameAction, NewConfirmRenameAction, NewConfirmTokensAction,
    NewKeymap, NewPickerAction, TextEntry, help_hint, help_move_hint, is_emergency_cancel,
};
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
    /// Resolved keybinds for the capture screens.
    pub keymap: &'a NewKeymap,
    /// Keybind resolution warnings, shown as TUI status (never stdout — `pb
    /// new` prints its result there).
    pub keybind_warnings: &'a [String],
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
        keymap,
        keybind_warnings,
    } = run;
    let warning_status = (!keybind_warnings.is_empty())
        .then(|| format!("keybind config: {}", keybind_warnings.join("; ")));

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
            match drive_picker(
                &mut terminal,
                &mut state,
                theme,
                &mut viewport_top,
                keymap,
                warning_status.as_deref(),
            )? {
                PickOutcome::Cancel => {
                    cleanup_terminal(viewport_top)?;
                    return Ok(CaptureOutcome::Cancelled);
                }
                PickOutcome::Pick(s) => s,
            }
        };

        let candidates = crate::new::capture_heuristics::detect_variables(&raw_command);
        let mut state = TokenConfirmState::new(name_opt.clone(), raw_command.clone(), candidates);
        state.hint = warning_status.clone();

        // Stage 2 + 3 loop so the target picker can step back to confirm.
        loop {
            let accept =
                match drive_confirm(&mut terminal, &mut state, theme, &mut viewport_top, keymap)? {
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
                match drive_target_picker(
                    &mut terminal,
                    &mut picker,
                    theme,
                    &mut viewport_top,
                    keymap,
                    warning_status.as_deref(),
                )? {
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
    keymap: &NewKeymap,
    status: Option<&str>,
) -> io::Result<PickOutcome> {
    let title = picker_title("pick a command", status);
    let footer = picker_footer(&keymap.picker, "pick", "cancel");
    loop {
        terminal.draw(|frame| {
            *viewport_top = viewport_top.or(Some(frame.area().y));
            render_picker(
                frame.area(),
                frame.buffer_mut(),
                state,
                theme,
                &title,
                &footer,
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
        match picker_key(state, key, &keymap.picker) {
            PickerAction::Continue => {}
            PickerAction::Cancel => return Ok(PickOutcome::Cancel),
            PickerAction::Pick(s) => return Ok(PickOutcome::Pick(s)),
        }
    }
}

fn picker_title(title: &str, status: Option<&str>) -> String {
    match status {
        Some(status) => format!("{title} · {status}"),
        None => title.to_string(),
    }
}

/// Build a picker footer from the resolved bindings, omitting unbound
/// actions. The "type to filter" hint is inherent to the widget, not a
/// binding.
fn picker_footer(
    keys: &ContextBindings<NewPickerAction>,
    accept_label: &str,
    cancel_label: &str,
) -> String {
    [
        help_move_hint(
            keys.hint(NewPickerAction::MoveUp),
            keys.hint(NewPickerAction::MoveDown),
            "move",
        ),
        help_hint(keys.hint(NewPickerAction::Accept), accept_label),
        Some("type to filter".to_string()),
        help_hint(keys.hint(NewPickerAction::CancelOrBack), cancel_label),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join("   ")
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

fn target_picker_key(
    state: &mut HistoryPickerState,
    key: KeyEvent,
    keys: &ContextBindings<NewPickerAction>,
) -> TargetPickAction {
    if is_emergency_cancel(&key) {
        return TargetPickAction::Cancel;
    }
    match keys.resolve(&key, TextEntry::WhenEmpty(state.filter().is_empty())) {
        Some(NewPickerAction::Accept) => match state.pick() {
            Some(s) if !s.trim().is_empty() => TargetPickAction::Pick(s),
            _ => TargetPickAction::Continue,
        },
        Some(NewPickerAction::CancelOrBack) => TargetPickAction::Back,
        Some(NewPickerAction::MoveUp) => {
            state.move_cursor(-1);
            TargetPickAction::Continue
        }
        Some(NewPickerAction::MoveDown) => {
            state.move_cursor(1);
            TargetPickAction::Continue
        }
        Some(NewPickerAction::Backspace) => {
            state.pop_filter();
            TargetPickAction::Continue
        }
        None => {
            filter_fallback(state, key);
            TargetPickAction::Continue
        }
    }
}

fn drive_target_picker(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut HistoryPickerState,
    theme: &Theme,
    viewport_top: &mut Option<u16>,
    keymap: &NewKeymap,
    status: Option<&str>,
) -> io::Result<TargetPickOutcome> {
    let title = picker_title("pick a destination file", status);
    let footer = picker_footer(&keymap.picker, "select", "back");
    loop {
        terminal.draw(|frame| {
            *viewport_top = viewport_top.or(Some(frame.area().y));
            render_picker(
                frame.area(),
                frame.buffer_mut(),
                state,
                theme,
                &title,
                &footer,
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
        match target_picker_key(state, key, &keymap.picker) {
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

/// Unresolved keys fall through to filter text entry, mirroring the widget's
/// pre-keymap behavior (any unmodified or shifted printable char types).
fn filter_fallback(state: &mut HistoryPickerState, key: KeyEvent) {
    if let KeyCode::Char(c) = key.code
        && !key.modifiers.contains(KeyModifiers::CONTROL)
    {
        state.append_filter(c);
    }
}

fn picker_key(
    state: &mut HistoryPickerState,
    key: KeyEvent,
    keys: &ContextBindings<NewPickerAction>,
) -> PickerAction {
    if is_emergency_cancel(&key) {
        return PickerAction::Cancel;
    }
    match keys.resolve(&key, TextEntry::WhenEmpty(state.filter().is_empty())) {
        Some(NewPickerAction::Accept) => match state.pick() {
            Some(s) if !s.trim().is_empty() => PickerAction::Pick(s),
            _ => PickerAction::Continue,
        },
        Some(NewPickerAction::CancelOrBack) => PickerAction::Cancel,
        Some(NewPickerAction::MoveUp) => {
            state.move_cursor(-1);
            PickerAction::Continue
        }
        Some(NewPickerAction::MoveDown) => {
            state.move_cursor(1);
            PickerAction::Continue
        }
        Some(NewPickerAction::Backspace) => {
            state.pop_filter();
            PickerAction::Continue
        }
        None => {
            filter_fallback(state, key);
            PickerAction::Continue
        }
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
    let content = crate::tui::chrome::Chrome {
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
    crate::tui::chrome::draw_divider(area, divider_y, buf, theme);

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
    keymap: &NewKeymap,
) -> io::Result<ConfirmOutcome> {
    loop {
        terminal.draw(|frame| {
            *viewport_top = viewport_top.or(Some(frame.area().y));
            render_confirm(frame.area(), frame.buffer_mut(), state, theme, keymap);
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
        match confirm_key(state, key, keymap) {
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

fn confirm_key(state: &mut TokenConfirmState, key: KeyEvent, keymap: &NewKeymap) -> ConfirmAction {
    if is_emergency_cancel(&key) {
        return ConfirmAction::Cancel;
    }
    match state.focus {
        // The name field always accepts text, so plain-letter bindings are
        // inert here by design (TextEntry::Always).
        Focus::Name => match keymap.confirm_name.resolve(&key, TextEntry::Always) {
            Some(NewConfirmNameAction::Cancel) => ConfirmAction::Cancel,
            Some(NewConfirmNameAction::Accept) => {
                if state.name.trim().is_empty() {
                    state.hint = Some("name required".to_string());
                } else {
                    state.focus = Focus::TokenList;
                    state.hint = None;
                }
                ConfirmAction::Continue
            }
            Some(NewConfirmNameAction::Backspace) => {
                state.name.pop();
                ConfirmAction::Continue
            }
            Some(NewConfirmNameAction::CompleteOrFocusTokens) => {
                state.focus = Focus::TokenList;
                ConfirmAction::Continue
            }
            None => {
                if let KeyCode::Char(c) = key.code
                    && !key.modifiers.contains(KeyModifiers::CONTROL)
                {
                    state.name.push(c);
                }
                ConfirmAction::Continue
            }
        },
        Focus::TokenList => match keymap.confirm_tokens.resolve(&key, TextEntry::None) {
            Some(NewConfirmTokensAction::Cancel) => ConfirmAction::Cancel,
            Some(NewConfirmTokensAction::Back) => ConfirmAction::Back,
            Some(NewConfirmTokensAction::MoveUp) => {
                if state.cursor > 0 {
                    state.cursor -= 1;
                }
                ConfirmAction::Continue
            }
            Some(NewConfirmTokensAction::MoveDown) => {
                if state.cursor + 1 < state.candidates.len() {
                    state.cursor += 1;
                }
                ConfirmAction::Continue
            }
            Some(NewConfirmTokensAction::ToggleVariable) => {
                state.toggle_focused();
                ConfirmAction::Continue
            }
            Some(NewConfirmTokensAction::Rename) => {
                state.start_rename();
                ConfirmAction::Continue
            }
            Some(NewConfirmTokensAction::EditName) => {
                state.focus = Focus::Name;
                ConfirmAction::Continue
            }
            Some(NewConfirmTokensAction::Accept) => match state.try_accept() {
                Some(payload) => ConfirmAction::Accept(payload),
                None => ConfirmAction::Continue,
            },
            None => ConfirmAction::Continue,
        },
        // The rename field always accepts text (TextEntry::Always).
        Focus::TokenEdit => match keymap.confirm_rename.resolve(&key, TextEntry::Always) {
            Some(NewConfirmRenameAction::Cancel) => {
                state.cancel_rename();
                ConfirmAction::Continue
            }
            Some(NewConfirmRenameAction::Accept) => {
                state.commit_rename();
                ConfirmAction::Continue
            }
            Some(NewConfirmRenameAction::Backspace) => {
                state.rename_buffer.pop();
                ConfirmAction::Continue
            }
            None => {
                if let KeyCode::Char(c) = key.code
                    && !key.modifiers.contains(KeyModifiers::CONTROL)
                {
                    state.rename_buffer.push(c);
                }
                ConfirmAction::Continue
            }
        },
    }
}

fn render_confirm(
    area: Rect,
    buf: &mut ratatui::buffer::Buffer,
    state: &TokenConfirmState,
    theme: &Theme,
    keymap: &NewKeymap,
) {
    use ratatui::widgets::Widget;

    let footer = match state.focus {
        Focus::Name => confirm_footer_parts(vec![
            Some("type name".to_string()),
            help_hint(
                keymap.confirm_name.hint(NewConfirmNameAction::Accept),
                "confirm",
            ),
            help_hint(
                keymap
                    .confirm_name
                    .hint(NewConfirmNameAction::CompleteOrFocusTokens),
                "tokens",
            ),
            help_hint(
                keymap.confirm_name.hint(NewConfirmNameAction::Cancel),
                "cancel",
            ),
        ]),
        Focus::TokenList => {
            let tokens = &keymap.confirm_tokens;
            let mut base = confirm_footer_parts(vec![
                help_hint(
                    tokens.hint(NewConfirmTokensAction::ToggleVariable),
                    "toggle",
                ),
                help_hint(tokens.hint(NewConfirmTokensAction::Rename), "rename"),
                help_hint(tokens.hint(NewConfirmTokensAction::EditName), "name"),
                help_move_hint(
                    tokens.hint(NewConfirmTokensAction::MoveUp),
                    tokens.hint(NewConfirmTokensAction::MoveDown),
                    "move",
                ),
                help_hint(tokens.hint(NewConfirmTokensAction::Accept), "accept"),
                help_hint(tokens.hint(NewConfirmTokensAction::Back), "back"),
                help_hint(tokens.hint(NewConfirmTokensAction::Cancel), "cancel"),
            ]);
            if let Some(hint) = &state.hint {
                base.push_str("   — ");
                base.push_str(hint);
            }
            base
        }
        Focus::TokenEdit => confirm_footer_parts(vec![
            Some("type new name".to_string()),
            help_hint(
                keymap.confirm_rename.hint(NewConfirmRenameAction::Accept),
                "commit",
            ),
            help_hint(
                keymap.confirm_rename.hint(NewConfirmRenameAction::Cancel),
                "cancel",
            ),
        ]),
    };
    let content = crate::tui::chrome::Chrome {
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
        crate::tui::chrome::draw_divider(area, y, buf, theme);
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
        crate::tui::chrome::draw_divider(area, y, buf, theme);
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

fn confirm_footer_parts(parts: Vec<Option<String>>) -> String {
    parts.into_iter().flatten().collect::<Vec<_>>().join("   ")
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

    fn default_keymap() -> NewKeymap {
        NewKeymap::default()
    }

    fn keymap_from(raw: &str) -> NewKeymap {
        let value: toml::Value = toml::from_str(raw).unwrap();
        crate::keybinds::Keymaps::resolve(value.get("keybinds")).new
    }

    fn ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    #[test]
    fn target_picker_enter_picks_filtered_entry() {
        let keymap = default_keymap();
        let mut state = HistoryPickerState::new(vec![
            "work/db.md".to_string(),
            "personal/notes.md".to_string(),
        ]);
        for c in "notes".chars() {
            state.append_filter(c);
        }
        match target_picker_key(&mut state, key(KeyCode::Enter), &keymap.picker) {
            TargetPickAction::Pick(s) => assert_eq!(s, "personal/notes.md"),
            _ => panic!("expected pick"),
        }
    }

    #[test]
    fn target_picker_esc_steps_back_ctrl_c_cancels() {
        let keymap = default_keymap();
        let mut state = HistoryPickerState::new(vec!["a.md".to_string()]);
        assert!(matches!(
            target_picker_key(&mut state, key(KeyCode::Esc), &keymap.picker),
            TargetPickAction::Back
        ));
        assert!(matches!(
            target_picker_key(&mut state, ctrl(KeyCode::Char('c')), &keymap.picker),
            TargetPickAction::Cancel
        ));
    }

    #[test]
    fn picker_letter_navigates_on_empty_filter_and_types_otherwise() {
        let keymap = default_keymap();
        let mut state =
            HistoryPickerState::new(vec!["a".to_string(), "b".to_string(), "kite".to_string()]);
        // Empty filter: `k` navigates.
        picker_key(&mut state, key(KeyCode::Char('k')), &keymap.picker);
        assert_eq!(state.cursor(), 2);
        assert_eq!(state.filter(), "");
        // Non-empty filter: `k` is text.
        picker_key(&mut state, key(KeyCode::Char('i')), &keymap.picker);
        picker_key(&mut state, key(KeyCode::Char('k')), &keymap.picker);
        assert_eq!(state.filter(), "ik");
        // Named keys still act mid-filter.
        assert!(matches!(
            picker_key(&mut state, key(KeyCode::Esc), &keymap.picker),
            PickerAction::Cancel
        ));
    }

    #[test]
    fn remapped_plain_letter_picker_action_respects_text_rule() {
        let keymap = keymap_from(
            r#"
[keybinds.new.picker]
move_down = ["n"]
"#,
        );
        let mut state = HistoryPickerState::new(vec!["one".to_string(), "two".to_string()]);
        // Empty filter: remapped `n` navigates.
        picker_key(&mut state, key(KeyCode::Char('n')), &keymap.picker);
        assert_eq!(state.cursor(), 1);
        assert_eq!(state.filter(), "");
        // Replaced defaults are inert as actions; `j` types into the filter.
        picker_key(&mut state, key(KeyCode::Char('j')), &keymap.picker);
        assert_eq!(state.filter(), "j");
        assert_eq!(state.cursor(), 0, "filter reset the cursor");
        // With a non-empty filter, `n` is text again.
        picker_key(&mut state, key(KeyCode::Char('n')), &keymap.picker);
        assert_eq!(state.filter(), "jn");
    }

    #[test]
    fn confirm_tokens_remapped_accept_toggle_rename_and_back() {
        let keymap = keymap_from(
            r#"
[keybinds.new.confirm_tokens]
accept = ["ctrl+a"]
toggle_variable = ["t"]
rename = ["f2"]
back = ["ctrl+b"]
"#,
        );
        let mut state = ssh_state();
        state.focus = Focus::TokenList;
        let was_selected = state.selected[0];
        confirm_key(&mut state, key(KeyCode::Char('t')), &keymap);
        assert_eq!(state.selected[0], !was_selected);
        // Replaced default is inert.
        confirm_key(&mut state, key(KeyCode::Char(' ')), &keymap);
        assert_eq!(state.selected[0], !was_selected);

        confirm_key(&mut state, key(KeyCode::F(2)), &keymap);
        assert_eq!(state.focus, Focus::TokenEdit);
        state.cancel_rename();

        assert!(matches!(
            confirm_key(&mut state, ctrl(KeyCode::Char('b')), &keymap),
            ConfirmAction::Back
        ));
        assert!(matches!(
            confirm_key(&mut state, ctrl(KeyCode::Char('a')), &keymap),
            ConfirmAction::Accept(_)
        ));
        // Replaced accept default is inert.
        assert!(matches!(
            confirm_key(&mut state, key(KeyCode::Enter), &keymap),
            ConfirmAction::Continue
        ));
    }

    #[test]
    fn confirm_name_remapped_complete_or_focus_tokens() {
        let keymap = keymap_from(
            r#"
[keybinds.new.confirm_name]
complete_or_focus_tokens = ["ctrl+t"]
"#,
        );
        let mut state = ssh_state();
        state.focus = Focus::Name;
        confirm_key(&mut state, ctrl(KeyCode::Char('t')), &keymap);
        assert_eq!(state.focus, Focus::TokenList);
        // Replaced default is inert.
        state.focus = Focus::Name;
        confirm_key(&mut state, key(KeyCode::Tab), &keymap);
        assert_eq!(state.focus, Focus::Name);
    }

    #[test]
    fn plain_letter_bindings_stay_inert_in_text_widgets() {
        // Plain letters can never be actions where text is always accepted.
        let keymap = keymap_from(
            r#"
[keybinds.new.confirm_name]
accept = ["a"]

[keybinds.new.confirm_rename]
cancel = ["z"]
"#,
        );
        let mut state = ssh_state();
        state.focus = Focus::Name;
        state.name.clear();
        confirm_key(&mut state, key(KeyCode::Char('a')), &keymap);
        assert_eq!(state.name, "a", "letter must be typed, not accepted");
        assert_eq!(state.focus, Focus::Name);

        state.focus = Focus::TokenList;
        state.cursor = 0;
        state.start_rename();
        confirm_key(&mut state, key(KeyCode::Char('z')), &keymap);
        assert_eq!(state.focus, Focus::TokenEdit, "rename must not cancel");
        assert!(state.rename_buffer.ends_with('z'), "letter must be typed");
    }

    #[test]
    fn ctrl_c_cancels_from_every_capture_screen() {
        let keymap = default_keymap();
        let ctrl_c = ctrl(KeyCode::Char('c'));

        let mut picker = HistoryPickerState::new(vec!["a".to_string()]);
        assert!(matches!(
            picker_key(&mut picker, ctrl_c, &keymap.picker),
            PickerAction::Cancel
        ));
        assert!(matches!(
            target_picker_key(&mut picker, ctrl_c, &keymap.picker),
            TargetPickAction::Cancel
        ));

        for focus in [Focus::Name, Focus::TokenList, Focus::TokenEdit] {
            let mut state = ssh_state();
            state.focus = focus;
            assert!(
                matches!(
                    confirm_key(&mut state, ctrl_c, &keymap),
                    ConfirmAction::Cancel
                ),
                "ctrl+c must cancel from {focus:?}"
            );
        }
    }

    #[test]
    fn confirm_footers_derive_from_keymap() {
        let keymap = keymap_from(
            r#"
[keybinds.new.confirm_tokens]
toggle_variable = ["f9"]
back = []
"#,
        );
        let footer = confirm_footer_parts(vec![
            help_hint(
                keymap
                    .confirm_tokens
                    .hint(NewConfirmTokensAction::ToggleVariable),
                "toggle",
            ),
            help_hint(
                keymap.confirm_tokens.hint(NewConfirmTokensAction::Back),
                "back",
            ),
        ]);
        assert_eq!(footer, "f9 toggle");

        let picker_help = picker_footer(&keymap.picker, "pick", "cancel");
        assert_eq!(
            picker_help,
            "up/down move   enter pick   type to filter   esc cancel"
        );
    }
}
