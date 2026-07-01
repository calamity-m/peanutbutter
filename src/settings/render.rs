//! Rendering for the interactive settings editor.

use crate::config::Theme;
use crate::settings::app::{
    ImpactBand, Readout, Screen, SettingsApp, TunerGroup, band, format_float,
};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};

const SECTION_ITEMS: &[&str] = &["search", "theme"];
const SEARCH_ITEMS: &[&str] = &["frecency", "fuzzy"];

/// Draw the current settings screen.
pub(crate) fn draw(frame: &mut Frame<'_>, app: &SettingsApp, theme: &Theme) {
    let (title, footer) = chrome_text(app);
    let content = crate::tui::Chrome {
        theme,
        mode: "pb settings",
        title: &title,
        footer,
    }
    .render(frame.area(), frame.buffer_mut());

    match app.screen() {
        Screen::Section => {
            draw_picker(frame, content, SECTION_ITEMS, app.section_selected(), theme)
        }
        Screen::Search => draw_picker(frame, content, SEARCH_ITEMS, app.search_selected(), theme),
        Screen::Tuner(group) => draw_tuner(frame, content, app, *group, theme),
        Screen::Theme => draw_picker(
            frame,
            content,
            Theme::built_in_names(),
            app.theme_selected(),
            theme,
        ),
    }
}

fn chrome_text(app: &SettingsApp) -> (String, &'static str) {
    let path = match app.screen() {
        Screen::Section => "settings".to_string(),
        Screen::Search => "settings / search".to_string(),
        Screen::Tuner(TunerGroup::Frecency) => "settings / search / frecency".to_string(),
        Screen::Tuner(TunerGroup::Fuzzy) => "settings / search / fuzzy".to_string(),
        Screen::Theme => "settings / theme".to_string(),
    };
    let title = match app.status() {
        Some(status) => format!("{path} · {status}"),
        None => path,
    };
    let footer = if app.confirm_quit() {
        "unsaved changes · q again discards · enter saves"
    } else {
        match app.screen() {
            Screen::Tuner(_) => {
                "↑/↓ field · ←/→ adjust · r reset+save · enter save · esc/backspace back · q quit"
            }
            Screen::Theme => {
                "↑/↓ or j/k move · enter save · r reset+save · esc/backspace back · q quit"
            }
            _ => "↑/↓ or j/k move · enter select · esc/backspace back · q quit",
        }
    };
    (title, footer)
}

fn draw_picker(frame: &mut Frame<'_>, area: Rect, items: &[&str], selected: usize, theme: &Theme) {
    let idx_width = items.len().to_string().len().max(1);
    let rows = items
        .iter()
        .enumerate()
        .map(|(idx, item)| {
            if idx == selected {
                ListItem::new(Line::from(vec![
                    Span::styled("▌ ", theme.selected_marker),
                    Span::styled(format!("{:>idx_width$}  ", idx + 1), theme.selected_item),
                    Span::styled(*item, theme.selected_item),
                ]))
            } else {
                ListItem::new(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(format!("{:>idx_width$}  ", idx + 1), theme.chrome),
                    Span::raw(*item),
                ]))
            }
        })
        .collect::<Vec<_>>();
    let mut state = ListState::default();
    state.select(Some(selected));
    frame.render_stateful_widget(
        List::new(rows),
        clamped(area, 40, items.len() as u16),
        &mut state,
    );
}

fn draw_tuner(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &SettingsApp,
    _group: TunerGroup,
    theme: &Theme,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(4), Constraint::Length(5)])
        .split(area);
    let fields = app.current_fields();
    let selected = app.field_selected().min(fields.len().saturating_sub(1));

    let rows = fields
        .iter()
        .enumerate()
        .map(|(idx, field)| {
            let selected = idx == selected;
            let dominant = band(field) == Some(ImpactBand::Dominant);
            let strong_style = strong_style(theme);
            let bar_style = match band(field) {
                Some(ImpactBand::Dominant) => strong_style,
                Some(ImpactBand::High) => Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
                Some(ImpactBand::Low | ImpactBand::Off) => theme.chrome,
                Some(ImpactBand::Moderate) | None => theme.emphasis,
            };
            let row_style = if selected {
                theme.selected_item
            } else {
                Style::default()
            };
            let marker = if selected {
                Span::styled("▌ ", theme.selected_marker)
            } else {
                Span::raw("  ")
            };
            let mut spans = vec![
                marker,
                Span::styled(
                    format!("{:<18} {:>7}  ", field.label, field.display_value()),
                    row_style,
                ),
            ];
            spans.extend(bar_spans(field, bar_style, theme.chrome));
            if dominant {
                spans.push(Span::styled("  ! strong", strong_style));
            }
            ListItem::new(Line::from(spans))
        })
        .collect::<Vec<_>>();
    frame.render_widget(List::new(rows), chunks[0]);

    draw_explanation(frame, chunks[1], fields.get(selected), theme);
}

fn strong_style(theme: &Theme) -> Style {
    Style::default()
        .fg(theme.error.fg.unwrap_or(Color::Red))
        .add_modifier(Modifier::BOLD)
}

fn bar_spans(
    field: &crate::settings::app::Field,
    filled_style: Style,
    empty_style: Style,
) -> Vec<Span<'static>> {
    const WIDTH: usize = 24;
    let ratio = if field.max <= field.min {
        0.0
    } else {
        ((field.value - field.min) / (field.max - field.min)).clamp(0.0, 1.0)
    };
    let filled = (ratio * WIDTH as f64).round() as usize;
    let filled = filled.min(WIDTH);
    let empty = WIDTH - filled;
    vec![
        Span::raw("["),
        Span::styled("█".repeat(filled), filled_style),
        Span::styled("░".repeat(empty), empty_style),
        Span::raw("]"),
    ]
}

fn draw_explanation(
    frame: &mut Frame<'_>,
    area: Rect,
    field: Option<&crate::settings::app::Field>,
    theme: &Theme,
) {
    let Some(field) = field else {
        return;
    };
    let mut lines = vec![
        Line::from(Span::styled(field.label, theme.emphasis)),
        Line::from(field.help),
    ];
    match field.readout {
        Readout::TimeConstant => lines.push(Line::from(format!(
            "{} days · default {} · half as strong after {} days",
            field.display_value(),
            format_float(field.default),
            field.display_value()
        ))),
        Readout::Multiplier => {
            let value = if field.value == 0.0 && field.min == 0.0 {
                "off".to_string()
            } else if (field.value - field.default).abs() < f64::EPSILON {
                "default".to_string()
            } else {
                format!("{}×", format_float(field.value / field.default))
            };
            let band = band(field).map(ImpactBand::label).unwrap_or("n/a");
            lines.push(Line::from(format!(
                "{} · default {} · {band} ({value})",
                field.display_value(),
                format_float(field.default)
            )));
        }
    }
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(theme.divider),
        ),
        area,
    );
}

fn clamped(area: Rect, width: u16, height: u16) -> Rect {
    Rect {
        x: area.x,
        y: area.y,
        width: area.width.min(width),
        height: area.height.min(height.max(1)),
    }
}
