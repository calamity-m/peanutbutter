use crate::config::Paths;
use crate::domain::SnippetId;
use crate::frecency::FrecencyStore;
use crate::index::load_from_roots;
use owo_colors::OwoColorize;
use std::collections::HashMap;
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// How to rank the least-used snippet list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum Sort {
    /// Stalest snippets first (lowest last-seen timestamp).
    #[default]
    Stale,
    /// Fewest uses first (lowest event count).
    Count,
}

/// Runtime options for `peanutbutter stats`.
#[derive(Debug, Clone)]
pub struct StatsOptions {
    /// How many snippets to show in the most-used and least-used lists.
    pub top_n: usize,
    /// Sort order for the least-used list.
    pub sort: Sort,
    /// Emit JSON instead of human-readable text.
    pub json: bool,
}

impl Default for StatsOptions {
    fn default() -> Self {
        Self {
            top_n: 10,
            sort: Sort::Stale,
            json: false,
        }
    }
}

#[derive(Debug, Clone, Default)]
struct EventSummary {
    count: usize,
    last_seen: u64,
    cwds: HashMap<PathBuf, usize>,
}

/// Per-snippet usage stats derived from the frecency store.
#[derive(Debug, Clone)]
pub struct SnippetStat {
    pub id: SnippetId,
    pub name: String,
    pub count: usize,
    pub last_seen: u64,
    /// Sorted by frequency descending.
    pub cwds: Vec<(PathBuf, usize)>,
}

/// Counts of snippets last used in each time window.
#[derive(Debug, Clone, Default)]
pub struct RecencyBuckets {
    pub today: usize,
    pub this_week: usize,
    pub this_month: usize,
    pub older: usize,
}

/// Computed usage statistics ready for rendering.
#[derive(Debug, Clone)]
pub struct StatsReport {
    pub most_used: Vec<SnippetStat>,
    pub least_used: Vec<SnippetStat>,
    /// Snippets that appear in the index but have no frecency events.
    pub never_used: Vec<(SnippetId, String)>,
    pub recency: RecencyBuckets,
    pub directory_affinity: Vec<SnippetStat>,
    pub orphaned_event_count: usize,
}

/// Compute `now` from wall clock, determine color from TTY/env, then delegate
/// to [`run_with`].
pub fn run<W: Write>(paths: &Paths, options: StatsOptions, writer: &mut W) -> io::Result<()> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let color =
        !options.json && std::env::var_os("NO_COLOR").is_none() && io::stdout().is_terminal();
    run_with(paths, options, now, color, writer)
}

/// Deterministic variant used in tests. `now` and `color` are injected by the
/// caller so the output is stable regardless of clock or terminal state.
pub fn run_with<W: Write>(
    paths: &Paths,
    options: StatsOptions,
    now: u64,
    color: bool,
    writer: &mut W,
) -> io::Result<()> {
    if !paths.state_file.exists() {
        if options.json {
            writeln!(writer, "{}", empty_json())?;
        } else {
            writeln!(writer, "No frecency history yet - use snippets first.")?;
        }
        return Ok(());
    }

    let index = load_from_roots(&paths.snippet_roots)?;

    if index.is_empty() {
        if options.json {
            writeln!(writer, "{}", empty_json())?;
        } else {
            writeln!(writer, "No snippets found in configured roots.")?;
        }
        return Ok(());
    }

    let store = FrecencyStore::load(&paths.state_file)?;
    let report = compute_report(&index, &store, &options, now);

    if options.json {
        write_json(writer, &report)
    } else {
        write_human(writer, &report, options.sort, color, now)
    }
}

