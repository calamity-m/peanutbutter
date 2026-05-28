//! Preview text rendering for the execute picker.
//!
//! Picker previews combine snippet metadata, markdown descriptions, syntax
//! highlighted shell bodies, and fuzzy match overlays. The functions here return
//! ratatui `Text` so the root renderer can focus on layout.

use ansi_to_tui::IntoText;
use ratatui::text::{Line, Span, Text};

use crate::browse::DirNode;
use crate::config::Theme;
use crate::fuzzy::FuzzyScorer;
use crate::index::IndexedSnippet;
use crate::search;

use super::super::highlight::highlight_shell;
use super::super::prompt::unique_variables;
use super::highlight::{
    HighlightPattern, highlight_text, highlighted_spans, match_positions, text_plain,
};

/// The preview content selected by the active navigation mode.
pub(super) enum PickerPreview<'a> {
    /// A real snippet preview with metadata, description, and body.
    Snippet(&'a IndexedSnippet),
    /// Already assembled markdown for synthetic previews such as directories.
    Markdown(String),
    /// Empty selection placeholder.
    Empty,
}

/// Renders picker preview content into terminal text.
pub(super) fn picker_preview_text(
    preview: PickerPreview<'_>,
    width: usize,
    patterns: &[HighlightPattern],
    scorer: &mut FuzzyScorer,
    theme: &Theme,
) -> Text<'static> {
    match preview {
        PickerPreview::Snippet(snippet) => {
            render_snippet_preview_text(snippet, width, theme, patterns, scorer)
        }
        PickerPreview::Markdown(markdown) => render_markdown_text(&markdown, width),
        PickerPreview::Empty => Text::from("No selection"),
    }
}

/// Renders the full preview for a snippet row.
///
/// The preview includes searchable metadata first, then markdown description
/// text, then the shell-highlighted snippet body.
pub(super) fn render_snippet_preview_text(
    snippet: &IndexedSnippet,
    width: usize,
    theme: &Theme,
    patterns: &[HighlightPattern],
    scorer: &mut FuzzyScorer,
) -> Text<'static> {
    let mut text = Text::default();

    // Title and metadata use field-scoped highlights so operators such as
    // `name:foo` and `path:bar` only mark the intended preview sections.
    let mut title = vec![Span::styled("▍ ".to_string(), theme.fuzzy_highlight)];
    title.extend(highlighted_spans(
        snippet.name(),
        &match_positions(
            scorer,
            patterns,
            Some(search::QueryField::Name),
            snippet.name(),
        ),
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
            &match_positions(scorer, patterns, Some(search::QueryField::Path), &path),
            ratatui::style::Style::default(),
            theme.fuzzy_highlight,
        ),
        theme,
    ));

    if let Some(lang) = snippet.language() {
        text.lines.push(metadata_line(
            "lang",
            vec![Span::raw(lang.to_string())],
            theme,
        ));
    }

    if !snippet.frontmatter.tags.is_empty() {
        let mut tag_spans = Vec::new();
        for (idx, tag) in snippet.frontmatter.tags.iter().enumerate() {
            if idx > 0 {
                tag_spans.push(Span::raw(" · "));
            }
            tag_spans.push(Span::raw("`"));
            tag_spans.extend(highlighted_spans(
                tag,
                &match_positions(scorer, patterns, Some(search::QueryField::Tag), tag),
                ratatui::style::Style::default(),
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
        // Match against rendered markdown text, not the raw source, so character
        // indices line up with the ratatui spans we are about to restyle.
        text.extend(highlight_text(
            description_text,
            &match_positions(scorer, patterns, None, &description_display),
            theme.fuzzy_highlight,
        ));
        text.lines.push(Line::default());
    }

    text.lines.push(divider_line(theme));
    let body_text = highlight_shell(snippet.body());
    text.extend(highlight_text(
        body_text,
        &match_positions(
            scorer,
            patterns,
            Some(search::QueryField::Body),
            snippet.body(),
        ),
        theme.fuzzy_highlight,
    ));
    text
}

/// Renders markdown into ratatui text sized for the preview pane.
pub(super) fn render_markdown_text(markdown: &str, width: usize) -> Text<'static> {
    let skin = preview_skin();
    let markdown = markdown_links_for_terminal(markdown);
    let fmt = termimad::FmtText::from(&skin, &markdown, Some(width.max(3)));
    let ansi = fmt.to_string();
    ansi.into_text()
        .unwrap_or_else(|_| Text::from(ansi.clone()))
}

/// Builds a markdown preview for a directory or markdown container node.
pub(super) fn container_preview_markdown(name: &str, path: &[String], node: &DirNode) -> String {
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

/// Creates the markdown skin used by picker previews.
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

/// Rewrites markdown links into terminal-visible text.
///
/// `termimad` may otherwise render labels without enough context for URLs. This
/// keeps links useful inside a plain terminal preview pane.
fn markdown_links_for_terminal(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut rest = markdown;

    while let Some(open) = rest.find('[') {
        out.push_str(&rest[..open]);
        let label_start = open + 1;
        let Some(close_offset) = rest[label_start..].find(']') else {
            out.push_str(&rest[open..]);
            return out;
        };
        let close = label_start + close_offset;
        let url_start = close + 2;
        if !rest[close..].starts_with("](") {
            out.push_str(&rest[open..=close]);
            rest = &rest[close + 1..];
            continue;
        }
        let Some(url_end_offset) = rest[url_start..].find(')') else {
            out.push_str(&rest[open..]);
            return out;
        };
        let url_end = url_start + url_end_offset;
        let label = &rest[label_start..close];
        let url = &rest[url_start..url_end];
        if label.is_empty() || url.is_empty() {
            out.push_str(&rest[open..=url_end]);
        } else if label == url {
            out.push_str(label);
        } else {
            out.push_str(label);
            out.push_str(" (");
            out.push_str(url);
            out.push(')');
        }
        rest = &rest[url_end + 1..];
    }

    out.push_str(rest);
    out
}

/// Renders a metadata row with a label and inline-code-style value.
fn metadata_line(label: &str, mut value: Vec<Span<'static>>, theme: &Theme) -> Line<'static> {
    let mut spans = vec![
        Span::styled(format!("{label} "), theme.chrome),
        Span::raw("`"),
    ];
    spans.append(&mut value);
    spans.push(Span::raw("`"));
    Line::from(spans)
}

/// Renders the short markdown-style divider used inside previews.
fn divider_line(theme: &Theme) -> Line<'static> {
    Line::from(vec![Span::styled("---".to_string(), theme.divider)])
}
