use std::env;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub snippet_roots: Vec<PathBuf>,
    pub state_file: PathBuf,
}

pub fn default_paths() -> Paths {
    Paths {
        snippet_roots: resolve_snippet_roots(),
        state_file: resolve_state_file(),
    }
}

fn resolve_snippet_roots() -> Vec<PathBuf> {
    let xdg_default = xdg_config_home().join("peanutbutter").join("snippets");
    let mut roots: Vec<PathBuf> = Vec::new();

    if let Ok(raw) = env::var("PEANUTBUTTER_PATH") {
        for extra in raw.split(':').filter(|s| !s.is_empty()).map(PathBuf::from) {
            if extra != xdg_default {
                roots.push(extra);
            }
        }
    }

    roots.push(xdg_default);
    roots
}

fn resolve_state_file() -> PathBuf {
    if let Ok(raw) = env::var("PB_STATE_FILE")
        && !raw.is_empty()
    {
        return PathBuf::from(raw);
    }
    xdg_state_home().join("peanutbutter").join("state.tsv")
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
        .unwrap_or_else(|| PathBuf::from("/"))
}