fn compute_report(
    index: &crate::index::SnippetIndex,
    store: &FrecencyStore,
    options: &StatsOptions,
    now: u64,
) -> StatsReport {
    // Single O(n) pass over events.
    let mut by_id: HashMap<SnippetId, EventSummary> = HashMap::new();
    for event in store.events() {
        let s = by_id.entry(event.id.clone()).or_default();
        s.count += 1;
        if event.timestamp > s.last_seen {
            s.last_seen = event.timestamp;
        }
        *s.cwds.entry(event.cwd.clone()).or_default() += 1;
    }

    let current_ids: std::collections::HashSet<&SnippetId> = index.iter().map(|s| s.id()).collect();

    let mut known: Vec<(SnippetId, EventSummary)> = Vec::new();
    let mut orphaned_event_count = 0usize;
    for (id, summary) in by_id {
        if current_ids.contains(&id) {
            known.push((id, summary));
        } else {
            orphaned_event_count += summary.count;
        }
    }

    let known_ids: std::collections::HashSet<&SnippetId> = known.iter().map(|(id, _)| id).collect();
    let never_used: Vec<(SnippetId, String)> = index
        .iter()
        .filter(|s| !known_ids.contains(s.id()))
        .map(|s| (s.id().clone(), s.name().to_string()))
        .collect();

    let build_stat = |id: &SnippetId, summary: &EventSummary| -> SnippetStat {
        let name = index
            .get(id)
            .map(|s| s.name().to_string())
            .unwrap_or_else(|| id.to_string());
        let mut cwds: Vec<(PathBuf, usize)> =
            summary.cwds.iter().map(|(p, &c)| (p.clone(), c)).collect();
        cwds.sort_by_key(|b| std::cmp::Reverse(b.1));
        SnippetStat {
            id: id.clone(),
            name,
            count: summary.count,
            last_seen: summary.last_seen,
            cwds,
        }
    };

    let mut most_used: Vec<SnippetStat> = known.iter().map(|(id, s)| build_stat(id, s)).collect();
    most_used.sort_by_key(|b| std::cmp::Reverse(b.count));
    most_used.truncate(options.top_n);

    let mut least_used: Vec<SnippetStat> = known.iter().map(|(id, s)| build_stat(id, s)).collect();
    match options.sort {
        Sort::Stale => least_used.sort_by_key(|a| a.last_seen),
        Sort::Count => least_used.sort_by_key(|a| a.count),
    }
    least_used.truncate(options.top_n);

    let mut recency = RecencyBuckets::default();
    for (_, summary) in &known {
        match classify_recency(summary.last_seen, now) {
            Recency::Today => recency.today += 1,
            Recency::ThisWeek => recency.this_week += 1,
            Recency::ThisMonth => recency.this_month += 1,
            Recency::Older => recency.older += 1,
        }
    }

    let mut directory_affinity: Vec<SnippetStat> = known
        .iter()
        .filter(|(_, s)| !s.cwds.is_empty())
        .map(|(id, s)| build_stat(id, s))
        .collect();
    directory_affinity.sort_by_key(|b| std::cmp::Reverse(b.count));

    StatsReport {
        most_used,
        least_used,
        never_used,
        recency,
        directory_affinity,
        orphaned_event_count,
    }
}

enum Recency {
    Today,
    ThisWeek,
    ThisMonth,
    Older,
}

fn classify_recency(last_seen: u64, now: u64) -> Recency {
    if now / 86400 == last_seen / 86400 {
        Recency::Today
    } else if now.saturating_sub(last_seen) < 7 * 86400 {
        Recency::ThisWeek
    } else if now.saturating_sub(last_seen) < 30 * 86400 {
        Recency::ThisMonth
    } else {
        Recency::Older
    }
}

fn recency_badge(last_seen: u64, now: u64) -> &'static str {
    match classify_recency(last_seen, now) {
        Recency::Today => "today",
        Recency::ThisWeek => "1 wk",
        Recency::ThisMonth => "1 mo",
        Recency::Older => "old",
    }
}

// ── JSON output ─────────────────────────────────────────────────────────────

fn empty_json() -> String {
    r#"{"most_used":[],"least_used":[],"never_used":[],"recency":{"today":0,"this_week":0,"this_month":0,"older":0},"directory_affinity":[],"orphaned_event_count":0}"#.to_string()
}

