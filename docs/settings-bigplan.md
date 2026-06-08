# BIGPLAN: `pb settings` — interactive config tuning TUI

## Plan Overview

Add a `settings` subcommand that opens an interactive TUI for browsing and
editing `AppConfig` (`src/config.rs:31`), starting with the **search** section as
the first and only v1 entry. The user drills `settings → search → frecency|fuzzy`
into slider-style tuners where each numeric weight is a grow/shrink progress bar.
Bars are purely value editors (no live snippet re-scoring in v1) but colour a
field when its weight is *overdominant* relative to its siblings, so the user gets
visual feedback on balance. On save, only the keys the user touched are written
back into `config.toml` surgically (comments and layout preserved) via
`toml_edit`. "Done" for v1: `pb settings` opens a section picker, the search
branch lets you adjust frecency and fuzzy weights with live bar feedback, and
saving rewrites just those keys in the user's existing config file.

## Risks

- **No existing config write path** — `config::load()` is read-only and *merges*
  env vars (`PEANUTBUTTER_PATH`, `PB_STATE_FILE`) into the resolved `AppConfig`.
  Serializing the whole struct back would bake env-derived values into the file.
  Mitigation: never serialize `AppConfig`; persistence touches only the specific
  `[search]` / `[search.frecency]` / `[search.fuzzy]` keys the user edited, using
  `toml_edit` against the raw file text. This is exactly why "surgical key edits"
  was chosen over "regenerate".
- **Mixed value domains break a shared bar scale** — `frecency_weight` defaults to
  `250.0` while the frecency sub-weights are `~1.0` and fuzzy weights are small
  ints (`8`–`30`). A single bar max would render the small values as invisible
  slivers. Mitigation: each field carries its own `min`/`max`/`step` domain used
  for bar scaling and clamping (see Pseudo-code).
- **TOML table targeting is non-uniform** — `frecency_weight` sits directly under
  `[search]`, but `half_life_days` / `location_weight` / `frequency_weight` live
  under `[search.frecency]`, and fuzzy weights under `[search.fuzzy]`. A naive
  "write key into search table" loses this. Mitigation: each editable field
  declares its full TOML path (table chain + key); persistence creates missing
  tables on the way down. Covered by a round-trip test.
- **Terminal restore on every exit path** — standard TUI invariant; a panic or
  early-return that skips raw-mode restore wrecks the user's shell. Mitigation:
  use the same Drop-based guards `run_scrollable_text` uses — enter **both**
  `StdoutTtyGuard` *and* `RawModeGuard`. Do **not** assume `StdoutTtyGuard` is
  optional here: `run_scrollable_text` enters it unconditionally and `stats.rs:394`
  explicitly notes the `is_terminal` gate does not apply to the TUI path, because
  crossterm still issues fd-1 DSR cursor queries regardless of `$(...)` capture
  (per CLAUDE.md). Keep the guard; an earlier draft wrongly called it unnecessary.
- **No generic TUI loop to reuse** — the plan initially assumed a shared "harness";
  in fact only `run_scrollable_text` (a text-scroller) exists and it isn't reusable
  for an editor (see Plan Details "Reuse boundary"). Mitigation: Deliverable 1
  extracts a small lifecycle primitive or exposes the guards `pub(crate)` and writes
  a new loop — sized into D1, not treated as free reuse.
- **Non-tty invocation has no defined behavior** — `pb settings | cat`, CI, or any
  non-terminal stdout would leave an interactive editor with nothing to drive it;
  `event::poll` could hang or output could garble. Unlike `stats`, settings has no
  meaningful text fallback. Mitigation: gate on `io::stdout().is_terminal()` and, if
  false, print an error and exit non-zero before entering raw mode (D1 acceptance,
  D5 test).

## Plan Details

`pb settings` is an interactive, full-screen-ish inline TUI like `stats --output
tui`, **not** part of the `execute` shell-capture path. Its only side effect is
writing the config file on save.

