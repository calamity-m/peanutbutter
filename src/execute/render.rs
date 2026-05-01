use ansi_to_tui::IntoText;
use ratatui::Frame;
use ratatui::layout::Position;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};

use crate::browse::{BrowseEntry, DirNode};
use crate::fuzzy::{FuzzyScorer, build_pattern};
use crate::index::IndexedSnippet;
use crate::search;
use nucleo_matcher::pattern::Pattern;

use super::app::{ExecutionApp, NavigationMode, Screen, SuggestionProvider};
use super::highlight::highlight_shell;
use super::prompt::{PromptState, cursor_in_template, render_command_text, unique_variables};

enum PickerPreview<'a> {
    Snippet(&'a IndexedSnippet),
    Markdown(String),
    Empty,
}

impl<P: SuggestionProvider> ExecutionApp<P> {
    pub fn render(&mut self, frame: &mut Frame<'_>) {
        if matches!(self.screen, Screen::Select) {
            self.render_select(frame);
            return;
        }
        let Screen::Prompt(prompt) = &self.screen else {
            unreachable!()
        };
        self.render_prompt(frame, prompt);
    }

    fn render_select(&mut self, frame: &mut Frame<'_>) {
        let outer = frame.area();
        let border = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.border);
        frame.render_widget(border, outer);
        let area = Block::default().borders(Borders::ALL).inner(outer);
        let fuzzy_hits = search::rank(
            &self.index,
            &self.fuzzy.query,
            &self.frecency,
            &self.cwd,
            self.now,
            &self.search_config,
        );
        let highlight_pattern = matches!(self.nav_mode, NavigationMode::Fuzzy)
            .then(|| self.fuzzy.query.trim())
            .filter(|query| !query.is_empty())
            .map(build_pattern);
        let mut highlighter = FuzzyScorer::new();
        let browse_visible = self.browse.visible(&self.tree);
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),
                Constraint::Length(2),
                Constraint::Length(1),
            ])
            .split(area);
        let main = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
            .split(chunks[0]);
        if let Some(cell) = frame.buffer_mut().cell_mut(Position {
            x: main[1].x,
            y: outer.y,
        }) {
            cell.set_char('┬').set_style(self.theme.border);
        }

        let (prompt, stats, mode) = match self.nav_mode {
            NavigationMode::Fuzzy => (
                self.fuzzy.query.clone(),
                format!("{}/{}", fuzzy_hits.len(), self.index.len()),
                "Fuzzy",
            ),
            NavigationMode::Browse => (
                format!("{}{}", self.browse.path_display(), self.browse.input),
                browse_visible.len().to_string(),
                "Browse",
            ),
        };
        frame.render_widget(
            select_header(
                &self.theme,
                &prompt,
                &stats,
                mode,
                chunks[1].width,
                main[1].x.saturating_sub(chunks[1].x),
            ),
            chunks[1],
        );

        match self.nav_mode {
            NavigationMode::Fuzzy => {
                let total = fuzzy_hits.len();
                let selected = self.fuzzy.selected().unwrap_or(0);
                let padding = (main[0].height as usize).saturating_sub(total);
                let mut items: Vec<ListItem<'_>> =
                    (0..padding).map(|_| ListItem::new("")).collect();
                for (idx, hit) in fuzzy_hits.iter().enumerate().rev() {
                    let content = fuzzy_snippet_row_spans(
                        &self.theme,
                        hit.snippet,
                        highlight_pattern.as_ref(),
                        &mut highlighter,
                        idx == selected,
                    );
                    items.push(ListItem::new(snippet_list_line(
                        &self.theme,
                        idx,
                        total,
                        selected,
                        content,
                    )));
                }
                let visual = padding + total.saturating_sub(1).saturating_sub(selected);
                let items_len = items.len();
                clamp_list_offset(&mut self.fuzzy_list, items_len, main[0].height as usize);
                self.fuzzy_list.select(Some(visual));
                frame.render_stateful_widget(List::new(items), main[0], &mut self.fuzzy_list);
            }
            NavigationMode::Browse => {
                let total = browse_visible.len();
                let selected = self.browse.selection.unwrap_or(0);
                let padding = (main[0].height as usize).saturating_sub(total);
                let mut items: Vec<ListItem<'_>> =
                    (0..padding).map(|_| ListItem::new("")).collect();
                items.extend(browse_visible.iter().enumerate().rev().map(|(idx, entry)| {
                    let label = match entry {
                        BrowseEntry::Directory(name) => format!("{name}/"),
                        BrowseEntry::Snippet(snippet) => snippet.name.clone(),
                    };
                    ListItem::new(snippet_list_line(
                        &self.theme,
                        idx,
                        total,
                        selected,
                        vec![Span::raw(label)],
                    ))
                }));
                let visual = padding + total.saturating_sub(1).saturating_sub(selected);
                let items_len = items.len();
                clamp_list_offset(&mut self.browse_list, items_len, main[0].height as usize);
                self.browse_list.select(Some(visual));
                frame.render_stateful_widget(List::new(items), main[0], &mut self.browse_list);
            }
        }

        let preview = match self.nav_mode {
            NavigationMode::Fuzzy => {
                let idx = self.fuzzy.selected().unwrap_or(0);
                fuzzy_hits
                    .get(idx)
                    .map(|hit| PickerPreview::Snippet(hit.snippet))
                    .unwrap_or(PickerPreview::Empty)
            }
            NavigationMode::Browse => self.browse_preview(&browse_visible),
        };
        frame.render_widget(
            Block::default()
                .borders(Borders::LEFT)
                .border_style(self.theme.divider),
            main[1],
        );
        let inner = ratatui::layout::Rect {
            x: main[1].x + 1,
            width: main[1].width.saturating_sub(1),
            ..main[1]
        };
        let preview_text = picker_preview_text(
            preview,
            inner.width as usize,
            highlight_pattern.as_ref(),
            &mut highlighter,
            &self.theme,
        );
        drop(fuzzy_hits);
        let total_lines = preview_text.height() as u16;
        let max_scroll = total_lines.saturating_sub(inner.height);
        self.preview_scroll = self.preview_scroll.min(max_scroll);
        frame.render_widget(
            Paragraph::new(preview_text).scroll((self.preview_scroll, 0)),
            inner,
        );

        let help = if let Some(status) = &self.status {
            status.clone()
        } else {
            match self.nav_mode {
                NavigationMode::Fuzzy => {
                    "enter accept  ctrl+e edit  ctrl+j/k/↑↓ preview  ctrl+t browse  esc cancel"
                        .to_string()
                }
                NavigationMode::Browse => {
                    let selected_is_dir = browse_visible
                        .get(self.browse.selection.unwrap_or(0))
                        .map(|e| matches!(e, BrowseEntry::Directory(_)))
                        .unwrap_or(false);
                    if selected_is_dir {
                        "tab complete  enter open  ctrl+j/k/↑↓ preview  ctrl+t search  esc cancel"
                            .to_string()
                    } else {
                        "tab complete  enter accept  ctrl+e edit  ctrl+j/k/↑↓ preview  ctrl+t search  esc cancel".to_string()
                    }
                }
            }
        };
        frame.render_widget(chrome_line(&self.theme, help), chunks[2]);

        if matches!(self.nav_mode, NavigationMode::Fuzzy) {
            let x = chunks[1].x + 2 + self.fuzzy.cursor_col() as u16;
            frame.set_cursor_position(Position {
                x,
                y: chunks[1].y + 1,
            });
        }
    }

    fn render_prompt(&self, frame: &mut Frame<'_>, prompt: &PromptState) {
        let outer = frame.area();
        let border = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.border);
        frame.render_widget(border, outer);
        let area = Block::default().borders(Borders::ALL).inner(outer);
        let preview = self.prompt_preview_text(prompt);
        let cmd_height = (preview.lines.len() as u16).max(1);

        let (cmd_area, status_area, sugg_area, help_area) = if prompt.suggestions.is_empty() {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(cmd_height),
                    Constraint::Length(1),
                    Constraint::Length(1),
                ])
                .split(area);
            (chunks[0], chunks[1], None, chunks[2])
        } else {
            let max_sugg = area.height.saturating_sub(cmd_height + 2);
            let sugg_height = (prompt.visible_suggestions().len() as u16)
                .min(max_sugg)
                .max(1);
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(cmd_height),
                    Constraint::Length(1),
                    Constraint::Length(sugg_height),
                    Constraint::Length(1),
                ])
                .split(area);
            (chunks[0], chunks[1], Some(chunks[2]), chunks[3])
        };
        frame.render_widget(Paragraph::new(preview).wrap(Wrap { trim: false }), cmd_area);

        let variable = prompt.current_variable();
        let label = prompt.error.as_deref().unwrap_or(variable.name.as_str());
        frame.render_widget(
            Paragraph::new(Line::from(prompt_status_line(
                &self.theme,
                prompt.index + 1,
                prompt.variables.len(),
                label,
                status_area.width,
            ))),
            status_area,
        );

        if let Some(area) = sugg_area {
            let visible = prompt.visible_suggestions();
            let total = visible.len();
            let selected = prompt.selection.unwrap_or(0);
            let items: Vec<ListItem<'_>> = visible
                .into_iter()
                .enumerate()
                .map(|(idx, value)| {
                    ListItem::new(snippet_list_line(
                        &self.theme,
                        idx,
                        total,
                        selected,
                        vec![Span::raw(value.to_string())],
                    ))
                })
                .collect();
            let mut list_state =
                ratatui::widgets::ListState::default().with_selected(prompt.selection);
            frame.render_stateful_widget(List::new(items), area, &mut list_state);
        }

        if let Some(snippet) = self.index.get(&prompt.snippet_id) {
            let mut values = prompt.values.clone();
            let value = prompt.current_value();
            if !value.is_empty() {
                values.insert(variable.name.clone(), value);
            }
            let (cursor_col, cursor_row) =
                cursor_in_template(snippet.body(), &values, &variable.name);
            frame.set_cursor_position(Position {
                x: cmd_area.x + cursor_col,
                y: cmd_area.y + cursor_row,
            });
        }

        frame.render_widget(
            chrome_line(
                &self.theme,
                "tab complete/next  shift+tab prev  enter accept  esc return",
            ),
            help_area,
        );
    }

    fn prompt_preview_text(&self, prompt: &PromptState) -> Text<'static> {
        let Some(snippet) = self.index.get(&prompt.snippet_id) else {
            return Text::default();
        };

        let mut values = prompt.values.clone();
        let value = prompt.current_value();
        if !value.is_empty() {
            values.insert(prompt.current_variable().name.clone(), value);
        }

        render_command_text(
            snippet.body(),
            &values,
            Some(prompt.current_variable().name.as_str()),
            &self.theme,
        )
    }

    fn browse_preview<'a>(&'a self, visible: &[BrowseEntry]) -> PickerPreview<'a> {
        let Some(entry) = visible.get(self.browse.selection.unwrap_or(0)) else {
            return PickerPreview::Empty;
        };
        match entry {
            BrowseEntry::Snippet(s) => self
                .index
                .get(&s.id)
                .map(PickerPreview::Snippet)
                .unwrap_or(PickerPreview::Empty),
            BrowseEntry::Directory(name) => {
                let mut path = self.browse.path.clone();
                path.push(name.clone());
                let Some(node) = self.tree.get(&path) else {
                    return PickerPreview::Empty;
                };
                PickerPreview::Markdown(container_preview_markdown(name, &path, node))
            }
        }
    }
}

