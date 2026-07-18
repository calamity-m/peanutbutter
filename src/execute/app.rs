use crate::config::{SearchConfig, SuggestionCommandsConfig, Theme, VariableInputConfig};
use crate::domain::{SnippetId, Variable, VariableSource, VariableSpec};
use crate::frecency::FrecencyStore;
use crate::fuzzy::FuzzyState;
use crate::index::{IndexedSnippet, SnippetIndex, TagKey};
use crate::keybinds::{BrowseAction, ExecuteKeymap, FuzzyAction, SelectAction, TagsAction};
use crate::search;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::widgets::ListState;
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use super::browse::{BrowseEntry, BrowseState, BrowseTree};
use super::prompt::{
    PromptState, PromptTransition, handle_prompt_key, load_prompt_state, unique_variables,
};
use super::{ExecutionOutcome, render_command};
use crate::syntax as command_template;

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

/// State for the tag-based picker view, backed by a [`TreePicker`] with at
/// most two levels: the tag list at depth 0 and a drilled snippet list at
/// depth 1. Going up (Esc / Backspace at empty filter) restores the cursor
/// onto the tag the user drilled into.
#[derive(Debug, Default, Clone)]
pub struct TagsState {
    pub picker: super::tree_picker::TreePicker<TagKey>,
}

impl TagsState {
    pub fn new() -> Self {
        Self {
            picker: super::tree_picker::TreePicker::new(),
        }
    }

    pub fn drill(&self) -> Option<&TagKey> {
        self.picker.path().first()
    }

    pub fn filter(&self) -> &str {
        if self.picker.depth() == 0 {
            self.picker.filter()
        } else {
            ""
        }
    }

    pub fn list_selection(&self) -> Option<usize> {
        if self.picker.depth() == 0 {
            self.picker.selection()
        } else {
            None
        }
    }

    pub fn drill_filter(&self) -> &str {
        if self.picker.depth() >= 1 {
            self.picker.filter()
        } else {
            ""
        }
    }

    pub fn drill_selection(&self) -> Option<usize> {
        if self.picker.depth() >= 1 {
            self.picker.selection()
        } else {
            None
        }
    }

    pub fn type_char(&mut self, c: char) {
        self.picker.type_char(c);
    }

    pub fn backspace(&mut self) -> bool {
        self.picker.input_backspace()
    }

    pub fn cursor_left(&mut self) {
        self.picker.cursor_left();
    }

    pub fn cursor_right(&mut self) {
        self.picker.cursor_right();
    }

    pub fn move_cursor(&mut self, delta: i32, visible_len: usize) {
        self.picker.move_selection(delta, visible_len);
    }

    pub fn type_drill_char(&mut self, c: char) {
        self.picker.type_char(c);
    }

    pub fn drill_backspace(&mut self) -> bool {
        self.picker.input_backspace()
    }

    pub fn drill_cursor_left(&mut self) {
        self.picker.cursor_left();
    }

    pub fn drill_cursor_right(&mut self) {
        self.picker.cursor_right();
    }

    /// Display-column offset of the cursor at the current level.
    pub fn cursor_col(&self) -> usize {
        self.picker.cursor_col()
    }

    pub fn drill_cursor_col(&self) -> usize {
        self.picker.cursor_col()
    }

    /// Descend into `tag`, recording it on the tag-list frame so a later
    /// ascend restores the cursor.
    pub fn enter_drill(&mut self, tag: TagKey) {
        // Only meaningful at the tag-list level.
        if self.picker.depth() == 0 {
            self.picker.descend(tag);
        }
    }

    /// Exit the drilled-snippet level. Caller is responsible for calling
    /// [`restore_after_drill_exit`] with the parent's visible tag list to
    /// restore the cursor onto the previously-drilled tag.
    pub fn exit_drill(&mut self) -> bool {
        self.picker.ascend().is_some()
    }

    /// After [`exit_drill`](Self::exit_drill), restore selection onto the
    /// tag we drilled into.
    pub(crate) fn restore_after_drill_exit(&mut self, visible: &[TagListEntry]) {
        self.picker
            .restore_selection(visible, |entry| Some(entry.key.clone()));
    }

    pub fn set_list_selection(&mut self, selection: Option<usize>) {
        if self.picker.depth() == 0 {
            self.picker.set_selection(selection);
        }
    }

    pub fn set_drill_selection(&mut self, selection: Option<usize>) {
        if self.picker.depth() >= 1 {
            self.picker.set_selection(selection);
        }
    }

