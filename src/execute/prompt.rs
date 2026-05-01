use crate::config::Theme;
use crate::domain::Variable;
use crate::index::SnippetIndex;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io;
use std::path::Path;
use std::process::Command;

use super::ExecutionOutcome;
use super::app::SuggestionProvider;

/// Mutable state for the variable-filling screen shown after a snippet is
/// selected.
///
/// The user moves through `variables` one at a time (tracked by `index`),
/// filling each with a typed value or a picked suggestion. Completed values
/// accumulate in `values`; Tab/Shift+Tab allow cycling between variables
/// non-linearly. Backspace on an empty input reverts to the previous variable.
#[derive(Debug)]
pub(crate) struct PromptState {
    /// Id of the snippet whose variables are being filled.
    pub(crate) snippet_id: crate::domain::SnippetId,
    /// Deduplicated list of variables to fill, in order.
    pub(crate) variables: Vec<Variable>,
    /// Index into `variables` of the variable currently being edited.
    pub(crate) index: usize,
    /// Values that have been confirmed for each variable name so far.
    pub(crate) values: BTreeMap<String, String>,
    /// Raw text the user has typed for the current variable.
    pub(crate) input: String,
    /// Full suggestion list for the current variable (unfiltered).
    pub(crate) suggestions: Vec<String>,
    /// Non-fatal error from the last suggestion provider call, shown in the UI.
    pub(crate) error: Option<String>,
    /// Currently highlighted row in the visible (filtered) suggestion list.
    pub(crate) selection: Option<usize>,
}

impl PromptState {
    pub(crate) fn new(snippet_id: crate::domain::SnippetId, variables: Vec<Variable>) -> Self {
        debug_assert!(
            !variables.is_empty(),
            "PromptState requires at least one variable; callers must complete the snippet directly when variables.is_empty()"
        );
        Self {
            snippet_id,
            variables,
            index: 0,
            values: BTreeMap::new(),
            input: String::new(),
            suggestions: Vec::new(),
            error: None,
            selection: None,
        }
    }

    pub(crate) fn current_variable(&self) -> &Variable {
        debug_assert!(
            self.index < self.variables.len(),
            "PromptState index {} out of bounds (variables.len() = {})",
            self.index,
            self.variables.len()
        );
        let safe_idx = self.index.min(self.variables.len().saturating_sub(1));
        &self.variables[safe_idx]
    }

    pub(crate) fn current_value(&self) -> String {
        if !self.input.is_empty() {
            return self.input.clone();
        }
        self.selected_visible_suggestion()
            .cloned()
            .unwrap_or_default()
    }

    pub(crate) fn visible_suggestions(&self) -> Vec<&String> {
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
        let idx = self.selection.unwrap_or(0);
        visible.get(idx).copied()
    }

    /// Append `text` to the in-progress input, normalizing line endings.
    ///
    /// Bracketed paste typically delivers `\r` (or `\r\n`) for newlines because
    /// that's what the terminal saw on Enter; we translate to `\n` so the
    /// cursor math (`advance_cursor`) and the line-splitting renderer stay in
    /// sync. Also strips other control characters that would corrupt the
    /// inline viewport (e.g. ESC sequences nested in the paste).
    pub(crate) fn append_input(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        for ch in normalized.chars() {
            if ch == '\n' || !ch.is_control() {
                self.input.push(ch);
            }
        }
        self.reset_selection();
    }

    pub(crate) fn reset_selection(&mut self) {
        if self.visible_suggestions().is_empty() {
            self.selection = None;
        } else {
            self.selection = Some(0);
        }
    }

    pub(crate) fn move_cursor(&mut self, delta: i32) {
        let visible_len = self.visible_suggestions().len();
        if visible_len == 0 {
            self.selection = None;
            return;
        }
        let current = self.selection.unwrap_or(0) as i32;
        let next = (current + delta).clamp(0, visible_len as i32 - 1);
        self.selection = Some(next as usize);
    }
}

/// What should happen after a key event is handled in the prompt screen.
pub(crate) enum PromptTransition {
    /// Remain on the prompt screen; re-render.
    Stay,
    /// Esc or backspace past the first variable: return to the select screen.
    ToSelect,
    /// All variables filled and Enter pressed on the last one.
    Completed(ExecutionOutcome),
}

