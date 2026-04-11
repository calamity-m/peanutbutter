use crate::browse::{BrowseEntry, BrowseState, BrowseTree};
use crate::config;
use crate::domain::{SnippetId, Variable, VariableSource};
use crate::frecency::FrecencyStore;
use crate::fuzzy::FuzzyState;
use crate::index::{IndexedSnippet, SnippetIndex};
use crate::search;
use crossterm::cursor;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::terminal::{self, ClearType, disable_raw_mode, enable_raw_mode};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Position;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use ratatui::{Frame, Terminal, TerminalOptions, Viewport};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io;
use std::io::IsTerminal;
use std::io::Write;
use std::os::fd::{FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionOutcome {
    pub snippet_id: SnippetId,
    pub command: String,
}

#[derive(Debug, Clone)]
pub struct ExecuteOptions {
    pub cwd: PathBuf,
    pub now: u64,
    pub viewport_height: u16,
}

impl Default for ExecuteOptions {
    fn default() -> Self {
        Self {
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            now: unix_now(),
            viewport_height: 20,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavigationMode {
    Fuzzy,
    Browse,
}

pub trait SuggestionProvider {
    fn suggestions(&self, variable: &Variable, cwd: &Path) -> io::Result<Vec<String>>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemSuggestionProvider;

impl SuggestionProvider for SystemSuggestionProvider {
    fn suggestions(&self, variable: &Variable, cwd: &Path) -> io::Result<Vec<String>> {
        match &variable.source {
            VariableSource::Command(cmd) => command_suggestions(cmd, cwd),
            VariableSource::Default(_) => Ok(Vec::new()),
            VariableSource::Free => builtin_suggestions(&variable.name, cwd),
        }
    }
}

enum Screen {
    Select,
    Preview { snippet_id: SnippetId },
    Prompt(PromptState),
}

struct PromptState {
    snippet_id: SnippetId,
    variables: Vec<Variable>,
    index: usize,
    values: BTreeMap<String, String>,
    input: String,
    suggestions: Vec<String>,
    error: Option<String>,
    list: ratatui::widgets::ListState,
}

impl PromptState {
    fn new(snippet_id: SnippetId, variables: Vec<Variable>) -> Self {
        Self {
            snippet_id,
            variables,
            index: 0,
            values: BTreeMap::new(),
            input: String::new(),
            suggestions: Vec::new(),
            error: None,
            list: ratatui::widgets::ListState::default(),
        }
    }

    fn current_variable(&self) -> &Variable {
        &self.variables[self.index]
    }

    fn current_value(&self) -> String {
        if !self.input.is_empty() {
            return self.input.clone();
        }
        self.selected_visible_suggestion()
            .cloned()
            .unwrap_or_default()
    }

    fn visible_suggestions(&self) -> Vec<&String> {
        let needle = self.input.to_lowercase();
        let mut out: Vec<&String> = self
            .suggestions
            .iter()
            .filter(|value| needle.is_empty() || value.to_lowercase().contains(&needle))
            .collect();
        out.sort();
        out
    }

    fn selected_visible_suggestion(&self) -> Option<&String> {
        let visible = self.visible_suggestions();
        let idx = self.list.selected().unwrap_or(0);
        visible.get(idx).copied()
    }

    fn reset_selection(&mut self) {
        if self.visible_suggestions().is_empty() {
            self.list.select(None);
        } else {
            self.list.select(Some(0));
        }
    }

    fn move_cursor(&mut self, delta: i32) {
        let visible_len = self.visible_suggestions().len();
        if visible_len == 0 {
            self.list.select(None);
            return;
        }
        let current = self.list.selected().unwrap_or(0) as i32;
        let next = (current + delta).clamp(0, visible_len as i32 - 1);
        self.list.select(Some(next as usize));
    }
}

pub struct ExecutionApp<P = SystemSuggestionProvider> {
    index: SnippetIndex,
    frecency: FrecencyStore,
    tree: BrowseTree,
    cwd: PathBuf,
    now: u64,
    provider: P,
    screen: Screen,
    nav_mode: NavigationMode,
    pub fuzzy: FuzzyState,
    pub browse: BrowseState,
    status: Option<String>,
}

pub enum AppEvent {
    Continue,
    Cancelled,
    Completed(ExecutionOutcome),
}

impl<P: SuggestionProvider> ExecutionApp<P> {
    pub fn new(
        index: SnippetIndex,
        frecency: FrecencyStore,
        cwd: PathBuf,
        now: u64,
        provider: P,
    ) -> Self {
        Self {
            tree: BrowseTree::from_index(&index),
            index,
            frecency,
            cwd,
            now,
            provider,
            screen: Screen::Select,
            nav_mode: NavigationMode::Fuzzy,
            fuzzy: FuzzyState::new(),
            browse: BrowseState::new(),
            status: None,
        }
    }

    pub fn navigation_mode(&self) -> NavigationMode {
        self.nav_mode
    }

    pub fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }

    pub fn selected_snippet(&self) -> Option<&IndexedSnippet> {
        match &self.screen {
            Screen::Select => match self.nav_mode {
                NavigationMode::Fuzzy => self.selected_fuzzy_snippet(),
                NavigationMode::Browse => self.selected_browse_snippet(),
            },
            Screen::Preview { snippet_id } => self.index.get(snippet_id),
            Screen::Prompt(prompt) => self.index.get(&prompt.snippet_id),
        }
    }

    pub fn partial_command(&self) -> Option<String> {
        match &self.screen {
            Screen::Prompt(prompt) => {
                let snippet = self.index.get(&prompt.snippet_id)?;
                let mut values = prompt.values.clone();
                let value = prompt.current_value();
                if !value.is_empty() {
                    values.insert(prompt.current_variable().name.clone(), value);
                }
                Some(render_command(snippet.body(), &values))
            }
            _ => self
                .selected_snippet()
                .map(|snippet| snippet.body().to_string()),
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> AppEvent {
        if key.kind == KeyEventKind::Release {
            return AppEvent::Continue;
        }

        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.screen = Screen::Select;
            self.status = Some("cancelled".to_string());
            return AppEvent::Cancelled;
        }

        match &mut self.screen {
            Screen::Select => self.handle_select_key(key),
            Screen::Preview { snippet_id } => {
                let snippet_id = snippet_id.clone();
                self.handle_preview_key(key, snippet_id)
            }
            Screen::Prompt(prompt) => match handle_prompt_key(
                key,
                prompt,
                &self.provider,
                &self.cwd,
                &self.index,
                &mut self.status,
            ) {
                PromptTransition::Stay => AppEvent::Continue,
                PromptTransition::ToPreview(snippet_id) => {
                    self.screen = Screen::Preview { snippet_id };
                    AppEvent::Continue
                }
                PromptTransition::Completed(outcome) => AppEvent::Completed(outcome),
            },
        }
    }

    fn handle_select_key(&mut self, key: KeyEvent) -> AppEvent {
        if matches!(key.code, KeyCode::Esc) {
            self.status = Some("cancelled".to_string());
            return AppEvent::Cancelled;
        }

        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('t')
            || key.code == KeyCode::F(2)
        {
            self.nav_mode = match self.nav_mode {
                NavigationMode::Fuzzy => NavigationMode::Browse,
                NavigationMode::Browse => NavigationMode::Fuzzy,
            };
            self.status = None;
            return AppEvent::Continue;
        }

        match self.nav_mode {
            NavigationMode::Fuzzy => self.handle_fuzzy_key(key),
            NavigationMode::Browse => self.handle_browse_key(key),
        }
    }

    fn handle_fuzzy_key(&mut self, key: KeyEvent) -> AppEvent {
        let hits = self.search_hits();
        match key.code {
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.fuzzy.type_char(c);
            }
            KeyCode::Backspace => {
                self.fuzzy.backspace();
            }
            KeyCode::Left => {
                self.fuzzy.cursor_left();
            }
            KeyCode::Right => {
                self.fuzzy.cursor_right();
            }
            KeyCode::Up => {
                self.fuzzy.move_cursor(-1, hits.len());
            }
            KeyCode::Down => {
                self.fuzzy.move_cursor(1, hits.len());
            }
            KeyCode::Enter => {
                if let Some(snippet) = self.selected_fuzzy_snippet() {
                    self.screen = Screen::Preview {
                        snippet_id: snippet.id().clone(),
                    };
                    self.status = None;
                }
            }
            _ => {}
        }
        AppEvent::Continue
    }

    fn handle_browse_key(&mut self, key: KeyEvent) -> AppEvent {
        let visible = self.browse.visible(&self.tree);
        match key.code {
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.browse.type_char(c);
            }
            KeyCode::Backspace => {
                self.browse.backspace();
            }
            KeyCode::Tab => {
                self.browse.tab_complete(&self.tree);
            }
            KeyCode::Up => {
                self.browse.move_cursor(-1, visible.len());
            }
            KeyCode::Down => {
                self.browse.move_cursor(1, visible.len());
            }
            KeyCode::Enter => {
                if let Some(id) = self.browse.activate(&self.tree) {
                    self.screen = Screen::Preview { snippet_id: id };
                    self.status = None;
                }
            }
            _ => {}
        }
        AppEvent::Continue
    }

    fn handle_preview_key(&mut self, key: KeyEvent, snippet_id: SnippetId) -> AppEvent {
        match key.code {
            KeyCode::Esc | KeyCode::Backspace => {
                self.screen = Screen::Select;
                self.status = None;
            }
            KeyCode::Enter => {
                if let AppEvent::Completed(outcome) = self.start_prompt_or_complete(snippet_id) {
                    return AppEvent::Completed(outcome);
                }
            }
            _ => {}
        }
        AppEvent::Continue
    }

    fn start_prompt_or_complete(&mut self, snippet_id: SnippetId) -> AppEvent {
        let Some(snippet) = self.index.get(&snippet_id) else {
            return AppEvent::Continue;
        };
        let variables = unique_variables(&snippet.snippet.variables);
        if variables.is_empty() {
            return AppEvent::Completed(ExecutionOutcome {
                snippet_id,
                command: snippet.body().to_string(),
            });
        }
        let mut prompt = PromptState::new(snippet_id, variables);
        load_prompt_state(&mut prompt, &self.provider, &self.cwd, &mut self.status);
        self.screen = Screen::Prompt(prompt);
        AppEvent::Continue
    }

    fn search_hits(&self) -> Vec<search::SearchHit<'_>> {
        search::rank(
            &self.index,
            &self.fuzzy.query,
            &self.frecency,
            &self.cwd,
            self.now,
        )
    }

    fn selected_fuzzy_snippet(&self) -> Option<&IndexedSnippet> {
        let hits = self.search_hits();
        let idx = self.fuzzy.selected().unwrap_or(0);
        hits.get(idx).map(|hit| hit.snippet)
    }

    fn selected_browse_snippet(&self) -> Option<&IndexedSnippet> {
        let visible = self.browse.visible(&self.tree);
        let idx = self.browse.list.selected().unwrap_or(0);
        let entry = visible.get(idx)?;
        match entry {
            BrowseEntry::Snippet(snippet) => self.index.get(&snippet.id),
            BrowseEntry::Directory(_) => None,
        }
    }

    pub fn render(&mut self, frame: &mut Frame<'_>) {
        if matches!(self.screen, Screen::Select) {
            self.render_select(frame);
            return;
        }
        match &self.screen {
            Screen::Select => unreachable!(),
            Screen::Preview { snippet_id } => self.render_preview(frame, snippet_id),
            Screen::Prompt(prompt) => self.render_prompt(frame, prompt),
        }
    }

    fn render_select(&mut self, frame: &mut Frame<'_>) {
        let area = frame.area();
        let fuzzy_hits = self.search_hits();
        let browse_visible = self.browse.visible(&self.tree);
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .split(area);
        let main = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
            .split(chunks[1]);

        let (prompt, stats, mode) = match self.nav_mode {
            NavigationMode::Fuzzy => (
                self.fuzzy.query.clone(),
                format!("{}/{}", fuzzy_hits.len(), self.index.len()),
                "Fuzzy",
            ),
            NavigationMode::Browse => (
                format!("{}{}", self.browse.path_display(), self.browse.input),
                browse_visible.len().to_string(),
                "Browse",
            ),
        };
        frame.render_widget(
            select_header(&prompt, &stats, mode, chunks[0].width),
            chunks[0],
        );

        match self.nav_mode {
            NavigationMode::Fuzzy => {
                let total = fuzzy_hits.len();
                let selected = self.fuzzy.selected().unwrap_or(0);
                let items: Vec<ListItem<'_>> = fuzzy_hits
                    .iter()
                    .enumerate()
                    .map(|(idx, hit)| {
                        let content = format!(
                            "{}  [{}]",
                            hit.snippet.name(),
                            hit.snippet.relative_path_display()
                        );
                        ListItem::new(snippet_list_line(idx, total, selected, &content))
                    })
                    .collect();
                let list = List::new(items);
                frame.render_stateful_widget(list, main[0], &mut self.fuzzy.list);
            }
            NavigationMode::Browse => {
                let total = browse_visible.len();
                let selected = self.browse.list.selected().unwrap_or(0);
                let items: Vec<ListItem<'_>> = browse_visible
                    .iter()
                    .enumerate()
                    .map(|(idx, entry)| {
                        let label = match entry {
                            BrowseEntry::Directory(name) => format!("{name}/"),
                            BrowseEntry::Snippet(snippet) => snippet.name.clone(),
                        };
                        ListItem::new(snippet_list_line(idx, total, selected, &label))
                    })
                    .collect();
                let list = List::new(items);
                frame.render_stateful_widget(list, main[0], &mut self.browse.list);
            }
        }

        self.render_snippet_code(frame, main[1], self.selected_snippet());

        let help = if let Some(status) = &self.status {
            status.clone()
        } else {
            match self.nav_mode {
                NavigationMode::Fuzzy => "enter preview  ctrl+t browse  esc cancel".to_string(),
                NavigationMode::Browse => {
                    "tab complete  enter preview  ctrl+t search  esc cancel".to_string()
                }
            }
        };
        frame.render_widget(chrome_line(help, Modifier::DIM), chunks[2]);

        if matches!(self.nav_mode, NavigationMode::Fuzzy) {
            // "> " prefix is 2 columns; cursor_col() gives char-count offset into the query
            let x = chunks[0].x + 2 + self.fuzzy.cursor_col() as u16;
            frame.set_cursor_position(Position { x, y: chunks[0].y });
        }
    }

    fn render_preview(&self, frame: &mut Frame<'_>, snippet_id: &SnippetId) {
        let area = frame.area();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(10), Constraint::Length(1)])
            .split(area);
        let snippet = self.index.get(snippet_id);
        self.render_snippet_preview(frame, chunks[0], snippet);
        frame.render_widget(
            chrome_line("enter accept  backspace/esc return", Modifier::DIM),
            chunks[1],
        );
    }

    fn render_prompt(&self, frame: &mut Frame<'_>, prompt: &PromptState) {
        let area = frame.area();

        // Build preview first so we can measure its height for the layout
        let preview = self.prompt_preview_text(prompt);
        let cmd_height = (preview.lines.len() as u16).max(1);

        let (cmd_area, status_area, sugg_area, help_area) = if prompt.suggestions.is_empty() {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(cmd_height),
                    Constraint::Length(1),
                    Constraint::Length(1),
                ])
                .split(area);
            (chunks[0], chunks[1], None, chunks[2])
        } else {
            // cap suggestions so help always has a row
            let max_sugg = area.height.saturating_sub(cmd_height + 2);
            let sugg_height = (prompt.visible_suggestions().len() as u16)
                .min(max_sugg)
                .max(1);
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(cmd_height),
                    Constraint::Length(1),
                    Constraint::Length(sugg_height),
                    Constraint::Length(1),
                ])
                .split(area);
            (chunks[0], chunks[1], Some(chunks[2]), chunks[3])
        };
        frame.render_widget(
            Paragraph::new(preview).wrap(Wrap { trim: false }),
            cmd_area,
        );

        // Status bar
        let variable = prompt.current_variable();
        let label = prompt
            .error
            .as_deref()
            .unwrap_or(variable.name.as_str());
        frame.render_widget(
            Paragraph::new(Line::from(prompt_status_line(
                prompt.index + 1,
                prompt.variables.len(),
                label,
                status_area.width,
            ))),
            status_area,
        );

        // Suggestions list
        if let Some(area) = sugg_area {
            let visible = prompt.visible_suggestions();
            let total = visible.len();
            let selected = prompt.list.selected().unwrap_or(0);
            let items: Vec<ListItem<'_>> = visible
                .into_iter()
                .enumerate()
                .map(|(idx, value)| {
                    ListItem::new(snippet_list_line(idx, total, selected, value.as_str()))
                })
                .collect();
            let mut list_state = prompt.list.clone();
            frame.render_stateful_widget(List::new(items), area, &mut list_state);
        }

        // Cursor: walk the template to find where the active variable lands
        if let Some(snippet) = self.index.get(&prompt.snippet_id) {
            let mut values = prompt.values.clone();
            let value = prompt.current_value();
            if !value.is_empty() {
                values.insert(variable.name.clone(), value);
            }
            let (cursor_col, cursor_row) =
                cursor_in_template(snippet.body(), &values, &variable.name);
            frame.set_cursor_position(Position {
                x: cmd_area.x + cursor_col,
                y: cmd_area.y + cursor_row,
            });
        }

        frame.render_widget(
            chrome_line("tab next  shift+tab prev  enter accept  esc return", Modifier::DIM),
            help_area,
        );
    }

    fn prompt_preview_text(&self, prompt: &PromptState) -> Text<'static> {
        let Some(snippet) = self.index.get(&prompt.snippet_id) else {
            return Text::default();
        };

        let mut values = prompt.values.clone();
        let value = prompt.current_value();
        if !value.is_empty() {
            values.insert(prompt.current_variable().name.clone(), value);
        }

        render_command_text(
            snippet.body(),
            &values,
            Some(prompt.current_variable().name.as_str()),
        )
    }

    fn render_snippet_code(
        &self,
        frame: &mut Frame<'_>,
        area: ratatui::layout::Rect,
        snippet: Option<&IndexedSnippet>,
    ) {
        let text = if let Some(snippet) = snippet {
            highlight_shell(snippet.body())
        } else {
            Text::from("No snippet selected")
        };
        frame.render_widget(
            Paragraph::new(text)
                .block(Block::default().borders(Borders::LEFT))
                .wrap(Wrap { trim: false }),
            area,
        );
    }

    fn render_snippet_preview(
        &self,
        frame: &mut Frame<'_>,
        area: ratatui::layout::Rect,
        snippet: Option<&IndexedSnippet>,
    ) {
        let text = if let Some(snippet) = snippet {
            let mut lines = Vec::new();
            lines.push(Line::from(snippet.name().to_string()));
            lines.push(Line::from(format!(
                "path: {}",
                snippet.relative_path_display()
            )));
            if !snippet.frontmatter.tags.is_empty() {
                lines.push(Line::from(format!(
                    "tags: {}",
                    snippet.frontmatter.tags.join(", ")
                )));
            }
            if !snippet.description().trim().is_empty() {
                lines.push(Line::from(String::new()));
                lines.extend(
                    snippet
                        .description()
                        .lines()
                        .map(|line| Line::from(line.to_string())),
                );
            }
            lines.push(Line::from(String::new()));
            lines.push(Line::from("```"));
            lines.extend(
                snippet
                    .body()
                    .lines()
                    .map(|line| Line::from(line.to_string())),
            );
            lines.push(Line::from("```"));
            Text::from(lines)
        } else {
            Text::from("No snippet selected")
        };
        frame.render_widget(
            Paragraph::new(text)
                .block(block("Preview"))
                .wrap(Wrap { trim: false }),
            area,
        );
    }
}

