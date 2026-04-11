# Part 02 - Ranking, Search, and Tree Navigation

Status: done
Parent: [`README.md`](README.md)
Context: [`PLAN_CONTEXT.md`](../../PLAN_CONTEXT.md)

This part file is a living working document. Keep the checklist, progress log, discoveries, validation, and handoff current while the work is active.

## Outcome

The application can turn parsed snippets into a ranked interactive data model: users can fuzzy-search across the right fields, browse a file-tree hierarchy, switch modes without losing context, and see results ordered with cwd-aware frecency.

## Scope

- In scope: normalize parsed snippets into search and browse indexes; build fuzzy matching across snippet names, descriptions, bodies, tags, and frontmatter; model the tree-navigation path and tab-completion behavior from the README; design a file-backed frecency store keyed by snippet and location; implement the location, recency, and frequency scoring formula; add ranking and navigation tests.
- Out of scope: terminal rendering details, shell integration, snippet-file editing, and destructive CLI commands.

## Dependencies

- Needs: Part 01 to provide parsed snippets, stable IDs, and configured snippet roots.
- Unblocks: Part 03 can render the state machine without owning ranking logic, and Part 04 can persist usage events through the same interfaces.

## Checklist

- [x] Define the search index shape for snippet metadata, content, and relative path information.
- [x] Implement fuzzy matching across snippet heading text, snippet body, frontmatter fields, and description text.
- [x] Implement a browse-tree model that supports path narrowing, tab completion, and backspacing through directory levels.
- [x] Preserve search or browse context so a user can back out of a snippet and continue from the same narrowed state.
- [x] Design and implement a simple frecency store keyed by snippet ID and cwd context, with enough structure to weight nearby paths higher.
- [x] Add tests that prove location weighting, recency influence, and high-frequency override behavior.
- [x] Decide when usage events become durable, such as only after the user confirms emission of a command.

## Progress Log

- [x] 2026-04-11: Captured the ranking and navigation slice directly from the README user-experience narrative.
- [x] 2026-04-11: Implemented `src/index.rs`, `src/fuzzy.rs`, `src/browse.rs`, `src/frecency.rs`, and `src/search.rs`. Added `nucleo-matcher` for fuzzy scoring and `ratatui` so the navigation states carry real `ListState`s. Validated with `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test` (37 unit + 6 integration passing).

## Discoveries

- Observation: The README requires two distinct but connected navigation modes rather than a single fuzzy list with optional grouping.
  Evidence: `README.md` user-experience examples for fuzzy search and file-style navigation
- Observation: The README also expects search context to survive backing out of a chosen snippet so users can try the next result quickly.
  Evidence: `README.md` paragraph describing backspacing to return to the previous snippet list state
- Decision: Use `nucleo-matcher` (the scoring half of the helix fuzzy matcher) instead of a hand-rolled subsequence scorer. It gives us case handling, normalization, word-boundary bonuses, and a stable scoring surface without owning any of that logic. `src/fuzzy.rs` wraps it with field-weighted aggregation (name 30, tag 20, fm-name 15, desc/path 10, body 8).
  Evidence: `src/fuzzy.rs::score_snippet`
- Decision: Adopt `ratatui` now, even though the TUI is Part 03, so `FuzzyState` and `BrowseState` can own real `ListState`s. Part 03 can render them directly with `List::new(...).highlight_style(...)` and there is no "intermediate selection type" to rewrite later.
  Evidence: `src/fuzzy.rs::FuzzyState`, `src/browse.rs::BrowseState`
- Decision: Browse mode hides the snippet-file layer. Each snippet is attached directly to the directory its source file lives in, matching the README's `docker/docker-snippet-1` tree example. Two snippets from the same file appear as siblings under that directory.
  Evidence: `src/browse.rs::BrowseTree::from_index`
- Decision: `BrowseState::backspace` deletes input characters first, then pops a directory when input is empty. Holding backspace walks out of nested directories in a single predictable motion, which is what the README prose asks for ("backspacing until they find relevant areas").
  Evidence: `src/browse.rs::BrowseState::backspace`
- Decision: Frecency events are stored as plain TSV (`timestamp\tsnippet_id\tcwd`) in the file from `config::default_paths().state_file`. No serde dependency, the file is hand-readable, and a user can grep or edit it. Events are append-only; only the `score` function interprets them.
  Evidence: `src/frecency.rs::FrecencyStore::{load,save}`
- Decision: Frecency scoring formula — each matching event contributes `time_decay(age) * (1 + path_affinity(event.cwd, cwd))`, plus a sublinear frequency bonus `ln(1 + count)`. This satisfies: recency dominates at the same cwd, an exact-cwd event roughly doubles a foreign-cwd event, and enough accumulated use overrides a single local match (the README's "git frequency override" case). This is the central tuning knob — revisit after live use.
  Evidence: `src/frecency.rs::FrecencyStore::score` and the three directional tests in that module
- Decision: `path_affinity` counts only `Component::Normal` segments. Counting the root `/` would make every pair of absolute paths look partially related.
  Evidence: `src/frecency.rs::path_affinity`
- Decision: Usage events become durable only when the caller invokes `FrecencyStore::record` / `record_now`. Part 04 will call that exactly once per confirmed emission (after the TUI writes the command to stdout), not on browse hovers. Keeping this out of the ranking layer lets us unit-test ranking without touching the filesystem.
  Evidence: `src/frecency.rs::FrecencyStore::{record,record_now}`

## Validation

- Command or check: `cargo test`
  Result: 37 unit + 6 integration passing, 2026-04-11. Includes `frecency::tests::location_weighting_raises_score_for_same_cwd`, `recency_influences_score`, `frequency_can_override_location`, browse tab/backspace tests, and ranked-search tiebreaker tests.
- Command or check: `cargo clippy --all-targets -- -D warnings`
  Result: Clean, 2026-04-11.
- Command or check: `cargo fmt --check`
  Result: Clean, 2026-04-11.

## Next Handoff

Keep ranking and browse state as pure application logic so the TUI can consume it without reimplementing search rules. Concrete starting points for Part 03:

- Call `index::load_default()` (or `load_from_roots`) once at startup to build the `SnippetIndex`, then `BrowseTree::from_index(&index)` once.
- Maintain a single `FuzzyState` and `BrowseState` for the whole session. Swapping modes must preserve both so backspacing out of a snippet returns the user to their exact prior state (the README's "same spot before they entered the snippet" requirement).
- On every keystroke in fuzzy mode, call `search::rank(&index, &state.query, &frecency, &cwd, now)` to produce the `Vec<SearchHit>` that feeds `ratatui::widgets::List`.
- In browse mode, call `state.visible(&tree)` per render and `state.tab_complete(&tree)` / `state.activate(&tree)` on key events.
- On a successful emission of a command, call `FrecencyStore::record_now(id, cwd)` and then `FrecencyStore::save(&paths.state_file)` — this is the only place usage events become durable.
- Tuning knob: `FrecencyStore::score` in `src/frecency.rs` is the blend of recency/location/frequency. Adjust the `HALF_LIFE` constant in `time_decay`, the `(1 + location)` multiplier, or the `ln_1p(count)` boost to change how aggressively the ranking tracks any one signal.
