---
issue: https://github.com/calamity-m/peanutbutter/issues/2
---

# BIGPLAN: Tags as a third picker view

## Plan Overview

Add a third navigation mode to the interactive picker that lets users browse snippets by tag. The mode is reached by cycling with Ctrl+T (already toggling Fuzzy ↔ Browse) and is the third stop: Fuzzy → Browse → Tags → Fuzzy. Tags mode is a drill-down: a filterable list of tags first, Enter drills into a flat snippet list filtered to that tag, Esc/Backspace pops back. State (filter input, selected tag, drill depth, list highlight) is preserved when the user cycles away and back, mirroring how Browse already retains its path/selection. The `tag:foo` query-prefix syntax mentioned in issue #2 is **deferred to a later issue**; this plan ships only the dedicated view.

"Done" looks like: a user in the picker hits Ctrl+T twice, sees a list of tags they can type-filter and arrow-through, presses Enter on `git`, sees only git-tagged snippets, and can press Enter again to select one (existing prompt/preview flow takes over). Cycling Ctrl+T from anywhere in tags mode returns to fuzzy; cycling Ctrl+T twice more returns the user to the same tag (or drilled snippet) they were on.

## Risks

- **Preview-pane assumes a selected snippet.** Tag-list phase has no selected snippet, so every render call in that phase passes `None` to preview helpers. If any helper unwraps, entering tags mode panics — and TUI panics interact poorly with raw-mode restoration (per `CLAUDE.md`). Mitigation: explicit audit task in Deliverable 2 with a render-no-panic test for tag-list phase.
- **Tag index can go stale on mid-session refresh.** `ExecutionApp::replace_index` (`src/execute/app.rs:255-273`) is the existing hook for in-session re-indexing (called after the `edit` flow rebuilds `BrowseTree`). The new tag index must rebuild here too, otherwise tags mode shows phantom or missing tags after an edit. Mitigation: extend `replace_index` to rebuild the tag index alongside the browse tree, and test it.
- **Cycle-with-state mechanic regresses Browse mode.** The current `Ctrl+T` toggle already implicitly preserves Browse's `path`/`selection` because `BrowseState` lives on the app. Extending the cycle to a 3-way enum without a careful audit could re-initialize state on each transition. Mitigation: write tests that cycle F → B → T → F → B and F → B → T → drill → T → F → B → T (drill restored).
- **Ctrl+T precedence over text-input handlers.** Tags-list and Fuzzy modes both consume printable characters into a filter buffer. Ctrl+T must be matched **before** any text-input branch in the key handler, or muscle-memory cycling mid-typing will silently insert characters or eat keys. Mitigation: keep the existing top-of-handler ctrl+t check unchanged and add it as the first branch of `handle_tags_key` too.
- **Ambiguous "tag" semantics.** Tags come from frontmatter (`Vec<String>` per snippet). They are case-sensitive in storage but users may expect `Git` and `git` to merge. We will treat tags as **case-sensitive** for v1 (matches existing parser behavior) and surface this in a Gotcha so it can be revisited.
- **Empty / sparse tag set.** A user with no frontmatter tags lands in tags mode and sees only `(untagged)`. Acceptable but easy to misread as a bug. Mitigation: render an empty-state hint line in the renderer when only the synthetic entry exists.
- **Renderer drift.** `src/execute/render.rs` is ~980 lines with separate code paths for fuzzy and browse. Adding a third path risks copy-paste rot. Mitigation: factor a `render_tag_view` helper as an explicit Deliverable 2 task; keep tests in `src/execute/tests.rs` for each branch.

## Plan Details

The picker today has a 2-way `NavigationMode { Fuzzy, Browse }` enum on `ExecutionApp` (`src/execute/app.rs:20-25`). Ctrl+T flips between the two (`src/execute/app.rs:331-340`). Tag data is already available: `IndexedSnippet::tags()` returns `&[String]` (`src/index.rs:42-44`), populated by `parser.rs` from frontmatter (`src/parser.rs:86-103`).