    /// Test/setup helper: overwrite the current level's filter text.
    pub fn set_filter(&mut self, filter: String) {
        let frame = self.picker.current_mut();
        frame.cursor = filter.len();
        frame.filter = filter;
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
    /// Return a reusable ghost default for a plain free-form variable.
    ///
    /// The default implementation preserves compatibility for custom
    /// suggestion providers that do not support `default_value`.
    fn default_value(
        &self,
        _variable: &Variable,
        _local_variables: &std::collections::BTreeMap<String, VariableSpec>,
        _confirmed: &BTreeMap<String, String>,
    ) -> Option<String> {
        None
    }
    /// Return the unrendered reusable ghost-default template, if any. This
    /// lets prompt dirty tracking include dependent `default_value` fields.
    fn default_value_source(
        &self,
        _variable: &Variable,
        _local_variables: &std::collections::BTreeMap<String, VariableSpec>,
    ) -> Option<String> {
        None
    }
    /// Return the ghost text to show while the input buffer is empty, if any,
    /// resolving inline `<@name:@hint>` first, then file-local and config
    /// variable specs. Hints are display-only and never become the value.
    fn hint(
        &self,
        variable: &Variable,
        local_variables: &std::collections::BTreeMap<String, VariableSpec>,
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
            // An inline hint only supplies ghost text; suggestions resolve
            // exactly like a free-form placeholder.
            VariableSource::Free | VariableSource::Hint(_) => {
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

    fn hint(
        &self,
        variable: &Variable,
        local_variables: &std::collections::BTreeMap<String, VariableSpec>,
    ) -> Option<String> {
        if let VariableSource::Hint(text) = &variable.source {
            return Some(text.clone());
        }
        local_variables
            .get(&variable.name)
            .and_then(|config| config.hint.clone())
            .or_else(|| {
                self.variable_inputs
                    .get(&variable.name)
                    .and_then(|config| config.hint.clone())
            })
    }

    fn command_source(
        &self,
        variable: &Variable,
        local_variables: &std::collections::BTreeMap<String, VariableSpec>,
    ) -> Option<String> {
        match &variable.source {
            VariableSource::Command(cmd) => Some(cmd.clone()),
            VariableSource::Default(_) => None,
            VariableSource::Free | VariableSource::Hint(_) => {
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
        _confirmed: &BTreeMap<String, String>,
    ) -> Option<String> {
        match &variable.source {
            VariableSource::Default(_) | VariableSource::Command(_) => None,
            // A reusable-spec default still pre-fills a hint placeholder; the
            // hint only shows once the buffer is cleared.
            VariableSource::Free | VariableSource::Hint(_) => local_variables
                .get(&variable.name)
                .and_then(|config| config.default.clone())
                .or_else(|| {
                    self.variable_inputs
                        .get(&variable.name)
                        .and_then(|config| config.default.clone())
                }),
        }
    }

    fn default_value(
        &self,
        variable: &Variable,
        local_variables: &std::collections::BTreeMap<String, VariableSpec>,
        confirmed: &BTreeMap<String, String>,
    ) -> Option<String> {
        let source = self.default_value_source(variable, local_variables)?;
        match command_template::parse_command_template(&source) {
            Ok(template) => command_template::render(&template, confirmed).ok(),
            // Match inline `:?` parsing: invalid dependent syntax remains
            // usable literal text while lint reports it to the author.
            Err(_) => Some(source),
        }
    }

    fn default_value_source(
        &self,
        variable: &Variable,
        local_variables: &std::collections::BTreeMap<String, VariableSpec>,
    ) -> Option<String> {
        if !matches!(variable.source, VariableSource::Free) {
            return None;
        }
        // A local definition is a complete layer for this source: an invalid
        // cross-layer mix remains deterministic rather than field-merged.
        local_variables
            .get(&variable.name)
            .map(|spec| spec.default_value.clone())
            .unwrap_or_else(|| {
                self.variable_inputs
                    .get(&variable.name)
                    .and_then(|spec| spec.default_value.clone())
            })
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
    /// Shell buffer content to seed into the first variable of the next
    /// selected snippet, if any. `None` when invoked without a buffer.
    pub(crate) initial_buffer: Option<String>,
    /// Resolved keybinds for this session. Ctrl+C cancel is handled before
    /// this keymap and is not remappable.
    pub(crate) keymap: ExecuteKeymap,
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
            initial_buffer: None,
            keymap: ExecuteKeymap::default(),
        }
    }

    /// Seed the shell buffer to be fed into the first variable of the next
    /// selected snippet. Empty/`None` leaves the default (insert) behavior.
    pub fn with_initial_buffer(mut self, buffer: Option<String>) -> Self {
        self.initial_buffer = buffer.filter(|s| !s.is_empty());
        self
    }

    /// Replace the default keymap with the session's resolved keybinds.
    pub fn with_keymap(mut self, keymap: ExecuteKeymap) -> Self {
        self.keymap = keymap;
        self
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
            .drill()
            .is_some_and(|tag| !self.tag_index.contains_key(tag))
        {
            self.tags.exit_drill();
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

        // Hard emergency cancel: not part of the configurable keymap so raw
        // mode always has an escape hatch regardless of user config.
        if ExecuteKeymap::is_emergency_cancel(&key) {
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
                &self.keymap.prompt,
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

    /// Select-level actions are resolved before mode-specific ones, matching
    /// the documented context precedence (`execute.select` first).
    fn handle_select_key(&mut self, key: KeyEvent) -> AppEvent {
        match self.keymap.select.action_for(&key) {
            Some(SelectAction::CancelOrBack) => {
                if matches!(self.nav_mode, NavigationMode::Tags) && self.tags.drill().is_some() {
                    // Backing out of a tag drill returns to the tag list, same
                    // as the tags context's `return_to_tags` action.
                    self.exit_tag_drill();
                    self.preview_scroll = 0;
                    return AppEvent::Continue;
                }
                if matches!(self.nav_mode, NavigationMode::Browse) && !self.browse.path().is_empty()
                {
                    self.browse.ascend(&self.tree);
                    self.preview_scroll = 0;
                    self.status = None;
                    return AppEvent::Continue;
                }
                self.status = Some("cancelled".to_string());
                return AppEvent::Cancelled;
            }
            Some(SelectAction::CycleMode) => {
                self.nav_mode = match self.nav_mode {
                    NavigationMode::Fuzzy => NavigationMode::Browse,
                    NavigationMode::Browse => NavigationMode::Tags,
                    NavigationMode::Tags => NavigationMode::Fuzzy,
                };
                self.preview_scroll = 0;
                self.status = None;
                return AppEvent::Continue;
            }
            Some(SelectAction::Edit) => {
                if let Some(id) = self.selected_snippet_id() {
                    self.status = None;
                    return AppEvent::EditSnippet(id);
                }
                return AppEvent::Continue;
            }
            Some(SelectAction::PreviewDown) => {
                self.preview_scroll = self.preview_scroll.saturating_add(3);
                return AppEvent::Continue;
            }
            Some(SelectAction::PreviewUp) => {
                self.preview_scroll = self.preview_scroll.saturating_sub(3);
                return AppEvent::Continue;
            }
            None => {}
        }

        match self.nav_mode {
            NavigationMode::Fuzzy => self.handle_fuzzy_key(key),
            NavigationMode::Browse => self.handle_browse_key(key),
            NavigationMode::Tags => self.handle_tags_key(key),
        }
    }

    fn handle_fuzzy_key(&mut self, key: KeyEvent) -> AppEvent {
        let hits = self.search_hits();
        match self.keymap.fuzzy.action_for(&key) {
            Some(FuzzyAction::Accept) => {
                if let Some(snippet) = self.selected_fuzzy_snippet() {
                    let id = snippet.id().clone();
                    self.status = None;
                    return self.start_prompt_or_complete(id);
                }
            }
            Some(FuzzyAction::Backspace) => {
                self.fuzzy.backspace();
                self.preview_scroll = 0;
            }
            Some(FuzzyAction::CursorLeft) => self.fuzzy.cursor_left(),
            Some(FuzzyAction::CursorRight) => self.fuzzy.cursor_right(),
            // Deltas are inverted because the list renders bottom-aligned;
            // action names describe on-screen intent.
            Some(FuzzyAction::MoveUp) => {
                self.fuzzy.move_cursor(1, hits.len());
                self.preview_scroll = 0;
            }
            Some(FuzzyAction::MoveDown) => {
                self.fuzzy.move_cursor(-1, hits.len());
                self.preview_scroll = 0;
            }
            Some(FuzzyAction::PageUp) => {
                if !hits.is_empty() {
                    self.fuzzy.selection = Some(hits.len() - 1);
                }
                self.preview_scroll = 0;
            }
            Some(FuzzyAction::PageDown) => {
                if !hits.is_empty() {
                    self.fuzzy.selection = Some(0);
                }
                self.preview_scroll = 0;
            }
            None => {
                // Text fallback: unmodified printable characters filter the
                // query; not modeled as a configurable action.
                if let KeyCode::Char(c) = key.code
                    && !key.modifiers.contains(KeyModifiers::CONTROL)
                {
                    self.fuzzy.type_char(c);
                    self.preview_scroll = 0;
                }
            }
        }
        AppEvent::Continue
    }

    fn handle_browse_key(&mut self, key: KeyEvent) -> AppEvent {
        let visible = self.browse.visible(&self.tree);
        match self.keymap.browse.action_for(&key) {
            Some(BrowseAction::AcceptOrOpen) => {
                if let Some(id) = self.browse.activate(&self.tree) {
                    self.status = None;
                    return self.start_prompt_or_complete(id);
                }
                self.preview_scroll = 0;
            }
            Some(BrowseAction::Backspace) => {
                self.browse.backspace(&self.tree);
                self.preview_scroll = 0;
            }
            Some(BrowseAction::Complete) => {
                self.browse.tab_complete(&self.tree);
                self.preview_scroll = 0;
            }
            Some(BrowseAction::CursorLeft) => self.browse.cursor_left(),
            Some(BrowseAction::CursorRight) => self.browse.cursor_right(),
            Some(BrowseAction::MoveUp) => {
                self.browse.move_cursor(1, visible.len());
                self.preview_scroll = 0;
            }
            Some(BrowseAction::MoveDown) => {
                self.browse.move_cursor(-1, visible.len());
                self.preview_scroll = 0;
            }
            Some(BrowseAction::PageUp) => {
                if !visible.is_empty() {
                    self.browse.set_selection(Some(visible.len() - 1));
                }
                self.preview_scroll = 0;
            }
            Some(BrowseAction::PageDown) => {
                if !visible.is_empty() {
                    self.browse.set_selection(Some(0));
                }
                self.preview_scroll = 0;
            }
            None => {
                if let KeyCode::Char(c) = key.code
                    && !key.modifiers.contains(KeyModifiers::CONTROL)
                {
                    self.browse.type_char(c);
                    self.preview_scroll = 0;
                }
            }
        }
        AppEvent::Continue
    }

    fn handle_tags_key(&mut self, key: KeyEvent) -> AppEvent {
        let action = self.keymap.tags.action_for(&key);
        if self.tags.drill().is_some() {
            let visible = self.visible_tag_snippets();
            match action {
                Some(TagsAction::AcceptOrDrill) => {
                    if let Some(snippet) = self.selected_tag_snippet() {
                        let id = snippet.id().clone();
                        self.status = None;
                        return self.start_prompt_or_complete(id);
                    }
                }
                Some(TagsAction::Backspace) => {
                    if !self.tags.drill_backspace() {
                        self.exit_tag_drill();
                    }
                    self.preview_scroll = 0;
                }
                Some(TagsAction::CursorLeft) => self.tags.drill_cursor_left(),
                Some(TagsAction::CursorRight) => self.tags.drill_cursor_right(),
                Some(TagsAction::MoveUp) => {
                    self.tags.move_cursor(1, visible.len());
                    self.preview_scroll = 0;
                }
                Some(TagsAction::MoveDown) => {
                    self.tags.move_cursor(-1, visible.len());
                    self.preview_scroll = 0;
                }
                Some(TagsAction::PageUp) => {
                    if !visible.is_empty() {
                        self.tags.set_drill_selection(Some(visible.len() - 1));
                    }
                    self.preview_scroll = 0;
                }
                Some(TagsAction::PageDown) => {
                    if !visible.is_empty() {
                        self.tags.set_drill_selection(Some(0));
                    }
                    self.preview_scroll = 0;
                }
                Some(TagsAction::ReturnToTags) => {
                    self.exit_tag_drill();
                    self.preview_scroll = 0;
                }
                None => {
                    if let KeyCode::Char(c) = key.code
                        && !key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        self.tags.type_drill_char(c);
                        self.preview_scroll = 0;
                    }
                }
            }
            return AppEvent::Continue;
        }

        let visible = self.visible_tags();
        match action {
            Some(TagsAction::AcceptOrDrill) => {
                let selected = self.tags.list_selection().unwrap_or(0);
                if let Some(entry) = visible.get(selected) {
                    self.tags.enter_drill(entry.key.clone());
                    self.preview_scroll = 0;
                    self.status = None;
                }
            }
            Some(TagsAction::Backspace) => {
                self.tags.backspace();
                self.preview_scroll = 0;
            }
            Some(TagsAction::CursorLeft) => self.tags.cursor_left(),
            Some(TagsAction::CursorRight) => self.tags.cursor_right(),
            Some(TagsAction::MoveUp) => {
                self.tags.move_cursor(1, visible.len());
                self.preview_scroll = 0;
            }
            Some(TagsAction::MoveDown) => {
                self.tags.move_cursor(-1, visible.len());
                self.preview_scroll = 0;
            }
            Some(TagsAction::PageUp) => {
                if !visible.is_empty() {
                    self.tags.set_list_selection(Some(visible.len() - 1));
                }
                self.preview_scroll = 0;
            }
            Some(TagsAction::PageDown) => {
                if !visible.is_empty() {
                    self.tags.set_list_selection(Some(0));
                }
                self.preview_scroll = 0;
            }
            // At the tag root, backing out is `select.cancel_or_back`'s job.
            Some(TagsAction::ReturnToTags) => {}
            None => {
                if let KeyCode::Char(c) = key.code
                    && !key.modifiers.contains(KeyModifiers::CONTROL)
                {
                    self.tags.type_char(c);
                    self.preview_scroll = 0;
                }
            }
        }
        AppEvent::Continue
    }

    /// Ascend out of the drilled tag, restoring the cursor in the tag list
    /// onto the tag we drilled into.
    fn exit_tag_drill(&mut self) {
        if !self.tags.exit_drill() {
            return;
        }
        let visible = self.visible_tags();
        self.tags.restore_after_drill_exit(&visible);
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
            // No variable to absorb the buffer, so it is not consumed; the shell
            // keeps its insert-at-cursor behavior.
            return AppEvent::Completed(ExecutionOutcome {
                snippet_id,
                command: snippet.body().to_string(),
                consumed_buffer: false,
            });
        }
        let mut prompt =
            PromptState::new(snippet_id, variables, snippet.frontmatter.variables.clone());
        load_prompt_state(&mut prompt, &self.provider, &self.cwd);
        // Feed the shell buffer into the first variable's editable input,
        // overriding any default. Marks the eventual outcome as consuming the
        // buffer so the shell replaces the whole line instead of inserting.
        if let Some(buffer) = self.initial_buffer.clone() {
            prompt.input = buffer;
            prompt.seeded_from_buffer = true;
        }
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
        let idx = self.browse.selection().unwrap_or(0);
        let entry = visible.get(idx)?;
        match entry {
            BrowseEntry::Snippet(snippet) => self.index.get(&snippet.id),
            BrowseEntry::Directory(_) => None,
        }
    }

    pub(crate) fn selected_tag_snippet(&self) -> Option<&IndexedSnippet> {
        let idx = self.tags.drill_selection().unwrap_or(0);
        let entry = self.visible_tag_snippets().get(idx)?.clone();
        self.index.get(&entry.id)
    }

    pub(crate) fn visible_tags(&self) -> Vec<TagListEntry> {
        self.tag_index
            .iter()
            .filter_map(|(key, ids)| {
                let label = tag_label(key);
                if tag_matches_filter(key, self.tags.filter()) {
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
        let Some(tag) = self.tags.drill() else {
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
                self.tags.drill_filter().is_empty()
                    || entry
                        .name
                        .to_lowercase()
                        .contains(&self.tags.drill_filter().to_lowercase())
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
        self.browse.trim_missing_path(&self.tree);
        let visible = self.browse.visible(&self.tree);
        if visible.is_empty() {
            self.browse.set_selection(None);
            return;
        }
        if let Some(position) = preferred_id.and_then(|id| {
            visible.iter().position(|entry| match entry {
                BrowseEntry::Snippet(snippet) => snippet.id == *id,
                BrowseEntry::Directory(_) => false,
            })
        }) {
            self.browse.set_selection(Some(position));
            return;
        }
        let current = self.browse.selection().unwrap_or(0).min(visible.len() - 1);
        self.browse.set_selection(Some(current));
    }

    fn restore_tags_selection(&mut self) {
        if self.tags.drill().is_some() {
            let visible_len = self.visible_tag_snippets().len();
            if visible_len == 0 {
                self.tags.set_drill_selection(None);
                return;
            }
            let current = self
                .tags
                .drill_selection()
                .unwrap_or(0)
                .min(visible_len - 1);
            self.tags.set_drill_selection(Some(current));
            return;
        }

        let visible_len = self.visible_tags().len();
        if visible_len == 0 {
            self.tags.set_list_selection(None);
            return;
        }
        let current = self.tags.list_selection().unwrap_or(0).min(visible_len - 1);
        self.tags.set_list_selection(Some(current));
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
