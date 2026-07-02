# BIGPLAN: Keybinds for the other command TUIs

## Plan Overview

Extend customizable keybinds from `pb execute` (shipped via `docs/keybinds-bigplan.md`) to the remaining interactive TUIs: `pb settings` and `pb new`. Both reuse the existing `src/keybinds.rs` machinery — `KeyChord`, `ContextBindings`, action-order conflict precedence, and warn-don't-fail validation — under new `[keybinds.settings.<context>]` and `[keybinds.new.<context>]` TOML tables. Done means a user can remap navigation, accept, back-out, quit, reset, token-toggle, and rename keys in both TUIs without code changes; footer help reflects remaps; invalid bindings warn inside each TUI; and `Ctrl+C` remains an unconditional cancel everywhere. `pb stats --output tui` renders without a key loop and is out of scope.

## Risks

- **Plain-letter actions vs. text entry** — `pb new`'s pickers treat `k`/`j` as list navigation *only while the filter is empty*; otherwise they are filter text. A naive "keymap wins" resolution (as in execute) would either eat typed letters or never navigate, and the rule spans five handlers (two pickers, name field, token rename, settings screens). Mitigation: one shared helper (see "Text-input precedence rule") implements the rule; every text-accepting widget gets its own test for both the typing and the empty-input case. Watch-for signal: any handler matching `KeyCode::Char` outside the shared helper.
- **Settings save clobbering user `[keybinds]` tables** — `pb settings` rewrites the config file it was configured from. `settings/persist.rs` parses the existing file into a `toml_edit::DocumentMut` and mutates only the keys it manages, which preserves unknown tables (`save_theme_name_preserves_other_theme_keys` already proves this for theme keys), so the design is safe — but nothing yet regression-tests it for `[keybinds]` specifically. Mitigation: Deliverable 2 adds a save-round-trip test with a populated `[keybinds]` table; failure there blocks D2 completion, not a late D4 discovery.
- **Execute hotkey-path regression from resolver widening** — Deliverable 1 restructures root keybind resolution for a shipped feature that guards the stdout-clean shell path. Mitigation: execute behavior and tests stay byte-for-byte identical; the only sanctioned test edit is `unknown_context_action_and_section_warn_without_failing` (the new sections stop warning). Watch-for signal: any other execute test needing edits is a red flag requiring an `## Issues` entry before proceeding.
- **Save/quit safety in `pb settings`** — `q`, `Esc`/`Backspace` back-out, and `Enter` (save) guard unsaved changes and the quit-confirmation flow. If a user unbinds every back-out and quit action, the TUI must still exit via the reserved `Ctrl+C`, and the unsaved-changes confirmation must not become unreachable. Keep `Ctrl+C` hardcoded and test the fully-unbound worst case.
- **Unknown-section warning back-compat** — the shipped resolver warns `unknown section` for anything but `keybinds.execute`, so `[keybinds.settings]` in today's config produces a warning. The new resolver must accept the new sections and keep warning for genuinely unknown ones.
- **Help drift across three TUIs** — settings and new both hard-code footer/help strings today. Each screen's help must derive from its context bindings, as execute's does, or remaps will teach stale keys.

## Plan Details

### Config shape

New sections beside the existing execute tables. Same grammar, same warning rules, same `ctrl+c` reservation. This block is the complete context list; the action inventory below is the source of truth for per-action defaults.