enum PromptTransition {
    Stay,
    ToPreview(SnippetId),
    Completed(ExecutionOutcome),
}

fn handle_prompt_key<P: SuggestionProvider>(
    key: KeyEvent,
    prompt: &mut PromptState,
    provider: &P,
    cwd: &Path,
    index: &SnippetIndex,
    status: &mut Option<String>,
) -> PromptTransition {
    match key.code {
        KeyCode::Esc => {
            *status = None;
            PromptTransition::ToPreview(prompt.snippet_id.clone())
        }
        KeyCode::Backspace => {
            if prompt.input.pop().is_some() {
                prompt.reset_selection();
                PromptTransition::Stay
            } else if prompt.index > 0 {
                prompt.index -= 1;
                load_prompt_state(prompt, provider, cwd, status);
                PromptTransition::Stay
            } else {
                PromptTransition::ToPreview(prompt.snippet_id.clone())
            }
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            prompt.input.push(c);
            prompt.reset_selection();
            PromptTransition::Stay
        }
        KeyCode::Up => {
            prompt.move_cursor(-1);
            PromptTransition::Stay
        }
        KeyCode::Down => {
            prompt.move_cursor(1);
            PromptTransition::Stay
        }
        KeyCode::Tab => {
            cycle_prompt_variable(prompt, 1, provider, cwd, status);
            PromptTransition::Stay
        }
        KeyCode::BackTab => {
            cycle_prompt_variable(prompt, -1, provider, cwd, status);
            PromptTransition::Stay
        }
        KeyCode::Enter => {
            store_current_value(prompt);
            if prompt.index + 1 < prompt.variables.len() {
                prompt.index += 1;
                load_prompt_state(prompt, provider, cwd, status);
                PromptTransition::Stay
            } else if let Some(snippet) = index.get(&prompt.snippet_id) {
                PromptTransition::Completed(ExecutionOutcome {
                    snippet_id: prompt.snippet_id.clone(),
                    command: render_command(snippet.body(), &prompt.values),
                })
            } else {
                PromptTransition::Stay
            }
        }
        _ => PromptTransition::Stay,
    }
}

