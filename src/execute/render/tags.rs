use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::Span;
use ratatui::widgets::{List, ListItem, Paragraph};

use crate::config::Theme;
use crate::index::TagKey;

use super::super::app::{TagListEntry, TagSnippetEntry, tag_label};
use super::{chrome_line, clamp_list_offset, snippet_list_line};

pub(super) struct TagView<'a> {
    pub(super) visible: &'a [TagListEntry],
    pub(super) snippets: &'a [TagSnippetEntry],
    pub(super) list_selected: usize,
    pub(super) drill_selected: usize,
    pub(super) drill: Option<&'a TagKey>,
    pub(super) only_untagged: bool,
}

pub(super) struct RenderChrome<'a> {
    pub(super) theme: &'a Theme,
    pub(super) list_state: &'a mut ratatui::widgets::ListState,
}

pub(super) fn render_tag_view(
    frame: &mut Frame<'_>,
    area: Rect,
    view: TagView<'_>,
    chrome: RenderChrome<'_>,
) {
    if let Some(tag) = view.drill {
        render_tag_drill_view(
            frame,
            area,
            tag,
            view.snippets,
            view.drill_selected,
            chrome.theme,
            chrome.list_state,
        );
        return;
    }

    if view.only_untagged {
        frame.render_widget(Paragraph::new("No tags yet"), area);
        return;
    }

    let total = view.visible.len();
    let padding = (area.height as usize).saturating_sub(total);
    let mut items: Vec<ListItem<'_>> = (0..padding).map(|_| ListItem::new("")).collect();
    items.extend(view.visible.iter().enumerate().rev().map(|(idx, entry)| {
        let label = if matches!(entry.key, TagKey::Untagged) {
            tag_label(&entry.key).to_string()
        } else {
            entry.label.clone()
        };
        ListItem::new(snippet_list_line(
            chrome.theme,
            idx,
            total,
            view.list_selected,
            vec![Span::raw(format!("{label} ({})", entry.count))],
        ))
    }));
    let visual = padding + total.saturating_sub(1).saturating_sub(view.list_selected);
    let items_len = items.len();
    clamp_list_offset(chrome.list_state, items_len, area.height as usize);
    chrome.list_state.select((total > 0).then_some(visual));
    frame.render_stateful_widget(List::new(items), area, chrome.list_state);
}

pub(super) fn tags_prompt(tag: Option<&TagKey>, drill_filter: &str) -> Option<String> {
    tag.map(|tag| format!("{}{}", tags_prompt_prefix(tag), drill_filter))
}

pub(super) fn tags_prompt_prefix_len(tag: &TagKey) -> usize {
    tags_prompt_prefix(tag).chars().count()
}

fn render_tag_drill_view(
    frame: &mut Frame<'_>,
    area: Rect,
    tag: &TagKey,
    snippets: &[TagSnippetEntry],
    selected: usize,
    theme: &Theme,
    list_state: &mut ratatui::widgets::ListState,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);
    frame.render_widget(
        chrome_line(theme, format!("tag: {}", tag_label(tag))),
        chunks[0],
    );

    let total = snippets.len();
    let padding = (chunks[1].height as usize).saturating_sub(total);
    let mut items: Vec<ListItem<'_>> = (0..padding).map(|_| ListItem::new("")).collect();
    items.extend(snippets.iter().enumerate().rev().map(|(idx, snippet)| {
        ListItem::new(snippet_list_line(
            theme,
            idx,
            total,
            selected,
            vec![Span::raw(snippet.name.clone())],
        ))
    }));
    let visual = padding + total.saturating_sub(1).saturating_sub(selected);
    let items_len = items.len();
    clamp_list_offset(list_state, items_len, chunks[1].height as usize);
    list_state.select((total > 0).then_some(visual));
    frame.render_stateful_widget(List::new(items), chunks[1], list_state);
}

fn tags_prompt_prefix(tag: &TagKey) -> String {
    format!("tag: {} > ", tag_label(tag))
}
