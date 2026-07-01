//! State and key handling for the interactive settings editor.

use crate::config::{AppConfig, FuzzyWeights, SearchConfig, Theme};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::PathBuf;

/// The high-level screen currently shown by `pb settings`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Screen {
    /// Top-level settings section picker.
    Section,
    /// Picker for the `search` settings groups.
    Search,
    /// Slider editor for one search group.
    Tuner(TunerGroup),
    /// Picker for the built-in theme palettes.
    Theme,
    /// Read-only list of registered snippet root directories.
    Paths,
}

/// Search tuner groups available in v1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TunerGroup {
    /// Frecency and fuzzy/frecency blend weights.
    Frecency,
    /// Fuzzy matching field weights.
    Fuzzy,
}

/// Numeric storage kind for a tunable config field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FieldKind {
    /// TOML float value.
    Float,
    /// TOML integer value.
    Int,
}

/// Impact readout mode for a tunable field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Readout {
    /// Linear multiplier compared to the default value.
    Multiplier,
    /// Non-linear half-life time constant.
    TimeConstant,
}

/// Qualitative impact band for multiplier fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImpactBand {
    /// Value is zero and the field can be disabled.
    Off,
    /// Value is below half the default.
    Low,
    /// Value is near the default.
    Moderate,
    /// Value is above the default, but not dominant.
    High,
    /// Value is at least three times the default.
    Dominant,
}

impl ImpactBand {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Low => "low",
            Self::Moderate => "moderate",
            Self::High => "high",
            Self::Dominant => "dominant",
        }
    }
}

/// Editable settings field used by rendering, adjustment, and persistence.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Field {
    /// User-visible label.
    pub(crate) label: &'static str,
    /// TOML table chain containing this key.
    pub(crate) toml_path: &'static [&'static str],
    /// TOML key to write.
    pub(crate) key: &'static str,
    /// Stored TOML value kind.
    pub(crate) kind: FieldKind,
    /// Minimum allowed value.
    pub(crate) min: f64,
    /// Maximum allowed value.
    pub(crate) max: f64,
    /// Adjustment step.
    pub(crate) step: f64,
    /// Built-in default used for impact readout.
    pub(crate) default: f64,
    /// Current in-memory value.
    pub(crate) value: f64,
    /// Value when the tuner opened or was last saved.
    pub(crate) original: f64,
    /// Plain-English help text.
    pub(crate) help: &'static str,
    /// Readout strategy.
    pub(crate) readout: Readout,
}

impl Field {
    /// Return true when this field's current value differs from the persisted baseline.
    pub(crate) fn changed(&self) -> bool {
        (self.value - self.original).abs() > f64::EPSILON
    }

    /// Adjust by `steps`, clamping to this field's domain.
    pub(crate) fn adjust(&mut self, steps: i32) {
        let raw = self.value + self.step * f64::from(steps);
        let clamped = raw.clamp(self.min, self.max);
        self.value = match self.kind {
            FieldKind::Float => round_to_step(clamped, self.step),
            FieldKind::Int => clamped.round(),
        };
    }

    /// Format the current value for display and TOML diagnostics.
    pub(crate) fn display_value(&self) -> String {
        match self.kind {
            FieldKind::Float => format_float(self.value),
            FieldKind::Int => format!("{}", self.value.round() as i64),
        }
    }

    /// Mark the current value as saved.
    pub(crate) fn accept_current(&mut self) {
        self.original = self.value;
    }
}

/// Return the qualitative impact band for a field.
pub(crate) fn band(field: &Field) -> Option<ImpactBand> {
    if field.readout == Readout::TimeConstant {
        return None;
    }
    if field.value == 0.0 && field.min == 0.0 {
        return Some(ImpactBand::Off);
    }
    if field.default <= 0.0 {
        return None;
    }
    let ratio = field.value / field.default;
    if ratio < 0.5 {
        Some(ImpactBand::Low)
    } else if ratio <= 1.5 {
        Some(ImpactBand::Moderate)
    } else if ratio < 3.0 {
        Some(ImpactBand::High)
    } else {
        Some(ImpactBand::Dominant)
    }
}

