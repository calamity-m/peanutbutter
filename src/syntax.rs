//! Snippet syntax helpers.

pub mod command_template;

pub use command_template::{
    CommandTemplate, Fragment, ParseError, RenderError, is_dependent, parse_command_template,
    referenced_names, render, shell_single_quote,
};
