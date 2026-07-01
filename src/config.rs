use ratatui::style::{Color, Modifier, Style};
use serde::Deserialize;
use std::collections::{BTreeMap, HashSet};
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Resolved file-system paths used throughout the application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    /// Directories (or files) that are searched recursively for `.md` snippets.
    /// Populated from `PEANUTBUTTER_PATH`, `[paths] snippets`, then the XDG
    /// default (`$XDG_CONFIG_HOME/peanutbutter/snippets`), in that order.
    pub snippet_roots: Vec<PathBuf>,
    /// XDG default snippets directory used by `pb init` and first-run auto-init.
    pub xdg_snippets_dir: PathBuf,
    /// Whether snippet roots were explicitly configured through env or config.
    pub snippet_overrides_active: bool,
    /// TSV file where usage events are appended for frecency scoring.
    /// Defaults to `$XDG_STATE_HOME/peanutbutter/state.tsv`.
    pub state_file: PathBuf,
    /// TOML config file that was loaded (may not exist on first run).
    /// Defaults to `$XDG_CONFIG_HOME/peanutbutter/config.toml`.
    pub config_file: PathBuf,
}

/// Top-level application configuration, assembled from the TOML file and
/// environment variables.
#[derive(Debug, Clone)]
pub struct AppConfig {
    /// Resolved filesystem paths.
    pub paths: Paths,
    /// TUI layout parameters.
    pub ui: UiConfig,
    /// Search ranking weights.
    pub search: SearchConfig,
    /// Per-variable overrides keyed by variable name.
    pub variables: BTreeMap<String, VariableInputConfig>,
    /// Visual theme applied to the TUI.
    pub theme: Theme,
    /// Controls how suggestion commands are executed.
    pub suggestion_commands: SuggestionCommandsConfig,
    /// Per-lint suppression and disable rules.
    pub lint: LintConfig,
    /// Resolved per-command keymaps plus the non-fatal warnings their
    /// resolution produced. Warnings are not printed by config loading;
    /// `pb execute` surfaces them as TUI status because it owns stdout safety
    /// on the hotkey path. Other commands ignore them.
    pub keybinds: crate::keybinds::Keymaps,
}

/// Per-lint suppression rules keyed by lint code without the `lint/` prefix.
pub type LintConfig = BTreeMap<String, LintRuleConfig>;

/// Suppression options for a single lint code.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct LintRuleConfig {
    /// Disable this lint entirely.
    pub disable: bool,
    /// Glob patterns matched against snippet paths relative to their root.
    #[serde(deserialize_with = "string_or_vec")]
    pub ignore_file: Vec<String>,
    /// Glob patterns matched against suggestion command text for command lints.
    #[serde(deserialize_with = "string_or_vec")]
    pub ignore_command: Vec<String>,
}

/// Controls how suggestion commands (`<@name:cmd>` and `command =` entries) are
/// executed. Applies globally to all suggestion commands in all snippets.
#[derive(Debug, Clone)]
pub struct SuggestionCommandsConfig {
    /// How long (in milliseconds) a suggestion command may run before it is
    /// killed and the variable falls back to manual input. Default: 2000.
    pub timeout_ms: u64,
    /// If `false`, no suggestion commands are executed at all; variables fall
    /// back to their static suggestions or manual input. Default: `true`.
    pub allow_commands: bool,
}

impl Default for SuggestionCommandsConfig {
    fn default() -> Self {
        Self {
            timeout_ms: 2000,
            allow_commands: true,
        }
    }
}

/// TUI layout parameters.
#[derive(Debug, Clone)]
pub struct UiConfig {
    /// Inline viewport height in terminal rows. Clamped to at least 1.
    pub height: u16,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self { height: 20 }
    }
}

/// Parameters controlling how fuzzy and frecency scores are combined.
#[derive(Debug, Clone)]
pub struct SearchConfig {
    /// Multiplier applied to the raw frecency score before it is added to the
    /// fuzzy score. Larger values make location/recency history dominate;
    /// smaller values make the query text dominate.
    pub frecency_weight: f64,
    /// Per-field weights for the fuzzy scorer.
    pub fuzzy: FuzzyWeights,
    /// Parameters for the frecency decay and path-affinity algorithm.
    pub frecency: FrecencyConfig,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            frecency_weight: 250.0,
            fuzzy: FuzzyWeights::default(),
            frecency: FrecencyConfig::default(),
        }
    }
}

