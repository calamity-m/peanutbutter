//! Rendering for the interactive execute UI.
//!
//! This module owns the high-level screen layout and delegates reusable drawing
//! details to focused submodules: fuzzy highlight handling, preview text
//! generation, and tag picker rendering.

mod highlight;
mod preview;
mod tags;

use ratatui::Frame;
use ratatui::layout::Position;
use ratatui::layout::Rect;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};

use crate::domain::SnippetId;
use crate::fuzzy::FuzzyScorer;
use crate::index::IndexedSnippet;
use crate::search;

use super::app::{ExecutionApp, NavigationMode, Screen, SuggestionProvider, tag_label};
use super::browse::{BrowseEntry, DirNode};
use super::prompt::{PromptState, cursor_in_template, render_command_text};

use highlight::{HighlightPattern, compile_highlight_patterns, highlighted_spans, match_positions};
use preview::{PickerPreview, container_preview_markdown, picker_preview_text};
use tags::{RenderChrome, TagView, render_tag_view, tags_prompt, tags_prompt_prefix_len};

impl<P: SuggestionProvider> ExecutionApp<P> {
    /// Renders the current execute screen into the provided terminal frame.
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

    /// Renders the snippet picker screen for fuzzy, browse, and tag navigation.
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
            .then(|| compile_highlight_patterns(&self.fuzzy.query))
            .unwrap_or_default();
        let mut highlighter = FuzzyScorer::new();
        let browse_visible = self.browse.visible(&self.tree);
        let tags_visible = self.visible_tags();
        let tag_snippets = self.visible_tag_snippets();
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
            // The preview pane's left border intersects the header border; patch
            // the shared cell so the box drawing remains continuous.
            cell.set_char('┬').set_style(self.theme.border);
        }

        let (prompt, stats, mode) = match self.nav_mode {
            NavigationMode::Fuzzy => (
                self.fuzzy.query.clone(),
                format!("{}/{}", fuzzy_hits.len(), self.index.len()),
                "Fuzzy",
            ),
            NavigationMode::Browse => (
                format!("{}{}", self.browse.path_display(), self.browse.input()),
                browse_visible.len().to_string(),
                "Browse",
            ),
            NavigationMode::Tags => (
                tags_prompt(self.tags.drill(), self.tags.drill_filter())
                    .unwrap_or_else(|| self.tags.filter().to_string()),
                self.tags
                    .drill()
                    .map(|_| tag_snippets.len().to_string())
                    .unwrap_or_else(|| format!("{}/{}", tags_visible.len(), self.tag_index.len())),
                "Tags",
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
                // The picker is bottom-aligned, so rows are rendered in reverse
                // and the selected logical index is translated to visual space.
                for (idx, hit) in fuzzy_hits.iter().enumerate().rev() {
                    let content = fuzzy_snippet_row_spans(
                        &self.theme,
                        hit.snippet,
                        &highlight_pattern,
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
                let selected = self.browse.selection().unwrap_or(0);
                let padding = (main[0].height as usize).saturating_sub(total);
                let mut items: Vec<ListItem<'_>> =
                    (0..padding).map(|_| ListItem::new("")).collect();
                let current_dir = self.tree.get(self.browse.path());
                // Keep browse mode visually aligned with fuzzy mode by anchoring
                // short lists at the bottom of the pane.
                items.extend(browse_visible.iter().enumerate().rev().map(|(idx, entry)| {
                    let label = match entry {
                        BrowseEntry::Directory(name) => {
                            let child = current_dir.and_then(|d| d.children.get(name));
                            let count = child.map(|c| c.recursive_count).unwrap_or(0);
                            let is_file = child.is_some_and(|c| c.children.is_empty());
                            if is_file {
                                let stem = name.strip_suffix(".md").unwrap_or(name);
                                format!("{stem} ({count})")
                            } else {
                                format!("{name}/ ({count})")
                            }
                        }
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
            NavigationMode::Tags => {
                render_tag_view(
                    frame,
                    main[0],
                    TagView {
                        visible: &tags_visible,
                        snippets: &tag_snippets,
                        list_selected: self.tags.list_selection().unwrap_or(0),
                        drill_selected: self.tags.drill_selection().unwrap_or(0),
                        drill: self.tags.drill(),
                        only_untagged: self.tag_index.len() == 1
                            && self.tag_index.contains_key(&crate::index::TagKey::Untagged),
                    },
                    RenderChrome {
                        theme: &self.theme,
                        list_state: &mut self.tags_list,
                    },
                );
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
            NavigationMode::Tags => {
                if self.tags.drill().is_some() {
                    self.selected_tag_snippet()
                        .map(PickerPreview::Snippet)
                        .unwrap_or(PickerPreview::Empty)
                } else {
                    self.tag_list_preview(&tags_visible)
                }
            }
        };
        frame.render_widget(
            Block::default()
                .borders(Borders::LEFT)
                .border_style(self.theme.divider),
            main[1],
        );
        let inner = Rect {
            x: main[1].x + 1,
            width: main[1].width.saturating_sub(1),
            ..main[1]
        };
        let preview_text = picker_preview_text(
            preview,
            inner.width as usize,
            &highlight_pattern,
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
                        .get(self.browse.selection().unwrap_or(0))
                        .map(|e| matches!(e, BrowseEntry::Directory(_)))
                        .unwrap_or(false);
                    if selected_is_dir {
                        "tab complete  enter open  ctrl+j/k/↑↓ preview  ctrl+t tags  esc cancel"
                            .to_string()
                    } else {
                        "tab complete  enter accept  ctrl+e edit  ctrl+j/k/↑↓ preview  ctrl+t tags  esc cancel".to_string()
                    }
                }
                NavigationMode::Tags => {
                    if self.tags.drill().is_some() {
                        "type filter  enter accept  esc tags  backspace clear/back  ctrl+t search"
                            .to_string()
                    } else {
                        "type filter  enter open  ctrl+j/k/↑↓ preview  ctrl+t search  esc cancel"
                            .to_string()
                    }
                }
            }
        };
        frame.render_widget(chrome_line(&self.theme, help), chunks[2]);

        if matches!(self.nav_mode, NavigationMode::Fuzzy | NavigationMode::Tags) {
            let cursor_col = match self.nav_mode {
                NavigationMode::Fuzzy => self.fuzzy.cursor_col(),
                NavigationMode::Tags => self.tags.drill().map_or_else(
                    || self.tags.cursor_col(),
                    |tag| tags_prompt_prefix_len(tag) + self.tags.drill_cursor_col(),
                ),
                NavigationMode::Browse => unreachable!(),
            };
            let x = chunks[1].x + 2 + cursor_col as u16;
            frame.set_cursor_position(Position {
                x,
                y: chunks[1].y + 1,
            });
        }
    }

    /// Renders the variable-entry prompt for a selected snippet.
    fn render_prompt(&self, frame: &mut Frame<'_>, prompt: &PromptState) {
        let outer = frame.area();
        let border = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.border);
        frame.render_widget(border, outer);
        let area = Block::default().borders(Borders::ALL).inner(outer);
        let preview = self.prompt_preview_text(prompt);
        let total_preview_lines = preview.lines.len() as u16;

        // Reserve rows for status (1) + help (1) + suggestions (variable). The
        // command preview takes whatever is left, capped so it never crowds out
        // the panes below — large multi-line values scroll within `cmd_area`
        // instead of pushing the suggestions/help off-screen.
        let (cmd_area, status_area, sugg_area, help_area) = if prompt.suggestions.is_empty() {
            let cmd_max = area.height.saturating_sub(2).max(1);
            let cmd_height = total_preview_lines.max(1).min(cmd_max);
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
            let visible_sugg = prompt.visible_suggestions().len() as u16;
            // Cap suggestions to leave at least 1 row for cmd + 2 for status/help.
            let max_sugg = area.height.saturating_sub(3);
            let sugg_height = visible_sugg.min(max_sugg).max(1);
            let cmd_max = area.height.saturating_sub(sugg_height + 2).max(1);
            let cmd_height = total_preview_lines.max(1).min(cmd_max);
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

        // Compute the cursor row first so we can scroll the preview to keep it
        // visible. `cursor_in_template` returns a row index in the unscrolled
        // text; subtract `scroll_y` when placing the on-screen cursor.
        let (cursor_col, cursor_row) = self
            .index
            .get(&prompt.snippet_id)
            .map(|snippet| {
                let mut values = prompt.values.clone();
                let value = prompt.current_value();
                if !value.is_empty() {
                    values.insert(prompt.current_variable().name.clone(), value);
                }
                cursor_in_template(snippet.body(), &values, &prompt.current_variable().name)
            })
            .unwrap_or((0, 0));
        let cmd_inner_height = cmd_area.height.max(1);
        let max_scroll = total_preview_lines.saturating_sub(cmd_inner_height);
        let scroll_y = cursor_row
            .saturating_sub(cmd_inner_height.saturating_sub(1))
            .min(max_scroll);
        frame.render_widget(
            Paragraph::new(preview)
                .wrap(Wrap { trim: false })
                .scroll((scroll_y, 0)),
            cmd_area,
        );

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

        if self.index.get(&prompt.snippet_id).is_some() {
            let visible_row = cursor_row.saturating_sub(scroll_y);
            frame.set_cursor_position(Position {
                x: cmd_area.x + cursor_col,
                y: cmd_area.y + visible_row,
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

    /// Builds the command preview shown while a prompt variable is being edited.
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

    /// Builds preview markdown for the currently selected tag row.
    fn tag_list_preview<'a>(&'a self, visible: &[super::app::TagListEntry]) -> PickerPreview<'a> {
        let Some(entry) = visible.get(self.tags.list_selection().unwrap_or(0)) else {
            return PickerPreview::Empty;
        };
        let Some(ids) = self.tag_index.get(&entry.key) else {
            return PickerPreview::Empty;
        };
        let mut md = String::new();
        md.push_str("# ");
        md.push_str(tag_label(&entry.key));
        md.push_str(&format!(" ({})\n\n---\n\n", ids.len()));
        if ids.is_empty() {
            md.push_str("_(no snippets)_\n");
            return PickerPreview::Markdown(md);
        }
        for id in ids {
            if let Some(snippet) = self.index.get(id) {
                md.push_str("- ");
                md.push_str(snippet.name());
                md.push('\n');
            }
        }
        PickerPreview::Markdown(md)
    }

    /// Builds the preview for the selected browse tree entry.
    fn browse_preview<'a>(&'a self, visible: &[BrowseEntry]) -> PickerPreview<'a> {
        let Some(entry) = visible.get(self.browse.selection().unwrap_or(0)) else {
            return PickerPreview::Empty;
        };
        match entry {
            BrowseEntry::Snippet(s) => self
                .index
                .get(&s.id)
                .map(PickerPreview::Snippet)
                .unwrap_or(PickerPreview::Empty),
            BrowseEntry::Directory(name) => {
                let mut path = self.browse.path().to_vec();
                path.push(name.clone());
                let Some(node) = self.tree.get(&path) else {
                    return PickerPreview::Empty;
                };
                let root = first_snippet_in_node(node)
                    .and_then(|id| self.index.get(id))
                    .map(|s| s.root_dir().to_path_buf());
                PickerPreview::Markdown(container_preview_markdown(
                    name,
                    &path,
                    node,
                    root.as_deref(),
                ))
            }
        }
    }
}

/// Finds the first snippet id in a DirNode subtree (breadth-first through snippets
/// then children), used to derive the root directory for directory previews.
fn first_snippet_in_node(node: &DirNode) -> Option<&SnippetId> {
    if let Some(s) = node.snippets.first() {
        return Some(&s.id);
    }
    for child in node.children.values() {
        if let Some(id) = first_snippet_in_node(child) {
            return Some(id);
        }
    }
    None
}

/// Creates a one-line chrome paragraph for status, help, and context bars.
fn chrome_line<'a, T: Into<Text<'a>>>(theme: &crate::config::Theme, text: T) -> Paragraph<'a> {
    Paragraph::new(text)
        .style(theme.chrome)
        .wrap(Wrap { trim: true })
}

/// Renders the picker header containing the query, result count, and active mode.
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

/// Renders the prompt progress/status line.
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

/// Renders one numbered picker row with the selected row marker when needed.
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

/// Returns the number of decimal digits needed to display `n`.
fn digits(n: usize) -> usize {
    if n == 0 { 1 } else { n.ilog10() as usize + 1 }
}

/// Keeps a ratatui list offset inside the current item and viewport bounds.
fn clamp_list_offset(state: &mut ratatui::widgets::ListState, items_len: usize, height: usize) {
    let max_offset = items_len.saturating_sub(height);
    if *state.offset_mut() > max_offset {
        *state.offset_mut() = max_offset;
    }
}

/// Renders fuzzy picker row text with field-aware query highlights.
fn fuzzy_snippet_row_spans(
    theme: &crate::config::Theme,
    snippet: &IndexedSnippet,
    patterns: &[HighlightPattern],
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
        &match_positions(
            scorer,
            patterns,
            Some(search::QueryField::Name),
            snippet.name(),
        ),
        base,
        theme.fuzzy_highlight,
    );
    spans.push(Span::styled("  [".to_string(), base));
    let path = snippet.relative_path_display();
    spans.extend(highlighted_spans(
        &path,
        &match_positions(scorer, patterns, Some(search::QueryField::Path), &path),
        base,
        theme.fuzzy_highlight,
    ));
    if let Some(lang) = snippet.language() {
        spans.push(Span::styled(format!(" · {lang}"), theme.chrome));
    }
    spans.push(Span::styled("]".to_string(), base));
    spans
}

#[cfg(test)]
mod tests {
    use super::highlight::{
        StyledChar, compile_highlight_patterns, text_plain, text_to_styled_chars,
    };
    use super::preview::render_snippet_preview_text;
    use super::*;
    use crate::domain::{Frontmatter, Snippet, SnippetId};
    use std::path::PathBuf;

    fn snippet(name: &str, description: &str, body: &str, tags: &[&str]) -> IndexedSnippet {
        snippet_with_language(name, description, body, tags, None)
    }

    fn snippet_with_language(
        name: &str,
        description: &str,
        body: &str,
        tags: &[&str],
        language: Option<&str>,
    ) -> IndexedSnippet {
        IndexedSnippet {
            path: PathBuf::from("demo.md"),
            snippet: Snippet {
                id: SnippetId::new("demo.md", "slug"),
                name: name.to_string(),
                description: description.to_string(),
                body: body.to_string(),
                variables: vec![],
                language: language.map(|l| l.to_string()),
            },
            relative_path: PathBuf::from("demo.md"),
            frontmatter: Frontmatter {
                name: None,
                description: None,
                tags: tags.iter().map(|tag| tag.to_string()).collect(),
                variables: Default::default(),
            },
        }
    }

    fn substr_styles(chars: &[StyledChar], needle: &str) -> Vec<Style> {
        let rendered: String = chars.iter().map(|styled| styled.ch).collect();
        let byte_idx = rendered.find(needle).expect("substring in rendered text");
        let idx = rendered[..byte_idx].chars().count();
        chars[idx..idx + needle.chars().count()]
            .iter()
            .map(|styled| styled.style)
            .collect()
    }

    fn assert_substr_style(chars: &[StyledChar], needle: &str, expected: Style) {
        let styles = substr_styles(chars, needle);
        assert!(
            styles.iter().all(|style| *style == expected),
            "expected {needle:?} to use {expected:?}, got {styles:?}"
        );
    }

    fn assert_substr_not_style(chars: &[StyledChar], needle: &str, unexpected: Style) {
        let styles = substr_styles(chars, needle);
        assert!(
            styles.iter().all(|style| *style != unexpected),
            "expected {needle:?} not to use {unexpected:?}"
        );
    }

    #[test]
    fn fuzzy_row_highlights_name_and_path_matches() {
        let theme = crate::config::Theme::default();
        let mut scorer = FuzzyScorer::new();
        let patterns = compile_highlight_patterns("dock");
        let spans = fuzzy_snippet_row_spans(
            &theme,
            &snippet("docker run", "desc", "echo hi", &[]),
            &patterns,
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
        let patterns = compile_highlight_patterns("docker");
        let preview = render_snippet_preview_text(
            &snippet("Demo", "docker setup", "echo hi", &["ops"]),
            80,
            &theme,
            &patterns,
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
    fn preview_highlights_single_operator_value_in_name() {
        let theme = crate::config::Theme::default();
        let mut scorer = FuzzyScorer::new();
        let patterns = compile_highlight_patterns("name:prompt");
        let preview = render_snippet_preview_text(
            &snippet("prompt helper", "desc", "echo hi", &[]),
            80,
            &theme,
            &patterns,
            &mut scorer,
        );
        let chars = text_to_styled_chars(&preview);
        let rendered: String = chars.iter().map(|styled| styled.ch).collect();
        let byte_idx = rendered.find("prompt").expect("name in preview");
        let idx = rendered[..byte_idx].chars().count();
        let prompt: Vec<_> = chars[idx..idx + "prompt".chars().count()].iter().collect();
        assert!(
            prompt
                .iter()
                .all(|styled| styled.style == theme.emphasis.patch(theme.fuzzy_highlight))
        );
    }

    #[test]
    fn preview_highlights_each_operator_in_its_field() {
        let theme = crate::config::Theme::default();
        let mut scorer = FuzzyScorer::new();
        let patterns = compile_highlight_patterns(
            "name:NameNeedle path:demo tag:TagNeedle snippet:BodyNeedle",
        );
        let preview = render_snippet_preview_text(
            &snippet(
                "NameNeedle helper",
                "DescriptionNeedle",
                "echo BodyNeedle",
                &["TagNeedle"],
            ),
            80,
            &theme,
            &patterns,
            &mut scorer,
        );
        let chars = text_to_styled_chars(&preview);

        assert_substr_style(
            &chars,
            "NameNeedle",
            theme.emphasis.patch(theme.fuzzy_highlight),
        );
        assert_substr_style(&chars, "demo", theme.fuzzy_highlight);
        assert_substr_style(&chars, "TagNeedle", theme.fuzzy_highlight);
        assert_substr_style(&chars, "BodyNeedle", theme.fuzzy_highlight);
        assert_substr_not_style(&chars, "DescriptionNeedle", theme.fuzzy_highlight);
    }

    #[test]
    fn preview_does_not_apply_field_operator_highlight_to_other_fields() {
        let theme = crate::config::Theme::default();
        let mut scorer = FuzzyScorer::new();
        let patterns = compile_highlight_patterns("snippet:NameNeedle");
        let preview = render_snippet_preview_text(
            &snippet("NameNeedle helper", "", "echo BodyNeedle", &[]),
            80,
            &theme,
            &patterns,
            &mut scorer,
        );
        let chars = text_to_styled_chars(&preview);

        assert_substr_not_style(
            &chars,
            "NameNeedle",
            theme.emphasis.patch(theme.fuzzy_highlight),
        );
    }

    #[test]
    fn preview_renders_markdown_description() {
        let theme = crate::config::Theme::default();
        let mut scorer = FuzzyScorer::new();
        let preview = render_snippet_preview_text(
            &snippet(
                "Demo",
                "### Setup\n\n- **Install** [Docker](https://example.com)\n- Run `pb`",
                "echo hi",
                &[],
            ),
            80,
            &theme,
            &[],
            &mut scorer,
        );
        let rendered = text_plain(&preview);

        assert!(rendered.contains("Setup"));
        assert!(rendered.contains("Install"));
        assert!(rendered.contains("Docker"));
        assert!(!rendered.contains("### Setup"));
        assert!(!rendered.contains("**Install**"));
        assert!(!rendered.contains("[Docker](https://example.com)"));
    }

    #[test]
    fn preview_preserves_plain_text_description() {
        let theme = crate::config::Theme::default();
        let mut scorer = FuzzyScorer::new();
        let preview = render_snippet_preview_text(
            &snippet("Demo", "plain text description", "echo hi", &[]),
            80,
            &theme,
            &[],
            &mut scorer,
        );

        assert!(text_plain(&preview).contains("plain text description"));
    }

    #[test]
    fn preview_renders_fenced_text_description_and_body() {
        let theme = crate::config::Theme::default();
        let mut scorer = FuzzyScorer::new();
        let preview = render_snippet_preview_text(
            &snippet(
                "Demo",
                "Example output:\n\n```text\nsource.txt -> dest.txt\n```",
                "cp <@source> <@dest>",
                &[],
            ),
            80,
            &theme,
            &[],
            &mut scorer,
        );
        let rendered = text_plain(&preview);

        assert!(rendered.contains("source.txt -> dest.txt"));
        assert!(rendered.contains("cp <@source> <@dest>"));
    }

    #[test]
    fn preview_does_not_render_body_as_markdown() {
        let theme = crate::config::Theme::default();
        let mut scorer = FuzzyScorer::new();
        let preview = render_snippet_preview_text(
            &snippet("Demo", "**description**", "echo **literal**", &[]),
            80,
            &theme,
            &[],
            &mut scorer,
        );

        assert!(text_plain(&preview).contains("echo **literal**"));
    }

    #[test]
    fn preview_body_highlight_patches_existing_shell_style() {
        let theme = crate::config::Theme::default();
        let mut scorer = FuzzyScorer::new();
        let patterns = compile_highlight_patterns("home");
        let preview = render_snippet_preview_text(
            &snippet("Demo", "", "echo $HOME", &[]),
            80,
            &theme,
            &patterns,
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

    #[test]
    fn fuzzy_row_includes_language_after_separator() {
        let theme = crate::config::Theme::default();
        let mut scorer = FuzzyScorer::new();
        let s = snippet_with_language("deploy", "desc", "kubectl apply", &[], Some("bash"));
        let spans = fuzzy_snippet_row_spans(&theme, &s, &[], &mut scorer, false);
        let rendered: String = spans.iter().map(|span| span.content.as_ref()).collect();
        assert!(
            rendered.contains(" · bash"),
            "expected language in row: {rendered}"
        );
    }

    #[test]
    fn preview_includes_lang_metadata_line() {
        let theme = crate::config::Theme::default();
        let mut scorer = FuzzyScorer::new();
        let s = snippet_with_language("deploy", "desc", "kubectl apply", &[], Some("bash"));
        let preview = render_snippet_preview_text(&s, 80, &theme, &[], &mut scorer);
        let rendered: String = preview
            .lines
            .iter()
            .flat_map(|line| line.spans.iter().map(|span| span.content.as_ref()))
            .collect::<Vec<_>>()
            .join("");
        assert!(
            rendered.contains("lang"),
            "expected lang label in preview: {rendered}"
        );
        assert!(
            rendered.contains("bash"),
            "expected language value in preview: {rendered}"
        );
    }
}
