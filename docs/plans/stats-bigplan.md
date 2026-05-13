# BIGPLAN: frecency stats command

## Plan Overview

Add a `peanutbutter stats` subcommand that reads the frecency store and the
snippet index, then surfaces actionable insights: most-used, least-used, never-
used, a recency breakdown, and per-directory affinity. The command is strictly
read-only — it never modifies the store or index. "Done" means a user can run
`peanutbutter stats` and get a visually clear, color-coded terminal summary
(with `--json` for scripting) that helps them curate their snippet library.
Colored output is a first-class requirement; the human-readable path is the
primary UX, not a fallback.

## Risks

- **Empty / missing state file** — A fresh install has no frecency data. The
  command must degrade gracefully (print a note, exit 0) rather than crash or
  show misleading zeros. Note: `FrecencyStore::load` silently returns an empty
  store when the file is absent — detect the "no history" case by checking
  `paths.state_file.exists()` before loading, not by testing whether the loaded
  store is empty (the file could exist but be empty due to GC).
- **Index vs. store drift** — Snippets in the store but absent from the index
  (orphans) already exist; the stats command must not count them as "used"
  snippets, or the "most used" list will show phantom entries. Cross-reference
  both data sources explicitly. Orphaned ids appear in the store but resolve to
  no name; they are excluded from all ranked lists and reported as a total count
  only.
- **Performance on large stores** — The frecency store is a flat TSV that is
  fully loaded into memory. Stats only needs counts and timestamps, so a single
  O(n) pass over `events()` is sufficient and must not call `score`.
- **Issue wording vs. raw usage stats** — Issue #24 mentions "combined frecency
  score" for the most-used list, but this command reports raw usage insights.
  Use event counts and last-seen timestamps here, not the interactive ranking
  score, so the output remains explainable and does not mix in cwd weighting or
  time decay. If #24 needs alignment, leave a short issue comment when shipping.
- **Color bleed when piping** — ANSI escape codes written unconditionally break
  `grep`, `less`, and log capture. Color must be disabled automatically when
  stdout is not a TTY (`std::io::IsTerminal`) and when the `NO_COLOR` env var
  is set. `--json` always implies no color.

## Plan Details

### Critical Files

- `src/frecency.rs` — `FrecencyStore`, `UsageEvent` — source of raw events.
  `events()` returns `&[UsageEvent]`; the stats module reads this slice directly.
- `src/index.rs` — `SnippetIndex`, `IndexedSnippet` — source of truth for which
  snippets currently exist. Used to classify events as current vs. orphaned, and
  to find never-used snippets.
- `src/config.rs` — `Paths` struct — carries `state_file: PathBuf` (frecency
  TSV path) and `snippet_roots: Vec<PathBuf>`. These are the two inputs to
  `stats::run`.
- `src/cli.rs` — `Command` enum and dispatch helpers. Add a `Stats` variant here.
- `src/main.rs` — `match command` arm for `Command::Stats`.
- `src/lib.rs` — add `pub mod stats;`.
- `src/gc.rs` — structural reference: Options struct, Result struct, `run` /
  `run_with` pair, testable via injected writer.
- `tests/examples.rs` — integration test location.
- `Cargo.toml` — add `owo-colors = "4"` dependency.

### UX & Visual Design

New dependency: `owo-colors = "4"`. Chosen over repurposing `crossterm::style`
(already a dep) because `crossterm` provides no `NO_COLOR`/isatty detection for
plain stdout and its styled-string API is not idiomatic outside the TUI context.
`owo-colors` is zero-allocation, zero-dep in its core, and works via an
`OwoColorize` trait on any `&str` / `String`.

**Color detection** (check in order; any match → plain text):
1. `--json` flag is set.
2. `NO_COLOR` env var is present (any value).
3. `std::io::stdout().is_terminal()` returns false (pipe / file redirect).

`run` computes a single `bool color` and passes it through `run_with` into the
formatter; do not embed the check inside every `write!` call. Tests call
`run_with` with `color: false`.