/// Multipliers applied to each snippet field's raw fuzzy match score.
///
/// Higher weight → matches in that field rank higher relative to other fields.
/// Defaults: `name`=30, `tag`=20, `frontmatter_name`=15, `description`=10,
/// `path`=10, `command`=8.
#[derive(Debug, Clone)]
pub struct FuzzyWeights {
    /// Weight for the snippet's `##` heading (display name).
    pub name: u32,
    /// Weight for frontmatter tags.
    pub tag: u32,
    /// Weight for the file-level frontmatter `name:` field.
    pub frontmatter_name: u32,
    /// Weight for the snippet's prose description.
    pub description: u32,
    /// Weight for the snippet's relative file path.
    pub path: u32,
    /// Weight for the snippet's executable command block.
    pub command: u32,
}

impl Default for FuzzyWeights {
    fn default() -> Self {
        Self {
            name: 30,
            tag: 20,
            frontmatter_name: 15,
            description: 10,
            path: 10,
            command: 8,
        }
    }
}

/// Tuning knobs for the frecency scoring algorithm.
///
/// See [`crate::frecency::FrecencyStore::score`] for the full formula.
#[derive(Debug, Clone)]
pub struct FrecencyConfig {
    /// Number of days after which an event contributes half as much to the
    /// score. Smaller → recency matters more; larger → older events survive.
    pub half_life_days: f64,
    /// Multiplier on the path-affinity term inside each event's contribution.
    /// Set to 0.0 to ignore cwd entirely.
    pub location_weight: f64,
    /// Multiplier on the `ln(1 + count)` frequency bonus added after summing
    /// per-event contributions. Set to 0.0 to ignore frequency.
    pub frequency_weight: f64,
}

impl Default for FrecencyConfig {
    fn default() -> Self {
        Self {
            half_life_days: 14.0,
            location_weight: 1.0,
            frequency_weight: 1.0,
        }
    }
}

/// Per-variable config overrides defined in `[variables.<name>]` TOML sections.
///
/// These override or supplement the inline `<@name[:source]>` syntax from the
/// snippet body. All fields are optional and independent.
pub type VariableInputConfig = crate::domain::VariableSpec;

/// Visual theme: a set of ratatui [`Style`] values covering every distinct UI
/// role. Build via [`Theme::default`] or one of the named constructors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    /// Muted style for decorative chrome (counters, separators, help text).
    pub chrome: Style,
    /// Bold style for important labels and headings.
    pub emphasis: Style,
    /// Accent colour applied to characters that matched the fuzzy query.
    pub fuzzy_highlight: Style,
    /// Accent+background for the `>` marker beside the selected list row.
    pub selected_marker: Style,
    /// Bold+background for the text of the selected list row.
    pub selected_item: Style,
    /// Muted style for unfilled `<@variable>` placeholders in the preview.
    pub placeholder: Style,
    /// Inverse style for the variable placeholder currently being edited.
    pub active_prompt: Style,
    /// Style for horizontal divider lines between UI sections.
    pub divider: Style,
    /// Style for panel borders.
    pub border: Style,
    /// Style for error messages (e.g. failed suggestion command).
    pub error: Style,
}

impl Default for Theme {
    fn default() -> Self {
        let accent = Color::Red;
        let selected_bg = Color::DarkGray;
        let muted = Color::Gray;
        Self {
            chrome: Style::default().fg(muted).add_modifier(Modifier::DIM),
            emphasis: Style::default().add_modifier(Modifier::BOLD),
            fuzzy_highlight: Style::default().fg(accent).add_modifier(Modifier::BOLD),
            selected_marker: Style::default().fg(accent).bg(selected_bg),
            selected_item: Style::default()
                .bg(selected_bg)
                .add_modifier(Modifier::BOLD),
            placeholder: Style::default().fg(muted).add_modifier(Modifier::DIM),
            active_prompt: Style::default()
                .fg(Color::Black)
                .bg(Color::White)
                .add_modifier(Modifier::BOLD),
            divider: Style::default().fg(Color::DarkGray),
            border: Style::default().fg(Color::DarkGray),
            error: Style::default().fg(accent).bg(selected_bg),
        }
    }
}