**Reuse boundary (verified against `src/tui/`):** the `tui` module exports only
`run_scrollable_text`, `Chrome`, and `compact_viewport_height`. `run_scrollable_text`
is a hardcoded scroll-a-blob-of-text loop and is **not** reusable for an
interactive editor. The lifecycle pieces it uses internally — `StdoutTtyGuard`,
`RawModeGuard`, `TuiOutputKind`, `build_terminal` — are module-private, and
`draw_divider` lives in `chrome.rs` but isn't re-exported. So settings must write
its **own** draw+event loop. The cheapest correct path is to extract that lifecycle
(enter `StdoutTtyGuard` → `RawModeGuard` → `build_terminal` → loop → cleanup) into
a small reusable primitive in `src/tui/terminal.rs` and call it from both
`run_scrollable_text` and settings, or — if that refactor feels too broad — make
the guards/`build_terminal` `pub(crate)` and have settings drive its own loop. Only
`Chrome`, `draw_divider`, and the `List`/`ListState` idiom from
`src/execute/render.rs:181` are reused as-is. This is real net-new loop code, not a
thin wrapper — Deliverable 1 owns it.

### Navigation model

```text
pb settings
   │
   ▼
┌─────────────────────────────┐
│ Section picker (List)       │   v1: only "search" is active;
│  > search                   │   others rendered dimmed/"(coming soon)"
│    ui            (soon)      │   or simply omitted until added.
│    suggestion_commands(soon)│
│    theme         (soon)      │
└─────────────────────────────┘
   │  Enter
   ▼
┌─────────────────────────────┐
│ search sub-picker (List)    │
│  > frecency                 │
│    fuzzy                     │
└─────────────────────────────┘
   │  Enter            │  Enter
   ▼                   ▼
┌──────────────────┐ ┌──────────────────┐
│ Frecency tuner   │ │ Fuzzy tuner      │
│ (slider screen)  │ │ (slider screen)  │
└──────────────────┘ └──────────────────┘

Keys (all screens):  ↑/↓ or j/k move • Enter descend/select • Esc back • q quit
Keys (tuner screen): ↑/↓ select field • ←/→ or -/+ adjust • Esc back • s save • q quit
```

### Tuner screen layout (mock)

```text
┌ peanutbutter · settings / search / frecency ───────────────────┐
│                                                                 │
│  half_life_days    14.0   [██████████░░░░░░░░░░░░░░]            │
│> location_weight    1.0   [████████░░░░░░░░░░░░░░░░]  ◀ editing  │
│  frequency_weight   1.0   [████████░░░░░░░░░░░░░░░░]            │
│  frecency_weight  250.0   [████████████████████░░░░]  ! strong  │
│                                                                 │
├─ location_weight ──────────────────────────────────────────────┤
│ How much "I've used this snippet in this directory before"      │
│ pulls a result up the list. 0 = ignore cwd entirely.            │
│                                                                 │
│ value 1.0  ·  default 1.0  ·  impact: ▎▎▎░░ moderate (= default)│
│  ←/→ adjust · ↑/↓ field · s save · esc back · q quit            │
└─────────────────────────────────────────────────────────────────┘

  '! strong'  = field flagged overdominant (warning colour on the bar)
  '◀ editing' = currently selected field
  explanation pane = what the SELECTED field means + its impact readout
```

The lower pane always reflects the selected field: a one/two-line plain-English
explanation of *what the weight does*, plus an **impact readout** that answers
"is 1.0 high?" by showing the value relative to its default and a qualitative
band (off / low / moderate / high / dominant). The fuzzy tuner is the same screen
with six fields (`name`, `tag`, `frontmatter_name`, `description`, `path`,
`command`), each with its own explanation.

### Impact band + overdominant heuristic (vs-default ratio — locked)

Cosmetic only — never blocks save. Both the explanation-pane band and the bar's
`! strong` tint are driven by **one** function comparing a field's value to **its
own default** (`ratio = value / default`). This sidesteps the mixed-domain problem
entirely (`frecency_weight≈250`, `location/frequency_weight≈1.0`, fuzzy ints) —
every field is judged against its own baseline, no cross-field "group mean":

```text
band(field):
    if field.readout == TimeConstant   -> None        # half_life_days: no band
    if field.value == 0 and field.min == 0 -> "off"
    r = value / default
    r < 0.5      -> "low"
    0.5 ..= 1.5  -> "moderate"          # includes "= default"
    1.5 ..  3.0  -> "high"
    >= 3.0       -> "dominant"          # this is the `! strong` overdominant flag
```

