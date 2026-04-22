use crate::browse::{BrowseEntry, BrowseState, BrowseTree};
use crate::config::{SearchConfig, Theme, VariableInputConfig};
use crate::domain::{SnippetId, Variable, VariableSource};
use crate::frecency::FrecencyStore;
use crate::fuzzy::FuzzyState;
use crate::index::{IndexedSnippet, SnippetIndex};
use crate::search;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::widgets::ListState;
use std::io;
use std::path::{Path, PathBuf};

use super::prompt::{
    PromptState, PromptTransition, handle_prompt_key, load_prompt_state, unique_variables,
};
use super::{ExecutionOutcome, render_command};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavigationMode {
    Fuzzy,
    Browse,
}

pub trait SuggestionProvider {
    fn suggestions(&self, variable: &Variable, cwd: &Path) -> io::Result<Vec<String>>;
    fn default_input(&self, variable: &Variable) -> Option<String>;
}

#[derive(Debug, Default, Clone)]
pub struct SystemSuggestionProvider {
    variable_inputs: std::collections::BTreeMap<String, VariableInputConfig>,
}

impl SystemSuggestionProvider {
    pub fn new(variable_inputs: std::collections::BTreeMap<String, VariableInputConfig>) -> Self {
        Self { variable_inputs }
    }
}

impl SuggestionProvider for SystemSuggestionProvider {
    fn suggestions(&self, variable: &Variable, cwd: &Path) -> io::Result<Vec<String>> {
        match &variable.source {
            VariableSource::Command(cmd) => super::prompt::command_suggestions(cmd, cwd),
            VariableSource::Default(_) => Ok(Vec::new()),
            VariableSource::Free => {
                if let Some(config) = self.variable_inputs.get(&variable.name) {
                    if !config.suggestions.is_empty() {
                        return Ok(config.suggestions.clone());
                    }
                    if let Some(command) = &config.command {
                        return super::prompt::command_suggestions(command, cwd);
                    }
                }
                super::prompt::builtin_suggestions(&variable.name, cwd)
            }
        }
    }

    fn default_input(&self, variable: &Variable) -> Option<String> {
        match &variable.source {
            VariableSource::Default(value) => Some(value.clone()),
            VariableSource::Command(_) => None,
            VariableSource::Free => self
                .variable_inputs
                .get(&variable.name)
                .and_then(|config| config.default.clone()),
        }
    }
}

#[derive(Debug)]
pub(crate) enum Screen {
    Select,
    Prompt(PromptState),
}

pub struct ExecutionApp<P = SystemSuggestionProvider> {
    pub(crate) index: SnippetIndex,
    pub(crate) frecency: FrecencyStore,
    pub(crate) tree: BrowseTree,
    pub(crate) cwd: PathBuf,
    pub(crate) now: u64,
    pub(crate) provider: P,
    pub(crate) screen: Screen,
    pub(crate) nav_mode: NavigationMode,
    pub fuzzy: FuzzyState,
    pub browse: BrowseState,
    pub(crate) status: Option<String>,
    pub(crate) preview_scroll: u16,
    pub(crate) fuzzy_list: ListState,
    pub(crate) browse_list: ListState,
    pub(crate) search_config: SearchConfig,
    pub(crate) theme: Theme,
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
        search_config: SearchConfig,
        theme: Theme,
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
            preview_scroll: 0,
            fuzzy_list: ListState::default(),
            browse_list: ListState::default(),
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

    pub fn selected_snippet(&self) -> Option<&IndexedSnippet> {
        match &self.screen {
            Screen::Select => match self.nav_mode {
                NavigationMode::Fuzzy => self.selected_fuzzy_snippet(),
                NavigationMode::Browse => self.selected_browse_snippet(),
            },
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
            self.preview_scroll = 0;
            self.status = None;
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
        load_prompt_state(&mut prompt, &self.provider, &self.cwd);
        self.status = prompt.error.clone();
        self.screen = Screen::Prompt(prompt);
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
}