impl Theme {
    /// Return the stable names of themes built into the binary.
    pub fn built_in_names() -> &'static [&'static str] {
        &["default", "gruvbox", "catppuccin", "nord", "monochrome"]
    }

    /// Build the named `gruvbox` theme.
    pub fn gruvbox() -> Self {
        let brown = Color::Rgb(0xa0, 0x6a, 0x3b);
        let tan = Color::Rgb(0xd6, 0xb4, 0x8a);

        let mut theme = Self::from_palette(ThemePalette {
            accent: tan,
            muted: Color::Rgb(0x8a, 0x74, 0x64),
            selected_bg: Color::Rgb(0x33, 0x28, 0x21),
            selected_fg: Color::Rgb(0xf2, 0xdf, 0xc7),
            prompt_fg: Color::Rgb(0x20, 0x1a, 0x17),
            prompt_bg: tan,
            error_fg: Color::Rgb(0xe0, 0x6c, 0x75),
        });
        theme.emphasis = theme.emphasis.fg(tan);
        theme.border = theme.border.fg(brown);
        theme
    }

    /// Build the named `catppuccin` theme.
    pub fn catppuccin() -> Self {
        Self::from_palette(ThemePalette {
            accent: Color::Rgb(0xf5, 0xc2, 0xe7),
            muted: Color::Rgb(0x6c, 0x70, 0x86),
            selected_bg: Color::Rgb(0x31, 0x34, 0x4a),
            selected_fg: Color::Rgb(0xcd, 0xd6, 0xf4),
            prompt_fg: Color::Rgb(0x1e, 0x1e, 0x2e),
            prompt_bg: Color::Rgb(0x89, 0xb4, 0xfa),
            error_fg: Color::Rgb(0xf3, 0x8b, 0xa8),
        })
    }

    /// Build the named `nord` theme.
    pub fn nord() -> Self {
        Self::from_palette(ThemePalette {
            accent: Color::Rgb(0x88, 0xc0, 0xd0),
            muted: Color::Rgb(0x81, 0xa1, 0xc1),
            selected_bg: Color::Rgb(0x3b, 0x42, 0x52),
            selected_fg: Color::Rgb(0xec, 0xef, 0xf4),
            prompt_fg: Color::Rgb(0x2e, 0x34, 0x40),
            prompt_bg: Color::Rgb(0xa3, 0xbe, 0x8c),
            error_fg: Color::Rgb(0xbf, 0x61, 0x6a),
        })
    }

    /// Build the named `monochrome` theme with no foreground colours.
    pub fn monochrome() -> Self {
        Self {
            chrome: Style::default().add_modifier(Modifier::DIM),
            emphasis: Style::default().add_modifier(Modifier::BOLD),
            fuzzy_highlight: Style::default().add_modifier(Modifier::BOLD),
            selected_marker: Style::default().add_modifier(Modifier::REVERSED),
            selected_item: Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED),
            placeholder: Style::default().add_modifier(Modifier::DIM),
            active_prompt: Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED),
            divider: Style::default().add_modifier(Modifier::DIM),
            border: Style::default().add_modifier(Modifier::DIM),
            error: Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED),
        }
    }

    pub(crate) fn named(name: &str) -> io::Result<Self> {
        match name {
            "default" => Ok(Theme::default()),
            "gruvbox" => Ok(Theme::gruvbox()),
            "catppuccin" => Ok(Theme::catppuccin()),
            "nord" => Ok(Theme::nord()),
            "monochrome" => Ok(Theme::monochrome()),
            other => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "unknown theme {other}; expected one of: {}",
                    Theme::built_in_names().join(", ")
                ),
            )),
        }
    }

    /// Return `(name, resolved theme)` for every selectable theme: the 5
    /// built-ins in order, followed by any `[theme.custom.<name>]` entries
    /// that parse successfully. A custom entry that's missing a required
    /// color is silently skipped rather than failing the whole list, so one
    /// broken entry doesn't block selecting any other theme.
    pub(crate) fn selectable(config_file: &PathBuf) -> Vec<(String, Theme)> {
        let mut themes: Vec<(String, Theme)> = Theme::built_in_names()
            .iter()
            .map(|name| {
                (
                    (*name).to_string(),
                    Theme::named(name).expect("built-in theme name"),
                )
            })
            .collect();
        if let Ok(file) = load_file_config(config_file) {
            for (name, colors) in &file.theme.custom {
                if let Ok(theme) = colors.to_theme() {
                    themes.push((name.clone(), theme));
                }
            }
        }
        themes
    }

    fn from_palette(palette: ThemePalette) -> Self {
        Self {
            chrome: Style::default()
                .fg(palette.muted)
                .add_modifier(Modifier::DIM),
            emphasis: Style::default().add_modifier(Modifier::BOLD),
            fuzzy_highlight: Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
            selected_marker: Style::default().fg(palette.accent).bg(palette.selected_bg),
            selected_item: Style::default()
                .fg(palette.selected_fg)
                .bg(palette.selected_bg)
                .add_modifier(Modifier::BOLD),
            placeholder: Style::default()
                .fg(palette.muted)
                .add_modifier(Modifier::DIM),
            active_prompt: Style::default()
                .fg(palette.prompt_fg)
                .bg(palette.prompt_bg)
                .add_modifier(Modifier::BOLD),
            divider: Style::default().fg(palette.muted),
            border: Style::default().fg(palette.muted),
            error: Style::default()
                .fg(palette.error_fg)
                .bg(palette.selected_bg),
        }
    }

    fn from_raw(raw: &ThemeFileConfig, cli_theme: Option<&str>) -> io::Result<Self> {
        if let Some(name) = cli_theme {
            return raw.base_theme(name);
        }

        let name = raw.name.as_deref().unwrap_or("default");
        let theme = raw.base_theme(name)?;
        raw.colors.apply(theme)
    }
}

