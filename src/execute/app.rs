use crate::browse::{BrowseEntry, BrowseState, BrowseTree};
use crate::command_template;
use crate::config::{SearchConfig, SuggestionCommandsConfig, Theme, VariableInputConfig};
use crate::domain::{SnippetId, Variable, VariableSource, VariableSpec};
use crate::frecency::FrecencyStore;
use crate::fuzzy::FuzzyState;
use crate::index::{IndexedSnippet, SnippetIndex, TagKey};
use crate::search;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::widgets::ListState;
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use super::prompt::{
    PromptState, PromptTransition, handle_prompt_key, load_prompt_state, unique_variables,
};
use super::{ExecutionOutcome, render_command};

/// Navigation mode currently active in the select screen.
///
/// `Ctrl+T` rotates through fuzzy search, browse, and tags mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavigationMode {
    /// Fuzzy search: typing filters and ranks snippets by query.
    Fuzzy,
    /// Directory tree browser: navigate by folder hierarchy with tab-completion.
    Browse,
    /// Tag browser: filter tags, then drill into snippets for a selected tag.
    Tags,
}

/// State for the tag-based picker view.
///
/// The tag view keeps its own filter buffer so it does not inherit fuzzy-search
/// ranking or cursor behavior. `drill` records whether the user is looking at
/// the tag list or the snippets for one tag.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TagsState {
    /// Case-sensitive tag-list filter.
    pub filter: String,
    /// Cursor position inside `filter`.
    pub cursor: usize,
    /// Highlighted row in the tag-list phase.
    pub list_selection: Option<usize>,
    /// Active drilled tag, or `None` while showing the tag list.
    pub drill: Option<TagKey>,
    /// Case-insensitive snippet-name filter while drilled into one tag.
    pub drill_filter: String,
    /// Cursor position inside `drill_filter`.
    pub drill_cursor: usize,
    /// Highlighted row in the drilled snippet-list phase.
    pub drill_selection: Option<usize>,
}

impl TagsState {
    pub fn new() -> Self {
        Self {
            filter: String::new(),
            cursor: 0,
            list_selection: Some(0),
            drill: None,
            drill_filter: String::new(),
            drill_cursor: 0,
            drill_selection: Some(0),
        }
    }

    pub fn type_char(&mut self, c: char) {
        self.filter.insert(self.cursor, c);
        self.cursor += c.len_utf8();
        self.list_selection = Some(0);
    }

    pub fn backspace(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        let prev = self.filter[..self.cursor]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
        self.filter.remove(prev);
        self.cursor = prev;
        self.list_selection = Some(0);
        true
    }

    pub fn cursor_left(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.cursor = self.filter[..self.cursor]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
    }

    pub fn cursor_right(&mut self) {
        if self.cursor >= self.filter.len() {
            return;
        }
        let c = self.filter[self.cursor..].chars().next().unwrap();
        self.cursor += c.len_utf8();
    }

    pub fn move_cursor(&mut self, delta: i32, visible_len: usize) {
        if visible_len == 0 {
            self.list_selection = None;
            return;
        }
        let current = self.list_selection.unwrap_or(0) as i32;
        let next = (current + delta).clamp(0, visible_len as i32 - 1);
        self.list_selection = Some(next as usize);
    }

    pub fn type_drill_char(&mut self, c: char) {
        self.drill_filter.insert(self.drill_cursor, c);
        self.drill_cursor += c.len_utf8();
        self.drill_selection = Some(0);
    }

    pub fn drill_backspace(&mut self) -> bool {
        if self.drill_cursor == 0 {
            return false;
        }
        let prev = self.drill_filter[..self.drill_cursor]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
        self.drill_filter.remove(prev);
        self.drill_cursor = prev;
        self.drill_selection = Some(0);
        true
    }

    pub fn drill_cursor_left(&mut self) {
        if self.drill_cursor == 0 {
            return;
        }
        self.drill_cursor = self.drill_filter[..self.drill_cursor]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
    }

    pub fn drill_cursor_right(&mut self) {
        if self.drill_cursor >= self.drill_filter.len() {
            return;
        }
        let c = self.drill_filter[self.drill_cursor..]
            .chars()
            .next()
            .unwrap();
        self.drill_cursor += c.len_utf8();
    }

