use crate::BINARY_NAME;
use crate::config::FrecencyConfig;
use crate::domain::SnippetId;
use std::fs;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// A single usage event: the snippet that was emitted, the cwd the user was
/// in at the time, and a unix timestamp. The store is append-only — the
/// score function is the only thing that interprets these rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageEvent {
    pub id: SnippetId,
    pub cwd: PathBuf,
    pub timestamp: u64,
}

/// File-backed frecency store. Persisted as one TSV line per event so it
/// can be read, diffed, or deleted by hand without a database. Events are
/// never mutated after write; scoring compresses them into a single number.
#[derive(Debug, Default, Clone)]
pub struct FrecencyStore {
    events: Vec<UsageEvent>,
}

impl FrecencyStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn load(path: &Path) -> io::Result<Self> {
        if !path.exists() {
            return Ok(Self::new());
        }
        let raw = fs::read_to_string(path)?;
        let mut events = Vec::new();
        let mut skipped = 0u32;
        for line in raw.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let mut parts = line.splitn(3, '\t');
            let (Some(ts), Some(id_str), Some(cwd_str)) =
                (parts.next(), parts.next(), parts.next())
            else {
                skipped += 1;
                continue;
            };
            let Ok(timestamp) = ts.parse::<u64>() else {
                skipped += 1;
                continue;
            };
            let Some((rel, slug)) = id_str.split_once('#') else {
                skipped += 1;
                continue;
            };
            events.push(UsageEvent {
                id: SnippetId::new(rel, slug),
                cwd: PathBuf::from(cwd_str),
                timestamp,
            });
        }
        if skipped > 0 {
            eprintln!(
                "{BINARY_NAME}: warning: skipped {skipped} malformed line(s) in state file {}",
                path.display()
            );
        }
        Ok(Self { events })
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut f = fs::File::create(path)?;
        for e in &self.events {
            writeln!(f, "{}\t{}\t{}", e.timestamp, e.id, e.cwd.display())?;
        }
        Ok(())
    }

    pub fn record(&mut self, id: SnippetId, cwd: PathBuf, timestamp: u64) {
        self.events.push(UsageEvent { id, cwd, timestamp });
    }

    /// Convenience: record with `SystemTime::now()`.
    pub fn record_now(&mut self, id: SnippetId, cwd: PathBuf) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.record(id, cwd, now);
    }

    pub fn events(&self) -> &[UsageEvent] {
        &self.events
    }

    /// Compute a frecency score for `id` given the current working directory
    /// and "now" (unix seconds). Higher is better; 0.0 means no events.
    ///
    /// Formula: each matching event contributes
    /// `time_decay(age) * (1 + path_affinity(event.cwd, cwd))`, and a
    /// sublinear frequency bonus (`ln(1 + count)`) is added on top. The
    /// factors are chosen so that:
    /// - a more recent event always beats an older one at the same cwd,
    /// - an event at the same cwd roughly doubles a recent-only contribution,
    /// - accumulated high-frequency usage eventually overrides a single
    ///   high-location-match event (demonstrated by `frequency_can_override_location`).
    ///
    /// This is the single biggest tuning knob in Part 02: swap the decay
    /// constant in `time_decay`, change the `(1 + affinity)` multiplier, or
    /// lift the frequency boost to make the ranking favour a different
    /// blend of signals.
    pub fn score(&self, id: &SnippetId, cwd: &Path, now: u64, config: &FrecencyConfig) -> f64 {
        let mut total = 0.0f64;
        let mut count: u32 = 0;
        for event in &self.events {
            if &event.id != id {
                continue;
            }
            count += 1;
            let age = now.saturating_sub(event.timestamp);
            let recency = time_decay(age, config.half_life_days);
            let location = path_affinity(&event.cwd, cwd);
            total += recency * (1.0 + location * config.location_weight);
        }
        if count == 0 {
            return 0.0;
        }
        let frequency_boost = (count as f64).ln_1p() * config.frequency_weight;
        total + frequency_boost
    }
}

/// Exponential decay with a 14-day half-life.
pub fn time_decay(age_seconds: u64, half_life_days: f64) -> f64 {
    let half_life = half_life_days.max(0.001) * 86400.0;
    (0.5f64).powf(age_seconds as f64 / half_life)
}