struct ThemePalette {
    accent: Color,
    muted: Color,
    selected_bg: Color,
    selected_fg: Color,
    prompt_fg: Color,
    prompt_bg: Color,
    error_fg: Color,
}

/// Load [`AppConfig`] from `$XDG_CONFIG_HOME/peanutbutter/config.toml` (or
/// `$PB_CONFIG_FILE`). Missing files are silently treated as empty; parse
/// errors are returned as `InvalidData` errors.
pub fn load() -> io::Result<AppConfig> {
    load_with_theme_override(None)
}

/// Load [`AppConfig`] and use `theme_name` as the theme base when provided.
pub fn load_with_theme_override(theme_name: Option<&str>) -> io::Result<AppConfig> {
    let config_file = resolve_config_file();
    let file = load_file_config(&config_file)?;
    let keybinds = crate::keybinds::Keymaps::resolve(file.keybinds.as_ref());
    let xdg_snippets_dir = xdg_snippets_dir();
    let paths = Paths {
        snippet_roots: resolve_snippet_roots(&file, &xdg_snippets_dir),
        xdg_snippets_dir,
        snippet_overrides_active: snippet_overrides_active(&file),
        state_file: resolve_state_file(&file),
        config_file,
    };

    Ok(AppConfig {
        paths,
        ui: UiConfig {
            height: file.ui.height.unwrap_or(20).max(1),
        },
        suggestion_commands: SuggestionCommandsConfig {
            timeout_ms: file.suggestion_commands.timeout_ms.unwrap_or(2000),
            allow_commands: file.suggestion_commands.allow_commands.unwrap_or(true),
        },
        search: SearchConfig {
            frecency_weight: file.search.frecency_weight.unwrap_or(250.0),
            fuzzy: FuzzyWeights {
                name: file.search.fuzzy.name.unwrap_or(30),
                tag: file.search.fuzzy.tag.unwrap_or(20),
                frontmatter_name: file.search.fuzzy.frontmatter_name.unwrap_or(15),
                description: file.search.fuzzy.description.unwrap_or(10),
                path: file.search.fuzzy.path.unwrap_or(10),
                command: file
                    .search
                    .fuzzy
                    .command
                    .or(file.search.fuzzy.body)
                    .unwrap_or(8),
            },
            frecency: FrecencyConfig {
                half_life_days: file.search.frecency.half_life_days.unwrap_or(14.0),
                location_weight: file.search.frecency.location_weight.unwrap_or(1.0),
                frequency_weight: file.search.frecency.frequency_weight.unwrap_or(1.0),
            },
        },
        variables: file.variables,
        theme: Theme::from_raw(&file.theme, theme_name)?,
        lint: file.lint,
        keybinds,
    })
}

/// Return names that shell completion should offer for the `--theme` flag.
pub fn theme_completion_names() -> io::Result<Vec<String>> {
    let config_file = resolve_config_file();
    let file = load_file_config(&config_file)?;
    let mut names = Theme::built_in_names()
        .iter()
        .map(|name| (*name).to_string())
        .collect::<Vec<_>>();
    names.extend(file.theme.custom.keys().cloned());
    Ok(names)
}

/// Return the theme name resolved from `config_file`'s `[theme] name`, or
/// `"default"` when unset or the file cannot be read.
pub(crate) fn resolved_theme_name(config_file: &PathBuf) -> String {
    load_file_config(config_file)
        .ok()
        .and_then(|file| file.theme.name)
        .unwrap_or_else(|| "default".to_string())
}

