use ansi_to_tui::IntoText;
use ratatui::Frame;
use ratatui::layout::Position;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};

use crate::browse::BrowseEntry;
use crate::index::IndexedSnippet;

use super::app::{ExecutionApp, NavigationMode, Screen, SuggestionProvider};
use super::highlight::highlight_shell;
use super::prompt::{PromptState, cursor_in_template, render_command_text, unique_variables};

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
        let area = frame.area();
        let fuzzy_hits = self.search_hits();
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
            select_header(&prompt, &stats, mode, chunks[1].width, main[1].x),
            chunks[1],
        );

        match self.nav_mode {
            NavigationMode::Fuzzy => {
                let total = fuzzy_hits.len();
                let selected = self.fuzzy.selected().unwrap_or(0);
                let padding = (main[0].height as usize).saturating_sub(total);
                let mut items: Vec<ListItem<'_>> =
                    (0..padding).map(|_| ListItem::new("")).collect();
                items.extend(fuzzy_hits.iter().enumerate().rev().map(|(idx, hit)| {
                    let content = format!(
                        "{}  [{}]",
                        hit.snippet.name(),
                        hit.snippet.relative_path_display()
                    );
                    ListItem::new(snippet_list_line(idx, total, selected, &content))
                }));
                let visual = padding + total.saturating_sub(1).saturating_sub(selected);
                let mut list_state =
                    ratatui::widgets::ListState::default().with_selected(Some(visual));
                frame.render_stateful_widget(List::new(items), main[0], &mut list_state);
            }
            NavigationMode::Browse => {
                let total = browse_visible.len();
                let selected = self.browse.list.selected().unwrap_or(0);
                let padding = (main[0].height as usize).saturating_sub(total);
                let mut items: Vec<ListItem<'_>> =
                    (0..padding).map(|_| ListItem::new("")).collect();
                items.extend(browse_visible.iter().enumerate().rev().map(|(idx, entry)| {
                    let label = match entry {
                        BrowseEntry::Directory(name) => format!("{name}/"),
                        BrowseEntry::Snippet(snippet) => snippet.name.clone(),
                    };
                    ListItem::new(snippet_list_line(idx, total, selected, &label))
                }));
                let visual = padding + total.saturating_sub(1).saturating_sub(selected);
                let mut list_state =
                    ratatui::widgets::ListState::default().with_selected(Some(visual));
                frame.render_stateful_widget(List::new(items), main[0], &mut list_state);
            }
        }

        let (picker_md, picker_body) = match self.nav_mode {
            NavigationMode::Fuzzy => extract_preview(self.selected_fuzzy_snippet()),
            NavigationMode::Browse => extract_preview(self.selected_browse_snippet()),
        };
        frame.render_widget(Block::default().borders(Borders::LEFT), main[1]);
        let inner = ratatui::layout::Rect {
            x: main[1].x + 1,
            width: main[1].width.saturating_sub(1),
            ..main[1]
        };
        self.render_snippet_preview(frame, inner, picker_md.as_deref(), picker_body.as_deref());

        let help = if let Some(status) = &self.status {
            status.clone()
        } else {
            match self.nav_mode {
                NavigationMode::Fuzzy => {
                    "enter accept  ctrl+j/k/↑↓ scroll preview  ctrl+t browse  esc cancel"
                        .to_string()
                }
                NavigationMode::Browse => {
                    let selected_is_dir = browse_visible
                        .get(self.browse.list.selected().unwrap_or(0))
                        .map(|e| matches!(e, BrowseEntry::Directory(_)))
                        .unwrap_or(false);
                    if selected_is_dir {
                        "tab complete  enter open  ctrl+j/k/↑↓ scroll preview  ctrl+t search  esc cancel".to_string()
                    } else {
                        "tab complete  enter accept  ctrl+j/k/↑↓ scroll preview  ctrl+t search  esc cancel".to_string()
                    }
                }
            }
        };
        frame.render_widget(chrome_line(help, Modifier::DIM), chunks[2]);

        if matches!(self.nav_mode, NavigationMode::Fuzzy) {
            let x = chunks[1].x + 2 + self.fuzzy.cursor_col() as u16;
            frame.set_cursor_position(Position {
                x,
                y: chunks[1].y + 1,
            });
        }
    }

    fn render_prompt(&self, frame: &mut Frame<'_>, prompt: &PromptState) {
        let area = frame.area();
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
            let selected = prompt.list.selected().unwrap_or(0);
            let items: Vec<ListItem<'_>> = visible
                .into_iter()
                .enumerate()
                .map(|(idx, value)| {
                    ListItem::new(snippet_list_line(idx, total, selected, value.as_str()))
                })
                .collect();
            let mut list_state = prompt.list;
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
                "tab next  shift+tab prev  enter accept  esc return",
                Modifier::DIM,
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
        )
    }

    fn render_snippet_preview(
        &mut self,
        frame: &mut Frame<'_>,
        area: ratatui::layout::Rect,
        markdown: Option<&str>,
        body: Option<&str>,
    ) {
        let text = match (markdown, body) {
            (Some(md), Some(body)) => {
                let mut text = render_markdown_text(md, area.width as usize);
                text.extend(highlight_shell(body));
                text
            }
            _ => Text::from("No snippet selected"),
        };

        let total_lines = text.height() as u16;
        let max_scroll = total_lines.saturating_sub(area.height);
        self.preview_scroll = self.preview_scroll.min(max_scroll);

        frame.render_widget(Paragraph::new(text).scroll((self.preview_scroll, 0)), area);
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

fn extract_preview(snippet: Option<&IndexedSnippet>) -> (Option<String>, Option<String>) {
    match snippet {
        Some(s) => (
            Some(snippet_preview_markdown(s)),
            Some(s.body().to_string()),
        ),
        None => (None, None),
    }
}

fn snippet_preview_markdown(snippet: &IndexedSnippet) -> String {
    let mut md = String::new();
    md.push_str("# ");
    md.push_str(snippet.name());
    md.push_str("\n\n");
    md.push_str("**path** `");
    md.push_str(&snippet.relative_path_display());
    md.push_str("`\n");

    if !snippet.frontmatter.tags.is_empty() {
        md.push_str("**tags** ");
        for (i, tag) in snippet.frontmatter.tags.iter().enumerate() {
            if i > 0 {
                md.push_str(" · ");
            }
            md.push('`');
            md.push_str(tag);
            md.push('`');
        }
        md.push('\n');
    }

    let vars = unique_variables(&snippet.snippet.variables);
    if !vars.is_empty() {
        md.push_str("**vars** ");
        for (i, var) in vars.iter().enumerate() {
            if i > 0 {
                md.push_str(" · ");
            }
            md.push('`');
            md.push_str(&var.name);
            md.push('`');
        }
        md.push('\n');
    }

    md.push('\n');

    let description = snippet.description().trim();
    if !description.is_empty() {
        md.push_str("---\n\n");
        md.push_str(description);
        md.push_str("\n\n");
    }

    md.push_str("---\n");
    md
}

fn chrome_line<'a, T: Into<Text<'a>>>(text: T, modifier: Modifier) -> Paragraph<'a> {
    Paragraph::new(text)
        .style(Style::default().add_modifier(modifier))
        .wrap(Wrap { trim: true })
}