fn write_json<W: Write>(writer: &mut W, report: &StatsReport) -> io::Result<()> {
    use serde_json::{Map, Value, json};

    let most_used: Vec<Value> = report
        .most_used
        .iter()
        .map(|s| json!({"id": s.id.to_string(), "name": s.name, "count": s.count, "last_seen": s.last_seen}))
        .collect();

    let least_used: Vec<Value> = report
        .least_used
        .iter()
        .map(|s| json!({"id": s.id.to_string(), "name": s.name, "count": s.count, "last_seen": s.last_seen}))
        .collect();

    let never_used: Vec<Value> = report
        .never_used
        .iter()
        .map(|(id, name)| json!({"id": id.to_string(), "name": name}))
        .collect();

    let recency = json!({
        "today": report.recency.today,
        "this_week": report.recency.this_week,
        "this_month": report.recency.this_month,
        "older": report.recency.older,
    });

    let directory_affinity: Vec<Value> = report
        .directory_affinity
        .iter()
        .map(|s| {
            let cwds: Vec<Value> = s
                .cwds
                .iter()
                .map(|(path, count)| json!({"path": path.to_string_lossy(), "count": count}))
                .collect();
            json!({"id": s.id.to_string(), "name": s.name, "cwds": cwds})
        })
        .collect();

    let mut obj = Map::new();
    obj.insert("most_used".into(), Value::Array(most_used));
    obj.insert("least_used".into(), Value::Array(least_used));
    obj.insert("never_used".into(), Value::Array(never_used));
    obj.insert("recency".into(), recency);
    obj.insert(
        "directory_affinity".into(),
        Value::Array(directory_affinity),
    );
    obj.insert(
        "orphaned_event_count".into(),
        Value::Number(report.orphaned_event_count.into()),
    );

    let out = serde_json::to_string(&Value::Object(obj)).map_err(io::Error::other)?;
    writeln!(writer, "{out}")
}

// ── Human-readable output ────────────────────────────────────────────────────

const BOX_WIDTH: usize = 53;

fn box_top(title: &str) -> String {
    // ┌─ Title ─────...─┐
    let inner = BOX_WIDTH - 2; // excluding corner chars
    let title_part = format!("─ {title} ");
    let remaining = inner.saturating_sub(title_part.len());
    format!("┌{}{}┐", title_part, "─".repeat(remaining))
}

fn box_bottom() -> String {
    format!("└{}┘", "─".repeat(BOX_WIDTH - 2))
}

/// Wrap content in `│  ...  │` padded to BOX_WIDTH.
fn box_row(content: &str) -> String {
    let inner = BOX_WIDTH - 6; // "│  " + content + "  │" = 3 + content + 3
    let truncated: String = content.chars().take(inner).collect();
    let pad = inner.saturating_sub(truncated.chars().count());
    format!("│  {}{}  │", truncated, " ".repeat(pad))
}

fn write_human<W: Write>(
    writer: &mut W,
    report: &StatsReport,
    sort: Sort,
    color: bool,
    now: u64,
) -> io::Result<()> {
    let least_used_title = match sort {
        Sort::Stale => "Least Used (stale)",
        Sort::Count => "Least Used (fewest)",
    };
    // Most Used
    write_ranked_section(writer, "Most Used", &report.most_used, color, now)?;

    // Least Used
    write_ranked_section(writer, least_used_title, &report.least_used, color, now)?;

    // Never Used
    if !report.never_used.is_empty() {
        write_never_used_section(writer, &report.never_used, color)?;
    }

    // Recency
    write_recency_section(writer, &report.recency, color)?;

    // Directory Affinity
    if !report.directory_affinity.is_empty() {
        write_affinity_section(writer, &report.directory_affinity, color)?;
    }

    // Orphan footer
    if report.orphaned_event_count > 0 {
        let footer = format!(
            "  {} orphaned event(s) (run `peanutbutter gc` to clean up)",
            report.orphaned_event_count
        );
        if color {
            writeln!(writer, "{}", footer.dimmed())?;
        } else {
            writeln!(writer, "{footer}")?;
        }
    }

    Ok(())
}