/// Return the resolved [`Paths`] from the config file, or compute defaults if
/// loading fails. Used by commands that need paths before a full config load.
pub fn default_paths() -> Paths {
    load().map(|config| config.paths).unwrap_or_else(|_| {
        let file = FileConfig::default();
        let xdg_snippets_dir = xdg_snippets_dir();
        Paths {
            snippet_roots: resolve_snippet_roots(&file, &xdg_snippets_dir),
            xdg_snippets_dir,
            snippet_overrides_active: snippet_overrides_active(&file),
            state_file: resolve_state_file(&file),
            config_file: resolve_config_file(),
        }
    })
}

#[derive(Debug, Default, Deserialize)]
struct FileConfig {
    #[serde(default)]
    paths: PathsFileConfig,
    #[serde(default)]
    ui: UiFileConfig,
    #[serde(default)]
    search: SearchFileConfig,
    #[serde(default)]
    variables: BTreeMap<String, VariableInputConfig>,
    #[serde(default)]
    theme: ThemeFileConfig,
    #[serde(default)]
    suggestion_commands: SuggestionCommandsFileConfig,
    #[serde(default)]
    lint: LintConfig,
    /// Kept as a raw TOML value so unknown contexts/actions and wrong value
    /// types become warnings during resolution instead of load errors.
    #[serde(default)]
    keybinds: Option<toml::Value>,
}

#[derive(Debug, Default, Deserialize)]
struct SuggestionCommandsFileConfig {
    timeout_ms: Option<u64>,
    allow_commands: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
struct PathsFileConfig {
    #[serde(default)]
    snippets: Vec<PathBuf>,
    state_file: Option<PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
struct UiFileConfig {
    height: Option<u16>,
}

#[derive(Debug, Default, Deserialize)]
struct SearchFileConfig {
    frecency_weight: Option<f64>,
    #[serde(default)]
    fuzzy: FuzzyWeightsFileConfig,
    #[serde(default)]
    frecency: FrecencyFileConfig,
}

#[derive(Debug, Default, Deserialize)]
struct FuzzyWeightsFileConfig {
    name: Option<u32>,
    tag: Option<u32>,
    frontmatter_name: Option<u32>,
    description: Option<u32>,
    path: Option<u32>,
    command: Option<u32>,
    body: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
struct FrecencyFileConfig {
    half_life_days: Option<f64>,
    location_weight: Option<f64>,
    frequency_weight: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
struct ThemeFileConfig {
    name: Option<String>,
    #[serde(flatten)]
    colors: ThemeColorConfig,
    /// Named custom palettes declared as `[theme.custom.<name>]` tables. Each
    /// entry is selectable by its key, alongside the 5 built-in names.
    #[serde(default)]
    custom: BTreeMap<String, ThemeColorConfig>,
}

impl ThemeFileConfig {
    fn base_theme(&self, name: &str) -> io::Result<Theme> {
        if let Some(custom) = self.custom.get(name) {
            return custom.to_theme();
        }
        if Theme::built_in_names().contains(&name) {
            return Theme::named(name);
        }
        let mut known = Theme::built_in_names().to_vec();
        known.extend(self.custom.keys().map(String::as_str));
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "unknown theme {name}; expected one of: {}",
                known.join(", ")
            ),
        ))
    }
}

#[derive(Debug, Default, Deserialize)]
struct ThemeColorConfig {
    accent: Option<String>,
    muted: Option<String>,
    selected_bg: Option<String>,
    selected_fg: Option<String>,
    prompt_active_fg: Option<String>,
    prompt_active_bg: Option<String>,
    prompt_fg: Option<String>,
    prompt_bg: Option<String>,
    error_fg: Option<String>,
}

impl ThemeColorConfig {
    fn apply(&self, mut theme: Theme) -> io::Result<Theme> {
        if let Some(color) = &self.muted {
            let color = parse_color(color)?;
            theme.chrome = theme.chrome.fg(color);
            theme.placeholder = theme.placeholder.fg(color);
        }

        if let Some(color) = &self.accent {
            let color = parse_color(color)?;
            theme.fuzzy_highlight = theme.fuzzy_highlight.fg(color);
            theme.selected_marker = theme.selected_marker.fg(color);
        }

        if let Some(color) = &self.selected_bg {
            let color = parse_color(color)?;
            theme.selected_marker = theme.selected_marker.bg(color);
            theme.selected_item = theme.selected_item.bg(color);
            theme.error = theme.error.bg(color);
        }

        if let Some(color) = &self.selected_fg {
            theme.selected_item = theme.selected_item.fg(parse_color(color)?);
        }

        if let Some(color) = self.prompt_fg() {
            theme.active_prompt = theme.active_prompt.fg(parse_color(color)?);
        }

        if let Some(color) = self.prompt_bg() {
            theme.active_prompt = theme.active_prompt.bg(parse_color(color)?);
        }

        if let Some(color) = &self.error_fg {
            theme.error = theme.error.fg(parse_color(color)?);
        }

        Ok(theme)
    }

