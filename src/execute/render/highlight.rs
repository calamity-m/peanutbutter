use crate::fuzzy::{FuzzyScorer, build_pattern};
use crate::search;
use nucleo_matcher::pattern::Pattern;
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};

pub(super) struct HighlightPattern {
    pub(super) field: Option<search::QueryField>,
    pub(super) pattern: Pattern,
}

pub(super) fn compile_highlight_patterns(query: &str) -> Vec<HighlightPattern> {
    search::highlight_terms(query)
        .into_iter()
        .map(|term| HighlightPattern {
            field: term.field,
            pattern: build_pattern(&term.value),
        })
        .collect()
}

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

#[derive(Clone, Copy)]
pub(super) struct StyledChar {
    pub(super) ch: char,
    pub(super) style: Style,
}

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
            styled.style = styled.style.patch(highlight_style);
        }
    }
    styled_chars_to_text(chars)
}

pub(super) fn text_plain(text: &Text<'_>) -> String {
    text_to_styled_chars(text)
        .into_iter()
        .map(|styled| styled.ch)
        .collect()
}

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

pub(super) fn styled_chars_to_text(chars: Vec<StyledChar>) -> Text<'static> {
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