`dominant` is exactly the overdominant condition, so the bar tint and the readout
band can never disagree. `half_life_days` is excluded (TimeConstant). Keep this in
one small pure function so the cutoffs are trivial to unit-test and tune later.

### Persistence (surgical)

```text
on save:
  raw = read_to_string(config_file)          # "" if file missing
  doc = raw.parse::<toml_edit::DocumentMut>() # err -> surface, KEEP edits, abort save
  for field in edited_fields where field.value != field.original:
      table = doc                              # walk/create table chain
      for seg in field.toml_path[..-1]:
          table = table.entry(seg).or_insert(implicit table)
      table[field.key] = field.value           # typed: f64 or i64
  # atomic write: tmp file in the SAME directory as config_file, then rename
  write tmp = config_file.parent() / ".config.toml.tmp-<pid>"
  fsync; rename(tmp, config_file)              # rename is only atomic same-fs

Untouched keys, comments, ordering, and unrelated sections are preserved
because toml_edit mutates the parsed document in place.
```

**Failure handling.** A save can fail three ways: the existing file doesn't parse
as TOML, the target is non-writable/read-only, or the write/rename errors. In all
cases the in-memory edits are **kept**, the error is surfaced in the chrome, and
the user can retry or quit — never silently lose tuning. The temp file lives in the
config file's own directory so `rename` stays atomic (a `/tmp` temp would `EXDEV`
across mounts). Note `config::load()` already rejects a malformed config at startup,
so the parse-failure path is an edge case (e.g. file edited out-of-band) but is
still handled rather than panicking.

### Field model (single source of truth)

Each editable field is described once and drives rendering, adjustment,
clamping, and persistence:

```text
struct Field {
    label: &str,            // "location_weight"
    toml_path: &[&str],     // ["search", "frecency"]  (table chain)
    key: &str,              // "location_weight"
    kind: Float | Int,      // f64 vs u32/i64
    min, max, step,         // domain for clamp + bar scale + adjust granularity
    default,                // the AppConfig default, for the "vs default" readout
    value, original,        // current edit value + value loaded from AppConfig
    help: &str,             // 1-2 line plain-English "what this weight does",
                            //   curated from the rustdoc in src/config.rs
}
```

### Explanation + impact readout

The lower pane answers two questions the bare bars can't: *what does this weight
do* and *is the current value high?* Two pieces:

1. **Help text** — a curated one/two-line sentence per field. Source copy from
   the existing field rustdoc in `src/config.rs` (e.g. `location_weight` →
   "Multiplier on the path-affinity term ... 0.0 to ignore cwd entirely") and
   rephrase for an end user. Stored as `Field.help`; no runtime doc parsing.

2. **Impact readout** — a value-relative judgement, *not* a re-score:
   - **vs default**: show `value` against `default` (e.g. `1.0 = default`,
     `2.5 = 2.5× default`, `0.0 = off`).
   - **qualitative band**: `off / low / moderate / high / dominant` from the
     `value / default` ratio defined in "Impact band + overdominant heuristic"
     above. `dominant` is the visible name for the `! strong` bar flag, so the two
     stay consistent by construction.

   **`half_life_days` is special-cased.** It is a non-linear time constant, not a
   linear multiplier, so a "× default" framing and the ratio band are
   actively misleading for it. For this one field, drop the multiplier and the
   band entirely; instead show its literal value plus a concrete meaning sentence
   (e.g. "a hit from 14 days ago counts half as much as a hit today"). The
   `Field` model carries a flag (e.g. `readout: Multiplier | TimeConstant`) so the
   render path picks the right treatment; only `half_life_days` uses
   `TimeConstant` today.

(See the locked `band()` definition under "Impact band + overdominant heuristic".)

Frecency group fields:
- `half_life_days`   → `[search.frecency]` · f64 · min>0
- `location_weight`  → `[search.frecency]` · f64 · min 0
- `frequency_weight` → `[search.frecency]` · f64 · min 0
- `frecency_weight`  → `[search]`           · f64 · min 0

Fuzzy group fields: `name`/`tag`/`frontmatter_name`/`description`/`path`/`command`
→ `[search.fuzzy]` · u32 · min 0. `body` is accepted as a deprecated config/query alias for existing users.