The new mode introduces:

1. A **third enum variant** `NavigationMode::Tags`.
2. A new state struct `TagsState` (analogous to `BrowseState`) holding: a small local input buffer (`String` + `usize` cursor — **not** `FuzzyState`), drill phase (`TagList | TagDrill { tag: TagKey }`), and per-phase list highlight.
3. A **tag index** keyed by an `enum TagKey { Tag(String), Untagged }` so the synthetic untagged bucket cannot collide with a user-defined tag string. Stored as `BTreeMap<TagKey, Vec<SnippetId>>` (alphabetical, with `Untagged` placed deterministically — convention: bottom of the list). Built in `ExecutionApp::new` and rebuilt in `replace_index` alongside `BrowseTree`.
4. Cycle logic updated to a 3-way rotation. Ctrl+T from anywhere (including drilled-down) advances to the next mode; the leaving mode's state is **not reset**. Ctrl+T match must come before any text-input branch in `handle_tags_key`.
5. Renderer branch in `src/execute/render.rs` for the tag view: tag list with filter input visible, or drilled snippet list with the active tag pinned in the header. Preview pane must tolerate `None` selected snippet (tag-list phase).
6. Filter algorithm for the tag list: **case-sensitive substring match** on the rendered tag name (the `String` payload of `TagKey::Tag`; `Untagged` matches when the substring is empty). No fuzzy ranking — tag mode does not depend on `src/search.rs`.
7. Counts shown beside each entry: number of snippets in that bucket. Multi-tagged snippets are counted once **per tag they hold**, so the sum of counts can exceed total snippets — this is intentional. The `(untagged)` bucket counts only snippets with zero tags.
8. Help-footer line in `src/execute/render.rs` (currently advertises `ctrl+t browse` / `ctrl+t search`) must be updated to reflect the 3-way cycle and a new variant for tags mode.

### Esc / Backspace state machine

```text
mode = Tags, phase = TagList:
  Esc       -> exit TUI (matches Fuzzy's Esc)           // see app.rs:326
  Backspace -> if filter non-empty: delete one char
              else: no-op (Esc is the cancel; do not pop to Fuzzy)
  Enter     -> drill into highlighted tag

mode = Tags, phase = TagDrill { tag }:
  Esc       -> pop drill (phase = TagList); do NOT exit TUI
  Backspace -> pop drill (drilled list is unfiltered in v1, so any Backspace pops)
  Enter     -> select highlighted snippet (existing prompt flow)
```

### Critical Files

- `src/execute/app.rs` — `NavigationMode` enum, `ExecutionApp` fields, `handle_key` cycle logic (lines 20-25, 155-203, 326-365). Most surgical work lives here.
- `src/execute/render.rs` — picker rendering; needs a new branch for tags mode.
- `src/execute/tests.rs` — existing TUI tests; new tests for cycling + drill behavior land here.
- `src/index.rs` — `IndexedSnippet::tags()` already exposed; may add a small helper `SnippetIndex::tag_index()` returning the `BTreeMap` for tag mode.
- `src/domain.rs` — `tags: Vec<String>` already on frontmatter, no change expected.
- `src/browse.rs` — reference shape for `BrowseState` / `BrowseEntry` patterns; informs the design of `TagsState` but no edits.
- `src/search.rs` — **not modified** in this plan; tag-as-search-prefix is the deferred follow-up.
- `README.md` — short addition documenting the third mode and the new keybinding behavior.

### Gotchas