    fn to_theme(&self) -> io::Result<Theme> {
        Ok(Theme::from_palette(ThemePalette {
            accent: self.required_color("accent", &self.accent)?,
            muted: self.required_color("muted", &self.muted)?,
            selected_bg: self.required_color("selected_bg", &self.selected_bg)?,
            selected_fg: self.required_color("selected_fg", &self.selected_fg)?,
            prompt_fg: self.required_color("prompt_fg", &self.prompt_fg().map(str::to_string))?,
            prompt_bg: self.required_color("prompt_bg", &self.prompt_bg().map(str::to_string))?,
            error_fg: self.required_color("error_fg", &self.error_fg)?,
        }))
    }

    fn prompt_fg(&self) -> Option<&str> {
        self.prompt_fg
            .as_deref()
            .or(self.prompt_active_fg.as_deref())
    }

    fn prompt_bg(&self) -> Option<&str> {
        self.prompt_bg
            .as_deref()
            .or(self.prompt_active_bg.as_deref())
    }

    fn required_color(&self, name: &str, value: &Option<String>) -> io::Result<Color> {
        let value = value.as_deref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("custom theme missing required color {name}"),
            )
        })?;
        parse_color(value)
    }
}

pub(crate) fn string_or_vec<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrVec {
        String(String),
        Vec(Vec<String>),
    }

    Ok(match Option::<StringOrVec>::deserialize(deserializer)? {
        Some(StringOrVec::String(value)) => vec![value],
        Some(StringOrVec::Vec(values)) => values,
        None => Vec::new(),
    })
}

fn load_file_config(path: &PathBuf) -> io::Result<FileConfig> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(FileConfig::default()),
        Err(err) => return Err(err),
    };
    toml::from_str(&raw).map_err(|err| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid config file {}: {err}", path.display()),
        )
    })
}

fn resolve_snippet_roots(file: &FileConfig, xdg_default: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let mut seen = HashSet::new();

    if let Ok(raw) = env::var("PEANUTBUTTER_PATH") {
        for path in env::split_paths(&raw).filter(|p| !p.as_os_str().is_empty()) {
            push_unique(&mut roots, &mut seen, path);
        }
    }

    for path in &file.paths.snippets {
        push_unique(&mut roots, &mut seen, path.clone());
    }

    push_unique(&mut roots, &mut seen, xdg_default.to_path_buf());
    roots
}

fn snippet_overrides_active(file: &FileConfig) -> bool {
    env::var_os("PEANUTBUTTER_PATH").is_some() || !file.paths.snippets.is_empty()
}

fn resolve_state_file(file: &FileConfig) -> PathBuf {
    if let Ok(raw) = env::var("PB_STATE_FILE")
        && !raw.is_empty()
    {
        return PathBuf::from(raw);
    }
    if let Some(path) = &file.paths.state_file {
        return path.clone();
    }
    xdg_state_home().join("peanutbutter").join("state.tsv")
}

fn resolve_config_file() -> PathBuf {
    if let Ok(raw) = env::var("PB_CONFIG_FILE")
        && !raw.is_empty()
    {
        return PathBuf::from(raw);
    }
    xdg_config_home().join("peanutbutter").join("config.toml")
}

fn push_unique(roots: &mut Vec<PathBuf>, seen: &mut HashSet<PathBuf>, path: PathBuf) {
    if seen.insert(path.clone()) {
        roots.push(path);
    }
}