**Color scheme (human-readable output):**

```
┌─ Most Used ─────────────────────────────────────┐   ← bold cyan header
│  1.  Git Log                     42 uses  today  │   ← name: white/default
│  2.  Docker Run                  17 uses  1 wk   │     count: bold yellow
│  3.  SSH Jump                     9 uses  1 mo   │     recency badge: dim
└─────────────────────────────────────────────────┘

┌─ Least Used (stale) ────────────────────────────┐   ← bold cyan header
│  1.  K8s Rollout                  1 use  8 mo   │
└─────────────────────────────────────────────────┘

┌─ Never Used ────────────────────────────────────┐   ← bold cyan header  
│  • Ansible Playbook                             │   ← bullet: dim yellow
│  • Helm Upgrade                                 │
└─────────────────────────────────────────────────┘

┌─ Recency ───────────────────────────────────────┐
│  Today        ████████  3                       │   ← bar: bright green
│  This week    ████████████████  12              │   ← bar: yellow
│  This month   ████████████████████████  25     │   ← bar: dim yellow
│  Older        ███████  7                        │   ← bar: dim red
└─────────────────────────────────────────────────┘

┌─ Directory Affinity ────────────────────────────┐
│  Git Log          ~/code/peanutbutter (31)      │   ← cwd: dim
│                   ~/code/work (8)               │
│                   ~/code/dotfiles (3)           │
└─────────────────────────────────────────────────┘

  2 orphaned events (run `peanutbutter gc` to clean up)   ← dim footer
```

Box-drawing characters (`┌ ─ ┐ │ └ ┘`) are Unicode. Do not add an ASCII fallback
unless a future `--no-unicode` flag is introduced.

The bar chart in the Recency section uses `█` characters scaled to a fixed
width of 20 columns. Scale bars proportionally to the largest bucket count.

### Gotchas

- `FrecencyStore::score` is not needed here; call `store.events()` and aggregate
  in one pass. Do not reuse the scoring path — it applies time-decay and
  location weighting, which would distort raw usage counts. This intentionally
  differs from issue #24's initial "combined frecency score" wording.
- `FrecencyStore::load` already silently handles a missing file by returning an
  empty store. The "no history yet" UX path in Deliverable 1 must check
  `paths.state_file.exists()` explicitly before calling `load`, so the two
  cases ("never run" vs. "all events GC'd") can produce different messages.
- One snippet id can have many events — group by id first, then derive
  per-id stats (total count, most-recent timestamp, unique cwds).
- `run_with` must accept an explicit `now: u64` parameter (unix seconds) so
  recency-bucket tests can control the clock. Do not call `SystemTime::now()`
  inside the report computation — mirror how `frecency.rs::score` handles it.
- Recency buckets classify snippets by their most recent use, not individual
  events. Bucket boundaries (compare per-snippet `last_seen` to `now`):
  - **Today**: within the same calendar day in UTC (i.e. `now / 86400 == last_seen / 86400`).
  - **This week**: `now - last_seen < 7 * 86400`.
  - **This month**: `now - last_seen < 30 * 86400`.
  - **Older**: everything else.
  Apply in priority order: today → week → month → older.
- `least_used` sorts by `last_seen asc` (stalest first) by default; a
  `--sort=count` flag switches to `count asc` (fewest uses first). These are
  different rankings and must be controlled by `StatsOptions::sort`.
- `load_from_roots` propagates `io::Err` on unreadable files — this is
  intentional, following the `gc.rs` pattern. Stats will exit with an I/O
  error if any snippet file is unreadable.
- The `UsageEvent.cwd` field is stored as-recorded. Two cwds that are the same
  logical directory may differ in representation (trailing slash, symlink
  resolution). Keep affinity output as-is; do not try to canonicalize.
- The `SnippetId` type implements `Display` as `"<rel_path>#<slug>"`. Use
  `IndexedSnippet::name()` for human-readable output, falling back to the id
  string for orphaned entries that have no index entry.