fn store_current_value(prompt: &mut PromptState) {
    let variable = prompt.current_variable().clone();
    prompt
        .values
        .insert(variable.name.clone(), prompt.current_value());
}

fn cycle_prompt_variable<P: SuggestionProvider>(
    prompt: &mut PromptState,
    delta: isize,
    provider: &P,
    cwd: &Path,
    status: &mut Option<String>,
) {
    store_current_value(prompt);
    if prompt.variables.len() <= 1 {
        return;
    }

    let len = prompt.variables.len() as isize;
    prompt.index = (prompt.index as isize + delta).rem_euclid(len) as usize;
    load_prompt_state(prompt, provider, cwd, status);
}

fn load_prompt_state<P: SuggestionProvider>(
    prompt: &mut PromptState,
    provider: &P,
    cwd: &Path,
    status: &mut Option<String>,
) {
    let variable = prompt.current_variable().clone();
    prompt.input = prompt
        .values
        .get(&variable.name)
        .cloned()
        .unwrap_or_else(|| default_input(&variable));
    prompt.error = None;
    prompt.suggestions = match provider.suggestions(&variable, cwd) {
        Ok(values) => values,
        Err(err) => {
            prompt.error = Some(err.to_string());
            Vec::new()
        }
    };
    if prompt.error.is_some() {
        *status = prompt.error.clone();
    } else {
        *status = None;
    }
    prompt.reset_selection();
}