fn preview_skin() -> termimad::MadSkin {
    let mut skin = termimad::MadSkin::default();
    for h in &mut skin.headers {
        h.align = termimad::Alignment::Left;
    }
    skin.code_block.left_margin = 2;
    skin.code_block.right_margin = 2;
    skin.code_block.compound_style.set_fg(termimad::gray(18));
    skin.code_block.compound_style.object_style.background_color = None;
    skin.inline_code.set_fg(termimad::gray(18));
    skin.inline_code.object_style.background_color = None;
    skin
}

fn render_markdown_text(markdown: &str, width: usize) -> Text<'static> {
    let skin = preview_skin();
    let fmt = termimad::FmtText::from(&skin, markdown, Some(width.max(3)));
    let ansi = fmt.to_string();
    ansi.into_text()
        .unwrap_or_else(|_| Text::from(ansi.clone()))
}

fn picker_preview_text(
    preview: PickerPreview<'_>,
    width: usize,
    pattern: Option<&Pattern>,
    scorer: &mut FuzzyScorer,
    theme: &crate::config::Theme,
) -> Text<'static> {
    match preview {
        PickerPreview::Snippet(snippet) => {
            render_snippet_preview_text(snippet, width, theme, pattern, scorer)
        }
        PickerPreview::Markdown(markdown) => render_markdown_text(&markdown, width),
        PickerPreview::Empty => Text::from("No selection"),
    }
}