pub(crate) fn handle_prompt_key<P: SuggestionProvider>(
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
            PromptTransition::ToSelect
        }
        KeyCode::Backspace => {
            if prompt.input.pop().is_some() {
                prompt.reset_selection();
                PromptTransition::Stay
            } else if prompt.index > 0 {
                prompt.index -= 1;
                load_prompt_state(prompt, provider, cwd);
                *status = prompt.error.clone();
                PromptTransition::Stay
            } else {
                PromptTransition::ToSelect
            }
        }
        // Alt+Enter — insert a literal newline rather than submitting. Lets the
        // user type multi-line values (e.g. multi-paragraph LLM prompts).
        KeyCode::Enter if key.modifiers.contains(KeyModifiers::ALT) => {
            prompt.input.push('\n');
            prompt.reset_selection();
            PromptTransition::Stay
        }
        // Ctrl+J — same intent as Alt+Enter, for terminals that don't deliver
        // Alt+Enter as a distinct key event. Ctrl+J is LF on the wire.
        KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            prompt.input.push('\n');
            prompt.reset_selection();
            PromptTransition::Stay
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
            if let Some(selected) = prompt.selected_visible_suggestion().cloned()
                && prompt.input != selected
            {
                prompt.input = selected;
                prompt.reset_selection();
                return PromptTransition::Stay;
            }
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
                load_prompt_state(prompt, provider, cwd);
                *status = prompt.error.clone();
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
    load_prompt_state(prompt, provider, cwd);
    *status = prompt.error.clone();
}

/// Refresh `prompt` for the variable at `prompt.index`: restore any
/// previously saved value (or the variable default), then fetch suggestions
/// from the provider. Errors from the provider are stored in `prompt.error`
/// rather than propagated, so the user can still type freely.
pub(crate) fn load_prompt_state<P: SuggestionProvider>(
    prompt: &mut PromptState,
    provider: &P,
    cwd: &Path,
) {
    let variable = prompt.current_variable().clone();
    prompt.input = prompt
        .values
        .get(&variable.name)
        .cloned()
        .or_else(|| default_input(&variable))
        .or_else(|| provider.default_input(&variable))
        .unwrap_or_default();
    prompt.error = None;
    prompt.suggestions = match provider.suggestions(&variable, cwd) {
        Ok(values) => values,
        Err(err) => {
            prompt.error = Some(err.to_string());
            Vec::new()
        }
    };
    prompt.reset_selection();
}

/// One piece of a parsed snippet template.
enum Segment<'a> {
    Literal(&'a str),
    Placeholder { name: &'a str, raw: &'a str },
}

/// Tokenize a template into literal runs and `<@name[:source]>` placeholders.
/// Malformed `<@...>` (rejected by [`placeholder_name`]) stays inside the
/// surrounding literal — callers reproduce it verbatim that way.
fn template_segments(template: &str) -> Vec<Segment<'_>> {
    let mut out = Vec::new();
    let bytes = template.as_bytes();
    let mut literal_start = 0;
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'<' && bytes[i + 1] == b'@' {
            let inner_start = i + 2;
            if let Some(end_offset) = template[inner_start..].find('>') {
                let inner_end = inner_start + end_offset;
                if let Some(name) = placeholder_name(&template[inner_start..inner_end]) {
                    if literal_start < i {
                        out.push(Segment::Literal(&template[literal_start..i]));
                    }
                    out.push(Segment::Placeholder {
                        name,
                        raw: &template[i..=inner_end],
                    });
                    i = inner_end + 1;
                    literal_start = i;
                    continue;
                }
            }
        }
        let ch = template[i..]
            .chars()
            .next()
            .expect("byte index sits on a char boundary");
        i += ch.len_utf8();
    }
    if literal_start < i {
        out.push(Segment::Literal(&template[literal_start..i]));
    }
    out
}