### Critical Files

- `src/cli.rs` — add a `Settings` variant to `enum Command` (`src/cli.rs:30`).
- `src/main.rs` — dispatch `Command::Settings` to `settings::run` (alongside the
  existing arms at `src/main.rs:81`+).
- `src/config.rs` — source of field defaults/values (`SearchConfig`,
  `FrecencyConfig`, `FuzzyWeights`) and `Paths.config_file` for the write target.
- `src/settings.rs` — new module root (new-style Rust, per CLAUDE.md §8.5);
  public `run(config: &AppConfig) -> io::Result<()>` entry.
- `src/settings/app.rs` — state machine (Section → SubPicker → Tuner), key
  handling, the `Field` model and overdominant heuristic.
- `src/settings/render.rs` — picker lists + tuner/bar drawing.
- `src/settings/persist.rs` — `toml_edit` surgical writer + atomic file write.
- `src/tui/terminal.rs`, `src/tui/chrome.rs` — home of the guards
  (`StdoutTtyGuard`/`RawModeGuard`, currently private), `build_terminal`, `Chrome`,
  and `draw_divider`. D1 extracts/exposes a reusable loop lifecycle here; only
  `run_scrollable_text` (the text-scroller) is *not* reusable. `stats.rs:368,394`
  is the reference for how the guards/`is_terminal` gate are used.
- `src/execute/render.rs:181` — reference `List::new` + `ListState` idiom.
- `Cargo.toml` — add `toml_edit` (companion to the existing `toml = "0.8"`).
- `examples/config.toml`, `README.md` — document the new command.

### Gotchas

- `frecency_weight` is **not** under `[search.frecency]` — it's a direct child of
  `[search]`. The `Field.toml_path` model above encodes this; don't assume one
  table per tuner screen.
- `FuzzyWeights` are `u32` in `AppConfig`; write them to TOML as integers, not
  floats, or the file round-trips as `30.0`.
- `config::load()` returns values with defaults already applied, so the tuner's
  `original` values are concrete (e.g. `250.0`) even when the file omits the key.
  `original` means exactly "the value when the screen opened" (seeded from
  `AppConfig`), and the **only** write rule is `value != original`. The tool does
  not distinguish file-set-to-default from defaulted, and never strips keys —
  adjust-then-revert writes nothing, and opening+saving untouched writes nothing.
- Seed `Field.default` and `Field.value` from the live `AppConfig` struct
  (`config.search.*`), **not** from typed-in literals, so the tuner can't drift
  from the `Default` impls in `src/config.rs`. Only the `help` strings are
  hand-maintained.
- Persistence MUST target `config.paths.config_file` — that path already honors
  `PB_CONFIG_FILE` (`resolve_config_file`, `src/config.rs:687`). Do not re-derive
  an XDG path; doing so would write to the wrong file under env override and break
  the D5 integration test.
- The config file may not exist yet (first run). Persistence must create it (and
  parent dir) rather than error.
- `examples/config.toml` is parsed by the `example_config_deserializes` unit test
  (`src/config.rs:901`). Keep D5 edits comment-only or otherwise valid TOML.
- This command is interactive and writes to the real terminal — keep debug prints
  out, restore raw mode on all exits, and enter `StdoutTtyGuard` (the `execute`
  fd1/`$(...)` *capture* invariants don't apply, but the fd-1 DSR-query reason for
  the guard still does — see Risks). If stdout is not a tty, error out before
  entering raw mode rather than launching an undriveable editor.

## Deliverables

### Deliverable 1. `settings` command skeleton + navigation shell

Wire a new `settings` subcommand end to end with the picker navigation but no
editing yet. `pb settings` enters the tui harness, shows the section picker with
**search** active, descends into the `search` sub-picker (`frecency` / `fuzzy`),
and into a placeholder tuner screen, with Esc/back and q/quit working and the
terminal restored cleanly on every exit. Establishes `src/settings.rs` +
`src/settings/{app,render}.rs` and the state machine. Demonstrable by running
`pb settings` and navigating in/out without terminal corruption.