- For `directory_affinity` in human-readable mode: show at most 3 cwds per
  snippet, sorted by frequency (most-used cwd first). Truncate long paths with
  `…` if they exceed terminal width (80 chars is a safe assumption for now).
- JSON: all array-valued keys (`most_used`, `least_used`, `never_used`,
  `directory_affinity`) must always be present as arrays, even when empty. Never
  emit `null` or omit a key.
- Color is controlled by a single `bool` computed once at the top of `run` and
  threaded into `run_with` / the formatter. `run_with` must accept an explicit
  color setting so tests and non-stdout callers are not coupled to the real
  process stdout. Do not re-check isatty or `NO_COLOR` inside individual
  `write!` calls — that makes tests and piping behavior unpredictable.
- `owo-colors` styles strings via the `OwoColorize` trait. Import it with
  `use owo_colors::OwoColorize;`. Each `.bold()`, `.cyan()`, etc. call returns
  a wrapper that implements `Display`; they compose: `"text".bold().cyan()`.
- The recency bar chart scales all bars relative to the largest bucket. If the
  largest bucket has count N, each bar is `round(count / N * 20)` `█` chars.
  Min bar width is 1 if count > 0.
- Follow the new-style Rust module convention: `src/stats.rs`, not
  `src/stats/mod.rs`.

### Pseudo-code / Sketches

```text
// Single pass over events → per-id aggregation
let mut by_id: HashMap<SnippetId, EventSummary> = HashMap::new();
for event in store.events() {
    let s = by_id.entry(event.id.clone()).or_default();
    s.count += 1;
    s.last_seen = s.last_seen.max(event.timestamp);
    s.cwds.entry(event.cwd.clone()).and_modify(|n| *n += 1).or_insert(1);
}

// Classify: current vs orphaned
let current_ids: HashSet<SnippetId> = index.iter().map(|s| s.id().clone()).collect();
let mut known: Vec<(SnippetId, EventSummary)> = Vec::new();
let mut orphan_event_count: usize = 0;
for (id, summary) in by_id {
    if current_ids.contains(&id) {
        known.push((id, summary));
    } else {
        orphan_event_count += summary.count;
    }
}

// Never-used: in index but not in any known entry
let known_ids: HashSet<&SnippetId> = known.iter().map(|(id, _)| id).collect();
let never_used: Vec<&IndexedSnippet> = index.iter()
    .filter(|s| !known_ids.contains(s.id()))
    .collect();

// Ranked lists (top_n controlled by StatsOptions)
let mut most_used = known.clone();
most_used.sort_by(|(_, a), (_, b)| b.count.cmp(&a.count));
most_used.truncate(top_n);

let mut least_used = known.clone();
match options.sort {
    Sort::Stale  => least_used.sort_by(|(_, a), (_, b)| a.last_seen.cmp(&b.last_seen)),
    Sort::Count  => least_used.sort_by(|(_, a), (_, b)| a.count.cmp(&b.count)),
}
least_used.truncate(top_n);

// Recency buckets (compare last_seen to now)
// today / this week / this month / older

// Output
human-readable: labelled sections, aligned columns
json: { most_used, least_used, never_used, recency, directory_affinity }
```

**Example JSON shape:**
```json
{
  "most_used": [
    { "id": "git.md#log", "name": "Git Log", "count": 42, "last_seen": 1715600000 }
  ],
  "least_used": [
    { "id": "docker.md#prune", "name": "Docker Prune", "count": 1, "last_seen": 1710000000 }
  ],
  "never_used": [
    { "id": "k8s.md#rollout", "name": "K8s Rollout" }
  ],
  "recency": {
    "today": 3, "this_week": 12, "this_month": 25, "older": 7
  },
  "directory_affinity": [
    {
      "id": "git.md#log",
      "name": "Git Log",
      "cwds": [
        { "path": "/home/me/repo", "count": 31 },
        { "path": "/home/me/work", "count": 8 }
      ]
    }
  ],
  "orphaned_event_count": 2
}
```