/// In-memory settings editor state.
#[derive(Debug, Clone)]
pub(crate) struct SettingsApp {
    screen: Screen,
    section_selected: usize,
    search_selected: usize,
    field_selected: usize,
    frecency_fields: Vec<Field>,
    fuzzy_fields: Vec<Field>,
    themes: Vec<(String, Theme)>,
    theme_selected: usize,
    theme_original: usize,
    paths: Vec<PathBuf>,
    paths_selected: usize,
    status: Option<String>,
    should_quit: bool,
    confirm_quit: bool,
}

impl SettingsApp {
    /// Build a settings state from the resolved application config.
    pub(crate) fn new(config: &AppConfig) -> Self {
        let themes = Theme::selectable(&config.paths.config_file);
        let theme_name = crate::config::resolved_theme_name(&config.paths.config_file);
        let theme_selected = themes
            .iter()
            .position(|(name, _)| *name == theme_name)
            .unwrap_or(0);
        Self {
            screen: Screen::Section,
            section_selected: 0,
            search_selected: 0,
            field_selected: 0,
            frecency_fields: frecency_fields(&config.search),
            fuzzy_fields: fuzzy_fields(&config.search.fuzzy),
            themes,
            theme_selected,
            theme_original: theme_selected,
            paths: config.paths.snippet_roots.clone(),
            paths_selected: 0,
            status: None,
            should_quit: false,
            confirm_quit: false,
        }
    }

    /// Currently displayed screen.
    pub(crate) fn screen(&self) -> &Screen {
        &self.screen
    }

    /// Top-level selected section index.
    pub(crate) fn section_selected(&self) -> usize {
        self.section_selected
    }

    /// Selected search group index.
    pub(crate) fn search_selected(&self) -> usize {
        self.search_selected
    }

    /// Selected field index for tuner screens.
    pub(crate) fn field_selected(&self) -> usize {
        self.field_selected
    }

    /// Selected index into the theme picker list (built-ins then customs).
    pub(crate) fn theme_selected(&self) -> usize {
        self.theme_selected
    }

    /// Names of every selectable theme, in picker order.
    pub(crate) fn theme_names(&self) -> Vec<&str> {
        self.themes.iter().map(|(name, _)| name.as_str()).collect()
    }

    /// Name of the currently selected (possibly unsaved) theme.
    pub(crate) fn theme_selected_name(&self) -> &str {
        &self.themes[self.theme_selected].0
    }

    /// Resolved [`Theme`] for the currently selected (possibly unsaved) entry,
    /// used to drive the live preview without re-reading the config file.
    pub(crate) fn theme_selected_preview(&self) -> &Theme {
        &self.themes[self.theme_selected].1
    }

    /// Whether the selected theme differs from the saved baseline.
    pub(crate) fn theme_changed(&self) -> bool {
        self.theme_selected != self.theme_original
    }

    /// The theme name to persist, if the selection has changed since save.
    pub(crate) fn pending_theme_name(&self) -> Option<&str> {
        self.theme_changed().then(|| self.theme_selected_name())
    }

    /// Registered snippet root directories, in resolution order.
    pub(crate) fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    /// Selected index into the paths list.
    pub(crate) fn paths_selected(&self) -> usize {
        self.paths_selected
    }

