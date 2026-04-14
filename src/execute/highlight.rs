use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};

pub(crate) fn highlight_shell(body: &str) -> Text<'static> {
    Text::from(body.lines().map(highlight_shell_line).collect::<Vec<_>>())
}

fn highlight_shell_line(line: &str) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut i = 0;
    let mut plain_start = 0;

    while i < line.len() {
        if let Some((len, span)) = try_highlight_match(line, i) {
            if plain_start < i {
                spans.push(Span::raw(line[plain_start..i].to_string()));
            }
            spans.push(span);
            i += len;
            plain_start = i;
        } else {
            i += line[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
        }
    }
    if plain_start < line.len() {
        spans.push(Span::raw(line[plain_start..].to_string()));
    }
    Line::from(spans)
}

fn try_highlight_match(line: &str, i: usize) -> Option<(usize, Span<'static>)> {
    let rest = &line[i..];
    let at_word_start = i == 0
        || if line.as_bytes()[i - 1].is_ascii_whitespace() {
            true
        } else {
            let pb = line.as_bytes()[i - 1];
            !pb.is_ascii_alphanumeric() && pb != b'_'
        };

    if rest.starts_with("<@")
        && let Some(end) = rest.find('>')
    {
        let token = &rest[..end + 1];
        return Some((
            token.len(),
            Span::styled(
                token.to_string(),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        ));
    }

    if rest.starts_with('#') {
        return Some((
            rest.len(),
            Span::styled(rest.to_string(), Style::default().fg(Color::DarkGray)),
        ));
    }

    if let Some(stripped) = rest.strip_prefix('"') {
        let end = stripped.find('"').map(|e| e + 2).unwrap_or(rest.len());
        return Some((
            end,
            Span::styled(rest[..end].to_string(), Style::default().fg(Color::Green)),
        ));
    }

    if let Some(stripped) = rest.strip_prefix('\'') {
        let end = stripped.find('\'').map(|e| e + 2).unwrap_or(rest.len());
        return Some((
            end,
            Span::styled(rest[..end].to_string(), Style::default().fg(Color::Green)),
        ));
    }

    if let Some(inner) = rest.strip_prefix('$') {
        let end = if rest.starts_with("${") {
            rest.find('}').map(|e| e + 1).unwrap_or(rest.len())
        } else {
            let e = inner
                .find(|c: char| !c.is_alphanumeric() && c != '_')
                .unwrap_or(inner.len());
            if e == 0 {
                return None;
            }
            e + 1
        };
        return Some((
            end,
            Span::styled(rest[..end].to_string(), Style::default().fg(Color::Cyan)),
        ));
    }

    if at_word_start && rest.starts_with('-') && rest.as_bytes().get(1).copied() != Some(b' ') {
        let end = rest.find([' ', '=', '\'', '"', ';']).unwrap_or(rest.len());
        if end > 1 {
            return Some((
                end,
                Span::styled(rest[..end].to_string(), Style::default().fg(Color::Blue)),
            ));
        }
    }

    if at_word_start {
        const KEYWORDS: &[&str] = &[
            "if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case",
            "esac", "in", "function", "return", "local", "export", "source", "readonly", "declare",
            "unset",
        ];
        for kw in KEYWORDS {
            if rest.starts_with(kw) {
                let after = kw.len();
                let end_ok = after == rest.len()
                    || if rest.as_bytes()[after].is_ascii_whitespace() {
                        true
                    } else {
                        let b = rest.as_bytes()[after];
                        !b.is_ascii_alphanumeric() && b != b'_'
                    };
                if end_ok {
                    return Some((
                        after,
                        Span::styled(kw.to_string(), Style::default().fg(Color::Magenta)),
                    ));
                }
            }
        }
    }

    None
}
