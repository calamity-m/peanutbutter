use crate::domain::SnippetId;
use crate::{config, config::VariableInputConfig};
use std::path::PathBuf;

mod app;
mod highlight;
mod prompt;
mod render;
mod terminal;

pub use app::{
    AppEvent, ExecutionApp, NavigationMode, SuggestionProvider, SystemSuggestionProvider,
};
pub use prompt::render_command;
pub use terminal::{execute_default, run_execute, run_execute_with_provider};

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
    pub search: config::SearchConfig,
    pub theme: config::Theme,
    pub variables: std::collections::BTreeMap<String, VariableInputConfig>,
}

impl Default for ExecuteOptions {
    fn default() -> Self {
        Self {
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            now: terminal::unix_now(),
            viewport_height: 20,
            search: config::SearchConfig::default(),
            theme: config::Theme::default(),
            variables: std::collections::BTreeMap::new(),
        }
    }
}

#[cfg(test)]
pub(crate) use prompt::{
    active_prompt_style, builtin_suggestions, command_suggestions, placeholder_prompt_style,
    render_command_text, unique_variables,
};
#[cfg(test)]
pub(crate) use terminal::compact_viewport_height;

#[cfg(test)]
mod tests;