```toml
[keybinds.settings.global]
quit = ["q", "shift+q"]

[keybinds.settings.list]        # section, theme, and paths screens
back = ["esc", "backspace"]
move_up = ["up", "k"]
move_down = ["down", "j"]
select = ["enter"]

[keybinds.settings.search]      # search tuner chooser screen
back = ["esc", "backspace"]
move_up = ["up", "k"]
move_down = ["down", "j"]
select = ["enter"]
reset = ["r", "shift+r"]

[keybinds.settings.tuner]
back = ["esc", "backspace"]
move_up = ["up", "k"]
move_down = ["down", "j"]
decrease = ["left", "-"]
increase = ["right", "plus", "="]
reset = ["r", "shift+r"]
save = ["enter"]

[keybinds.new.picker]           # target picker and history picker
accept = ["enter"]
cancel_or_back = ["esc"]
move_up = ["up", "k"]
move_down = ["down", "j"]
backspace = ["backspace"]

[keybinds.new.confirm_name]
accept = ["enter"]
cancel = ["esc"]
complete_or_focus_tokens = ["tab", "down"]
backspace = ["backspace"]

[keybinds.new.confirm_tokens]
cancel = ["esc"]
back = ["b"]
move_up = ["up", "k"]
move_down = ["down", "j"]
toggle_variable = ["space"]
rename = ["e"]
edit_name = ["n"]
accept = ["enter"]

[keybinds.new.confirm_rename]
cancel = ["esc"]
accept = ["enter"]
backspace = ["backspace"]
```

### Context inventory

Each screen resolves exactly one context (plus `settings.global`, which is checked before screen dispatch, mirroring how `execute.select` precedes mode contexts). No screen draws from two action contexts — the search screen gets its own `settings.search` context precisely because it carries `reset` on top of the list actions, so a `settings.tuner` remap never silently changes a non-tuner screen. This inventory was verified against `src/settings/app.rs` and `src/new/capture.rs` at planning time (2026-07-02); if implementation still finds an unlisted behavior, extend the table before coding the handler.

`pb settings` (`src/settings/app.rs`):

| Context | Action | Current defaults | Notes |
| --- | --- | --- | --- |
| app emergency | `force_cancel` | `ctrl+c` | Not configurable; quits without saving. |
| `settings.global` | `quit` | `q`, `shift+q` | Checked before screen dispatch; triggers unsaved-changes confirmation. |
| `settings.list` | `back` | `esc`, `backspace` | Section screen: quit request; theme/paths: return to Section. |
| `settings.list` | `move_up` | `up`, `k` | |
| `settings.list` | `move_down` | `down`, `j` | |
| `settings.list` | `select` | `enter` | Enter section / apply theme / save from paths. |
| `settings.search` | `back` | `esc`, `backspace` | Return to Section. |
| `settings.search` | `move_up` | `up`, `k` | |
| `settings.search` | `move_down` | `down`, `j` | |
| `settings.search` | `select` | `enter` | Open the frecency/fuzzy tuner. |
| `settings.search` | `reset` | `r`, `shift+r` | Reset weights from the chooser screen. |
| `settings.tuner` | `back` | `esc`, `backspace` | Return to search screen. |
| `settings.tuner` | `move_up` | `up`, `k` | |
| `settings.tuner` | `move_down` | `down`, `j` | |
| `settings.tuner` | `decrease` | `left`, `-` | |
| `settings.tuner` | `increase` | `right`, `plus`, `=` | |
| `settings.tuner` | `reset` | `r`, `shift+r` | |
| `settings.tuner` | `save` | `enter` | |

`pb new` (`src/new/capture.rs`) — the target picker and history picker share `new.picker` (their arms are near-identical; `cancel_or_back` backs out in the target picker and cancels in the history picker); the token-confirmation screen maps its three `Focus` modes to three contexts:

| Context | Action | Current defaults | Notes |
| --- | --- | --- | --- |
| app emergency | `force_cancel` | `ctrl+c` | Not configurable. |
| `new.picker` | `accept` | `enter` | Pick highlighted entry. |
| `new.picker` | `cancel_or_back` | `esc` | |
| `new.picker` | `move_up` | `up`, `k` | `k`/`j` navigate only while the filter is empty (see precedence rule). |
| `new.picker` | `move_down` | `down`, `j` | |
| `new.picker` | `backspace` | `backspace` | Delete filter char. |
| `new.confirm_name` | `accept` | `enter` | Confirm snippet name / advance. |
| `new.confirm_name` | `cancel` | `esc` | |
| `new.confirm_name` | `complete_or_focus_tokens` | `tab`, `down` | One action, not two — matches today's single match arm. |
| `new.confirm_name` | `backspace` | `backspace` | |
| `new.confirm_tokens` | `cancel` | `esc` | |
| `new.confirm_tokens` | `back` | `b` | Return to picker. |
| `new.confirm_tokens` | `move_up` | `up`, `k` | |
| `new.confirm_tokens` | `move_down` | `down`, `j` | |
| `new.confirm_tokens` | `toggle_variable` | `space` | |
| `new.confirm_tokens` | `rename` | `e` | Start token rename. |
| `new.confirm_tokens` | `edit_name` | `n` | Focus the name field. |
| `new.confirm_tokens` | `accept` | `enter` | Accept and write the snippet. |
| `new.confirm_rename` | `cancel` | `esc` | Abort rename. |
| `new.confirm_rename` | `accept` | `enter` | Commit rename. |
| `new.confirm_rename` | `backspace` | `backspace` | |
| text fallback | `type_char` | unmodified printable chars | Not configurable; applies in filters, name field, and rename field. |

### Key string grammar additions

Two gaps, both verified against the shipped parser:

- `space` — the execute grammar only supports printable single characters, and a literal `" "` in TOML is too easy to misread. Add `space` as a named base key; display it as `space`.
- `plus` — bare `"+"` does **not** parse today: `KeyChord::parse` splits on `'+'`, so `"+"` produces an empty base token and errors. `-` and `=` already parse as single printable characters. Add a `plus` named key (canonical form and display `plus`); optionally also special-case a trailing `+` token (`"ctrl++"`), but `plus` alone is sufficient for the tuner defaults.

Both need parse/display/round-trip tests in Deliverable 1.

### Text-input precedence rule

Execute resolves keymap actions before the text fallback, which is safe there because no default execute binding is an unmodified letter. Settings and new introduce letter defaults (`q`, `k`, `j`, `b`, `e`, `n`, `r`, `space`). The rule:

- In widgets with *conditional* text entry (the two `pb new` picker filters): a chord consisting of an unmodified printable character resolves as an action only while the input is empty; otherwise it is text. Modified chords and named keys always resolve as actions.
- In widgets that *always* accept text (`new.confirm_name`, `new.confirm_rename`): unmodified printable chords never resolve as actions there — a plain-letter binding for `new.confirm_name.accept` is inert by design. Document this in the config reference; do not silently half-apply it.
- In widgets with *no* text entry (all `pb settings` screens, `new.confirm_tokens`): keymap resolution runs first, unconditionally.

Implement this once as a shared helper in `src/keybinds.rs` (e.g. `TextEntry::{None, WhenEmpty(input_is_empty), Always}` passed to a `resolve` wrapper) so the rule has exactly one implementation, and test it per widget.

### Critical Files

- `src/keybinds.rs` — add `SettingsGlobalAction`/`SettingsListAction`/`SettingsSearchAction`/`SettingsTunerAction` and the four `New*Action` enums, `space`/`plus` base keys, the text-entry-aware resolve helper, and grow the existing `Keymaps` aggregate (`{ execute, warnings }` today) with `settings` and `new` fields, dispatching per section in `Keymaps::resolve`.
- `src/config.rs` — no structural change expected — `AppConfig.keybinds` already carries the `Keymaps` aggregate, whose `warnings` list stays shared; every TUI shows the full list (decided — no per-TUI filtering).
- `src/settings/app.rs` — replace hard-coded matches in `handle_quit_key`, `handle_section_key`, `handle_search_key`, `handle_tuner_key`, `handle_theme_key`, `handle_paths_key`.
- `src/settings/render.rs` — derive footer/help from the settings keymap.
- `src/settings.rs` — pass keymap and warnings into the settings TUI entry point; surface warnings as initial status.
- `src/settings/persist.rs` — no structural change expected (in-place `DocumentMut` edits preserve unknown tables); gains a `[keybinds]`-preservation regression test.
- `src/new/capture.rs` — replace hard-coded matches in `target_picker_key`, `picker_key`, and the confirm-screen `Focus` arms; derive help lines; surface warnings as status.
- `src/new.rs` — wire config keymap and warnings into the capture flow.
- `examples/config.toml`, `README.md` — extend the keybind docs with the settings/new sections.