/// A 0.0..=1.0 affinity between two directories, based on shared leading
/// named components. Identical paths score 1.0; completely unrelated paths
/// score 0.0; a child of a parent scores somewhere in between. The root
/// separator is intentionally ignored — every absolute path shares it, so
/// counting it would make `/tmp` look related to `/home/me/repo`.
pub fn path_affinity(a: &Path, b: &Path) -> f64 {
    fn named(p: &Path) -> Vec<String> {
        p.components()
            .filter_map(|c| match c {
                Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
                _ => None,
            })
            .collect()
    }
    let ac = named(a);
    let bc = named(b);
    let shared = ac.iter().zip(bc.iter()).take_while(|(x, y)| x == y).count();
    if shared == 0 {
        return 0.0;
    }
    let max_len = ac.len().max(bc.len()).max(1);
    shared as f64 / max_len as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(slug: &str) -> SnippetId {
        SnippetId::new("t.md", slug)
    }

    #[test]
    fn path_affinity_monotonic_with_shared_depth() {
        let cwd = Path::new("/home/me/projects/alpha");
        let exact = path_affinity(cwd, Path::new("/home/me/projects/alpha"));
        let sibling = path_affinity(cwd, Path::new("/home/me/projects/beta"));
        let unrelated = path_affinity(cwd, Path::new("/tmp"));
        assert!(
            exact > sibling,
            "exact {exact} should beat sibling {sibling}"
        );
        assert!(
            sibling > unrelated,
            "sibling {sibling} should beat unrelated {unrelated}"
        );
        assert_eq!(unrelated, 0.0);
    }

    #[test]
    fn time_decay_is_monotonic() {
        let now = time_decay(0, 14.0);
        let day = time_decay(86_400, 14.0);
        let month = time_decay(30 * 86_400, 14.0);
        assert!(now > day);
        assert!(day > month);
        assert!(month > 0.0);
    }

    #[test]
    fn location_weighting_raises_score_for_same_cwd() {
        let mut store = FrecencyStore::new();
        let now = 1_000_000u64;
        let target = id("echo");
        store.record(target.clone(), PathBuf::from("/home/me/repo"), now);
        let same_cwd = store.score(
            &target,
            Path::new("/home/me/repo"),
            now,
            &FrecencyConfig::default(),
        );
        let foreign = store.score(&target, Path::new("/tmp"), now, &FrecencyConfig::default());
        assert!(
            same_cwd > foreign,
            "same cwd {same_cwd} should beat foreign {foreign}"
        );
    }

    #[test]
    fn recency_influences_score() {
        let mut store = FrecencyStore::new();
        let now = 10_000_000u64;
        let recent = id("a");
        let stale = id("b");
        store.record(recent.clone(), PathBuf::from("/same"), now);
        store.record(stale.clone(), PathBuf::from("/same"), now - 60 * 86_400);
        let sr = store.score(&recent, Path::new("/same"), now, &FrecencyConfig::default());
        let ss = store.score(&stale, Path::new("/same"), now, &FrecencyConfig::default());
        assert!(sr > ss, "recent {sr} should beat stale {ss}");
    }

    #[test]
    fn frequency_can_override_location() {
        let mut store = FrecencyStore::new();
        let now = 2_000_000u64;
        let popular = id("git");
        let local = id("obscure");

        // `git` has been used many times, never at the current cwd.
        for i in 0..20 {
            store.record(
                popular.clone(),
                PathBuf::from("/elsewhere"),
                now - i * 3_600,
            );
        }
        // `obscure` has a single recent use at the exact current cwd.
        store.record(local.clone(), PathBuf::from("/home/me/repo"), now);

        let here = Path::new("/home/me/repo");
        let popular_score = store.score(&popular, here, now, &FrecencyConfig::default());
        let local_score = store.score(&local, here, now, &FrecencyConfig::default());
        assert!(
            popular_score > local_score,
            "frequency {popular_score} should override location {local_score}"
        );
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = std::env::temp_dir().join("pb-frecency-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.tsv");

        let mut store = FrecencyStore::new();
        store.record(id("a"), PathBuf::from("/x"), 1_111);
        store.record(id("b"), PathBuf::from("/y/z"), 2_222);
        store.save(&path).unwrap();

        let loaded = FrecencyStore::load(&path).unwrap();
        assert_eq!(loaded.events().len(), 2);
        assert_eq!(loaded.events()[0].id.as_str(), "t.md#a");
        assert_eq!(loaded.events()[1].cwd, PathBuf::from("/y/z"));
    }

    #[test]
    fn missing_state_file_loads_as_empty() {
        let path = std::env::temp_dir().join("pb-nonexistent-frecency.tsv");
        let _ = fs::remove_file(&path);
        let store = FrecencyStore::load(&path).unwrap();
        assert!(store.events().is_empty());
    }
}