    /// Display-column offset of the cursor within the tag filter.
    pub fn cursor_col(&self) -> usize {
        self.filter[..self.cursor].chars().count()
    }

    /// Display-column offset of the cursor within the drilled snippet filter.
    pub fn drill_cursor_col(&self) -> usize {
        self.drill_filter[..self.drill_cursor].chars().count()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TagListEntry {
    pub key: TagKey,
    pub label: String,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TagSnippetEntry {
    pub id: SnippetId,
    pub name: String,
}

/// Pluggable backend that supplies suggestions and defaults for a variable.
///
/// The production implementation is [`SystemSuggestionProvider`]. Tests inject
/// a `TestProvider` that returns hard-coded values without touching the file
/// system or running shell commands.
pub trait SuggestionProvider {
    /// Return the candidate values to show in the suggestion list for `variable`
    /// when the user is in the given `cwd`. Returns an empty vec (not an error)
    /// when there are no suggestions.
    ///
    /// `confirmed` carries previously-confirmed variable values keyed by name,
    /// used to substitute `<#name>` / `<#name:raw>` dependent references inside
    /// the suggestion command before it is executed.
    fn suggestions(
        &self,
        variable: &Variable,
        cwd: &Path,
        local_variables: &std::collections::BTreeMap<String, VariableSpec>,
        confirmed: &BTreeMap<String, String>,
    ) -> io::Result<Vec<String>>;
    /// Return the value to pre-populate the input box with, if any.
    fn default_input(
        &self,
        variable: &Variable,
        local_variables: &std::collections::BTreeMap<String, VariableSpec>,
        confirmed: &BTreeMap<String, String>,
    ) -> Option<String>;
    /// Return the raw suggestion-command source for `variable`, if any, by
    /// inspecting inline source, file-local spec, and config overrides in that
    /// order. The string is the *unrendered* template — `<#...>` references
    /// are not yet substituted. Returns `None` for variables that do not run
    /// a shell command (free-form, default-only, builtins, static suggestion
    /// lists).
    fn command_source(
        &self,
        variable: &Variable,
        local_variables: &std::collections::BTreeMap<String, VariableSpec>,
    ) -> Option<String>;
}

/// Production [`SuggestionProvider`] backed by config overrides and built-in
/// sources (`file`, `directory`) and shell commands from `VariableSource`.
#[derive(Debug, Default, Clone)]
pub struct SystemSuggestionProvider {
    variable_inputs: std::collections::BTreeMap<String, VariableInputConfig>,
    suggestion_commands: SuggestionCommandsConfig,
}

impl SystemSuggestionProvider {
    pub fn new(
        variable_inputs: std::collections::BTreeMap<String, VariableInputConfig>,
        suggestion_commands: SuggestionCommandsConfig,
    ) -> Self {
        Self {
            variable_inputs,
            suggestion_commands,
        }
    }
}

impl SuggestionProvider for SystemSuggestionProvider {
    fn suggestions(
        &self,
        variable: &Variable,
        cwd: &Path,
        local_variables: &std::collections::BTreeMap<String, VariableSpec>,
        confirmed: &BTreeMap<String, String>,
    ) -> io::Result<Vec<String>> {
        let timeout_ms = self.suggestion_commands.timeout_ms;
        let run = |command: &str| -> io::Result<Vec<String>> {
            if !self.suggestion_commands.allow_commands {
                return Ok(Vec::new());
            }
            let rendered = render_template_for_exec(command, confirmed)?;
            super::prompt::command_suggestions(&rendered, cwd, timeout_ms)
        };
        match &variable.source {
            VariableSource::Command(cmd) => run(cmd),
            VariableSource::Default(_) => Ok(Vec::new()),
            VariableSource::Free => {
                if let Some(config) = local_variables.get(&variable.name) {
                    if !config.suggestions.is_empty() {
                        return Ok(config.suggestions.clone());
                    }
                    if let Some(command) = &config.command {
                        return run(command);
                    }
                }
                if let Some(config) = self.variable_inputs.get(&variable.name) {
                    if !config.suggestions.is_empty() {
                        return Ok(config.suggestions.clone());
                    }
                    if let Some(command) = &config.command {
                        return run(command);
                    }
                }
                super::prompt::builtin_suggestions(&variable.name, cwd)
            }
        }
    }

    fn command_source(
        &self,
        variable: &Variable,
        local_variables: &std::collections::BTreeMap<String, VariableSpec>,
    ) -> Option<String> {
        match &variable.source {
            VariableSource::Command(cmd) => Some(cmd.clone()),
            VariableSource::Default(_) => None,
            VariableSource::Free => {
                if let Some(config) = local_variables.get(&variable.name) {
                    if !config.suggestions.is_empty() {
                        return None;
                    }
                    if let Some(command) = &config.command {
                        return Some(command.clone());
                    }
                }
                if let Some(config) = self.variable_inputs.get(&variable.name) {
                    if !config.suggestions.is_empty() {
                        return None;
                    }
                    if let Some(command) = &config.command {
                        return Some(command.clone());
                    }
                }
                None
            }
        }
    }

    fn default_input(
        &self,
        variable: &Variable,
        local_variables: &std::collections::BTreeMap<String, VariableSpec>,
        confirmed: &BTreeMap<String, String>,
    ) -> Option<String> {
        match &variable.source {
            VariableSource::Default(template) => command_template::render(template, confirmed).ok(),
            VariableSource::Command(_) => None,
            VariableSource::Free => local_variables
                .get(&variable.name)
                .and_then(|config| config.default.clone())
                .or_else(|| {
                    self.variable_inputs
                        .get(&variable.name)
                        .and_then(|config| config.default.clone())
                }),
        }
    }
}

#[derive(Debug)]
/// The top-level TUI screens.
pub(crate) enum Screen {
    /// The snippet picker (fuzzy search, browse tree, or tags view).
    Select,
    /// Variable-filling dialog for the snippet identified by `PromptState`.
    ///
    /// `PromptState` is boxed because it is significantly larger than the
    /// other (zero-sized) variant of this enum.
    Prompt(Box<PromptState>),
}

/// Root application state for the interactive TUI session.
///
/// `P` is the [`SuggestionProvider`] — production code uses
/// [`SystemSuggestionProvider`]; tests inject a mock.
pub struct ExecutionApp<P = SystemSuggestionProvider> {
    pub(crate) index: SnippetIndex,
    pub(crate) frecency: FrecencyStore,
    pub(crate) tree: BrowseTree,
    pub(crate) tag_index: BTreeMap<TagKey, Vec<SnippetId>>,
    pub(crate) cwd: PathBuf,
    pub(crate) now: u64,
    pub(crate) provider: P,
    pub(crate) screen: Screen,
    pub(crate) nav_mode: NavigationMode,
    pub fuzzy: FuzzyState,
    pub browse: BrowseState,
    pub tags: TagsState,
    pub(crate) status: Option<String>,
    pub(crate) preview_scroll: u16,
    pub(crate) fuzzy_list: ListState,
    pub(crate) browse_list: ListState,
    pub(crate) tags_list: ListState,
    pub(crate) search_config: SearchConfig,
    pub(crate) theme: Theme,
}

/// Outcome returned by [`ExecutionApp::handle_key`] after processing one key event.
pub enum AppEvent {
    /// The event was handled; keep running the event loop.
    Continue,
    /// The user requested to edit the currently selected snippet.
    EditSnippet(SnippetId),
    /// The user pressed Esc or Ctrl+C; the TUI should exit without a result.
    Cancelled,
    /// The user confirmed a fully filled-in snippet; the TUI should exit with
    /// this outcome.
    Completed(ExecutionOutcome),
}

impl<P: SuggestionProvider> ExecutionApp<P> {
    pub fn new(
        index: SnippetIndex,
        frecency: FrecencyStore,
        cwd: PathBuf,
        now: u64,
        search_config: SearchConfig,
        theme: Theme,
        provider: P,
    ) -> Self {
        Self {
            tree: BrowseTree::from_index(&index),
            tag_index: index.tag_index(),
            index,
            frecency,
            cwd,
            now,
            provider,
            screen: Screen::Select,
            nav_mode: NavigationMode::Fuzzy,
            fuzzy: FuzzyState::new(),
            browse: BrowseState::new(),
            tags: TagsState::new(),
            status: None,
            preview_scroll: 0,
            fuzzy_list: ListState::default(),
            browse_list: ListState::default(),
            tags_list: ListState::default(),
            search_config,
            theme,
        }
    }

    pub fn navigation_mode(&self) -> NavigationMode {
        self.nav_mode
    }

    pub fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }

    /// Return the snippet currently highlighted in either the select or prompt
    /// screen, so the renderer can show its preview.
    pub fn selected_snippet(&self) -> Option<&IndexedSnippet> {
        match &self.screen {
            Screen::Select => match self.nav_mode {
                NavigationMode::Fuzzy => self.selected_fuzzy_snippet(),
                NavigationMode::Browse => self.selected_browse_snippet(),
                NavigationMode::Tags => self.selected_tag_snippet(),
            },
            Screen::Prompt(prompt) => self.index.get(&prompt.snippet_id),
        }
    }

    /// Return the best guess at the command the user will emit, for live preview.
    ///
    /// During prompting, already-filled values are substituted; the current
    /// input is included if non-empty. On the select screen, the raw snippet
    /// body is returned verbatim (placeholders not yet filled).
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

    /// Replace the snippet index after an on-disk edit, rebuilding derived
    /// picker state while preserving the current navigation mode and query.
    ///
    /// Returns `true` when `preferred_id` still exists in the refreshed index.
    pub(crate) fn replace_index(
        &mut self,
        index: SnippetIndex,
        preferred_id: Option<&SnippetId>,
    ) -> bool {
        self.tree = BrowseTree::from_index(&index);
        self.tag_index = index.tag_index();
        if self
            .tags
            .drill
            .as_ref()
            .is_some_and(|tag| !self.tag_index.contains_key(tag))
        {
            self.tags.drill = None;
            self.tags.drill_selection = Some(0);
        }
        self.index = index;
        self.screen = Screen::Select;

        let preferred_found = preferred_id
            .and_then(|id| self.index.get(id))
            .map(|_| true)
            .unwrap_or(false);
        match self.nav_mode {
            NavigationMode::Fuzzy => self.restore_fuzzy_selection(preferred_id),
            NavigationMode::Browse => self.restore_browse_selection(preferred_id),
            NavigationMode::Tags => self.restore_tags_selection(),
        }
        preferred_found
    }

    /// Dispatch a crossterm key event to the appropriate screen handler and
    /// return the resulting [`AppEvent`].
    ///
    /// Key-release events are ignored (relevant for the kitty keyboard protocol).
    /// Ctrl+C is handled globally here as an unconditional cancel; Esc is
    /// screen-specific (it backs out of the prompt to select, or cancels from
    /// select).
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
            Screen::Prompt(prompt) => match handle_prompt_key(
                key,
                prompt,
                &self.provider,
                &self.cwd,
                &self.index,
                &mut self.status,
            ) {
                PromptTransition::Stay => AppEvent::Continue,
                PromptTransition::ToSelect => {
                    self.screen = Screen::Select;
                    AppEvent::Continue
                }
                PromptTransition::Completed(outcome) => AppEvent::Completed(outcome),
            },
        }
    }