fn write_ranked_section<W: Write>(
    writer: &mut W,
    title: &str,
    snippets: &[SnippetStat],
    color: bool,
    now: u64,
) -> io::Result<()> {
    let header = box_top(title);
    if color {
        writeln!(writer, "{}", header.bold().cyan())?;
    } else {
        writeln!(writer, "{header}")?;
    }

    if snippets.is_empty() {
        writeln!(writer, "{}", box_row("  (none)"))?;
    } else {
        for (i, s) in snippets.iter().enumerate() {
            let rank = format!("{:2}.", i + 1);
            let badge = recency_badge(s.last_seen, now);
            let uses_label = if s.count == 1 { "use " } else { "uses" };
            // name field: up to 26 chars
            let name: String = s.name.chars().take(26).collect();
            let name_pad = 26usize.saturating_sub(name.chars().count());
            let count_str = format!("{:3} {}", s.count, uses_label);
            let content = format!(
                "{}  {}{} {}  {}",
                rank,
                name,
                " ".repeat(name_pad),
                count_str,
                badge
            );
            let row = box_row(&content);
            writeln!(writer, "{row}")?;
        }
    }

    let footer = box_bottom();
    if color {
        writeln!(writer, "{}", footer.cyan())?;
    } else {
        writeln!(writer, "{footer}")?;
    }
    writeln!(writer)
}

fn write_never_used_section<W: Write>(
    writer: &mut W,
    never_used: &[(SnippetId, String)],
    color: bool,
) -> io::Result<()> {
    let header = box_top("Never Used");
    if color {
        writeln!(writer, "{}", header.bold().cyan())?;
    } else {
        writeln!(writer, "{header}")?;
    }

    for (_, name) in never_used {
        let name_trunc: String = name.chars().take(44).collect();
        let content = format!("• {name_trunc}");
        let row = box_row(&content);
        if color {
            writeln!(writer, "{}", row.dimmed())?;
        } else {
            writeln!(writer, "{row}")?;
        }
    }

    let footer = box_bottom();
    if color {
        writeln!(writer, "{}", footer.cyan())?;
    } else {
        writeln!(writer, "{footer}")?;
    }
    writeln!(writer)
}

fn write_recency_section<W: Write>(
    writer: &mut W,
    recency: &RecencyBuckets,
    color: bool,
) -> io::Result<()> {
    let header = box_top("Recency");
    if color {
        writeln!(writer, "{}", header.bold().cyan())?;
    } else {
        writeln!(writer, "{header}")?;
    }

    let max = [
        recency.today,
        recency.this_week,
        recency.this_month,
        recency.older,
    ]
    .into_iter()
    .max()
    .unwrap_or(0);

    let rows = [
        ("Today      ", recency.today, 0u8),   // bright green → color tag
        ("This week  ", recency.this_week, 1), // yellow
        ("This month ", recency.this_month, 2), // dim yellow
        ("Older      ", recency.older, 3),     // dim red
    ];

    for (label, count, color_idx) in rows {
        let bar = render_bar(count, max);
        let content = format!("{label}  {bar:<20}  {count}");
        let row = box_row(&content);
        if color {
            let colored = match color_idx {
                0 => format!("{}", row.bright_green()),
                1 => format!("{}", row.yellow()),
                2 => format!("{}", row.yellow().dimmed()),
                _ => format!("{}", row.red().dimmed()),
            };
            writeln!(writer, "{colored}")?;
        } else {
            writeln!(writer, "{row}")?;
        }
    }

    let footer = box_bottom();
    if color {
        writeln!(writer, "{}", footer.cyan())?;
    } else {
        writeln!(writer, "{footer}")?;
    }
    writeln!(writer)
}