fn parse_color(raw: &str) -> io::Result<Color> {
    let raw = raw.trim();
    if let Some(hex) = raw.strip_prefix('#') {
        if hex.len() != 6 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid color {raw}: expected #RRGGBB"),
            ));
        }
        let r = u8::from_str_radix(&hex[0..2], 16).map_err(invalid_color(raw))?;
        let g = u8::from_str_radix(&hex[2..4], 16).map_err(invalid_color(raw))?;
        let b = u8::from_str_radix(&hex[4..6], 16).map_err(invalid_color(raw))?;
        return Ok(Color::Rgb(r, g, b));
    }

    let color = match raw.to_ascii_lowercase().as_str() {
        "reset" => Color::Reset,
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" => Color::Magenta,
        "cyan" => Color::Cyan,
        "gray" | "grey" => Color::Gray,
        "darkgray" | "dark_gray" | "dark-grey" | "darkgrey" => Color::DarkGray,
        "lightred" | "light_red" => Color::LightRed,
        "lightgreen" | "light_green" => Color::LightGreen,
        "lightyellow" | "light_yellow" => Color::LightYellow,
        "lightblue" | "light_blue" => Color::LightBlue,
        "lightmagenta" | "light_magenta" => Color::LightMagenta,
        "lightcyan" | "light_cyan" => Color::LightCyan,
        "white" => Color::White,
        other => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unknown color {other}"),
            ));
        }
    };
    Ok(color)
}

fn invalid_color(raw: &str) -> impl FnOnce(std::num::ParseIntError) -> io::Error + '_ {
    move |_| io::Error::new(io::ErrorKind::InvalidData, format!("invalid color {raw}"))
}

fn xdg_snippets_dir() -> PathBuf {
    xdg_config_home().join("peanutbutter").join("snippets")
}

fn xdg_config_home() -> PathBuf {
    env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| home_dir().join(".config"))
}

fn xdg_state_home() -> PathBuf {
    env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| home_dir().join(".local").join("state"))
}

