//! `peanutbutter` — location-aware terminal snippet manager.
//!
//! Snippets live in plain Markdown files; this library parses them, indexes
//! them, scores them with a frecency algorithm, and drives an inline TUI that
//! writes the selected (and filled-in) command back into the shell's readline
//! buffer.
//!
//! # Module map
//!
//! | Module | Role |
//! |--------|------|
//! | [`domain`] | Core value types (snippets, variables, ids) |
//! | [`parser`] | Markdown → `SnippetFile` |
//! | [`discovery`] | Recursive `.md` file finder |
//! | [`index`] | In-memory snippet index |
//! | [`frecency`] | Usage history and recency/location scoring |
//! | [`lint`] | Read-only snippet authoring checks |
//! | [`gc`] | Frecency garbage collection |
//! | [`stats`] | Usage statistics from frecency history |
//! | [`edit`] | Snippet file target resolution and editor launch for `pb edit` |
//! | [`new`] | Command capture and snippet creation |
//! | [`syntax`] | Snippet syntax helpers and command-template rendering |
//! | [`completions`] | Shell integration and completion scripts |
//! | [`fuzzy`] | nucleo-backed fuzzy matching |
//! | [`search`] | Combined fuzzy + frecency ranking |
//! | [`config`] | TOML config loading and theme |
//! | [`keybinds`] | Configurable key chords and the execute keymap |
//! | [`settings`] | Interactive config tuning TUI |
//! | [`cli`] | Argument parsing and command dispatch |
//! | [`docs`] | Embedded syntax/config reference printed by `pb docs` |
//! | [`execute`] | Interactive TUI (ratatui, crossterm) |
//! | [`lsp`] | Language Server Protocol server (diagnostics, completions, hover, go-to-def) |

pub mod cli;
pub mod completions;
pub mod config;
pub mod discovery;
pub mod docs;
pub mod domain;
pub mod edit;
pub mod execute;
pub mod frecency;
pub mod fuzzy;
pub mod gc;
pub mod index;
pub mod keybinds;
pub mod lint;
pub mod lsp;
pub mod new;
pub mod parser;
pub mod search;
pub mod settings;
pub(crate) mod shell;
pub mod stats;
pub mod syntax;
pub(crate) mod tui;

/// The binary name used in help text and warning messages.
pub const BINARY_NAME: &str = "peanutbutter";
/// The shell alias installed by `peanutbutter completions bash`.
pub const BASH_ALIAS_NAME: &str = "pb";

/// Exit status emitted by `execute` when the selected snippet consumed the
/// shell buffer into its first variable. The shell integration interprets this
/// as "replace the whole line" rather than the default insert-at-cursor.
pub const REPLACE_BUFFER_EXIT_CODE: i32 = 10;