- [x] Add `Settings` variant to `Command` in `src/cli.rs` with rustdoc.
- [x] Dispatch it in `src/main.rs` to `settings::run(&config)`.
- [x] Create `src/settings.rs` module root + `mod app; mod render;` and crate docs.
- [x] Implement the Section → SubPicker state machine in `app.rs` (search-only).
- [x] Render picker lists via the `List`/`ListState` idiom in `render.rs`.
- [x] Stand up the draw+event loop: extract a reusable lifecycle primitive in `src/tui/terminal.rs` (or make `StdoutTtyGuard`/`RawModeGuard`/`build_terminal` `pub(crate)`) and enter **both** guards; reuse `Chrome` + `draw_divider`.
- [x] Gate on `io::stdout().is_terminal()`: if not a tty, print an error and exit non-zero before raw mode (no editor launched).
- [ ] Verify navigation + clean exit manually and add a state-transition unit test.

### Deliverable 2. Tuner widget + frecency screen (in-memory editing)

Build the reusable slider/tuner screen and back it with the frecency field group.
Each field renders as `label  value  [bar]`; ↑/↓ moves selection, ←/→ (and -/+)
adjusts by the field's `step` with clamping to `[min,max]`, and bars scale per the
field's own domain. The overdominant heuristic tints flagged bars. A lower
**explanation pane** always describes the selected field — its plain-English
`help` text plus an **impact readout** (value vs default + qualitative band) so
the user can tell what a weight does and whether its value is high. Editing is
in-memory only here (save lands in Deliverable 4). Introduces the `Field` model.

- [x] Define the `Field` struct (label, toml_path, key, kind, min/max/step, default, value, original, help, readout).
- [x] Seed the frecency group's `default`/`value`/`original` from the live `config.search.frecency` + `config.search.frecency_weight` structs (not literals), with curated `help` copy adapted from the `src/config.rs` rustdoc.
- [x] Render fields as labelled bars with selection + "editing" marker.
- [x] Implement ↑/↓ select and ←/→ / -/+ adjust with per-kind step and clamping.
- [x] Implement the single `band()` function (`value/default` ratio, locked cutoffs) and apply the `! strong` (`dominant`) warning style to flagged bars.
- [x] Render the explanation pane for the selected field: `help` text + impact readout (vs-default ratio + off/low/moderate/high/dominant band from `band()`).
- [x] Special-case `half_life_days` (`readout: TimeConstant`): no ratio/band; show literal value + concrete meaning sentence instead.
- [x] Unit-test adjust/clamp math; `band()` at every cutoff (0, 0.49×, 0.5×, 1.5×, 3.0× default); that `half_life_days` returns no band; and that adjust-then-revert leaves `value == original`.

### Deliverable 3. Fuzzy tuner screen

Reuse the Deliverable 2 tuner with the six fuzzy weight fields (`u32`). Confirms
the widget is genuinely group-agnostic and that int vs float kinds adjust and
render correctly. Demonstrable: `settings → search → fuzzy` shows six adjustable
bars.

- [x] Seed the fuzzy field group from `config.search.fuzzy` as `Int` kind, with curated `help` copy per field.
- [x] Route the `fuzzy` sub-picker entry into the tuner with that group.
- [x] Verify int stepping/clamping and that values render without a decimal point.

### Deliverable 4. Surgical persistence to `config.toml`

Save (`s`) writes only changed fields into the user's config file using
`toml_edit`, preserving comments, key order, and unrelated sections, creating
missing tables/file/parent dir as needed, and writing atomically. Unchanged
fields (still equal to `original`) are not written, so defaults aren't
materialized. Demonstrable: edit a populated config, tweak one bar, save, and
`git diff` shows only that one line changed.

- [x] Add `toml_edit` to `Cargo.toml`.
- [x] Implement `persist.rs`: parse-or-empty doc, walk/create table chain per field, set typed value; on parse/IO error keep edits in memory and surface the error (no silent loss).
- [x] Write atomically to `config.paths.config_file`: temp file in the **same directory**, fsync, then rename; create parent dir if missing.
- [x] Wire the tuner `s` key to persist and surface success/error in the chrome; allow retry after a failed save.
- [x] Round-trip tests: comment/format preservation, missing-file creation, correct table targeting (incl. `frecency_weight` under `[search]` not `[search.frecency]`), int vs float emission, unchanged-keys-not-written, and an unparseable existing file (error surfaced, file left untouched).