fn home_dir() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_accepts_named_and_hex_colors() {
        let theme = Theme::from_raw(
            &ThemeFileConfig {
                colors: ThemeColorConfig {
                    accent: Some("#112233".to_string()),
                    muted: Some("dark_gray".to_string()),
                    selected_bg: Some("blue".to_string()),
                    selected_fg: Some("white".to_string()),
                    prompt_active_fg: Some("yellow".to_string()),
                    prompt_active_bg: Some("#445566".to_string()),
                    error_fg: Some("red".to_string()),
                    ..ThemeColorConfig::default()
                },
                ..ThemeFileConfig::default()
            },
            None,
        )
        .unwrap();

        assert_eq!(theme.selected_marker.fg, Some(Color::Rgb(0x11, 0x22, 0x33)));
        assert_eq!(theme.selected_item.bg, Some(Color::Blue));
        assert_eq!(theme.active_prompt.bg, Some(Color::Rgb(0x44, 0x55, 0x66)));
    }

    #[test]
    fn theme_name_selects_builtin_and_cli_skips_overrides() {
        let raw = ThemeFileConfig {
            name: Some("gruvbox".to_string()),
            colors: ThemeColorConfig {
                accent: Some("red".to_string()),
                ..ThemeColorConfig::default()
            },
            ..ThemeFileConfig::default()
        };

        let config_theme = Theme::from_raw(&raw, None).unwrap();
        let cli_theme = Theme::from_raw(&raw, Some("nord")).unwrap();

        assert_eq!(config_theme.selected_marker.fg, Some(Color::Red));
        assert_eq!(cli_theme, Theme::nord());
    }

    #[test]
    fn named_custom_theme_requires_complete_block() {
        let raw = r##"
[theme]
name = "mytheme"
accent = "red"

[theme.custom.mytheme]
accent = "#c678dd"
muted = "#5c6370"
selected_bg = "#3e4451"
selected_fg = "#abb2bf"
prompt_fg = "#282c34"
prompt_bg = "#61afef"
error_fg = "#e06c75"
"##;
        let parsed: FileConfig = toml::from_str(raw).unwrap();
        let theme = Theme::from_raw(&parsed.theme, None).unwrap();

        assert_eq!(theme.selected_marker.fg, Some(Color::Red));
        assert_eq!(theme.active_prompt.bg, Some(Color::Rgb(0x61, 0xaf, 0xef)));
    }

    #[test]
    fn multiple_named_custom_themes_are_independently_selectable() {
        let raw = r##"
[theme.custom.one]
accent = "#111111"
muted = "#222222"
selected_bg = "#333333"
selected_fg = "#444444"
prompt_fg = "#555555"
prompt_bg = "#666666"
error_fg = "#777777"

[theme.custom.two]
accent = "#aaaaaa"
muted = "#bbbbbb"
selected_bg = "#cccccc"
selected_fg = "#dddddd"
prompt_fg = "#eeeeee"
prompt_bg = "#ffffff"
error_fg = "#123456"
"##;
        let parsed: FileConfig = toml::from_str(raw).unwrap();

        let one = parsed.theme.base_theme("one").unwrap();
        let two = parsed.theme.base_theme("two").unwrap();
        assert_eq!(one.selected_marker.fg, Some(Color::Rgb(0x11, 0x11, 0x11)));
        assert_eq!(two.selected_marker.fg, Some(Color::Rgb(0xaa, 0xaa, 0xaa)));

        let err = parsed.theme.base_theme("three").unwrap_err();
        assert!(err.to_string().contains("one, two"));
    }

    #[test]
    fn unknown_theme_name_is_clear_error() {
        let raw = ThemeFileConfig {
            name: Some("wat".to_string()),
            ..ThemeFileConfig::default()
        };
        let err = Theme::from_raw(&raw, None).unwrap_err();
        assert!(err.to_string().contains("unknown theme wat"));
    }

    #[test]
    fn parse_color_rejects_unknown_values() {
        let err = parse_color("wat").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn variable_input_defaults_deserialize() {
        let raw = r#"
[variables.http_method]
default = "GET"
suggestions = ["GET", "POST"]
"#;
        let parsed: FileConfig = toml::from_str(raw).unwrap();
        let variable = parsed.variables.get("http_method").unwrap();
        assert_eq!(variable.default.as_deref(), Some("GET"));
        assert_eq!(variable.suggestions, vec!["GET", "POST"]);
    }

    #[test]
    fn variable_input_command_only_deserializes() {
        let raw = r#"
[variables.file]
command = "find . -type f"
"#;
        let parsed: FileConfig = toml::from_str(raw).unwrap();
        let variable = parsed.variables.get("file").unwrap();
        assert_eq!(variable.default, None);
        assert!(variable.suggestions.is_empty());
        assert_eq!(variable.command.as_deref(), Some("find . -type f"));
    }

    #[test]
    fn lint_rule_config_deserializes_string_and_list_patterns() {
        let raw = r#"
[lint.invalid-dependent-reference]
ignore_command = "*rg*"
ignore_file = ["test*", "fixtures/*"]
disable = true
"#;
        let parsed: FileConfig = toml::from_str(raw).unwrap();
        let rule = parsed.lint.get("invalid-dependent-reference").unwrap();
        assert!(rule.disable);
        assert_eq!(rule.ignore_command, vec!["*rg*"]);
        assert_eq!(rule.ignore_file, vec!["test*", "fixtures/*"]);
    }

    #[test]
    fn example_config_deserializes() {
        // Reads the same canonical file that `pb docs config` embeds (via
        // build.rs/OUT_DIR). Kept on the direct path intentionally so this test
        // stays decoupled from the docs build pipeline — not a divergent source.
        let raw = include_str!("../examples/config.toml");
        let parsed: FileConfig = toml::from_str(raw).unwrap();
        let variable = parsed.variables.get("file").unwrap();
        assert!(variable.suggestions.is_empty());
        assert_eq!(
            variable.command.as_deref(),
            Some("find . -maxdepth 1 -type f | sed 's#^./##' | sort")
        );
        assert_eq!(parsed.search.fuzzy.command, Some(8));
    }

    #[test]
    fn keybinds_section_deserializes_permissively() {
        // Wrong value types inside [keybinds] must not fail the config load;
        // they surface as warnings during resolution instead.
        let raw = r#"
[keybinds.execute.select]
cycle_mode = ["ctrl+n"]
edit = 5
"#;
        let parsed: FileConfig = toml::from_str(raw).unwrap();
        let keymaps = crate::keybinds::Keymaps::resolve(parsed.keybinds.as_ref());

        let chord = crate::keybinds::KeyChord::parse("ctrl+n").unwrap();
        assert_eq!(
            keymaps.execute.select.action(&chord),
            Some(crate::keybinds::SelectAction::CycleMode)
        );
        assert_eq!(keymaps.warnings.len(), 1);
        assert!(keymaps.warnings[0].contains("execute.select.edit"));
    }

    #[test]
    fn fuzzy_command_weight_accepts_legacy_body_alias() {
        let legacy: FileConfig = toml::from_str("[search.fuzzy]\nbody = 7\n").unwrap();
        assert_eq!(
            legacy.search.fuzzy.command.or(legacy.search.fuzzy.body),
            Some(7)
        );

        let both: FileConfig = toml::from_str("[search.fuzzy]\ncommand = 9\nbody = 7\n").unwrap();
        assert_eq!(
            both.search.fuzzy.command.or(both.search.fuzzy.body),
            Some(9)
        );
    }
}