fn write_affinity_section<W: Write>(
    writer: &mut W,
    affinity: &[SnippetStat],
    color: bool,
) -> io::Result<()> {
    let header = box_top("Directory Affinity");
    if color {
        writeln!(writer, "{}", header.bold().cyan())?;
    } else {
        writeln!(writer, "{header}")?;
    }

    for s in affinity {
        let name: String = s.name.chars().take(18).collect();
        // First cwd on same line as name, rest indented
        let top_cwds: Vec<&(PathBuf, usize)> = s.cwds.iter().take(3).collect();
        for (i, (path, count)) in top_cwds.iter().enumerate() {
            let path_str = shorten_path(&path.to_string_lossy());
            let path_trunc: String = path_str.chars().take(28).collect();
            let content = if i == 0 {
                let name_pad = 18usize.saturating_sub(name.chars().count());
                format!(
                    "  {}{} {} ({})",
                    name,
                    " ".repeat(name_pad),
                    path_trunc,
                    count
                )
            } else {
                format!("  {}   {} ({})", " ".repeat(18), path_trunc, count)
            };
            let row = box_row(&content);
            if color {
                writeln!(writer, "{}", row.dimmed())?;
            } else {
                writeln!(writer, "{row}")?;
            }
        }
    }

    let footer = box_bottom();
    if color {
        writeln!(writer, "{}", footer.cyan())?;
    } else {
        writeln!(writer, "{footer}")?;
    }
    writeln!(writer)
}

fn render_bar(count: usize, max: usize) -> String {
    if max == 0 || count == 0 {
        return String::new();
    }
    let width = ((count as f64 / max as f64) * 20.0).round() as usize;
    "█".repeat(width.max(1))
}

