use crate::domain::{Variable, VariableSource};
use crate::index::SnippetIndex;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io;
use std::path::Path;
use std::process::Command;

use super::ExecutionOutcome;
use super::app::SuggestionProvider;

#[derive(Debug)]
pub(crate) struct PromptState {
    pub(crate) snippet_id: crate::domain::SnippetId,
    pub(crate) variables: Vec<Variable>,
    pub(crate) index: usize,
    pub(crate) values: BTreeMap<String, String>,
    pub(crate) input: String,
    pub(crate) suggestions: Vec<String>,
    pub(crate) error: Option<String>,
    pub(crate) list: ratatui::widgets::ListState,
}

impl PromptState {
    pub(crate) fn new(snippet_id: crate::domain::SnippetId, variables: Vec<Variable>) -> Self {
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

    pub(crate) fn current_variable(&self) -> &Variable {
        &self.variables[self.index]
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
        let idx = self.list.selected().unwrap_or(0);
        visible.get(idx).copied()
    }

    pub(crate) fn reset_selection(&mut self) {
        if self.visible_suggestions().is_empty() {
            self.list.select(None);
        } else {
            self.list.select(Some(0));
        }
    }

    pub(crate) fn move_cursor(&mut self, delta: i32) {
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

pub(crate) enum PromptTransition {
    Stay,
    ToSelect,
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
                load_prompt_state(prompt, provider, cwd, status);
                PromptTransition::Stay
            } else {
                PromptTransition::ToSelect
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

pub(crate) fn load_prompt_state<P: SuggestionProvider>(
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

pub(crate) fn render_command_text(
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

pub(crate) fn active_prompt_style() -> Style {
    Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED)
}

pub(crate) fn placeholder_prompt_style() -> Style {
    Style::default().add_modifier(Modifier::DIM)
}

pub(crate) fn cursor_in_template(
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

pub(crate) fn command_suggestions(command: &str, cwd: &Path) -> io::Result<Vec<String>> {
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