### Gotchas

- `settings` treats `q`/`Q` and `r`/`R` as the same action; canonicalization maps `shift+q` → `Q`, so defaults need both spellings (`["q", "shift+q"]`) to preserve behavior.
- `tab | down` is one arm in `new.confirm_name` today (`complete_or_focus_tokens`); keep it one action rather than splitting complete/focus.
- `settings/persist.rs` parses the config into `toml_edit::DocumentMut` and mutates only managed keys, so hand-written `[keybinds]` tables survive a settings save; the regression test in Deliverable 2 pins this so a future persistence rewrite can't regress it silently.
- Keybind warnings load into the single shared `Keymaps::warnings` list and every TUI (execute, settings, new) shows all of them — a settings typo visible in `pb execute` is acceptable and simpler than per-context filtering.
- `pb new` prints results to stdout but is not a shell-capture hotkey path; keep warnings off stdout anyway for consistency (TUI status line only).

### Pseudo-code / Sketches

```text
resolve keybinds root table
  for section in table: execute | settings | new -> dispatch to that keymap's contexts
  anything else -> unknown-section warning
  conflict pass per context, as shipped

settings handle_key(key)
  if emergency cancel: quit
  if settings.global.quit action: quit flow (unsaved-changes confirmation unchanged)
  dispatch screen handler; resolve that screen's single context; no text fallback

new picker key(state, key)
  if emergency cancel: cancel
  action = resolve(picker context, key, TextEntry::WhenEmpty(filter.is_empty()))
  match action { accept, cancel_or_back, move_up, move_down, backspace }
  else if unmodified printable: type into filter

confirm name/rename key(state, key)
  action = resolve(context, key, TextEntry::Always)   # plain-letter chords never match
  match action { accept, cancel, ... } else type char
```

## Deliverables

### Deliverable 1. Keybind model covers settings and new sections

Generalize the resolver so `[keybinds.execute]`, `[keybinds.settings]`, and `[keybinds.new]` coexist: new action enums with documented precedence order, `space`/`plus` in the key grammar, the shared text-entry-aware resolve helper, and new `settings`/`new` fields on the existing `Keymaps` aggregate. Existing execute behavior and its tests must remain byte-for-byte identical; the only sanctioned test edit is the unknown-section assertion.

- [x] Add the settings and new context enums with defaults matching the inventory above.
- [x] Add `space` and `plus` as named base keys with parse/display/round-trip coverage, plus parse tests for the `-` and `=` single-char defaults.
- [x] Add the `TextEntry`-aware resolve helper with unit tests for all three widget kinds.
- [x] Extend root resolution to dispatch the three command sections and update the unknown-section test (no other execute test changes).
- [x] Unit tests: defaults resolve, per-section remap, cross-section chord reuse, `shift+q` canonicalization, conflict precedence inside the new contexts.

### Deliverable 2. `pb settings` honors its keymap

Depends on Deliverable 1. Replace hard-coded key matches across the settings screens with single-context resolution per screen, keep `Ctrl+C` unconditional, preserve the unsaved-changes confirmation flow, derive all footer/help strings from the active bindings, and pin the persistence guarantee.

- [x] Wire settings keymap and the full shared warning list from `AppConfig` into `SettingsApp`; show warnings as initial status.
- [x] Replace quit/section/search/tuner/theme/paths key matches with `settings.global`, `settings.list`, `settings.search`, and `settings.tuner` action resolution.
- [x] Derive settings footer/help text from the keymap, omitting unbound actions.
- [x] Add a persistence regression test: saving settings over a config containing `[keybinds.*]` tables preserves those tables byte-for-byte.
- [x] Tests: remapped quit, navigation, search-screen reset, tuner adjust/reset/save, back-out; fully-unbound back-out and quit still exit via Ctrl+C; help text reflects a remap.