    /// Forward a bracketed-paste event to the active screen.
    ///
    /// Only the variable-prompt screen consumes pastes — multi-line snippet
    /// values are the use case. Pastes on the select screen (fuzzy/browse) are
    /// dropped intentionally so a stray multi-line clipboard doesn't garble the
    /// search query.
    pub fn handle_paste(&mut self, text: &str) {
        if let Screen::Prompt(prompt) = &mut self.screen {
            prompt.append_input(text);
        }
    }

    fn handle_select_key(&mut self, key: KeyEvent) -> AppEvent {
        if matches!(key.code, KeyCode::Esc) {
            // Tag-drill handles esc in its own key handler (climbs out of drill).
            if matches!(self.nav_mode, NavigationMode::Tags) && self.tags.drill.is_some() {
                // fall through to mode-specific handler
            } else if matches!(self.nav_mode, NavigationMode::Browse)
                && !self.browse.path.is_empty()
            {
                self.browse.path.pop();
                self.browse.input.clear();
                self.browse.selection = Some(0);
                self.preview_scroll = 0;
                self.status = None;
                return AppEvent::Continue;
            } else {
                self.status = Some("cancelled".to_string());
                return AppEvent::Cancelled;
            }
        }

        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('t')
            || key.code == KeyCode::F(2)
        {
            self.nav_mode = match self.nav_mode {
                NavigationMode::Fuzzy => NavigationMode::Browse,
                NavigationMode::Browse => NavigationMode::Tags,
                NavigationMode::Tags => NavigationMode::Fuzzy,
            };
            self.preview_scroll = 0;
            self.status = None;
            return AppEvent::Continue;
        }

        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('e') {
            if let Some(id) = self.selected_snippet_id() {
                self.status = None;
                return AppEvent::EditSnippet(id);
            }
            return AppEvent::Continue;
        }

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && matches!(key.code, KeyCode::Char('j') | KeyCode::Down) {
            self.preview_scroll = self.preview_scroll.saturating_add(3);
            return AppEvent::Continue;
        }
        if ctrl && matches!(key.code, KeyCode::Char('k') | KeyCode::Up) {
            self.preview_scroll = self.preview_scroll.saturating_sub(3);
            return AppEvent::Continue;
        }

        match self.nav_mode {
            NavigationMode::Fuzzy => self.handle_fuzzy_key(key),
            NavigationMode::Browse => self.handle_browse_key(key),
            NavigationMode::Tags => self.handle_tags_key(key),
        }
    }