pub fn render_command(template: &str, values: &BTreeMap<String, String>) -> String {
    let mut out = String::with_capacity(template.len());
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'<' && bytes[i + 1] == b'@' {
            let start = i + 2;
            if let Some(end_offset) = template[start..].find('>') {
                let end = start + end_offset;
                if let Some(name) = placeholder_name(&template[start..end])
                    && let Some(value) = values.get(name)
                {
                    out.push_str(value);
                    i = end + 1;
                    continue;
                }
            }
        }
        let ch = template[i..]
            .chars()
            .next()
            .expect("slice at valid char boundary");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn render_command_text(
    template: &str,
    values: &BTreeMap<String, String>,
    active_variable: Option<&str>,
) -> Text<'static> {
    let mut chunks = Vec::new();
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'<' && bytes[i + 1] == b'@' {
            let start = i + 2;
            if let Some(end_offset) = template[start..].find('>') {
                let end = start + end_offset;
                let placeholder = &template[i..=end];
                if let Some(name) = placeholder_name(&template[start..end]) {
                    if let Some(value) = values.get(name) {
                        let style = if Some(name) == active_variable {
                            active_prompt_style()
                        } else {
                            Style::default()
                        };
                        chunks.push(StyledChunk::new(value.clone(), style));
                        i = end + 1;
                        continue;
                    }

                    let style = if Some(name) == active_variable {
                        active_prompt_style()
                    } else {
                        placeholder_prompt_style()
                    };
                    chunks.push(StyledChunk::new(placeholder.to_string(), style));
                    i = end + 1;
                    continue;
                }
            }
        }

        let ch = template[i..]
            .chars()
            .next()
            .expect("slice at valid char boundary");
        chunks.push(StyledChunk::plain(ch.to_string()));
        i += ch.len_utf8();
    }

    styled_text(chunks)
}