    /// Status message shown in the chrome.
    pub(crate) fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }

    /// Whether the event loop should exit.
    pub(crate) fn should_quit(&self) -> bool {
        self.should_quit
    }

    /// Whether a quit was requested with unsaved changes pending confirmation.
    pub(crate) fn confirm_quit(&self) -> bool {
        self.confirm_quit
    }

    /// Fields for the current tuner, if any.
    pub(crate) fn current_fields(&self) -> &[Field] {
        match self.screen {
            Screen::Tuner(TunerGroup::Frecency) => &self.frecency_fields,
            Screen::Tuner(TunerGroup::Fuzzy) => &self.fuzzy_fields,
            _ => &[],
        }
    }

    /// Mutable fields for the current tuner, if any.
    pub(crate) fn current_fields_mut(&mut self) -> &mut [Field] {
        match self.screen {
            Screen::Tuner(TunerGroup::Frecency) => &mut self.frecency_fields,
            Screen::Tuner(TunerGroup::Fuzzy) => &mut self.fuzzy_fields,
            _ => &mut [],
        }
    }

    /// All editable fields across settings groups.
    pub(crate) fn all_fields(&self) -> impl Iterator<Item = &Field> {
        self.frecency_fields.iter().chain(self.fuzzy_fields.iter())
    }

    /// Mark all current values as saved and update the chrome message.
    pub(crate) fn mark_saved(&mut self) {
        for field in &mut self.frecency_fields {
            field.accept_current();
        }
        for field in &mut self.fuzzy_fields {
            field.accept_current();
        }
        self.theme_original = self.theme_selected;
        self.status = Some("saved".to_string());
    }

    /// Show a non-fatal status/error message.
    pub(crate) fn set_status(&mut self, status: impl Into<String>) {
        self.status = Some(status.into());
    }

    /// Apply a key event. Returns true when save was requested.
    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> bool {
        if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c')) {
            self.should_quit = true;
            return false;
        }
        if matches!(key.code, KeyCode::Char('q') | KeyCode::Char('Q')) {
            return self.handle_quit_key();
        }
        self.confirm_quit = false;
        match self.screen {
            Screen::Section => self.handle_section_key(key),
            Screen::Search => self.handle_search_key(key),
            Screen::Tuner(_) => self.handle_tuner_key(key),
            Screen::Theme => self.handle_theme_key(key),
            Screen::Paths => self.handle_paths_key(key),
        }
    }

    fn handle_quit_key(&mut self) -> bool {
        if self.confirm_quit || (!self.all_fields().any(Field::changed) && !self.theme_changed()) {
            self.should_quit = true;
        } else {
            self.confirm_quit = true;
            self.status =
                Some("unsaved changes — press q again to discard, enter to save".to_string());
        }
        false
    }

    fn handle_section_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc | KeyCode::Backspace => self.should_quit = true,
            KeyCode::Up | KeyCode::Char('k') => {
                self.section_selected = self.section_selected.saturating_sub(1)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.section_selected = (self.section_selected + 1).min(2)
            }
            KeyCode::Enter => {
                self.screen = match self.section_selected {
                    0 => Screen::Search,
                    1 => Screen::Theme,
                    _ => Screen::Paths,
                };
                self.status = None;
            }
            _ => {}
        }
        false
    }

    fn handle_paths_key(&mut self, key: KeyEvent) -> bool {
        let max = self.paths.len().saturating_sub(1);
        match key.code {
            KeyCode::Esc | KeyCode::Backspace => {
                self.screen = Screen::Section;
                self.status = None;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.paths_selected = self.paths_selected.saturating_sub(1)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.paths_selected = (self.paths_selected + 1).min(max)
            }
            _ => {}
        }
        false
    }

    fn handle_theme_key(&mut self, key: KeyEvent) -> bool {
        let max = self.themes.len().saturating_sub(1);
        match key.code {
            KeyCode::Esc | KeyCode::Backspace => {
                self.screen = Screen::Section;
                self.status = None;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.theme_selected = self.theme_selected.saturating_sub(1)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.theme_selected = (self.theme_selected + 1).min(max)
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                self.theme_selected = 0;
                return true;
            }
            KeyCode::Enter => return true,
            _ => {}
        }
        false
    }

    fn handle_search_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc | KeyCode::Backspace => self.screen = Screen::Section,
            KeyCode::Up | KeyCode::Char('k') => {
                self.search_selected = self.search_selected.saturating_sub(1)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.search_selected = (self.search_selected + 1).min(1)
            }
            KeyCode::Enter => {
                self.field_selected = 0;
                self.screen = Screen::Tuner(if self.search_selected == 0 {
                    TunerGroup::Frecency
                } else {
                    TunerGroup::Fuzzy
                });
            }
            _ => {}
        }
        false
    }

    fn handle_tuner_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc | KeyCode::Backspace => {
                self.screen = Screen::Search;
                self.status = None;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.field_selected = self.field_selected.saturating_sub(1)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let max = self.current_fields().len().saturating_sub(1);
                self.field_selected = (self.field_selected + 1).min(max);
            }
            KeyCode::Left | KeyCode::Char('-') => self.adjust_selected(-1),
            KeyCode::Right | KeyCode::Char('+') | KeyCode::Char('=') => self.adjust_selected(1),
            KeyCode::Char('r') | KeyCode::Char('R') => {
                self.reset_current_group();
                return true;
            }
            KeyCode::Enter => return true,
            _ => {}
        }
        false
    }

    fn reset_current_group(&mut self) {
        for field in self.current_fields_mut() {
            field.value = field.default;
        }
        self.status = Some("reset to defaults".to_string());
    }

    fn adjust_selected(&mut self, steps: i32) {
        let idx = self.field_selected;
        if let Some(field) = self.current_fields_mut().get_mut(idx) {
            field.adjust(steps);
            self.status = None;
        }
    }
}

