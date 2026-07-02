//! Rendering for the interactive settings editor.

use crate::config::Theme;
use crate::keybinds::{
    SettingsGlobalAction, SettingsKeybindsAction, SettingsKeymap, SettingsListAction,
    SettingsSearchAction, SettingsTunerAction, help_hint as hint, help_move_hint as move_hint,
};
use crate::settings::app::{
    ImpactBand, Readout, Screen, SettingsApp, TunerGroup, band, format_float,
};
use crate::settings::keybinds::COMMANDS;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};

const SECTION_ITEMS: &[&str] = &["search", "theme", "paths", "keybinds"];
const SEARCH_ITEMS: &[&str] = &["frecency", "fuzzy"];

/// Draw the current settings screen.
pub(crate) fn draw(frame: &mut Frame<'_>, app: &SettingsApp, theme: &Theme) {
    let (title, footer) = chrome_text(app);
    let content = crate::tui::Chrome {
        theme,
        mode: "pb settings",
        title: &title,
        footer: &footer,
    }
    .render(frame.area(), frame.buffer_mut());

    match app.screen() {
        Screen::Section => draw_picker(
            frame,
            content,
            SECTION_ITEMS,
            app.section_selected(),
            theme,
            40,
        ),
        Screen::Search => draw_picker(
            frame,
            content,
            SEARCH_ITEMS,
            app.search_selected(),
            theme,
            40,
        ),
        Screen::Tuner(group) => draw_tuner(frame, content, app, *group, theme),
        Screen::Theme => draw_picker(
            frame,
            content,
            &app.theme_names(),
            app.theme_selected(),
            theme,
            40,
        ),
        Screen::Paths => {
            let labels: Vec<String> = app
                .paths()
                .iter()
                .map(|path| path.display().to_string())
                .collect();
            let items: Vec<&str> = labels.iter().map(String::as_str).collect();
            let idx_width = items.len().to_string().len().max(1) as u16;
            let content_width = items.iter().map(|item| item.len()).max().unwrap_or(0) as u16;
            let width = content_width + idx_width + 4;
            draw_picker(frame, content, &items, app.paths_selected(), theme, width);
        }
        Screen::KeybindCommands => draw_picker(
            frame,
            content,
            COMMANDS,
            app.keybind_command_selected(),
            theme,
            40,
        ),
        Screen::KeybindActions => draw_keybind_actions(frame, content, app, theme),
    }
}

fn chrome_text(app: &SettingsApp) -> (String, String) {
    let path = match app.screen() {
        Screen::Section => "settings".to_string(),
        Screen::Search => "settings / search".to_string(),
        Screen::Tuner(TunerGroup::Frecency) => "settings / search / frecency".to_string(),
        Screen::Tuner(TunerGroup::Fuzzy) => "settings / search / fuzzy".to_string(),
        Screen::Theme => "settings / theme".to_string(),
        Screen::Paths => "settings / paths".to_string(),
        Screen::KeybindCommands => "settings / keybinds".to_string(),
        Screen::KeybindActions => format!("settings / keybinds / {}", app.keybind_command()),
    };
    let title = match app.status() {
        Some(status) => format!("{path} · {status}"),
        None => path,
    };
    let keymap = app.keymap();
    let footer = if app.confirm_quit() {
        let quit = keymap
            .global
            .hint(SettingsGlobalAction::Quit)
            .unwrap_or_else(|| "q".to_string());
        format!("unsaved changes · {quit} again discards · enter saves")
    } else {
        footer_for(app.screen(), keymap)
    };
    (title, footer)
}