- `BrowseState` survives mode toggles today only because it lives on the app and the toggle code never re-initializes it. Don't accidentally re-init `tags` in the cycle handler — preserve it the same way.
- The tag index must be rebuilt by `replace_index` (called after the `edit` flow re-scans the snippet directory). Drill state may reference a tag that no longer exists post-rebuild — when restoring after rebuild, drop drill if the drilled `TagKey` is missing from the new index.
- Tags are case-sensitive in v1. Two snippets tagged `git` and `Git` produce two entries. Tag normalization (trim, splitting, etc.) is whatever `parser.rs` already does — do not re-normalize at index-build time; consume `IndexedSnippet::tags()` as-is.
- Using `enum TagKey { Tag(String), Untagged }` as the BTreeMap key (not a sentinel string) ensures the untagged bucket cannot collide with a user-defined tag literal.
- Do **not** re-use `FuzzyState` for the tag-list filter — it would couple tags-filter UX to fuzzy-search internals. Use a minimal local `String + cursor` on `TagsState`.
- The picker's preview pane currently expects a selected `IndexedSnippet`. In the tag-list phase there is no selected snippet — render the preview pane empty and audit existing preview helpers for `None` handling.
- Multi-tagged snippets are counted once per tag — `sum(counts) > total snippets` is expected, not a bug.
- The help-footer string in `render.rs` is mode-aware and currently mentions `ctrl+t browse` / `ctrl+t search`. Updating those literals is part of the deliverable; missing this leaves a stale UI hint after the cycle ships.

### Pseudo-code / Sketches

```text
enum NavigationMode { Fuzzy, Browse, Tags }
enum TagKey { Tag(String), Untagged }   // BTreeMap key — no sentinel string

struct TagsState {
    filter: String,                 // typed query, narrows visible tag list
    cursor: usize,                  // cursor position in `filter`
    list_selection: Option<usize>,
    drill: Option<TagKey>,          // None = tag-list phase; Some = drilled
    drill_selection: Option<usize>,
}

cycle(mode) = match mode {
    Fuzzy  => Browse,
    Browse => Tags,
    Tags   => Fuzzy,
}

handle_key(key):
    // Ctrl+T must match BEFORE any text-input branch, in every mode.
    if ctrl && key == 't' { self.nav_mode = cycle(self.nav_mode); return Continue }
    ...

handle_tags_key(key):  // already past the Ctrl+T branch
    if self.tags.drill.is_some() {
        match key {
            Esc | Backspace => self.tags.drill = None,        // pop drill
            Enter           => activate snippet at drill_selection,
            Up/Down         => move drill_selection,
            _               => no-op (drilled list is unfiltered in v1)
        }
    } else {
        match key {
            Char(c)   => filter.insert(c at cursor); cursor += 1,
            Backspace => if !filter.empty() { filter.pop(); cursor -= 1 }
                         else { no-op — Esc is the cancel },
            Up/Down   => move list_selection,
            Enter     => self.tags.drill = Some(visible[list_selection].key),
            Esc       => exit TUI (same as Fuzzy's Esc, app.rs:326),
        }
    }
```

## Deliverables

### Deliverable 1. Three-way mode cycle with state preservation

Replace the 2-way `NavigationMode` toggle with a 3-way cycle (Fuzzy → Browse → Tags → Fuzzy) on Ctrl+T. `Tags` is wired up as an enum variant only — its rendering is a placeholder ("tags view — TODO") so this deliverable can land independently and prove the cycle mechanic without a full UI.

The deliverable produces: an `ExecutionApp` whose `nav_mode` enum has three variants; the Ctrl+T handler that rotates through them; a `TagsState` field on the app initialized to defaults; tests asserting that (a) Ctrl+T cycles in the documented order, (b) Browse's `path` and selection are preserved across a full F→B→T→F→B cycle.