#[derive(Debug, Clone)]
struct StyledChunk {
    text: String,
    style: Style,
}

impl StyledChunk {
    fn plain(text: String) -> Self {
        Self {
            text,
            style: Style::default(),
        }
    }

    fn new(text: String, style: Style) -> Self {
        Self { text, style }
    }
}

fn styled_text(chunks: Vec<StyledChunk>) -> Text<'static> {
    let mut lines: Vec<Line<'static>> = vec![Line::default()];
    for chunk in chunks {
        let mut parts = chunk.text.split('\n').peekable();
        while let Some(part) = parts.next() {
            if !part.is_empty() {
                lines
                    .last_mut()
                    .expect("text always has at least one line")
                    .spans
                    .push(Span::styled(part.to_string(), chunk.style));
            }
            if parts.peek().is_some() {
                lines.push(Line::default());
            }
        }
    }
    Text::from(lines)
}

fn active_prompt_style() -> Style {
    Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED)
}

fn placeholder_prompt_style() -> Style {
    Style::default().add_modifier(Modifier::DIM)
}

pub fn execute_default() -> io::Result<Option<ExecutionOutcome>> {
    let paths = config::default_paths();
    let index = crate::index::load_from_roots(&paths.snippet_roots)?;
    let frecency = FrecencyStore::load(&paths.state_file)?;
    let options = ExecuteOptions::default();
    run_execute(index, frecency, options)
}

pub fn run_execute(
    index: SnippetIndex,
    frecency: FrecencyStore,
    options: ExecuteOptions,
) -> io::Result<Option<ExecutionOutcome>> {
    let provider = SystemSuggestionProvider;
    run_execute_with_provider(index, frecency, options, provider)
}

pub fn run_execute_with_provider<P: SuggestionProvider>(
    index: SnippetIndex,
    frecency: FrecencyStore,
    options: ExecuteOptions,
    provider: P,
) -> io::Result<Option<ExecutionOutcome>> {
    let mut app = ExecutionApp::new(index, frecency, options.cwd, options.now, provider);
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

fn compact_viewport_height(rows: u16, max_height: u16) -> u16 {
    let compact = (rows / 3).max(6);
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

fn block(title: &str) -> Block<'_> {
    Block::default().borders(Borders::ALL).title(title)
}

fn highlight_shell(body: &str) -> Text<'static> {
    Text::from(body.lines().map(highlight_shell_line).collect::<Vec<_>>())
}

fn highlight_shell_line(line: &str) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut i = 0;
    let mut plain_start = 0;

    while i < line.len() {
        if let Some((len, span)) = try_highlight_match(line, i) {
            if plain_start < i {
                spans.push(Span::raw(line[plain_start..i].to_string()));
            }
            spans.push(span);
            i += len;
            plain_start = i;
        } else {
            i += line[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
        }
    }
    if plain_start < line.len() {
        spans.push(Span::raw(line[plain_start..].to_string()));
    }
    Line::from(spans)
}