## Deliverables

### Deliverable 1. `src/stats.rs` module

Implement the pure analysis logic: a `StatsOptions` struct, a `StatsReport`
data type, and a `run` / `run_with` function pair that takes `&Paths`, a
`StatsOptions`, a `now: u64`, an explicit `color: bool`, and an output writer,
then loads the index and store, computes the report, and prints it. `run`
computes `now` and the stdout/env-based color setting before delegating to
`run_with`; `run_with` remains deterministic for tests. The module must not
mutate either the index or the store.

`StatsOptions`:
- `top_n: usize` — how many snippets to show in most-used / least-used lists
  (default 10)
- `sort: Sort` — enum `Stale` (default, sort by last_seen asc) or `Count`
  (sort by count asc) for the least-used list
- `json: bool` — emit JSON instead of human-readable text

`StatsReport` holds computed data; the printing logic is a separate function so
the report can be tested without capturing stdout.

Orphaned events are excluded from all ranked lists. Their total event count is
reported as a single `orphaned_event_count` field (JSON) or a footer line
(human-readable).

- [x] Create `src/stats.rs` with `StatsOptions` (with `Default`), `Sort` enum,
      `StatsReport`, `run`, and `run_with(paths, options, now, color, writer)`
- [x] In `run`: compute `now` from `SystemTime`, compute `color` as false if
      `options.json`, `NO_COLOR` is set, or `stdout().is_terminal()` is false,
      then call `run_with`
- [x] In `run_with`: check `paths.state_file.exists()` before calling
      `FrecencyStore::load`; if missing, print
      `"No frecency history yet - use snippets first."` and return early
      (exit 0)
- [x] When the index is empty, print `"No snippets found in configured roots."`
      and return early (exit 0)
- [x] For `--json` empty states, emit valid JSON with all array keys as `[]`
      and `orphaned_event_count: 0` rather than prose messages
- [x] Single-pass event aggregation: count, last-seen timestamp, cwd frequency
      map per snippet id
- [x] Classify snippet ids: current (in index) vs. orphaned (not in index);
      accumulate `orphan_event_count`
- [x] Compute never-used list: index entries with no events (use `known_ids`
      HashSet built from classified known entries)
- [x] Compute recency buckets from per-snippet `last_seen` values with injected
      `now` using the UTC-day / 7-day / 30-day boundaries defined in Gotchas
- [x] Compute `most_used`: sort known by count desc, truncate to `top_n`
- [x] Compute `least_used`: sort by `Sort::Stale` (last_seen asc) or
      `Sort::Count` (count asc), truncate to `top_n`
- [x] Compute directory affinity per snippet: cwd sorted by frequency desc,
      capped at 3 in human-readable mode (unlimited in JSON)
- [x] Human-readable formatter: box-drawing section headers, aligned columns,
      recency bar chart, orphan count footer — all styled per the UX Design
      spec using `owo-colors`
- [x] JSON formatter: schema as specified above; all array keys always present;
      `directory_affinity.cwds` entries include both `path` and `count`;
      `orphaned_event_count` always present; no ANSI codes regardless of TTY
- [x] Unit tests: empty store + existing index (never-used path), store with
      orphaned ids only, mixed known + orphan + never-used, recency bucket
      assignment with controlled `now`, `Sort::Stale` vs `Sort::Count` ordering,
      missing state file prose output, `--json` missing state file output, JSON
      output shape (all keys present, empty arrays when appropriate),
      color=false produces plain text with no ANSI escape sequences

### Deliverable 2. Wire `stats` into CLI

Add `Stats` to the `Command` enum, register the dispatch arm, and expose the
new module.

- [x] Add `owo-colors = "4"` to `[dependencies]` in `Cargo.toml`
- [x] Add `pub mod stats;` to `src/lib.rs`
- [x] Import `crate::stats` in `src/cli.rs` so the `Stats` variant can use
      `stats::Sort`