- [x] Add `Tags` variant to `NavigationMode` in `src/execute/app.rs`.
- [x] Add `TagKey` enum (`Tag(String) | Untagged`) in `src/index.rs` or a new module sibling.
- [x] Add `TagsState` struct (filter `String`, cursor `usize`, list_selection, drill: `Option<TagKey>`, drill_selection) and field on `ExecutionApp`.
- [x] Update Ctrl+T handler to 3-way rotation; do not reset any other mode's state. Confirm Ctrl+T match precedes any text-input branch.
- [x] Update `selected_snippet` / `selected_*_snippet` dispatch to handle `Tags` (returns `None` for now).
- [x] Update help-footer strings in `src/execute/render.rs` to reflect 3-way cycle in all three modes (Fuzzy/Browse/Tags).
- [x] Add a placeholder render branch for `NavigationMode::Tags` in `src/execute/render.rs` so the screen doesn't blank.
- [x] Audit preview-pane render path for `None` snippet handling; fix any unwrap.
- [x] Test: cycle F→B→T→F→B, assert Browse `path`/`selection` survive.
- [x] Test: cycle order matches Fuzzy → Browse → Tags → Fuzzy on repeated Ctrl+T.
- [x] Test: render of Tags mode (placeholder) does not panic.
- [x] Note in the deliverable: full Tags-state-survival assertions land in D2/D3, not here.
- [x] `cargo fmt && cargo build && cargo test && cargo clippy -- -D warnings -A dead_code` clean.

### Deliverable 2. Tag index + tag-list view (no drill yet)

Compute the tag → snippet-ids index from `SnippetIndex` and render an alphabetical, type-filterable list of tags in tags mode. Selection is highlight-only at this stage (Enter does nothing yet). Untagged snippets group under `TagKey::Untagged`, rendered as `(untagged)`.

The deliverable produces: a `tag_index()` helper; rendering of the tag list with the filter input visible; key handling for typing/backspace/up/down within the tag-list phase; freshness on `replace_index`. Behavior is verifiable by snapshot-style assertions in `src/execute/tests.rs`.

- [x] Add `SnippetIndex::tag_index()` returning `BTreeMap<TagKey, Vec<SnippetId>>`. `TagKey::Untagged` only present when at least one snippet has no tags.
- [x] Build the tag index eagerly in `ExecutionApp::new` and store it on the app.
- [x] Extend `ExecutionApp::replace_index` (`src/execute/app.rs:255-273`) to rebuild the tag index alongside `BrowseTree`. If a drill is active and the drilled `TagKey` is missing post-rebuild, drop the drill.
- [x] Factor a `render_tag_view` helper in `src/execute/render.rs` (avoid copy-paste with fuzzy/browse rendering).
- [x] Render the tag-list view: filter line at top, alphabetical tag list, count next to each (`git (12)`). `Untagged` rendered as `(untagged)`, placed at the bottom.
- [x] Render an empty-state hint when the only entry is `(untagged)`.
- [x] Wire `handle_tags_key` (tag-list phase) per the Esc/Backspace state machine in Plan Details. Filter is case-sensitive substring on the tag name; `(untagged)` matches only when the filter is empty.
- [x] Confirm preview-pane render handles `None` snippet (tag-list phase) without panic; add a regression test.
- [x] Test: tag list is alphabetical; `(untagged)` appears only when at least one untagged snippet exists, placed at the bottom.
- [x] Test: typing narrows the visible list (case-sensitive substring).
- [x] Test: counts are per-bucket; multi-tagged snippets counted once per tag.
- [x] Test: a snippet whose literal tag is the string `__pb_untagged__` is **not** merged into `(untagged)` — regression guard for the `TagKey` enum approach.
- [x] Test: cycling F→B→T→F→B→T preserves filter text and list_selection.
- [x] Test: `replace_index` rebuilds the tag index (a new tag in the new index is visible; a removed tag is gone).
- [x] Test: rendering Tags mode at tag-list phase does not panic on a populated index.
- [x] Lint/test gates clean.

### Deliverable 3. Drill into tag → snippet list → selection

Enable Enter on a tag to drill into a flat list of that tag's snippets, and Enter on a snippet to feed into the existing selection/prompt flow. Esc and Backspace-on-empty-filter pop drill back to the tag list. Drill state is preserved across Ctrl+T cycle-out-and-back.