fn frecency_fields(search: &SearchConfig) -> Vec<Field> {
    let defaults = SearchConfig::default();
    vec![
        Field {
            label: "half_life_days",
            toml_path: &["search", "frecency"],
            key: "half_life_days",
            kind: FieldKind::Float,
            min: 1.0,
            max: 120.0,
            step: 1.0,
            default: defaults.frecency.half_life_days,
            value: search.frecency.half_life_days,
            original: search.frecency.half_life_days,
            help: "How quickly old usage fades. Smaller values make recent hits matter more.",
            readout: Readout::TimeConstant,
        },
        Field {
            label: "location_weight",
            toml_path: &["search", "frecency"],
            key: "location_weight",
            kind: FieldKind::Float,
            min: 0.0,
            max: 5.0,
            step: 0.1,
            default: defaults.frecency.location_weight,
            value: search.frecency.location_weight,
            original: search.frecency.location_weight,
            help: "How much prior use in this directory pulls snippets up. 0 ignores cwd entirely.",
            readout: Readout::Multiplier,
        },
        Field {
            label: "frequency_weight",
            toml_path: &["search", "frecency"],
            key: "frequency_weight",
            kind: FieldKind::Float,
            min: 0.0,
            max: 5.0,
            step: 0.1,
            default: defaults.frecency.frequency_weight,
            value: search.frecency.frequency_weight,
            original: search.frecency.frequency_weight,
            help: "How much repeated use boosts a snippet after recency/location scoring.",
            readout: Readout::Multiplier,
        },
        Field {
            label: "frecency_weight",
            toml_path: &["search"],
            key: "frecency_weight",
            kind: FieldKind::Float,
            min: 0.0,
            max: 1000.0,
            step: 10.0,
            default: defaults.frecency_weight,
            value: search.frecency_weight,
            original: search.frecency_weight,
            help: "How strongly usage history is blended with fuzzy text matching.",
            readout: Readout::Multiplier,
        },
    ]
}

fn fuzzy_fields(fuzzy: &FuzzyWeights) -> Vec<Field> {
    let defaults = FuzzyWeights::default();
    vec![
        fuzzy_field(
            "name",
            fuzzy.name,
            defaults.name,
            "Matches in the snippet heading.",
        ),
        fuzzy_field(
            "tag",
            fuzzy.tag,
            defaults.tag,
            "Matches in frontmatter tags.",
        ),
        fuzzy_field(
            "frontmatter_name",
            fuzzy.frontmatter_name,
            defaults.frontmatter_name,
            "Matches in the file-level frontmatter name.",
        ),
        fuzzy_field(
            "description",
            fuzzy.description,
            defaults.description,
            "Matches in snippet prose descriptions.",
        ),
        fuzzy_field(
            "path",
            fuzzy.path,
            defaults.path,
            "Matches in the relative snippet file path.",
        ),
        fuzzy_field(
            "command",
            fuzzy.command,
            defaults.command,
            "Matches in the executable command block.",
        ),
    ]
}