/// Substitute variable values into a snippet template string.
///
/// Each `<@name[:source]>` placeholder whose `name` appears in `values` is
/// replaced by its value; unrecognised or unfilled placeholders are kept
/// verbatim so partial renders remain valid shell.
pub fn render_command(template: &str, values: &BTreeMap<String, String>) -> String {
    let mut out = String::with_capacity(template.len());
    for segment in template_segments(template) {
        match segment {
            Segment::Literal(text) => out.push_str(text),
            Segment::Placeholder { name, raw } => match values.get(name) {
                Some(value) => out.push_str(value),
                None => out.push_str(raw),
            },
        }
    }
    out
}

pub(crate) fn render_command_text(
    template: &str,
    values: &BTreeMap<String, String>,
    active_variable: Option<&str>,
    theme: &Theme,
) -> Text<'static> {
    let mut chunks = Vec::new();
    for segment in template_segments(template) {
        match segment {
            Segment::Literal(text) => chunks.push(StyledChunk::plain(text.to_string())),
            Segment::Placeholder { name, raw } => {
                let is_active = Some(name) == active_variable;
                match values.get(name) {
                    Some(value) => {
                        let style = if is_active {
                            active_prompt_style(theme)
                        } else {
                            Style::default()
                        };
                        chunks.push(StyledChunk::new(value.clone(), style));
                    }
                    None => {
                        let style = if is_active {
                            active_prompt_style(theme)
                        } else {
                            placeholder_prompt_style(theme)
                        };
                        chunks.push(StyledChunk::new(raw.to_string(), style));
                    }
                }
            }
        }
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

pub(crate) fn active_prompt_style(theme: &Theme) -> Style {
    theme.active_prompt
}

pub(crate) fn placeholder_prompt_style(theme: &Theme) -> Style {
    theme.placeholder
}

fn default_input(variable: &Variable) -> Option<String> {
    match variable {
        Variable {
            source: crate::domain::VariableSource::Default(value),
            ..
        } => Some(value.clone()),
        _ => None,
    }
}

pub(crate) fn cursor_in_template(
    template: &str,
    values: &BTreeMap<String, String>,
    active_variable: &str,
) -> (u16, u16) {
    let mut col: u16 = 0;
    let mut row: u16 = 0;
    for segment in template_segments(template) {
        match segment {
            Segment::Literal(text) => advance_cursor(&mut col, &mut row, text),
            Segment::Placeholder { name, raw } => {
                if name == active_variable {
                    if let Some(val) = values.get(name) {
                        advance_cursor(&mut col, &mut row, val);
                    }
                    return (col, row);
                }
                let rendered = values.get(name).map(String::as_str).unwrap_or(raw);
                advance_cursor(&mut col, &mut row, rendered);
            }
        }
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

/// Deduplicate variables by name, preserving first-occurrence order.
///
/// A snippet body can reference the same `<@name>` placeholder more than once
/// (e.g. `<@file>` appears twice). The user should only be prompted once.
pub(crate) fn unique_variables(variables: &[Variable]) -> Vec<Variable> {
    let mut seen = HashMap::new();
    let mut out = Vec::new();
    for variable in variables {
        if seen.insert(variable.name.clone(), ()).is_none() {
            out.push(variable.clone());
        }
    }
    out
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

/// Return suggestions for built-in variable names.
///
/// - `"file"` → sorted list of files in `cwd`
/// - `"directory"` → sorted list of directories in `cwd`
/// - anything else → empty list (not an error)
pub(crate) fn builtin_suggestions(name: &str, cwd: &Path) -> io::Result<Vec<String>> {
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

/// Run `bash -c <command>` in `cwd` and split stdout into suggestion strings.
///
/// Each real newline and each literal `\n` in the output is treated as a
/// separator, and blank items are dropped. Returns an error if the command
/// exits non-zero, including the stderr output in the error message.
pub(crate) fn command_suggestions(command: &str, cwd: &Path) -> io::Result<Vec<String>> {
    // `-c` (not `-lc`): a login shell would source the user's profile/bashrc,
    // whose startup output (e.g. `Agent pid NNNN` from ssh-agent) leaks into
    // stdout as fake suggestions and whose prompts (e.g. ssh-add passphrase)
    // block on captured stdin.
    let output = Command::new("bash")
        .arg("-c")
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