fn container_preview_markdown(name: &str, path: &[String], node: &DirNode) -> String {
    let mut md = String::new();
    md.push_str("# ");
    md.push_str(name);
    md.push_str("/\n\n");
    md.push_str("**path** `/");
    md.push_str(&path.join("/"));
    md.push_str("/`\n\n---\n\n");

    if node.children.is_empty() && node.snippets.is_empty() {
        md.push_str("_(empty)_\n");
        return md;
    }

    for child_name in node.children.keys() {
        md.push_str("- `");
        md.push_str(child_name);
        md.push_str("/`\n");
    }
    for snippet in &node.snippets {
        md.push_str("- ");
        md.push_str(&snippet.name);
        md.push('\n');
    }
    md
}

fn chrome_line<'a, T: Into<Text<'a>>>(theme: &crate::config::Theme, text: T) -> Paragraph<'a> {
    Paragraph::new(text)
        .style(theme.chrome)
        .wrap(Wrap { trim: true })
}

fn select_header(
    theme: &crate::config::Theme,
    prompt: &str,
    stats: &str,
    mode: &str,
    width: u16,
    divider_col: u16,
) -> Paragraph<'static> {
    let prompt_line = Line::from(vec![
        Span::raw("> "),
        Span::styled(prompt.to_string(), theme.emphasis),
    ]);
    let prefix = format!("[{stats}] ─ ");
    let mode_label = format!("[ {mode} ]");
    let fixed_len = prefix.chars().count() + mode_label.chars().count() + 1;
    let total_right = (width as usize).saturating_sub(fixed_len);
    let right_start = fixed_len;

    let divider = divider_col as usize;
    let right_span = if divider >= right_start && divider < right_start + total_right {
        let pos = divider - right_start;
        let left_dashes = "─".repeat(pos);
        let right_dashes = "─".repeat(total_right.saturating_sub(pos + 1));
        format!(" {left_dashes}┴{right_dashes}")
    } else {
        format!(" {}", "─".repeat(total_right))
    };

    let status_line = Line::from(vec![
        Span::styled(prefix, theme.divider),
        Span::styled(mode_label, theme.emphasis),
        Span::styled(right_span, theme.divider),
    ]);
    Paragraph::new(Text::from(vec![status_line, prompt_line]))
}