/// Build one screen's footer from its resolved bindings, omitting unbound
/// actions so remapped or removed keys never teach stale defaults.
fn footer_for(screen: &Screen, keymap: &SettingsKeymap) -> String {
    let quit = keymap
        .global
        .hint(SettingsGlobalAction::Quit)
        .map(|k| format!("{k} quit"));
    let parts: Vec<Option<String>> = match screen {
        Screen::Tuner(_) => vec![
            move_hint(
                keymap.tuner.hint(SettingsTunerAction::MoveUp),
                keymap.tuner.hint(SettingsTunerAction::MoveDown),
                "field",
            ),
            move_hint(
                keymap.tuner.hint(SettingsTunerAction::Decrease),
                keymap.tuner.hint(SettingsTunerAction::Increase),
                "adjust",
            ),
            hint(keymap.tuner.hint(SettingsTunerAction::Reset), "reset+save"),
            hint(keymap.tuner.hint(SettingsTunerAction::Save), "save"),
            hint(keymap.tuner.hint(SettingsTunerAction::Back), "back"),
            quit,
        ],
        Screen::Search => vec![
            move_hint(
                keymap.search.hint(SettingsSearchAction::MoveUp),
                keymap.search.hint(SettingsSearchAction::MoveDown),
                "move",
            ),
            hint(keymap.search.hint(SettingsSearchAction::Select), "select"),
            hint(
                keymap.search.hint(SettingsSearchAction::Reset),
                "reset+save",
            ),
            hint(keymap.search.hint(SettingsSearchAction::Back), "back"),
            quit,
        ],
        Screen::Theme => vec![
            move_hint(
                keymap.list.hint(SettingsListAction::MoveUp),
                keymap.list.hint(SettingsListAction::MoveDown),
                "move",
            ),
            hint(keymap.list.hint(SettingsListAction::Select), "save"),
            hint(keymap.list.hint(SettingsListAction::Reset), "reset+save"),
            hint(keymap.list.hint(SettingsListAction::Back), "back"),
            quit,
        ],
        Screen::Paths => vec![
            move_hint(
                keymap.list.hint(SettingsListAction::MoveUp),
                keymap.list.hint(SettingsListAction::MoveDown),
                "move",
            ),
            hint(keymap.list.hint(SettingsListAction::Back), "back"),
            quit,
        ],
        Screen::KeybindCommands => vec![
            move_hint(
                keymap.list.hint(SettingsListAction::MoveUp),
                keymap.list.hint(SettingsListAction::MoveDown),
                "move",
            ),
            hint(keymap.list.hint(SettingsListAction::Select), "select"),
            hint(keymap.list.hint(SettingsListAction::Back), "back"),
            quit,
        ],
        Screen::KeybindActions => vec![
            move_hint(
                keymap.keybinds.hint(SettingsKeybindsAction::MoveUp),
                keymap.keybinds.hint(SettingsKeybindsAction::MoveDown),
                "action",
            ),
            move_hint(
                keymap.keybinds.hint(SettingsKeybindsAction::ChordLeft),
                keymap.keybinds.hint(SettingsKeybindsAction::ChordRight),
                "chord",
            ),
            hint(keymap.keybinds.hint(SettingsKeybindsAction::Capture), "add"),
            hint(
                keymap.keybinds.hint(SettingsKeybindsAction::DeleteChord),
                "delete",
            ),
            hint(keymap.keybinds.hint(SettingsKeybindsAction::Reset), "reset"),
            hint(
                keymap.keybinds.hint(SettingsKeybindsAction::Unbind),
                "unbind",
            ),
            hint(keymap.keybinds.hint(SettingsKeybindsAction::Save), "save"),
            hint(keymap.keybinds.hint(SettingsKeybindsAction::Back), "back"),
            quit,
        ],
        Screen::Section => vec![
            move_hint(
                keymap.list.hint(SettingsListAction::MoveUp),
                keymap.list.hint(SettingsListAction::MoveDown),
                "move",
            ),
            hint(keymap.list.hint(SettingsListAction::Select), "select"),
            hint(keymap.list.hint(SettingsListAction::Back), "back"),
            quit,
        ],
    };
    parts.into_iter().flatten().collect::<Vec<_>>().join(" · ")
}

