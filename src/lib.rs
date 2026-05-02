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
//! | [`fuzzy`] | nucleo-backed fuzzy matching |
//! | [`search`] | Combined fuzzy + frecency ranking |
//! | [`browse`] | Directory-tree navigation state |
//! | [`config`] | TOML config loading and theme |
//! | [`cli`] | Argument parsing and command dispatch |
//! | [`execute`] | Interactive TUI (ratatui, crossterm) |

pub mod browse;
pub mod cli;
pub mod completions;
pub mod config;
pub mod discovery;
pub mod domain;
pub mod editor;
pub mod execute;
pub mod frecency;
pub mod fuzzy;
pub mod index;
pub mod parser;
pub mod search;

/// The binary name used in help text and warning messages.
pub const BINARY_NAME: &str = "peanutbutter";
/// The shell alias installed by `peanutbutter bash`.
pub const BASH_ALIAS_NAME: &str = "pb";