### Deliverable 3. `pb new` honors its keymap

Depends on Deliverable 1. Make the target picker, history picker, and token-confirmation screens configurable while preserving text entry in filters, the name field, and token rename, using the shared precedence helper.

- [x] Wire the new-command keymap and the shared warning list into the capture flow entry points; show warnings as TUI status (never stdout).
- [x] Replace `target_picker_key`/`picker_key` matches with `new.picker` resolution through `TextEntry::WhenEmpty`.
- [x] Replace the confirm screen's `Focus::Name`/`Focus::TokenList`/`Focus::TokenEdit` matches with `new.confirm_name`/`new.confirm_tokens`/`new.confirm_rename` (name and rename via `TextEntry::Always`).
- [x] Derive the capture screens' help lines from the keymap.
- [x] Tests: remapped accept/toggle/rename/back in the token list; remapped `complete_or_focus_tokens` in the name field; typing `k` into a non-empty filter still filters while `k` on an empty filter navigates; a remapped plain-letter picker action respects the same rule; plain-letter bindings in `confirm_name`/`confirm_rename` stay inert; Ctrl+C cancels from every `pb new` screen.

### Deliverable 4. Docs and verification

Depends on Deliverables 1–3.

- [x] Extend the commented `[keybinds]` reference in `examples/config.toml` with all settings and new contexts from the config-shape block (flows into `pb docs config`), including the always-text-widget inertness rule for plain letters.
- [x] Update `README.md`'s Keybinds section to mention all three commands.
- [x] `cargo fmt --check`, `cargo test`, `cargo clippy -- -D warnings`.
- [x] Interactive TUI pass over `pb settings` and `pb new` with a remapped config: remaps work, replaced defaults are inert, help shows remapped keys, warnings appear as status, Ctrl+C always exits.

## Issues

- **2026-07-02 — agent:claude (implementation)** — All four deliverables implemented and verified (480 lib tests, clippy/fmt clean, examples lint clean, interactive zellij pass over both TUIs). Deviations from the plan, per the "extend the table before coding" instruction: (1) the theme screen's existing `r`/`R` reset-to-default-theme was missing from the context inventory — `settings.list` gained a `reset` action (`["r", "shift+r"]`) that only takes effect on the theme screen, rather than adding a fourth settings context; (2) the inventory listed `settings.search` `reset` as a *current* default, but the shipped chooser screen had no `r` handler — reset-from-chooser is new behavior, implemented as "reset the highlighted group's fields and save", matching the tuner's reset semantics; (3) `README.md` had no existing Keybinds section, so one was created under Configuration. Interactive-pass note: `ctrl+i` is delivered as Tab by real terminals, so it silently cannot be a distinct binding — covered by the existing "best-effort chords" caveat in the config reference.
- **2026-07-02 — agent:claude (adversarial review)** — Plan reviewed by 2 adversarial sub-agents (Risks & Assumptions, Completeness & Scope). 13 findings after dedup; 13 merged into plan. Most significant changes: bare `"+"` was verified to be unparseable in the shipped grammar (tuner defaults now use a new `plus` named key with D1 tests); the search screen got its own `settings.search` context so no screen resolves two contexts; the text-precedence rule became a single shared helper with an explicit always-text-widget inertness rule; `pb new` warning display, `confirm_name` remap tests, and per-screen Ctrl+C tests were added to Deliverable 3; the toml_edit round-trip question was resolved from `settings/persist.rs` (in-place `DocumentMut` edits preserve unknown tables) and pinned as a D2 regression test; warning scope decided as "every TUI shows the full shared list"; deliverable dependencies stated.
- **2026-07-02 — agent:claude** — Initial plan drafted as the follow-on committed in `docs/keybinds-bigplan.md`. Grill skipped: the brief directly extends a shipped, adversarially-reviewed design whose config model reserved these sections, and all open questions were resolvable from the code. Context inventory verified against `src/settings/app.rs` and `src/new/capture.rs` during drafting.