fn shorten_path(path: &str) -> String {
    if let Some(home) = std::env::var_os("HOME").and_then(|h| h.into_string().ok())
        && path.starts_with(&home)
    {
        return format!("~{}", &path[home.len()..]);
    }
    path.to_string()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Paths;
    use crate::domain::SnippetId;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn temp_dir(prefix: &str) -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let path = std::env::temp_dir().join(format!(
            "pb-stats-{prefix}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn test_paths(root: &std::path::Path) -> Paths {
        Paths {
            snippet_roots: vec![root.to_path_buf()],
            xdg_snippets_dir: root.to_path_buf(),
            snippet_overrides_active: false,
            state_file: root.join("state.tsv"),
            config_file: root.join("config.toml"),
        }
    }

    fn opts_plain() -> StatsOptions {
        StatsOptions {
            top_n: 10,
            sort: Sort::Stale,
            json: false,
        }
    }

    fn write_snippet(root: &std::path::Path, file: &str, _slug: &str, name: &str) {
        let content = format!("## {name}\n\n```\necho hi\n```\n");
        let path = root.join(file);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    fn record_event(store: &mut FrecencyStore, file: &str, slug: &str, cwd: &str, ts: u64) {
        store.record(SnippetId::new(file, slug), PathBuf::from(cwd), ts);
    }

    const NOW: u64 = 1_715_600_000; // fixed "now" for tests

    #[test]
    fn missing_state_file_prints_no_history_note() {
        let root = temp_dir("no-state");
        write_snippet(&root, "snippets.md", "echo", "Echo");
        let paths = test_paths(&root);
        // state file does not exist

        let mut out = Vec::new();
        run_with(&paths, opts_plain(), NOW, false, &mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("No frecency history yet"));
    }

    #[test]
    fn missing_state_file_json_emits_empty_json() {
        let root = temp_dir("no-state-json");
        write_snippet(&root, "snippets.md", "echo", "Echo");
        let paths = test_paths(&root);

        let mut out = Vec::new();
        run_with(
            &paths,
            StatsOptions {
                json: true,
                ..opts_plain()
            },
            NOW,
            false,
            &mut out,
        )
        .unwrap();
        let s = String::from_utf8(out).unwrap();
        let v: serde_json::Value = serde_json::from_str(s.trim()).unwrap();
        assert_eq!(v["most_used"], serde_json::json!([]));
        assert_eq!(v["orphaned_event_count"], 0);
    }

    #[test]
    fn empty_store_file_produces_report_not_no_history_note() {
        let root = temp_dir("empty-store");
        write_snippet(&root, "snippets.md", "echo", "Echo");
        let paths = test_paths(&root);
        // Create an empty (but existing) state file
        fs::write(&paths.state_file, "").unwrap();

        let mut out = Vec::new();
        run_with(&paths, opts_plain(), NOW, false, &mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        // Should NOT print the "no history" message — the file exists, just no events
        assert!(!s.contains("No frecency history yet"));
        // Should show never-used section since snippet exists but has no events
        assert!(s.contains("Never Used") || s.contains("Echo"));
    }

    #[test]
    fn orphaned_ids_excluded_from_ranked_lists() {
        let root = temp_dir("orphans-only");
        write_snippet(&root, "snippets.md", "echo", "Echo");
        let paths = test_paths(&root);
        let mut store = FrecencyStore::new();
        // Only orphaned events (id not in index)
        record_event(&mut store, "gone.md", "vanished", "/repo", NOW - 100);
        store.save(&paths.state_file).unwrap();

        let mut out = Vec::new();
        run_with(&paths, opts_plain(), NOW, false, &mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        // vanished should not appear in any list
        assert!(!s.contains("vanished"));
        // orphan count should appear
        assert!(s.contains("orphaned"));
    }

    #[test]
    fn never_used_shows_snippets_with_no_events() {
        let root = temp_dir("never-used");
        // slugify("A") = "a", slugify("B") = "b"
        write_snippet(&root, "a.md", "a", "A");
        write_snippet(&root, "b.md", "b", "B");
        let paths = test_paths(&root);
        let mut store = FrecencyStore::new();
        record_event(&mut store, "a.md", "a", "/repo", NOW - 100);
        store.save(&paths.state_file).unwrap();

        let mut out = Vec::new();
        run_with(&paths, opts_plain(), NOW, false, &mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("Never Used"));
        assert!(s.contains("Most Used"));
    }

    #[test]
    fn recency_bucket_assignment_uses_injected_now() {
        let root = temp_dir("recency");
        write_snippet(&root, "a.md", "a", "A");
        write_snippet(&root, "b.md", "b", "B");
        write_snippet(&root, "c.md", "c", "C");
        write_snippet(&root, "d.md", "d", "D");
        let paths = test_paths(&root);
        let mut store = FrecencyStore::new();
        // today
        record_event(&mut store, "a.md", "a", "/repo", NOW);
        // this week (3 days ago)
        record_event(&mut store, "b.md", "b", "/repo", NOW - 3 * 86400);
        // this month (15 days ago)
        record_event(&mut store, "c.md", "c", "/repo", NOW - 15 * 86400);
        // older (60 days ago)
        record_event(&mut store, "d.md", "d", "/repo", NOW - 60 * 86400);
        store.save(&paths.state_file).unwrap();

        let index = load_from_roots(&paths.snippet_roots).unwrap();
        let report = compute_report(&index, &store, &StatsOptions::default(), NOW);

        assert_eq!(report.recency.today, 1);
        assert_eq!(report.recency.this_week, 1);
        assert_eq!(report.recency.this_month, 1);
        assert_eq!(report.recency.older, 1);
    }

    #[test]
    fn sort_stale_orders_by_last_seen_asc() {
        let root = temp_dir("sort-stale");
        write_snippet(&root, "a.md", "a", "A");
        write_snippet(&root, "b.md", "b", "B");
        let paths = test_paths(&root);
        let mut store = FrecencyStore::new();
        record_event(&mut store, "a.md", "a", "/repo", NOW - 1000); // older
        record_event(&mut store, "b.md", "b", "/repo", NOW - 100); // newer
        store.save(&paths.state_file).unwrap();

        let index = load_from_roots(&paths.snippet_roots).unwrap();
        let opts = StatsOptions {
            sort: Sort::Stale,
            ..Default::default()
        };
        let report = compute_report(&index, &store, &opts, NOW);

        assert_eq!(report.least_used[0].id.as_str(), "a.md#a");
        assert_eq!(report.least_used[1].id.as_str(), "b.md#b");
    }

    #[test]
    fn sort_count_orders_by_count_asc() {
        let root = temp_dir("sort-count");
        write_snippet(&root, "a.md", "a", "A");
        write_snippet(&root, "b.md", "b", "B");
        let paths = test_paths(&root);
        let mut store = FrecencyStore::new();
        record_event(&mut store, "a.md", "a", "/repo", NOW - 100);
        record_event(&mut store, "b.md", "b", "/repo", NOW - 200);
        record_event(&mut store, "b.md", "b", "/repo", NOW - 300);
        store.save(&paths.state_file).unwrap();

        let index = load_from_roots(&paths.snippet_roots).unwrap();
        let opts = StatsOptions {
            sort: Sort::Count,
            ..Default::default()
        };
        let report = compute_report(&index, &store, &opts, NOW);

        // a has 1 use (fewer), b has 2 uses
        assert_eq!(report.least_used[0].id.as_str(), "a.md#a");
    }

    #[test]
    fn color_false_produces_no_ansi_codes() {
        let root = temp_dir("no-color");
        write_snippet(&root, "a.md", "a", "Alpha Snippet");
        let paths = test_paths(&root);
        let mut store = FrecencyStore::new();
        record_event(&mut store, "a.md", "a", "/repo", NOW);
        store.save(&paths.state_file).unwrap();

        let mut out = Vec::new();
        run_with(&paths, opts_plain(), NOW, false, &mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(!s.contains("\x1b["), "ANSI escape found in plain output");
    }

    #[test]
    fn json_output_contains_all_required_keys() {
        let root = temp_dir("json-keys");
        write_snippet(&root, "a.md", "a", "Alpha");
        write_snippet(&root, "b.md", "b", "Beta");
        let paths = test_paths(&root);
        let mut store = FrecencyStore::new();
        record_event(&mut store, "a.md", "a", "/repo", NOW);
        store.save(&paths.state_file).unwrap();

        let mut out = Vec::new();
        run_with(
            &paths,
            StatsOptions {
                json: true,
                ..opts_plain()
            },
            NOW,
            false,
            &mut out,
        )
        .unwrap();
        let s = String::from_utf8(out).unwrap();
        let v: serde_json::Value = serde_json::from_str(s.trim()).unwrap();

        assert!(v["most_used"].is_array());
        assert!(v["least_used"].is_array());
        assert!(v["never_used"].is_array());
        assert!(v["recency"].is_object());
        assert!(v["directory_affinity"].is_array());
        assert!(v["orphaned_event_count"].is_number());
        // Beta is never-used
        assert!(!v["never_used"].as_array().unwrap().is_empty());
    }

    #[test]
    fn mixed_known_orphan_never_used() {
        let root = temp_dir("mixed");
        // Single-letter names → slugify("A") = "a", slugify("B") = "b"
        write_snippet(&root, "a.md", "a", "A");
        write_snippet(&root, "b.md", "b", "B");
        // c.md not created → no index entry → c events are orphans
        let paths = test_paths(&root);
        let mut store = FrecencyStore::new();
        record_event(&mut store, "a.md", "a", "/repo", NOW);
        record_event(&mut store, "c.md", "c", "/repo", NOW - 50); // orphan
        store.save(&paths.state_file).unwrap();

        let index = load_from_roots(&paths.snippet_roots).unwrap();
        let report = compute_report(&index, &store, &StatsOptions::default(), NOW);

        assert_eq!(report.most_used.len(), 1);
        assert_eq!(report.most_used[0].id.as_str(), "a.md#a");
        assert_eq!(report.orphaned_event_count, 1);
        assert_eq!(report.never_used.len(), 1);
        assert_eq!(report.never_used[0].1, "B");
    }
}
