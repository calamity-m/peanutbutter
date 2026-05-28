//! Helpers for applying fuzzy-search highlights to ratatui text.
//!
//! Search matching reports character positions in plain strings, while previews
//! often arrive as styled `Text`. This module bridges those shapes without
//! dropping syntax highlighting or markdown styling that was already present.

use crate::fuzzy::{FuzzyScorer, build_pattern};
use crate::search;
use nucleo_matcher::pattern::Pattern;
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};

/// A compiled search term plus the field it should highlight, if any.
pub(super) struct HighlightPattern {
    /// The query field this pattern is scoped to.
    pub(super) field: Option<search::QueryField>,
    /// The fuzzy matcher pattern built from the user-entered term.
    pub(super) pattern: Pattern,
}

/// Compiles the current query into field-aware highlight patterns.
pub(super) fn compile_highlight_patterns(query: &str) -> Vec<HighlightPattern> {
    search::highlight_terms(query)
        .into_iter()
        .map(|term| HighlightPattern {
            field: term.field,
            pattern: build_pattern(&term.value),
        })
        .collect()
}

/// Returns sorted, deduplicated character positions matched in `haystack`.
///
/// Patterns scoped to a different field are skipped. Unscoped patterns apply to
/// every field so broad fuzzy searches still highlight preview text.
pub(super) fn match_positions(
    scorer: &mut FuzzyScorer,
    patterns: &[HighlightPattern],
    field: Option<search::QueryField>,
    haystack: &str,
) -> Vec<usize> {
    let mut indices = Vec::new();
    for pattern in patterns {
        if pattern.field.is_some() && pattern.field != field {
            continue;
        }
        if let Some(mut matched) = scorer.indices(&pattern.pattern, haystack) {
            indices.append(&mut matched);
        }
    }
    indices.sort_unstable();
    indices.dedup();
    indices
}

/// Converts plain text into spans with `highlight_style` patched onto matches.
pub(super) fn highlighted_spans(
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

/// A single rendered character with its resolved style.
#[derive(Clone, Copy)]
pub(super) struct StyledChar {
    /// The rendered character.
    pub(super) ch: char,
    /// The effective style for the character after line/span styles are merged.
    pub(super) style: Style,
}

/// Applies highlights to existing styled text while preserving prior styling.
pub(super) fn highlight_text(
    text: Text<'static>,
    indices: &[usize],
    highlight_style: Style,
) -> Text<'static> {
    if indices.is_empty() {
        return text;
    }

    let mut chars = text_to_styled_chars(&text);
    for idx in indices {
        if let Some(styled) = chars.get_mut(*idx)
            && styled.ch != '\n'
        {
            // Preserve syntax/markdown styling and layer the search highlight on top.
            styled.style = styled.style.patch(highlight_style);
        }
    }
    styled_chars_to_text(chars)
}

/// Flattens ratatui text into its visible plain-text representation.
pub(super) fn text_plain(text: &Text<'_>) -> String {
    text_to_styled_chars(text)
        .into_iter()
        .map(|styled| styled.ch)
        .collect()
}

/// Flattens ratatui text into per-character style records.
pub(super) fn text_to_styled_chars(text: &Text<'_>) -> Vec<StyledChar> {
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

/// Rebuilds ratatui text from per-character style records.
pub(super) fn styled_chars_to_text(chars: Vec<StyledChar>) -> Text<'static> {
    let mut lines = vec![Line::default()];
    let mut current = String::new();
    let mut current_style = Style::default();
    let mut has_style = false;

    // Coalesce adjacent characters with the same style back into spans so the
    // rebuilt `Text` stays compact enough for repeated preview rendering.
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