- [x] Add to `Command` enum in `src/cli.rs`:
      ```rust
      /// Show usage statistics from frecency history.
      Stats {
          #[arg(long, default_value_t = 10)]
          top: usize,
          #[arg(long, value_enum, default_value_t = stats::Sort::Stale)]
          sort: stats::Sort,
          #[arg(long)]
          json: bool,
      }
      ```
- [x] Derive `clap::ValueEnum` on `stats::Sort` and rustdoc its public variants
- [x] Add `Command::Stats { top, sort, json }` arm in `src/main.rs` dispatching
      to `peanutbutter::stats::run(&paths, StatsOptions { top_n: top, sort, json }, &mut stdout)`
- [x] Add integration test in `tests/examples.rs` covering:
      - `peanutbutter stats` with a missing state file (no history note)
      - `peanutbutter stats` with an existing empty state file (zero-history
        report, not the fresh-install note)
      - `peanutbutter stats` with a fixture TSV containing known events
      - `peanutbutter stats --json` produces valid JSON with all required keys
- [x] Verify `cargo test` and pre-commit hooks pass

### Deliverable 3. Align issue #24 wording with raw usage semantics

The plan intentionally reports raw usage counts rather than the combined
interactive frecency score. Keep that product choice visible on the originating
issue so future review does not mistake it for a missed acceptance criterion.

- [x] Post a short comment on issue #24 noting that `stats` reports raw event
      counts, last-seen timestamps, and directory frequencies instead of
      `FrecencyStore::score`, because the command is for explainable usage
      analysis rather than interactive ranking

### Deliverable 4. Notify issue #21 of stats theming dependency

Post a comment on https://github.com/calamity-m/peanutbutter/issues/21 noting
that the `stats` subcommand ships with a hardcoded color scheme (via
`owo-colors`) and should be updated to respect the resolved theme once the
theming system from that issue is implemented. This keeps the future work
visible on the theming issue itself.

- [x] Post comment on issue #21: "The `stats` subcommand (tracked in #24) ships
      with a hardcoded color scheme using `owo-colors`. When this theming issue
      is implemented, `stats` should be updated to derive its accent/muted colors
      from the resolved `Theme` rather than the current hardcoded palette."

## Issues

- **2026-05-13 — agent:claude** — All four deliverables implemented and shipped.
  `src/stats.rs` created (203 tests passing, clippy clean). CLI wired in
  `cli.rs` / `main.rs` / `lib.rs`. Integration tests added to
  `tests/examples.rs`. Issue comments posted on #24 and #21. PR opened on
  `feat/stats`.

- **2026-05-13 — agent:codex** — Applied review fixes: clarified raw counts vs
  issue #24's frecency-score wording, moved color detection to `run` with
  explicit `run_with` color input, added CLI/import/ValueEnum notes, changed
  JSON directory affinity to include cwd counts, clarified recency buckets as
  per-snippet `last_seen`, folded empty-state handling into Deliverable 1, and
  fixed the Unicode box-drawing note.
- **2026-05-13 — agent:claude** — User added pretty/colored output as a core
  requirement. Added `owo-colors = "4"` as new dep; added UX Design section
  with full color scheme and ASCII mockup; added color detection logic (isatty +
  NO_COLOR + json flag); added recency bar chart spec; added color tasks to
  Deliverable 1 and 2.
- **2026-05-13 — agent:claude (adversarial review)** — Plan reviewed by 2
  adversarial sub-agents (Risks & Assumptions, Completeness & Scope). 13
  findings; 12 merged directly into plan. One decision deferred to user:
  `least_used` sort semantics — resolved as `--sort=[stale|count]` flag,
  defaulting to stale.
- **2026-05-13 — agent:claude** — Skipped grill because the issue acceptance
  criteria are explicit, the data model is fully readable in `frecency.rs`, and
  scope is read-only (no store mutations). Brief is crisp.
