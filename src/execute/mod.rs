//! Interactive snippet execution: TUI loop, variable prompting, and terminal
//! management.
//!
//! # Flow
//!
//! 1. [`terminal::run_execute_with_provider`] initialises the terminal in raw
//!    mode and enters the event loop.
//! 2. [`app::ExecutionApp`] owns all UI state and translates key events into
//!    [`AppEvent`]s.
//! 3. [`render`](mod@render) draws each frame using ratatui.
//! 4. When the user confirms a snippet, [`app::ExecutionApp`] transitions to
//!    the [`prompt`](mod@prompt) screen to collect variable values one at a time.
//! 5. On completion the loop returns an [`ExecutionOutcome`] with the fully
//!    rendered command string.

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

/// The result of a completed TUI session: the snippet the user chose and the
/// fully rendered command string with all variable placeholders filled in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionOutcome {
    /// Stable id of the selected snippet (used to record a frecency event).
    pub snippet_id: SnippetId,
    /// The command body after substituting all variable values.
    pub command: String,
}

/// Configuration passed into the execute TUI session.
#[derive(Debug, Clone)]
pub struct ExecuteOptions {
    /// Working directory used for frecency location scoring and suggestion
    /// commands.
    pub cwd: PathBuf,
    /// Unix timestamp (seconds) representing "now" — injected so tests can
    /// control time without sleeping.
    pub now: u64,
    /// Maximum number of terminal rows the inline viewport may occupy.
    pub viewport_height: u16,
    /// Search ranking weights for this session.
    pub search: config::SearchConfig,
    /// Visual theme for this session.
    pub theme: config::Theme,
    /// Per-variable config overrides (keyed by variable name).
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