fn prompt_status_line(
    theme: &crate::config::Theme,
    idx: usize,
    total: usize,
    label: &str,
    width: u16,
) -> Vec<Span<'static>> {
    let prefix = format!("[{idx}/{total}] ─ ");
    let mode = format!("[ {label} ]");
    let used = prefix.chars().count() + mode.chars().count() + 1;
    let right = "─".repeat((width as usize).saturating_sub(used));
    vec![
        Span::raw(prefix),
        Span::styled(mode, theme.emphasis),
        Span::raw(format!(" {right}")),
    ]
}

fn snippet_list_line<'a>(
    theme: &crate::config::Theme,
    idx: usize,
    total: usize,
    selected: usize,
    content: Vec<Span<'a>>,
) -> Line<'a> {
    let w = digits(total);
    if idx == selected {
        let mut spans = vec![
            Span::styled("▌ ", theme.selected_marker),
            Span::styled(format!("{:>w$}  ", idx + 1), theme.selected_item),
        ];
        spans.extend(content);
        Line::from(spans)
    } else {
        let mut spans = vec![
            Span::raw("  "),
            Span::styled(format!("{:>w$}  ", idx + 1), theme.chrome),
        ];
        spans.extend(content);
        Line::from(spans)
    }
}

fn digits(n: usize) -> usize {
    if n == 0 { 1 } else { n.ilog10() as usize + 1 }
}