fn try_highlight_match(line: &str, i: usize) -> Option<(usize, Span<'static>)> {
    let rest = &line[i..];
    let at_word_start = i == 0
        || line.as_bytes()[i - 1]
            .is_ascii_whitespace()
            .then_some(true)
            .unwrap_or_else(|| {
                let pb = line.as_bytes()[i - 1];
                !pb.is_ascii_alphanumeric() && pb != b'_'
            });

    // <@custom var> — the tool's own variable syntax
    if rest.starts_with("<@") {
        if let Some(end) = rest.find('>') {
            let token = &rest[..end + 1];
            return Some((
                token.len(),
                Span::styled(
                    token.to_string(),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
            ));
        }
    }

    // comment
    if rest.starts_with('#') {
        return Some((
            rest.len(),
            Span::styled(rest.to_string(), Style::default().fg(Color::DarkGray)),
        ));
    }

    // double-quoted string
    if rest.starts_with('"') {
        let end = rest[1..].find('"').map(|e| e + 2).unwrap_or(rest.len());
        return Some((
            end,
            Span::styled(rest[..end].to_string(), Style::default().fg(Color::Green)),
        ));
    }

    // single-quoted string
    if rest.starts_with('\'') {
        let end = rest[1..].find('\'').map(|e| e + 2).unwrap_or(rest.len());
        return Some((
            end,
            Span::styled(rest[..end].to_string(), Style::default().fg(Color::Green)),
        ));
    }

    // $var or ${var}
    if rest.starts_with('$') {
        let end = if rest.starts_with("${") {
            rest.find('}').map(|e| e + 1).unwrap_or(rest.len())
        } else {
            let inner = &rest[1..];
            let e = inner
                .find(|c: char| !c.is_alphanumeric() && c != '_')
                .unwrap_or(inner.len());
            if e == 0 {
                return None;
            }
            e + 1
        };
        return Some((
            end,
            Span::styled(rest[..end].to_string(), Style::default().fg(Color::Cyan)),
        ));
    }

    // flags: --flag or -f at word boundaries
    if at_word_start && rest.starts_with('-') && rest.as_bytes().get(1).copied() != Some(b' ') {
        let end = rest
            .find(|c: char| c == ' ' || c == '=' || c == '\'' || c == '"' || c == ';')
            .unwrap_or(rest.len());
        if end > 1 {
            return Some((
                end,
                Span::styled(rest[..end].to_string(), Style::default().fg(Color::Blue)),
            ));
        }
    }

    // shell keywords at word boundaries
    if at_word_start {
        const KEYWORDS: &[&str] = &[
            "if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case",
            "esac", "in", "function", "return", "local", "export", "source", "readonly", "declare",
            "unset",
        ];
        for kw in KEYWORDS {
            if rest.starts_with(kw) {
                let after = kw.len();
                let end_ok = after == rest.len()
                    || rest.as_bytes()[after]
                        .is_ascii_whitespace()
                        .then_some(true)
                        .unwrap_or_else(|| {
                            let b = rest.as_bytes()[after];
                            !b.is_ascii_alphanumeric() && b != b'_'
                        });
                if end_ok {
                    return Some((
                        after,
                        Span::styled(kw.to_string(), Style::default().fg(Color::Magenta)),
                    ));
                }
            }
        }
    }

    None
}

fn chrome_line<'a, T: Into<Text<'a>>>(text: T, modifier: Modifier) -> Paragraph<'a> {
    Paragraph::new(text)
        .style(Style::default().add_modifier(modifier))
        .wrap(Wrap { trim: true })
}

fn select_header(prompt: &str, stats: &str, mode: &str, width: u16) -> Paragraph<'static> {
    let prompt_line = Line::from(vec![
        Span::raw("> "),
        Span::styled(
            prompt.to_string(),
            Style::default().add_modifier(Modifier::BOLD),
        ),
    ]);
    let prefix = format!("[{stats}] ─ ");
    let mode_label = format!("[ {mode} ]");
    let used = prefix.chars().count() + mode_label.chars().count() + 1;
    let right = "─".repeat((width as usize).saturating_sub(used));
    let status_line = Line::from(vec![
        Span::raw(prefix),
        Span::styled(mode_label, Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(format!(" {right}")),
    ]);
    Paragraph::new(Text::from(vec![prompt_line, status_line]))
}

fn prompt_status_line(idx: usize, total: usize, label: &str, width: u16) -> Vec<Span<'static>> {
    let prefix = format!("[{idx}/{total}] ─ ");
    let mode = format!("[ {label} ]");
    let used = prefix.chars().count() + mode.chars().count() + 1;
    let right = "─".repeat((width as usize).saturating_sub(used));
    vec![
        Span::raw(prefix),
        Span::styled(mode, Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(format!(" {right}")),
    ]
}

/// Walk `template`, rendering variables from `values`, and return the
/// (col, row) position within the rendered output where `active_variable` ends.
fn cursor_in_template(
    template: &str,
    values: &BTreeMap<String, String>,
    active_variable: &str,
) -> (u16, u16) {
    let mut col: u16 = 0;
    let mut row: u16 = 0;
    let mut i = 0;
    let bytes = template.as_bytes();

    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'<' && bytes[i + 1] == b'@' {
            let start = i + 2;
            if let Some(end_offset) = template[start..].find('>') {
                let end = start + end_offset;
                if let Some(name) = placeholder_name(&template[start..end]) {
                    if name == active_variable {
                        if let Some(val) = values.get(name) {
                            advance_cursor(&mut col, &mut row, val);
                        }
                        return (col, row);
                    }
                    let rendered = values
                        .get(name)
                        .cloned()
                        .unwrap_or_else(|| template[i..=end].to_string());
                    advance_cursor(&mut col, &mut row, &rendered);
                    i = end + 1;
                    continue;
                }
            }
        }
        let ch = template[i..].chars().next().unwrap();
        if ch == '\n' {
            row += 1;
            col = 0;
        } else {
            col += 1;
        }
        i += ch.len_utf8();
    }
    (col, row)
}

fn advance_cursor(col: &mut u16, row: &mut u16, text: &str) {
    for ch in text.chars() {
        if ch == '\n' {
            *row += 1;
            *col = 0;
        } else {
            *col += 1;
        }
    }
}

fn snippet_list_line<'a>(idx: usize, total: usize, selected: usize, content: &str) -> Line<'a> {
    let w = digits(total);
    if idx == selected {
        Line::from(vec![
            Span::styled("▌ ", Style::default().fg(Color::Red).bg(Color::DarkGray)),
            Span::styled(
                format!("{:>w$}  {content}", idx + 1),
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            ),
        ])
    } else {
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                format!("{:>w$}  ", idx + 1),
                Style::default().add_modifier(Modifier::DIM),
            ),
            Span::raw(content.to_string()),
        ])
    }
}

fn digits(n: usize) -> usize {
    if n == 0 { 1 } else { n.ilog10() as usize + 1 }
}

fn unique_variables(variables: &[Variable]) -> Vec<Variable> {
    let mut seen = HashMap::new();
    let mut out = Vec::new();
    for variable in variables {
        if seen.insert(variable.name.clone(), ()).is_none() {
            out.push(variable.clone());
        }
    }
    out
}