fn draw_picker(
    frame: &mut Frame<'_>,
    area: Rect,
    items: &[&str],
    selected: usize,
    theme: &Theme,
    width: u16,
) {
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
        clamped(area, width, items.len() as u16),
        &mut state,
    );
}

fn draw_keybind_actions(frame: &mut Frame<'_>, area: Rect, app: &SettingsApp, theme: &Theme) {
    let command = app.keybind_command();
    let entries = app
        .keybind_entries()
        .iter()
        .filter(|entry| entry.section == command)
        .collect::<Vec<_>>();
    let mut rows = Vec::new();
    let mut last_context = None;
    let mut selected_render_idx = 0usize;
    for (row_idx, entry) in entries.iter().enumerate() {
        if last_context != Some(entry.context) {
            rows.push(ListItem::new(Line::from(vec![Span::styled(
                format!("[{}]", entry.context),
                theme.chrome,
            )])));
            last_context = Some(entry.context);
        }
        let selected = row_idx == app.keybind_row_selected();
        if selected {
            selected_render_idx = rows.len();
        }
        let marker = if selected { "▌ " } else { "  " };
        let changed = if entry.differs_from_default() {
            "*"
        } else {
            " "
        };
        let chords = if entry.current.is_empty() {
            vec![Span::styled("unbound", theme.chrome)]
        } else {
            entry
                .current
                .iter()
                .enumerate()
                .flat_map(|(idx, chord)| {
                    let style = if selected && idx == app.keybind_chord_selected() {
                        theme.selected_item
                    } else {
                        Style::default()
                    };
                    let sep = (idx > 0).then(|| Span::raw(", "));
                    sep.into_iter()
                        .chain(std::iter::once(Span::styled(chord.to_string(), style)))
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        };
        let mut spans = vec![
            Span::styled(
                marker,
                if selected {
                    theme.selected_marker
                } else {
                    theme.chrome
                },
            ),
            Span::styled(
                format!("{changed} {:<22} ", entry.action),
                if selected {
                    theme.selected_item
                } else {
                    Style::default()
                },
            ),
        ];
        spans.extend(chords);
        rows.push(ListItem::new(Line::from(spans)));
    }
    let mut state = ListState::default();
    state.select(Some(selected_render_idx));
    frame.render_stateful_widget(List::new(rows), area, &mut state);

    if app.capturing_keybind() {
        let overlay = Paragraph::new("Press a key to bind\nEsc cancels; Ctrl+C quits\nBare Esc can only be restored by reset or config edit")
            .block(Block::default().borders(Borders::ALL).title("capture key"));
        frame.render_widget(overlay, centered(area, 54, 5));
    }
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

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = area.width.min(width);
    let height = area.height.min(height);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keybinds::Keymaps;

    fn keymap_from(raw: &str) -> SettingsKeymap {
        let value: toml::Value = toml::from_str(raw).unwrap();
        Keymaps::resolve(value.get("keybinds")).settings
    }

    #[test]
    fn footer_reflects_defaults() {
        let keymap = SettingsKeymap::default();
        assert_eq!(
            footer_for(&Screen::Section, &keymap),
            "up/down move · enter select · esc back · q quit"
        );
        assert_eq!(
            footer_for(&Screen::Tuner(TunerGroup::Frecency), &keymap),
            "up/down field · left/right adjust · r reset+save · enter save · esc back · q quit"
        );
    }

    #[test]
    fn footer_reflects_remaps_and_omits_unbound_actions() {
        let keymap = keymap_from(
            r#"
[keybinds.settings.global]
quit = ["ctrl+q"]

[keybinds.settings.list]
move_up = ["ctrl+p"]
select = []
"#,
        );
        let footer = footer_for(&Screen::Section, &keymap);
        assert_eq!(footer, "ctrl+p/down move · esc back · ctrl+q quit");
        assert!(!footer.contains("select"), "unbound action must be omitted");
    }
}