fn clamp_list_offset(state: &mut ratatui::widgets::ListState, items_len: usize, height: usize) {
    let max_offset = items_len.saturating_sub(height);
    if *state.offset_mut() > max_offset {
        *state.offset_mut() = max_offset;
    }
}

fn fuzzy_snippet_row_spans(
    theme: &crate::config::Theme,
    snippet: &IndexedSnippet,
    pattern: Option<&Pattern>,
    scorer: &mut FuzzyScorer,
    selected: bool,
) -> Vec<Span<'static>> {
    let base = if selected {
        theme.selected_item
    } else {
        Style::default()
    };
    let mut spans = highlighted_spans(
        snippet.name(),
        &match_positions(scorer, pattern, snippet.name()),
        base,
        theme.fuzzy_highlight,
    );
    spans.push(Span::styled("  [".to_string(), base));
    let path = snippet.relative_path_display();
    spans.extend(highlighted_spans(
        &path,
        &match_positions(scorer, pattern, &path),
        base,
        theme.fuzzy_highlight,
    ));
    spans.push(Span::styled("]".to_string(), base));
    spans
}

fn render_snippet_preview_text(
    snippet: &IndexedSnippet,
    width: usize,
    theme: &crate::config::Theme,
    pattern: Option<&Pattern>,
    scorer: &mut FuzzyScorer,
) -> Text<'static> {
    let mut text = Text::default();

    let mut title = vec![Span::styled("▍ ".to_string(), theme.fuzzy_highlight)];
    title.extend(highlighted_spans(
        snippet.name(),
        &match_positions(scorer, pattern, snippet.name()),
        theme.emphasis,
        theme.fuzzy_highlight,
    ));
    text.lines.push(Line::from(title));
    text.lines.push(Line::default());

    let path = snippet.relative_path_display();
    text.lines.push(metadata_line(
        "path",
        highlighted_spans(
            &path,
            &match_positions(scorer, pattern, &path),
            Style::default(),
            theme.fuzzy_highlight,
        ),
        theme,
    ));

    if !snippet.frontmatter.tags.is_empty() {
        let mut tag_spans = Vec::new();
        for (idx, tag) in snippet.frontmatter.tags.iter().enumerate() {
            if idx > 0 {
                tag_spans.push(Span::raw(" · "));
            }
            tag_spans.push(Span::raw("`"));
            tag_spans.extend(highlighted_spans(
                tag,
                &match_positions(scorer, pattern, tag),
                Style::default(),
                theme.fuzzy_highlight,
            ));
            tag_spans.push(Span::raw("`"));
        }
        text.lines.push(metadata_line("tags", tag_spans, theme));
    }

    let vars = unique_variables(&snippet.snippet.variables);
    if !vars.is_empty() {
        let mut var_spans = Vec::new();
        for (idx, var) in vars.iter().enumerate() {
            if idx > 0 {
                var_spans.push(Span::raw(" · "));
            }
            var_spans.push(Span::raw("`"));
            var_spans.push(Span::raw(var.name.clone()));
            var_spans.push(Span::raw("`"));
        }
        text.lines.push(metadata_line("vars", var_spans, theme));
    }

    text.lines.push(Line::default());

    let description = snippet.description().trim();
    if !description.is_empty() {
        text.lines.push(divider_line(theme));
        text.lines.push(Line::default());
        let description_text = render_markdown_text(description, width);
        let description_display = text_plain(&description_text);
        text.extend(highlight_text(
            description_text,
            &match_positions(scorer, pattern, &description_display),
            theme.fuzzy_highlight,
        ));
        text.lines.push(Line::default());
    }

    text.lines.push(divider_line(theme));
    let body_text = highlight_shell(snippet.body());
    text.extend(highlight_text(
        body_text,
        &match_positions(scorer, pattern, snippet.body()),
        theme.fuzzy_highlight,
    ));
    text
}