    fn handle_fuzzy_key(&mut self, key: KeyEvent) -> AppEvent {
        let hits = self.search_hits();
        match key.code {
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.fuzzy.type_char(c);
                self.preview_scroll = 0;
            }
            KeyCode::Backspace => {
                self.fuzzy.backspace();
                self.preview_scroll = 0;
            }
            KeyCode::Left => self.fuzzy.cursor_left(),
            KeyCode::Right => self.fuzzy.cursor_right(),
            KeyCode::Up => {
                self.fuzzy.move_cursor(1, hits.len());
                self.preview_scroll = 0;
            }
            KeyCode::Down => {
                self.fuzzy.move_cursor(-1, hits.len());
                self.preview_scroll = 0;
            }
            KeyCode::PageUp => {
                if !hits.is_empty() {
                    self.fuzzy.selection = Some(hits.len() - 1);
                }
                self.preview_scroll = 0;
            }
            KeyCode::PageDown => {
                if !hits.is_empty() {
                    self.fuzzy.selection = Some(0);
                }
                self.preview_scroll = 0;
            }
            KeyCode::Enter => {
                if let Some(snippet) = self.selected_fuzzy_snippet() {
                    let id = snippet.id().clone();
                    self.status = None;
                    return self.start_prompt_or_complete(id);
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
                self.preview_scroll = 0;
            }
            KeyCode::Backspace => {
                self.browse.backspace();
                self.preview_scroll = 0;
            }
            KeyCode::Tab => {
                self.browse.tab_complete(&self.tree);
                self.preview_scroll = 0;
            }
            KeyCode::Up => {
                self.browse.move_cursor(1, visible.len());
                self.preview_scroll = 0;
            }
            KeyCode::Down => {
                self.browse.move_cursor(-1, visible.len());
                self.preview_scroll = 0;
            }
            KeyCode::PageUp => {
                if !visible.is_empty() {
                    self.browse.selection = Some(visible.len() - 1);
                }
                self.preview_scroll = 0;
            }
            KeyCode::PageDown => {
                if !visible.is_empty() {
                    self.browse.selection = Some(0);
                }
                self.preview_scroll = 0;
            }
            KeyCode::Enter => {
                if let Some(id) = self.browse.activate(&self.tree) {
                    self.status = None;
                    return self.start_prompt_or_complete(id);
                }
                self.preview_scroll = 0;
            }
            _ => {}
        }
        AppEvent::Continue
    }

