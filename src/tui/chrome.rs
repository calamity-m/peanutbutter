//! Shared TUI chrome: outer border, top title bar, bottom footer hint row.
//!
//! Wraps a content area with shared border/divider styling so pb-binary
//! screens feel consistent without duplicating the styling glue.

use crate::config::Theme;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

/// One screen's chrome — the title strip up top, footer hint at the bottom,
/// outer border around everything. Holds no state.
pub struct Chrome<'a> {
    pub theme: &'a Theme,
    /// Label shown on the right of the title bar, e.g. `pb new`.
    pub mode: &'a str,
    /// Free-text title shown on the left (mode-specific context like the
    /// snippet name or "pick a command").
    pub title: &'a str,
    /// One-line footer with keybinding hints.
    pub footer: &'a str,
}

impl<'a> Chrome<'a> {
    /// Render the outer border (with title) plus a bottom footer row. Returns
    /// the inner content [`Rect`] (excluding border, title, and footer).
    pub fn render(&self, area: Rect, buf: &mut Buffer) -> Rect {
        let title_text = if self.mode.is_empty() {
            self.title.to_string()
        } else if self.title.is_empty() {
            format!(" {} ", self.mode)
        } else {
            format!(" {} — {} ", self.mode, self.title)
        };
        let border = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.border)
            .title(Span::styled(title_text, self.theme.emphasis));
        let inside = border.inner(area);
        border.render(area, buf);

        if inside.height == 0 {
            return inside;
        }

        // Bottom footer row, drawn inside the bordered area, with a divider
        // row separating it from the content above.
        if !self.footer.is_empty() && inside.height >= 2 {
            let divider_y = inside.y + inside.height - 2;
            draw_divider(area, divider_y, buf, self.theme);
            let footer_area = Rect {
                x: inside.x + 1,
                y: inside.y + inside.height - 1,
                width: inside.width.saturating_sub(2),
                height: 1,
            };
            Paragraph::new(Line::from(Span::styled(
                self.footer.to_string(),
                self.theme.chrome,
            )))
            .render(footer_area, buf);
        }

        // Content area = inside minus (footer + its divider), with a 1-col gutter.
        let footer_rows = if self.footer.is_empty() { 0 } else { 2 };
        let content_h = inside.height.saturating_sub(footer_rows);
        Rect {
            x: inside.x + 1,
            y: inside.y,
            width: inside.width.saturating_sub(2),
            height: content_h,
        }
    }
}

/// Draw a full-width `├────┤` divider row across `area` at row `y`. `area`
/// must be the *outer* bordered rect (the one passed to [`Chrome::render`])
/// so the T-junctions land on the border columns.
pub fn draw_divider(outer: Rect, y: u16, buf: &mut Buffer, theme: &Theme) {
    if y <= outer.y || y + 1 >= outer.y + outer.height {
        return;
    }
    let left = outer.x;
    let right = outer.x + outer.width - 1;
    if let Some(cell) = buf.cell_mut(Position { x: left, y }) {
        cell.set_char('├').set_style(theme.border);
    }
    if let Some(cell) = buf.cell_mut(Position { x: right, y }) {
        cell.set_char('┤').set_style(theme.border);
    }
    for x in (left + 1)..right {
        if let Some(cell) = buf.cell_mut(Position { x, y }) {
            cell.set_char('─').set_style(theme.divider);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;

    #[test]
    fn render_returns_inner_content_rect() {
        let theme = Theme::default();
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 10));
        let chrome = Chrome {
            theme: &theme,
            mode: "pb new",
            title: "pick a command",
            footer: "esc cancel",
        };
        let content = chrome.render(buf.area, &mut buf);
        // 1-col gutter on each side; bottom reserves 2 rows (divider + footer);
        // top border absorbs the title.
        assert_eq!(content.x, 2);
        assert_eq!(content.y, 1);
        assert_eq!(content.width, 36);
        assert_eq!(content.height, 6);
    }

    #[test]
    fn draw_divider_writes_t_junctions() {
        let theme = Theme::default();
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 5));
        Block::default()
            .borders(Borders::ALL)
            .render(Rect::new(0, 0, 10, 5), &mut buf);
        draw_divider(Rect::new(0, 0, 10, 5), 2, &mut buf, &theme);
        assert_eq!(buf[(0, 2)].symbol(), "├");
        assert_eq!(buf[(9, 2)].symbol(), "┤");
        assert_eq!(buf[(5, 2)].symbol(), "─");
    }
}