fn metadata_line(
    label: &str,
    mut value: Vec<Span<'static>>,
    theme: &crate::config::Theme,
) -> Line<'static> {
    let mut spans = vec![
        Span::styled(format!("{label} "), theme.chrome),
        Span::raw("`"),
    ];
    spans.append(&mut value);
    spans.push(Span::raw("`"));
    Line::from(spans)
}

fn divider_line(theme: &crate::config::Theme) -> Line<'static> {
    Line::from(vec![Span::styled("---".to_string(), theme.divider)])
}

fn match_positions(
    scorer: &mut FuzzyScorer,
    pattern: Option<&Pattern>,
    haystack: &str,
) -> Vec<usize> {
    pattern
        .and_then(|pattern| scorer.indices(pattern, haystack))
        .unwrap_or_default()
}

fn highlighted_spans(
    text: &str,
    indices: &[usize],
    base_style: Style,
    highlight_style: Style,
) -> Vec<Span<'static>> {
    if text.is_empty() {
        return Vec::new();
    }

    let mut spans = Vec::new();
    let mut highlights = indices.iter().copied().peekable();
    let mut current = String::new();
    let mut current_style = if highlights.peek() == Some(&0) {
        base_style.patch(highlight_style)
    } else {
        base_style
    };

    for (idx, ch) in text.chars().enumerate() {
        let is_highlight = highlights.peek() == Some(&idx);
        if is_highlight {
            highlights.next();
        }
        let style = if is_highlight {
            base_style.patch(highlight_style)
        } else {
            base_style
        };
        if style != current_style && !current.is_empty() {
            spans.push(Span::styled(std::mem::take(&mut current), current_style));
            current_style = style;
        }
        current.push(ch);
    }

    if !current.is_empty() {
        spans.push(Span::styled(current, current_style));
    }

    spans
}

#[derive(Clone, Copy)]
struct StyledChar {
    ch: char,
    style: Style,
}