- [x] On Enter in tag-list phase, set `tags.drill = Some(<TagKey>)`; reset `drill_selection` to 0.
- [x] Render drilled snippet list: pin active tag in the header (`tag: git` or `tag: (untagged)`), reuse existing row helpers where possible.
- [x] On Enter in drill phase, route through the existing snippet-selection path (matches Fuzzy/Browse Enter behavior) so the prompt screen is reached unchanged.
- [x] On Esc in drill phase, set `tags.drill = None`; do not exit the TUI.
- [x] On Backspace in drill phase: drilled list is unfiltered in v1, so Backspace pops drill.
- [x] Update `selected_snippet`: returns `Some(snippet)` only when `nav_mode == Tags`, drill is `Some`, AND `drill_selection` is in range. Returns `None` otherwise (including empty drilled list).
- [x] Confirm Ctrl+T from drill phase cycles to Fuzzy and that returning to Tags (after F→B→T) restores the drill (active tag + drill_selection).
- [x] Test: Enter on tag, then Enter on snippet, completes the same outcome as picking the snippet from Fuzzy.
- [x] Test: Esc from drill returns to tag list without exiting TUI.
- [x] Test: full cycle from drilled state — F→B→T→drill→Ctrl+T→F→Ctrl+T→B→Ctrl+T→T — lands back on the same drilled tag with the same `drill_selection`.
- [x] Test: `selected_snippet` returns `None` when drill is `Some` but the drilled list is empty.
- [x] Test: drill drop on `replace_index` when the drilled tag no longer exists.
- [x] Lint/test gates clean.

### Deliverable 4. Docs + README

Document the third mode and updated keybinding behavior in the user-facing README and any in-code rustdoc on `NavigationMode`. No new external surface beyond Ctrl+T's rotation.

- [x] Update `README.md` keybindings/usage section to describe the 3-way Ctrl+T cycle and the tags view (filter, drill, untagged bucket, multi-tag count semantics).
- [x] Update rustdoc on `NavigationMode`, `TagsState`, and `TagKey`.
- [x] Note in README that `tag:foo` query-prefix syntax is tracked in a follow-up issue.
- [x] (In-TUI help footer literals are handled in D1 alongside the cycle change — no further work here.)
- [x] No new test gates expected; lint/build clean.

## Issues

- **2026-05-09 — agent:codex** — Cleanup note from implementation: filtering tags by `untagged` should match the synthetic `(untagged)` bucket, even though it is not a stored tag string. Added this as final-deliverable cleanup.
- **2026-05-09 — agent:codex** — Post-plan cleanup: drilled tag snippet lists should also be filterable, similar to browse mode, so users can refine snippets after selecting a tag.
- **2026-05-09 — agent:claude (adversarial review)** — Plan reviewed by 2 adversarial sub-agents (Risks & Assumptions, Completeness & Scope). 19 findings; merged into plan. Most significant changes: (a) added a Risks entry + tasks for `replace_index` rebuild of the tag index (mid-session edit flow makes the static-index assumption wrong); (b) replaced the `__pb_untagged__` sentinel string with an `enum TagKey { Tag(String), Untagged }` BTreeMap key to eliminate collision risk; (c) pinned the Esc/Backspace state machine in Plan Details and added preview-pane `None`-handling tasks; (d) made help-footer-string updates an explicit D1 task.
- **2026-05-09 — agent:claude** — `tag:foo` query-prefix syntax from issue #2's acceptance criteria is intentionally **deferred** to a follow-up issue per user direction. This plan ships only the third-cycle-mode UI. The follow-up will need to land in `src/search.rs`, which is untouched here.
- **2026-05-09 — agent:claude** — Tag matching is case-sensitive for v1 (matches existing parser storage). Worth revisiting if users tag inconsistently — flag in followup issue alongside `tag:` prefix work.