fn default_input(variable: &Variable) -> String {
    match &variable.source {
        VariableSource::Default(value) => value.clone(),
        _ => String::new(),
    }
}

fn placeholder_name(inner: &str) -> Option<&str> {
    let name = inner
        .split_once(':')
        .map(|(name, _)| name)
        .unwrap_or(inner)
        .trim();
    if name.is_empty()
        || !name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        return None;
    }
    Some(name)
}


fn builtin_suggestions(name: &str, cwd: &Path) -> io::Result<Vec<String>> {
    match name {
        "file" => read_dir_entries(cwd, true),
        "directory" => read_dir_entries(cwd, false),
        _ => Ok(Vec::new()),
    }
}

fn read_dir_entries(cwd: &Path, want_files: bool) -> io::Result<Vec<String>> {
    let mut out = Vec::new();
    for entry in fs::read_dir(cwd)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let include = if want_files {
            file_type.is_file()
        } else {
            file_type.is_dir()
        };
        if include {
            out.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    out.sort();
    Ok(out)
}

fn command_suggestions(command: &str, cwd: &Path) -> io::Result<Vec<String>> {
    let output = Command::new("bash")
        .arg("-lc")
        .arg(command)
        .current_dir(cwd)
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let message = if stderr.is_empty() {
            format!("suggestion command failed: {command}")
        } else {
            format!("suggestion command failed: {stderr}")
        };
        return Err(io::Error::other(message));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut values = Vec::new();
    for line in stdout.lines() {
        values.extend(
            line.split("\\n")
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_string),
        );
    }
    Ok(values)
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Frontmatter, Snippet, SnippetFile};
    use std::cell::RefCell;

    #[derive(Default)]
    struct TestProvider {
        values: RefCell<HashMap<String, Vec<String>>>,
    }

    impl TestProvider {
        fn with(self, name: &str, values: &[&str]) -> Self {
            self.values.borrow_mut().insert(
                name.to_string(),
                values.iter().map(|value| value.to_string()).collect(),
            );
            self
        }
    }

    impl SuggestionProvider for TestProvider {
        fn suggestions(&self, variable: &Variable, _cwd: &Path) -> io::Result<Vec<String>> {
            Ok(self
                .values
                .borrow()
                .get(&variable.name)
                .cloned()
                .unwrap_or_default())
        }
    }

    fn snippet_file(rel: &str, name: &str, body: &str, variables: Vec<Variable>) -> SnippetFile {
        SnippetFile {
            path: PathBuf::from(rel),
            relative_path: PathBuf::from(rel),
            frontmatter: Frontmatter::default(),
            snippets: vec![Snippet {
                id: SnippetId::new(rel, "slug"),
                name: name.to_string(),
                description: "desc".to_string(),
                body: body.to_string(),
                variables,
            }],
        }
    }

    fn app_with_body(
        body: &str,
        variables: Vec<Variable>,
        provider: TestProvider,
    ) -> ExecutionApp<TestProvider> {
        let index = SnippetIndex::from_files([snippet_file("x.md", "Demo", body, variables)]);
        let frecency = FrecencyStore::new();
        ExecutionApp::new(index, frecency, PathBuf::from("."), 0, provider)
    }

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn completed(event: AppEvent) -> ExecutionOutcome {
        match event {
            AppEvent::Completed(outcome) => outcome,
            AppEvent::Continue => panic!("expected completed event, got continue"),
            AppEvent::Cancelled => panic!("expected completed event, got cancelled"),
        }
    }

    fn line_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn render_command_replaces_each_placeholder_form() {
        let mut values = BTreeMap::new();
        values.insert("file".to_string(), "Cargo.toml".to_string());
        values.insert("pattern".to_string(), "needle".to_string());
        values.insert("method".to_string(), "POST".to_string());
        let rendered = render_command(
            "cat <@file> | grep <@pattern:?hi> && curl -X <@method:echo GET>",
            &values,
        );
        assert_eq!(rendered, "cat Cargo.toml | grep needle && curl -X POST");
    }

    #[test]
    fn render_command_keeps_unresolved_placeholders() {
        let values = BTreeMap::new();
        let rendered = render_command("echo <@missing>", &values);
        assert_eq!(rendered, "echo <@missing>");
    }

    #[test]
    fn render_command_text_highlights_active_value() {
        let mut values = BTreeMap::new();
        values.insert("file".to_string(), "Cargo.toml".to_string());
        let rendered = render_command_text("cat <@file>", &values, Some("file"));
        assert_eq!(line_text(&rendered.lines[0]), "cat Cargo.toml");
        assert_eq!(rendered.lines[0].spans[4].style, active_prompt_style());
    }

    #[test]
    fn render_command_text_highlights_active_placeholder_and_dims_others() {
        let values = BTreeMap::new();
        let rendered = render_command_text("echo <@missing> <@later>", &values, Some("missing"));
        assert_eq!(line_text(&rendered.lines[0]), "echo <@missing> <@later>");
        assert_eq!(rendered.lines[0].spans[5].style, active_prompt_style());
        assert_eq!(rendered.lines[0].spans[7].style, placeholder_prompt_style());
    }

    #[test]
    fn compact_viewport_height_stays_small_but_respects_limits() {
        assert_eq!(compact_viewport_height(60, 12), 12);
        assert_eq!(compact_viewport_height(24, 12), 8);
        assert_eq!(compact_viewport_height(9, 12), 6);
        assert_eq!(compact_viewport_height(60, 4), 4);
    }

    #[test]
    fn unique_variables_prompt_only_once_for_duplicate_names() {
        let variables = vec![
            Variable {
                name: "file".to_string(),
                source: VariableSource::Free,
            },
            Variable {
                name: "file".to_string(),
                source: VariableSource::Free,
            },
        ];
        let uniq = unique_variables(&variables);
        assert_eq!(uniq.len(), 1);
        assert_eq!(uniq[0].name, "file");
    }

    #[test]
    fn built_in_file_and_directory_sources_list_cwd_entries() {
        let dir = std::env::temp_dir().join(format!("pb-execute-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("subdir")).unwrap();
        fs::write(dir.join("alpha.txt"), "hi").unwrap();
        fs::write(dir.join("beta.txt"), "hi").unwrap();

        let files = builtin_suggestions("file", &dir).unwrap();
        let dirs = builtin_suggestions("directory", &dir).unwrap();
        assert_eq!(files, vec!["alpha.txt".to_string(), "beta.txt".to_string()]);
        assert_eq!(dirs, vec!["subdir".to_string()]);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn command_suggestions_split_literal_backslash_n_sequences() {
        let dir = Path::new(".");
        let values = command_suggestions("printf 'GET\\\\nPOST\\\\nPUT'", dir).unwrap();
        assert_eq!(
            values,
            vec!["GET".to_string(), "POST".to_string(), "PUT".to_string()]
        );
    }

    #[test]
    fn search_selection_survives_preview_round_trip() {
        let variables = vec![];
        let mut app = app_with_body("echo hi", variables, TestProvider::default());
        app.fuzzy.set_query("Demo");
        let _ = app.handle_key(press(KeyCode::Enter));
        assert!(matches!(app.screen, Screen::Preview { .. }));
        let _ = app.handle_key(press(KeyCode::Backspace));
        assert!(matches!(app.screen, Screen::Select));
        assert_eq!(app.fuzzy.query, "Demo");
    }

    #[test]
    fn ctrl_t_toggles_between_search_and_browse() {
        let mut app = app_with_body("echo hi", vec![], TestProvider::default());
        let _ = app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL));
        assert_eq!(app.navigation_mode(), NavigationMode::Browse);
        let _ = app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL));
        assert_eq!(app.navigation_mode(), NavigationMode::Fuzzy);
    }

    #[test]
    fn variable_flow_accepts_default_and_emits_rendered_command() {
        let variables = vec![Variable {
            name: "target".to_string(),
            source: VariableSource::Default("world".to_string()),
        }];
        let mut app = app_with_body(
            "echo hello <@target:?world>",
            variables,
            TestProvider::default(),
        );
        let _ = app.handle_key(press(KeyCode::Enter));
        let _ = app.handle_key(press(KeyCode::Enter));
        let outcome = completed(app.handle_key(press(KeyCode::Enter)));
        assert_eq!(outcome.command, "echo hello world");
    }

    #[test]
    fn variable_flow_accepts_default_suggestion() {
        let variables = vec![Variable {
            name: "method".to_string(),
            source: VariableSource::Command("ignored".to_string()),
        }];
        let provider = TestProvider::default().with("method", &["GET", "POST"]);
        let mut app = app_with_body("curl -X <@method:ignored>", variables, provider);
        let _ = app.handle_key(press(KeyCode::Enter));
        let _ = app.handle_key(press(KeyCode::Enter));
        let outcome = completed(app.handle_key(press(KeyCode::Enter)));
        assert_eq!(outcome.command, "curl -X GET");
    }

    #[test]
    fn prompt_tab_cycles_forward_between_variables() {
        let variables = vec![
            Variable {
                name: "one".to_string(),
                source: VariableSource::Free,
            },
            Variable {
                name: "two".to_string(),
                source: VariableSource::Free,
            },
        ];
        let mut app = app_with_body("echo <@one> <@two>", variables, TestProvider::default());
        let _ = app.handle_key(press(KeyCode::Enter));
        let _ = app.handle_key(press(KeyCode::Enter));
        let _ = app.handle_key(press(KeyCode::Char('a')));
        let _ = app.handle_key(press(KeyCode::Tab));

        let Screen::Prompt(prompt) = &app.screen else {
            panic!("expected prompt");
        };
        assert_eq!(prompt.current_variable().name, "two");
        assert_eq!(prompt.values.get("one").map(String::as_str), Some("a"));

        let _ = app.handle_key(press(KeyCode::Tab));
        let Screen::Prompt(prompt) = &app.screen else {
            panic!("expected prompt");
        };
        assert_eq!(prompt.current_variable().name, "one");
        assert_eq!(prompt.input, "a");
    }

    #[test]
    fn prompt_shift_tab_cycles_backward_between_variables() {
        let variables = vec![
            Variable {
                name: "one".to_string(),
                source: VariableSource::Free,
            },
            Variable {
                name: "two".to_string(),
                source: VariableSource::Free,
            },
        ];
        let mut app = app_with_body("echo <@one> <@two>", variables, TestProvider::default());
        let _ = app.handle_key(press(KeyCode::Enter));
        let _ = app.handle_key(press(KeyCode::Enter));
        let _ = app.handle_key(press(KeyCode::BackTab));

        let Screen::Prompt(prompt) = &app.screen else {
            panic!("expected prompt");
        };
        assert_eq!(prompt.current_variable().name, "two");
    }

    #[test]
    fn prompt_backspace_walks_to_previous_variable() {
        let variables = vec![
            Variable {
                name: "one".to_string(),
                source: VariableSource::Free,
            },
            Variable {
                name: "two".to_string(),
                source: VariableSource::Free,
            },
        ];
        let mut app = app_with_body("echo <@one> <@two>", variables, TestProvider::default());
        let _ = app.handle_key(press(KeyCode::Enter));
        let _ = app.handle_key(press(KeyCode::Enter));
        let _ = app.handle_key(press(KeyCode::Char('a')));
        let _ = app.handle_key(press(KeyCode::Enter));
        let _ = app.handle_key(press(KeyCode::Backspace));
        let Screen::Prompt(prompt) = &app.screen else {
            panic!("expected prompt");
        };
        assert_eq!(prompt.current_variable().name, "one");
        assert_eq!(prompt.input, "a");
    }
}