fn select_header(
    prompt: &str,
    stats: &str,
    mode: &str,
    width: u16,
    divider_col: u16,
) -> Paragraph<'static> {
    let prompt_line = Line::from(vec![
        Span::raw("> "),
        Span::styled(
            prompt.to_string(),
            Style::default().add_modifier(Modifier::BOLD),
        ),
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
        Span::raw(prefix),
        Span::styled(mode_label, Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(right_span),
    ]);
    Paragraph::new(Text::from(vec![status_line, prompt_line]))
}

fn prompt_status_line(idx: usize, total: usize, label: &str, width: u16) -> Vec<Span<'static>> {
    let prefix = format!("[{idx}/{total}] ─ ");
    let mode = format!("[ {label} ]");
    let used = prefix.chars().count() + mode.chars().count() + 1;
    let right = "─".repeat((width as usize).saturating_sub(used));
    vec![
        Span::raw(prefix),
        Span::styled(mode, Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(format!(" {right}")),
    ]
}

fn snippet_list_line<'a>(idx: usize, total: usize, selected: usize, content: &str) -> Line<'a> {
    let w = digits(total);
    if idx == selected {
        Line::from(vec![
            Span::styled(
                "▌ ",
                Style::default()
                    .fg(ratatui::style::Color::Red)
                    .bg(ratatui::style::Color::DarkGray),
            ),
            Span::styled(
                format!("{:>w$}  {content}", idx + 1),
                Style::default()
                    .bg(ratatui::style::Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            ),
        ])
    } else {
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                format!("{:>w$}  ", idx + 1),
                Style::default().add_modifier(Modifier::DIM),
            ),
            Span::raw(content.to_string()),
        ])
    }
}

fn digits(n: usize) -> usize {
    if n == 0 { 1 } else { n.ilog10() as usize + 1 }
}