fn fuzzy_field(label: &'static str, value: u32, default: u32, help: &'static str) -> Field {
    Field {
        label,
        toml_path: &["search", "fuzzy"],
        key: label,
        kind: FieldKind::Int,
        min: 0.0,
        max: 100.0,
        step: 1.0,
        default: f64::from(default),
        value: f64::from(value),
        original: f64::from(value),
        help,
        readout: Readout::Multiplier,
    }
}

fn round_to_step(value: f64, step: f64) -> f64 {
    if step <= 0.0 {
        return value;
    }
    (value / step).round() * step
}

pub(crate) fn format_float(value: f64) -> String {
    let rounded_tenth = (value * 10.0).round() / 10.0;
    if (value - rounded_tenth).abs() < 0.000_001 {
        format!("{rounded_tenth:.1}")
    } else {
        format!("{value:.2}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn field(value: f64, default: f64, min: f64, readout: Readout) -> Field {
        Field {
            label: "x",
            toml_path: &["search"],
            key: "x",
            kind: FieldKind::Float,
            min,
            max: 10.0,
            step: 0.1,
            default,
            value,
            original: value,
            help: "help",
            readout,
        }
    }

    #[test]
    fn navigates_section_search_tuner_and_back() {
        let config = AppConfig {
            paths: crate::config::Paths {
                snippet_roots: vec![],
                xdg_snippets_dir: std::path::PathBuf::new(),
                snippet_overrides_active: false,
                state_file: std::path::PathBuf::new(),
                config_file: std::path::PathBuf::new(),
            },
            ui: crate::config::UiConfig::default(),
            search: SearchConfig::default(),
            variables: Default::default(),
            theme: crate::config::Theme::default(),
            suggestion_commands: crate::config::SuggestionCommandsConfig::default(),
            lint: Default::default(),
            keybinds: Default::default(),
        };
        let mut app = SettingsApp::new(&config);
        assert_eq!(app.screen(), &Screen::Section);
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.screen(), &Screen::Search);
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.screen(), &Screen::Tuner(TunerGroup::Fuzzy));
        app.handle_key(key(KeyCode::Esc));
        assert_eq!(app.screen(), &Screen::Search);
    }

    #[test]
    fn adjust_clamps_and_revert_clears_changed() {
        let mut item = field(1.0, 1.0, 0.0, Readout::Multiplier);
        item.adjust(1000);
        assert_eq!(item.value, 10.0);
        item.adjust(-1000);
        assert_eq!(item.value, 0.0);
        item.adjust(10);
        assert_eq!(item.value, item.original);
        assert!(!item.changed());
    }

    #[test]
    fn band_cutoffs_are_locked() {
        assert_eq!(
            band(&field(0.0, 1.0, 0.0, Readout::Multiplier)),
            Some(ImpactBand::Off)
        );
        assert_eq!(
            band(&field(0.49, 1.0, 0.0, Readout::Multiplier)),
            Some(ImpactBand::Low)
        );
        assert_eq!(
            band(&field(0.5, 1.0, 0.0, Readout::Multiplier)),
            Some(ImpactBand::Moderate)
        );
        assert_eq!(
            band(&field(1.5, 1.0, 0.0, Readout::Multiplier)),
            Some(ImpactBand::Moderate)
        );
        assert_eq!(
            band(&field(2.99, 1.0, 0.0, Readout::Multiplier)),
            Some(ImpactBand::High)
        );
        assert_eq!(
            band(&field(3.0, 1.0, 0.0, Readout::Multiplier)),
            Some(ImpactBand::Dominant)
        );
    }

    #[test]
    fn half_life_has_no_band() {
        assert_eq!(band(&field(14.0, 14.0, 1.0, Readout::TimeConstant)), None);
    }

    #[test]
    fn reset_only_applies_on_tuner_screen() {
        let mut app = SettingsApp::new(&AppConfig {
            paths: crate::config::Paths {
                snippet_roots: vec![],
                xdg_snippets_dir: std::path::PathBuf::new(),
                snippet_overrides_active: false,
                state_file: std::path::PathBuf::new(),
                config_file: std::path::PathBuf::new(),
            },
            ui: crate::config::UiConfig::default(),
            search: SearchConfig {
                frecency_weight: 900.0,
                ..SearchConfig::default()
            },
            variables: Default::default(),
            theme: crate::config::Theme::default(),
            suggestion_commands: crate::config::SuggestionCommandsConfig::default(),
            lint: Default::default(),
            keybinds: Default::default(),
        });

        assert!(!app.handle_key(key(KeyCode::Char('r'))));
        assert_eq!(app.frecency_fields[3].value, 900.0);
        app.handle_key(key(KeyCode::Enter));
        app.handle_key(key(KeyCode::Enter));
        assert!(app.handle_key(key(KeyCode::Char('r'))));
        assert_eq!(
            app.frecency_fields[3].value,
            SearchConfig::default().frecency_weight
        );
        assert!(app.frecency_fields[3].changed());
    }

    #[test]
    fn save_key_is_reported() {
        let mut app = SettingsApp::new(&AppConfig {
            paths: crate::config::Paths {
                snippet_roots: vec![],
                xdg_snippets_dir: std::path::PathBuf::new(),
                snippet_overrides_active: false,
                state_file: std::path::PathBuf::new(),
                config_file: std::path::PathBuf::new(),
            },
            ui: crate::config::UiConfig::default(),
            search: SearchConfig::default(),
            variables: Default::default(),
            theme: crate::config::Theme::default(),
            suggestion_commands: crate::config::SuggestionCommandsConfig::default(),
            lint: Default::default(),
            keybinds: Default::default(),
        });
        app.handle_key(key(KeyCode::Enter));
        app.handle_key(key(KeyCode::Enter));
        assert!(app.handle_key(key(KeyCode::Enter)));
    }

    #[test]
    fn quit_with_unsaved_changes_requires_confirmation() {
        let mut app = SettingsApp::new(&AppConfig {
            paths: crate::config::Paths {
                snippet_roots: vec![],
                xdg_snippets_dir: std::path::PathBuf::new(),
                snippet_overrides_active: false,
                state_file: std::path::PathBuf::new(),
                config_file: std::path::PathBuf::new(),
            },
            ui: crate::config::UiConfig::default(),
            search: SearchConfig::default(),
            variables: Default::default(),
            theme: crate::config::Theme::default(),
            suggestion_commands: crate::config::SuggestionCommandsConfig::default(),
            lint: Default::default(),
            keybinds: Default::default(),
        });
        app.handle_key(key(KeyCode::Enter));
        app.handle_key(key(KeyCode::Enter));
        app.handle_key(key(KeyCode::Right));
        assert!(app.frecency_fields[0].changed());

        assert!(!app.handle_key(key(KeyCode::Char('q'))));
        assert!(!app.should_quit());
        assert!(app.confirm_quit());

        assert!(!app.handle_key(key(KeyCode::Char('q'))));
        assert!(app.should_quit());
    }

    #[test]
    fn quit_without_unsaved_changes_is_immediate() {
        let mut app = SettingsApp::new(&AppConfig {
            paths: crate::config::Paths {
                snippet_roots: vec![],
                xdg_snippets_dir: std::path::PathBuf::new(),
                snippet_overrides_active: false,
                state_file: std::path::PathBuf::new(),
                config_file: std::path::PathBuf::new(),
            },
            ui: crate::config::UiConfig::default(),
            search: SearchConfig::default(),
            variables: Default::default(),
            theme: crate::config::Theme::default(),
            suggestion_commands: crate::config::SuggestionCommandsConfig::default(),
            lint: Default::default(),
            keybinds: Default::default(),
        });
        assert!(!app.handle_key(key(KeyCode::Char('q'))));
        assert!(app.should_quit());
    }

    fn test_config() -> AppConfig {
        AppConfig {
            paths: crate::config::Paths {
                snippet_roots: vec![],
                xdg_snippets_dir: std::path::PathBuf::new(),
                snippet_overrides_active: false,
                state_file: std::path::PathBuf::new(),
                config_file: std::path::PathBuf::new(),
            },
            ui: crate::config::UiConfig::default(),
            search: SearchConfig::default(),
            variables: Default::default(),
            theme: crate::config::Theme::default(),
            suggestion_commands: crate::config::SuggestionCommandsConfig::default(),
            lint: Default::default(),
            keybinds: Default::default(),
        }
    }

    fn goto_theme_screen(app: &mut SettingsApp) {
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Enter));
    }

    #[test]
    fn theme_section_is_reachable_from_section_screen() {
        let mut app = SettingsApp::new(&test_config());
        goto_theme_screen(&mut app);
        assert_eq!(app.screen(), &Screen::Theme);
    }

    #[test]
    fn theme_cycles_and_clamps_at_bounds() {
        let mut app = SettingsApp::new(&test_config());
        goto_theme_screen(&mut app);
        assert_eq!(app.theme_selected(), 0);

        app.handle_key(key(KeyCode::Up));
        assert_eq!(app.theme_selected(), 0, "clamps at the low end");

        let max = app.theme_names().len() - 1;
        for _ in 0..max + 5 {
            app.handle_key(key(KeyCode::Down));
        }
        assert_eq!(app.theme_selected(), max, "clamps at the high end");
    }

    #[test]
    fn theme_change_requires_quit_confirmation() {
        let mut app = SettingsApp::new(&test_config());
        goto_theme_screen(&mut app);
        app.handle_key(key(KeyCode::Down));
        assert!(app.theme_changed());

        assert!(!app.handle_key(key(KeyCode::Char('q'))));
        assert!(!app.should_quit());
        assert!(app.confirm_quit());

        assert!(!app.handle_key(key(KeyCode::Char('q'))));
        assert!(app.should_quit());
    }

    #[test]
    fn theme_enter_reports_save_and_clears_dirty_state() {
        let mut app = SettingsApp::new(&test_config());
        goto_theme_screen(&mut app);
        app.handle_key(key(KeyCode::Down));
        assert!(app.theme_changed());
        assert_eq!(app.pending_theme_name(), Some(app.theme_names()[1]));

        assert!(app.handle_key(key(KeyCode::Enter)));
        app.mark_saved();
        assert!(!app.theme_changed());
        assert_eq!(app.pending_theme_name(), None);
    }

    #[test]
    fn theme_reset_selects_default_and_reports_save() {
        let mut app = SettingsApp::new(&test_config());
        goto_theme_screen(&mut app);
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Down));

        assert!(app.handle_key(key(KeyCode::Char('r'))));
        assert_eq!(app.theme_selected(), 0);
        assert_eq!(app.theme_selected_name(), "default");
    }

    fn temp_config_file(prefix: &str, contents: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let dir = std::env::temp_dir().join(format!(
            "pb-settings-app-{prefix}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let config_file = dir.join("config.toml");
        std::fs::write(&config_file, contents).unwrap();
        config_file
    }

    #[test]
    fn registered_custom_themes_are_selectable_and_previewed() {
        let config_file = temp_config_file(
            "custom",
            r##"
[theme]
name = "mytheme"

[theme.custom.mytheme]
accent = "#c678dd"
muted = "#5c6370"
selected_bg = "#3e4451"
selected_fg = "#abb2bf"
prompt_fg = "#282c34"
prompt_bg = "#61afef"
error_fg = "#e06c75"
"##,
        );
        let mut config = test_config();
        config.paths.config_file = config_file;

        let mut app = SettingsApp::new(&config);
        goto_theme_screen(&mut app);

        let names = app.theme_names();
        assert_eq!(names.last(), Some(&"mytheme"));
        assert_eq!(app.theme_selected(), names.len() - 1);
        assert_eq!(app.theme_selected_name(), "mytheme");
        assert_eq!(
            app.theme_selected_preview().selected_marker.fg,
            Some(ratatui::style::Color::Rgb(0xc6, 0x78, 0xdd))
        );
    }

    #[test]
    fn invalid_custom_theme_is_skipped_from_picker() {
        let config_file =
            temp_config_file("invalid", "[theme.custom.broken]\naccent = \"#c678dd\"\n");
        let mut config = test_config();
        config.paths.config_file = config_file;

        let app = SettingsApp::new(&config);
        assert!(!app.theme_names().contains(&"broken"));
    }
}