### Deliverable 5. Docs, help text, and integration coverage

Make the feature discoverable and lock behaviour with a test that drives the
command. Update `examples/config.toml` (note the keys are tunable via
`pb settings`), `README.md`, and ensure the subcommand `about`/rustdoc reads
well. Add an integration test exercising the navigate→edit→save flow against a
temp config + `PB_CONFIG_FILE`.

- [x] Update `README.md` with a `pb settings` section.
- [x] Mention `pb settings` in `examples/config.toml` comments for the search keys.
- [x] Polish command `about` text and module rustdoc.
- [x] Integration test: temp `PB_CONFIG_FILE`, drive edit+save, assert save lands at that path (env override honored) and on file contents.
- [x] Confirm `settings` appears in generated shell completions (clap-derive should auto-cover it; verify, given completions were recently restructured).
- [x] `cargo fmt`, `cargo clippy -- -D warnings`, `cargo test` all green.

## Issues

- **2026-06-08 — agent:claude (adversarial review)** — Plan reviewed by 2
  adversarial sub-agents (Risks & Assumptions, Completeness & Scope), both of which
  read the actual source. 13 findings; 12 merged. Most significant: the "reuse the
  `tui` harness" premise was false — only `run_scrollable_text` (a text-scroller)
  exists, so D1 now owns net-new loop code and the wrong "`StdoutTtyGuard` not
  required" claim was reversed (the guard is kept). Also added: non-tty
  error-and-exit behavior, same-directory atomic temp-file (rename is only atomic
  on one filesystem), failed-save keeps edits, and the impact band/overdominant rule
  was pinned to a `value/default` ratio scheme (user-chosen) so D2's tests are
  actionable and the mixed-domain frecency group is no longer compared by a
  meaningless group mean. One minor finding (shell-completion coverage) added to D5
  as a verify step.
- **2026-06-08 — agent:claude** — Scope added on user feedback: the tuner must
  *explain itself*. Each field now carries `help` copy and the screen gains an
  explanation pane with an impact readout (value-vs-default + off/low/moderate/
  high/dominant band) so the user can tell what a weight does and whether a value
  like `1.0` is high — see Plan Details "Explanation + impact readout" and
  Deliverable 2. Still no live snippet re-scoring; the band is value-relative, not
  a measured ranking effect.
- **2026-06-08 — agent:claude (deliberation)** — DECISION (locked): v1 uses the
  **value-relative readout**, not live empirical re-scoring. Settled by a binary
  deliberation — the live-re-scoring advocate withdrew at high confidence. Reasons:
  re-scoring couples the snippet index + frecency store into a niche, rarely-opened
  settings screen (against the simplicity-first ethos), and because frecency is
  cwd-/time-dependent the preview is only true *here, now* — it encourages
  over-fitting weights to one directory and is uninformative on empty/cold
  collections. The readout instead surfaces relative balance (sibling-share), which
  generalizes and needs only `AppConfig`. Live re-scoring stays a legitimate
  post-v1 enhancement (see follow-up below). Refinement adopted from the debate:
  special-case `half_life_days` so the non-linear time constant isn't mislabeled
  with a "× default" multiplier.
- **2026-06-08 — agent:claude** — Open follow-up (deferred scope): live
  re-scoring of the user's real snippets against cwd while tuning was explicitly
  cut from v1 (see locked decision above). If pursued later it needs the snippet
  index + frecency store wired into the tuner, a re-score on each adjustment, and
  graceful handling of empty/cold collections — track as a future deliverable, not
  part of this plan.
- **2026-06-08 — agent:claude** — Sections beyond `search` (`ui`,
  `suggestion_commands`, `lint`, `theme`, `variables`) are out of scope for v1
  per the picker-scope decision. The section picker should leave room for them
  (dimmed "coming soon" rows or omission) without implying they work yet.
- **2026-06-08 — agent:claude** — Exact key bindings for adjust (`←/→` vs `-/+`,
  and whether a coarse "shift = ×10 step" modifier is wanted) are a low-stakes
  detail left to implementation; both adjust keys are listed in the mock. Flag if
  you want a specific scheme pinned before coding.