    fn handle_tags_key(&mut self, key: KeyEvent) -> AppEvent {
        if self.tags.drill.is_some() {
            let visible = self.visible_tag_snippets();
            match key.code {
                KeyCode::Esc => {
                    self.tags.drill = None;
                    self.preview_scroll = 0;
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.tags.type_drill_char(c);
                    self.preview_scroll = 0;
                }
                KeyCode::Backspace => {
                    if !self.tags.drill_backspace() {
                        self.tags.drill = None;
                    }
                    self.preview_scroll = 0;
                }
                KeyCode::Left => self.tags.drill_cursor_left(),
                KeyCode::Right => self.tags.drill_cursor_right(),
                KeyCode::Up => {
                    move_selection(&mut self.tags.drill_selection, 1, visible.len());
                    self.preview_scroll = 0;
                }
                KeyCode::Down => {
                    move_selection(&mut self.tags.drill_selection, -1, visible.len());
                    self.preview_scroll = 0;
                }
                KeyCode::PageUp => {
                    if !visible.is_empty() {
                        self.tags.drill_selection = Some(visible.len() - 1);
                    }
                    self.preview_scroll = 0;
                }
                KeyCode::PageDown => {
                    if !visible.is_empty() {
                        self.tags.drill_selection = Some(0);
                    }
                    self.preview_scroll = 0;
                }
                KeyCode::Enter => {
                    if let Some(snippet) = self.selected_tag_snippet() {
                        let id = snippet.id().clone();
                        self.status = None;
                        return self.start_prompt_or_complete(id);
                    }
                }
                _ => {}
            }
            return AppEvent::Continue;
        }

        let visible = self.visible_tags();
        match key.code {
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.tags.type_char(c);
                self.preview_scroll = 0;
            }
            KeyCode::Backspace => {
                self.tags.backspace();
                self.preview_scroll = 0;
            }
            KeyCode::Left => self.tags.cursor_left(),
            KeyCode::Right => self.tags.cursor_right(),
            KeyCode::Up => {
                self.tags.move_cursor(1, visible.len());
                self.preview_scroll = 0;
            }
            KeyCode::Down => {
                self.tags.move_cursor(-1, visible.len());
                self.preview_scroll = 0;
            }
            KeyCode::PageUp => {
                if !visible.is_empty() {
                    self.tags.list_selection = Some(visible.len() - 1);
                }
                self.preview_scroll = 0;
            }
            KeyCode::PageDown => {
                if !visible.is_empty() {
                    self.tags.list_selection = Some(0);
                }
                self.preview_scroll = 0;
            }
            KeyCode::Enter => {
                let selected = self.tags.list_selection.unwrap_or(0);
                if let Some(entry) = visible.get(selected) {
                    self.tags.drill = Some(entry.key.clone());
                    self.tags.drill_filter.clear();
                    self.tags.drill_cursor = 0;
                    self.tags.drill_selection = Some(0);
                    self.preview_scroll = 0;
                    self.status = None;
                }
            }
            _ => {}
        }
        AppEvent::Continue
    }

    /// Transition to the prompt screen, or complete immediately if the snippet
    /// has no variables. Deduplicates variables before creating the prompt so
    /// the user isn't asked for the same variable name twice.
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
        let mut prompt =
            PromptState::new(snippet_id, variables, snippet.frontmatter.variables.clone());
        load_prompt_state(&mut prompt, &self.provider, &self.cwd);
        self.status = prompt.error.clone();
        self.screen = Screen::Prompt(Box::new(prompt));
        AppEvent::Continue
    }

    pub(crate) fn search_hits(&self) -> Vec<search::SearchHit<'_>> {
        search::rank(
            &self.index,
            &self.fuzzy.query,
            &self.frecency,
            &self.cwd,
            self.now,
            &self.search_config,
        )
    }

    pub(crate) fn selected_fuzzy_snippet(&self) -> Option<&IndexedSnippet> {
        let hits = self.search_hits();
        let idx = self.fuzzy.selected().unwrap_or(0);
        hits.get(idx).map(|hit| hit.snippet)
    }

    pub(crate) fn selected_browse_snippet(&self) -> Option<&IndexedSnippet> {
        let visible = self.browse.visible(&self.tree);
        let idx = self.browse.selection.unwrap_or(0);
        let entry = visible.get(idx)?;
        match entry {
            BrowseEntry::Snippet(snippet) => self.index.get(&snippet.id),
            BrowseEntry::Directory(_) => None,
        }
    }

    pub(crate) fn selected_tag_snippet(&self) -> Option<&IndexedSnippet> {
        let idx = self.tags.drill_selection.unwrap_or(0);
        let entry = self.visible_tag_snippets().get(idx)?.clone();
        self.index.get(&entry.id)
    }

    pub(crate) fn visible_tags(&self) -> Vec<TagListEntry> {
        self.tag_index
            .iter()
            .filter_map(|(key, ids)| {
                let label = tag_label(key);
                if tag_matches_filter(key, &self.tags.filter) {
                    Some(TagListEntry {
                        key: key.clone(),
                        label: label.to_string(),
                        count: ids.len(),
                    })
                } else {
                    None
                }
            })
            .collect()
    }

    pub(crate) fn visible_tag_snippets(&self) -> Vec<TagSnippetEntry> {
        let Some(tag) = self.tags.drill.as_ref() else {
            return Vec::new();
        };
        let Some(ids) = self.tag_index.get(tag) else {
            return Vec::new();
        };
        ids.iter()
            .filter_map(|id| {
                self.index.get(id).map(|snippet| TagSnippetEntry {
                    id: id.clone(),
                    name: snippet.name().to_string(),
                })
            })
            .filter(|entry| {
                self.tags.drill_filter.is_empty()
                    || entry
                        .name
                        .to_lowercase()
                        .contains(&self.tags.drill_filter.to_lowercase())
            })
            .collect()
    }

    fn selected_snippet_id(&self) -> Option<SnippetId> {
        self.selected_snippet().map(|snippet| snippet.id().clone())
    }

    fn restore_fuzzy_selection(&mut self, preferred_id: Option<&SnippetId>) {
        let hits = self.search_hits();
        if hits.is_empty() {
            self.fuzzy.selection = None;
            return;
        }
        if let Some(position) =
            preferred_id.and_then(|id| hits.iter().position(|hit| hit.snippet.id() == id))
        {
            self.fuzzy.selection = Some(position);
            return;
        }
        let current = self.fuzzy.selection.unwrap_or(0).min(hits.len() - 1);
        self.fuzzy.selection = Some(current);
    }

    fn restore_browse_selection(&mut self, preferred_id: Option<&SnippetId>) {
        while !self.browse.path.is_empty() && self.tree.get(&self.browse.path).is_none() {
            self.browse.path.pop();
        }
        let visible = self.browse.visible(&self.tree);
        if visible.is_empty() {
            self.browse.selection = None;
            return;
        }
        if let Some(position) = preferred_id.and_then(|id| {
            visible.iter().position(|entry| match entry {
                BrowseEntry::Snippet(snippet) => snippet.id == *id,
                BrowseEntry::Directory(_) => false,
            })
        }) {
            self.browse.selection = Some(position);
            return;
        }
        let current = self.browse.selection.unwrap_or(0).min(visible.len() - 1);
        self.browse.selection = Some(current);
    }

    fn restore_tags_selection(&mut self) {
        if self.tags.drill.is_some() {
            let visible_len = self.visible_tag_snippets().len();
            if visible_len == 0 {
                self.tags.drill_selection = None;
                return;
            }
            let current = self.tags.drill_selection.unwrap_or(0).min(visible_len - 1);
            self.tags.drill_selection = Some(current);
            return;
        }

        let visible_len = self.visible_tags().len();
        if visible_len == 0 {
            self.tags.list_selection = None;
            return;
        }
        let current = self.tags.list_selection.unwrap_or(0).min(visible_len - 1);
        self.tags.list_selection = Some(current);
    }
}

/// Parse `command` for `<#...>` references, substitute confirmed values, and
/// return the rendered string ready for `bash -c`. Parse and render errors
/// become `io::Error` so they flow through the existing suggestion-error path.
pub(crate) fn render_template_for_exec(
    command: &str,
    confirmed: &BTreeMap<String, String>,
) -> io::Result<String> {
    let template = command_template::parse_command_template(command).map_err(io::Error::other)?;
    command_template::render(&template, confirmed).map_err(io::Error::other)
}

pub(crate) fn tag_label(key: &TagKey) -> &str {
    match key {
        TagKey::Tag(tag) => tag,
        TagKey::Untagged => "(untagged)",
    }
}

fn tag_matches_filter(key: &TagKey, filter: &str) -> bool {
    match key {
        TagKey::Tag(tag) => filter.is_empty() || tag.contains(filter),
        TagKey::Untagged => filter.is_empty() || tag_label(key).contains(filter),
    }
}

fn move_selection(selection: &mut Option<usize>, delta: i32, visible_len: usize) {
    if visible_len == 0 {
        *selection = None;
        return;
    }
    let current = selection.unwrap_or(0) as i32;
    let next = (current + delta).clamp(0, visible_len as i32 - 1);
    *selection = Some(next as usize);
}