fn highlight_text(text: Text<'static>, indices: &[usize], highlight_style: Style) -> Text<'static> {
    if indices.is_empty() {
        return text;
    }

    let mut chars = text_to_styled_chars(&text);
    for idx in indices {
        if let Some(styled) = chars.get_mut(*idx)
            && styled.ch != '\n'
        {
            styled.style = styled.style.patch(highlight_style);
        }
    }
    styled_chars_to_text(chars)
}

fn text_plain(text: &Text<'_>) -> String {
    text_to_styled_chars(text)
        .into_iter()
        .map(|styled| styled.ch)
        .collect()
}

fn text_to_styled_chars(text: &Text<'_>) -> Vec<StyledChar> {
    let mut chars = Vec::new();
    for (line_idx, line) in text.lines.iter().enumerate() {
        for span in &line.spans {
            let style = line.style.patch(span.style);
            for ch in span.content.chars() {
                chars.push(StyledChar { ch, style });
            }
        }
        if line_idx + 1 < text.lines.len() {
            chars.push(StyledChar {
                ch: '\n',
                style: Style::default(),
            });
        }
    }
    chars
}

fn styled_chars_to_text(chars: Vec<StyledChar>) -> Text<'static> {
    let mut lines = vec![Line::default()];
    let mut current = String::new();
    let mut current_style = Style::default();
    let mut has_style = false;

    let flush = |lines: &mut Vec<Line<'static>>,
                 current: &mut String,
                 current_style: &mut Style,
                 has_style: &mut bool| {
        if !current.is_empty() {
            lines
                .last_mut()
                .expect("text always has a line")
                .spans
                .push(Span::styled(std::mem::take(current), *current_style));
        }
        *current_style = Style::default();
        *has_style = false;
    };

    for styled in chars {
        if styled.ch == '\n' {
            flush(&mut lines, &mut current, &mut current_style, &mut has_style);
            lines.push(Line::default());
            continue;
        }

        if !has_style {
            current_style = styled.style;
            has_style = true;
        } else if styled.style != current_style {
            flush(&mut lines, &mut current, &mut current_style, &mut has_style);
            current_style = styled.style;
            has_style = true;
        }

        current.push(styled.ch);
    }

    flush(&mut lines, &mut current, &mut current_style, &mut has_style);

    Text::from(lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Frontmatter, Snippet, SnippetId};
    use std::path::PathBuf;

    fn snippet(name: &str, description: &str, body: &str, tags: &[&str]) -> IndexedSnippet {
        IndexedSnippet {
            path: PathBuf::from("demo.md"),
            snippet: Snippet {
                id: SnippetId::new("demo.md", "slug"),
                name: name.to_string(),
                description: description.to_string(),
                body: body.to_string(),
                variables: vec![],
            },
            relative_path: PathBuf::from("demo.md"),
            frontmatter: Frontmatter {
                name: None,
                description: None,
                tags: tags.iter().map(|tag| tag.to_string()).collect(),
            },
        }
    }

    #[test]
    fn fuzzy_row_highlights_name_and_path_matches() {
        let theme = crate::config::Theme::default();
        let mut scorer = FuzzyScorer::new();
        let pattern = build_pattern("dock");
        let spans = fuzzy_snippet_row_spans(
            &theme,
            &snippet("docker run", "desc", "echo hi", &[]),
            Some(&pattern),
            &mut scorer,
            false,
        );
        let rendered: String = spans.iter().map(|span| span.content.as_ref()).collect();
        assert_eq!(rendered, "docker run  [demo.md]");
        assert_eq!(spans[0].style, theme.fuzzy_highlight);
    }

    #[test]
    fn preview_highlights_markdown_description_text() {
        let theme = crate::config::Theme::default();
        let mut scorer = FuzzyScorer::new();
        let pattern = build_pattern("docker");
        let preview = render_snippet_preview_text(
            &snippet("Demo", "docker setup", "echo hi", &["ops"]),
            80,
            &theme,
            Some(&pattern),
            &mut scorer,
        );
        let chars = text_to_styled_chars(&preview);
        let rendered: String = chars.iter().map(|styled| styled.ch).collect();
        let byte_idx = rendered.find("docker").expect("description in preview");
        let idx = rendered[..byte_idx].chars().count();
        let docker: Vec<_> = chars[idx..idx + "docker".chars().count()].iter().collect();
        assert!(
            docker
                .iter()
                .all(|styled| styled.style == theme.fuzzy_highlight)
        );
    }

    #[test]
    fn preview_body_highlight_patches_existing_shell_style() {
        let theme = crate::config::Theme::default();
        let mut scorer = FuzzyScorer::new();
        let pattern = build_pattern("home");
        let preview = render_snippet_preview_text(
            &snippet("Demo", "", "echo $HOME", &[]),
            80,
            &theme,
            Some(&pattern),
            &mut scorer,
        );
        let chars = text_to_styled_chars(&preview);
        let rendered: String = chars.iter().map(|styled| styled.ch).collect();
        let byte_idx = rendered.find("$HOME").expect("shell body in preview");
        let idx = rendered[..byte_idx].chars().count() + 1;
        let home: Vec<_> = chars[idx..idx + "HOME".chars().count()].iter().collect();
        let expected = Style::default()
            .fg(ratatui::style::Color::Cyan)
            .patch(theme.fuzzy_highlight);
        assert!(home.iter().all(|styled| styled.style == expected));
    }
}
