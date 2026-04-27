use ratatui::style::{Color, Modifier, Style};
use serde::Deserialize;
use std::collections::{BTreeMap, HashSet};
use std::env;
use std::fs;
use std::io;
use std::path::PathBuf;

/// Resolved file-system paths used throughout the application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    /// Directories (or files) that are searched recursively for `.md` snippets.
    /// Populated from `PEANUTBUTTER_PATH`, `[paths] snippets`, then the XDG
    /// default (`$XDG_CONFIG_HOME/peanutbutter/snippets`), in that order.
    pub snippet_roots: Vec<PathBuf>,
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
/// `path`=10, `body`=8.
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
    /// Weight for the snippet's command body.
    pub body: u32,
}

impl Default for FuzzyWeights {
    fn default() -> Self {
        Self {
            name: 30,
            tag: 20,
            frontmatter_name: 15,
            description: 10,
            path: 10,
            body: 8,
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
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct VariableInputConfig {
    /// Pre-populated default value shown in the prompt input box.
    pub default: Option<String>,
    /// Fixed list of suggestions shown in the suggestion list.
    pub suggestions: Vec<String>,
    /// Shell command whose stdout lines are used as suggestions (overrides
    /// `suggestions` when both are set and `suggestions` is empty).
    pub command: Option<String>,
}

/// Visual theme: a set of ratatui [`Style`] values covering every distinct UI
/// role. Build via [`Theme::default`] or [`Theme::from_raw`] (the TOML path).
#[derive(Debug, Clone)]
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
    fn from_raw(raw: ThemeFileConfig) -> io::Result<Self> {
        let mut theme = Theme::default();

        if let Some(color) = raw.muted {
            let color = parse_color(&color)?;
            theme.chrome = theme.chrome.fg(color);
            theme.placeholder = theme.placeholder.fg(color);
        }

        if let Some(color) = raw.accent {
            let color = parse_color(&color)?;
            theme.fuzzy_highlight = theme.fuzzy_highlight.fg(color);
            theme.selected_marker = theme.selected_marker.fg(color);
        }

        if let Some(color) = raw.selected_bg {
            let color = parse_color(&color)?;
            theme.selected_marker = theme.selected_marker.bg(color);
            theme.selected_item = theme.selected_item.bg(color);
            theme.error = theme.error.bg(color);
        }

        if let Some(color) = raw.selected_fg {
            theme.selected_item = theme.selected_item.fg(parse_color(&color)?);
        }

        if let Some(color) = raw.prompt_active_fg {
            theme.active_prompt = theme.active_prompt.fg(parse_color(&color)?);
        }

        if let Some(color) = raw.prompt_active_bg {
            theme.active_prompt = theme.active_prompt.bg(parse_color(&color)?);
        }

        if let Some(color) = raw.error_fg {
            theme.error = theme.error.fg(parse_color(&color)?);
        }

        Ok(theme)
    }
}

/// Load [`AppConfig`] from `$XDG_CONFIG_HOME/peanutbutter/config.toml` (or
/// `$PB_CONFIG_FILE`). Missing files are silently treated as empty; parse
/// errors are returned as `InvalidData` errors.
pub fn load() -> io::Result<AppConfig> {
    let config_file = resolve_config_file();
    let file = load_file_config(&config_file)?;
    let paths = Paths {
        snippet_roots: resolve_snippet_roots(&file),
        state_file: resolve_state_file(&file),
        config_file,
    };

    Ok(AppConfig {
        paths,
        ui: UiConfig {
            height: file.ui.height.unwrap_or(20).max(1),
        },
        search: SearchConfig {
            frecency_weight: file.search.frecency_weight.unwrap_or(250.0),
            fuzzy: FuzzyWeights {
                name: file.search.fuzzy.name.unwrap_or(30),
                tag: file.search.fuzzy.tag.unwrap_or(20),
                frontmatter_name: file.search.fuzzy.frontmatter_name.unwrap_or(15),
                description: file.search.fuzzy.description.unwrap_or(10),
                path: file.search.fuzzy.path.unwrap_or(10),
                body: file.search.fuzzy.body.unwrap_or(8),
            },
            frecency: FrecencyConfig {
                half_life_days: file.search.frecency.half_life_days.unwrap_or(14.0),
                location_weight: file.search.frecency.location_weight.unwrap_or(1.0),
                frequency_weight: file.search.frecency.frequency_weight.unwrap_or(1.0),
            },
        },
        variables: file.variables,
        theme: Theme::from_raw(file.theme)?,
    })
}

/// Return the resolved [`Paths`] from the config file, or compute defaults if
/// loading fails. Used by commands that need paths before a full config load.
pub fn default_paths() -> Paths {
    load().map(|config| config.paths).unwrap_or_else(|_| Paths {
        snippet_roots: resolve_snippet_roots(&FileConfig::default()),
        state_file: resolve_state_file(&FileConfig::default()),
        config_file: resolve_config_file(),
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
    accent: Option<String>,
    muted: Option<String>,
    selected_bg: Option<String>,
    selected_fg: Option<String>,
    prompt_active_fg: Option<String>,
    prompt_active_bg: Option<String>,
    error_fg: Option<String>,
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

fn resolve_snippet_roots(file: &FileConfig) -> Vec<PathBuf> {
    let xdg_default = xdg_config_home().join("peanutbutter").join("snippets");
    let mut roots = Vec::new();
    let mut seen = HashSet::new();

    if let Ok(raw) = env::var("PEANUTBUTTER_PATH") {
        for path in raw.split(':').filter(|s| !s.is_empty()).map(PathBuf::from) {
            push_unique(&mut roots, &mut seen, path);
        }
    }

    for path in &file.paths.snippets {
        push_unique(&mut roots, &mut seen, path.clone());
    }

    push_unique(&mut roots, &mut seen, xdg_default);
    roots
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
        let theme = Theme::from_raw(ThemeFileConfig {
            accent: Some("#112233".to_string()),
            muted: Some("dark_gray".to_string()),
            selected_bg: Some("blue".to_string()),
            selected_fg: Some("white".to_string()),
            prompt_active_fg: Some("yellow".to_string()),
            prompt_active_bg: Some("#445566".to_string()),
            error_fg: Some("red".to_string()),
        })
        .unwrap();

        assert_eq!(theme.selected_marker.fg, Some(Color::Rgb(0x11, 0x22, 0x33)));
        assert_eq!(theme.selected_item.bg, Some(Color::Blue));
        assert_eq!(theme.active_prompt.bg, Some(Color::Rgb(0x44, 0x55, 0x66)));
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
    fn example_config_deserializes() {
        let raw = include_str!("../examples/config.toml");
        let parsed: FileConfig = toml::from_str(raw).unwrap();
        let variable = parsed.variables.get("file").unwrap();
        assert!(variable.suggestions.is_empty());
        assert_eq!(
            variable.command.as_deref(),
            Some("find . -maxdepth 1 -type f | sed 's#^./##' | sort")
        );
    }
}
