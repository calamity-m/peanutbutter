# BIGPLAN: LSP code actions — Extract Variable / Inline Variable

> Tracks GitHub issue [#44 — Add Code Actions to LSP - Extract Variable/Inline Variable](https://github.com/calamity-m/peanutbutter/issues/44).
>
> Builds on the existing LSP server in `src/lsp.rs`, the snippet parser in
> `src/parser.rs`, and the `Frontmatter` / `VariableSpec` model in `src/domain.rs`.

## Plan Overview

Snippet authors currently keep two sources of truth in sync by hand: inline
placeholder behavior (`<@branch:git branch ...>`, `<@path:?.>`) in the snippet
body, and `variables.<name>` specs in the file's YAML frontmatter. This effort
adds two `textDocument/codeAction` providers to the LSP so common refactors are
one keystroke instead of a manual two-site edit:

- **Extract Variable to Frontmatter** — cursor on an inline placeholder with
  behavior; lift the `default`/`command` into a `variables.<name>` spec and
  reduce the placeholder to its plain `<@name>` form.
- **Inline Variable from Frontmatter** — cursor on a frontmatter variable spec;
  push its `default`/`command` back into the first matching `<@name>`
  placeholder in the body and drop the now-unused frontmatter entry.

"Done" means: the server advertises code action support; actions appear only on
eligible placeholders/frontmatter entries; edits are returned as
`WorkspaceEdit`s (never applied server-side); all edits are confined to the
**current file**; existing parse/lint/hover/completion behavior is unchanged;
and the test matrix in the issue's acceptance criteria passes.

## Risks

- **No YAML serializer — frontmatter must be edited as text.** `src/parser.rs`
  reads frontmatter with a hand-rolled line scanner (`parse_yaml_frontmatter`,
  `parse_variable_block`); there is no `serde_yaml` round-trip and no writer.
  Every code action edit must be produced as a textual `TextEdit` against the
  raw document, computing exact line/character ranges. Mitigation: reuse the
  existing line-scanning helpers (`frontmatter_end_line`,
  `find_variable_declaration_line` in `src/lsp.rs`) to locate insertion points,
  and emit minimal `TextEdit`s rather than rewriting the whole block. The two
  concrete defect sources here are **indentation** and **scalar quoting**, not
  vague drift: (1) pin emitted indentation to the block's existing 2-space
  nesting; (2) a `default`/`command` value can contain `:`, `#`, `{`, leading/
  trailing spaces, quotes, and `<#ref>` tokens (e.g. `<#a:raw>.<#b:raw>.out`) —
  inserting such a value into *unquoted* YAML produces invalid frontmatter or a
  silently truncated value. The emitter must YAML-quote-when-needed (see
  Deliverable 3) and round-trip tests must assert a reversible Extract→Inline
  yields a byte-identical body. Watch-for: clobbering unrelated frontmatter keys.
- **Extract/Inline are not behavior-neutral across snippets.** Frontmatter is
  file-scoped; a name used by multiple snippets means extracting an inline
  default into frontmatter retroactively applies it to *every* sibling snippet
  using `<@name>` plainly, and inlining hides the spec from those siblings. The
  chosen mitigation is to keep offering the action but **title it with the
  affected-snippet count** so the consequence is explicit, plus refuse inline
  when a config spec overlays the name. Watch-for: treating the round-trip as
  lossless when a sibling snippet relied on the prior (no-spec) behavior.
- **Capability must be verified at the real handler boundary, not just the pure
  fn.** Unit-testing `compute_code_actions` can pass while the advertised
  capability or `WorkspaceEdit` shape is wrong in a real editor (CLAUDE.md warns
  the LSP path differs from direct invocation). Mitigation: assert
  `code_action_provider` is present in the `initialize` response, and add a
  handler-level test that the returned `WorkspaceEdit` targets the correct URI
  and ranges (Deliverable 7).
- **Placeholder rewrites must preserve `<#name>` dependent references verbatim.**
  An inline command or default may embed `<#secret>` / `<#secret:raw>` tokens
  (see SNIPPET_SYNTAX "Dependent Variables"). When extracting to frontmatter the
  `command:`/`default:` string must carry those tokens through unchanged, and
  when inlining they must come back unchanged. Mitigation: treat the inline
  source as an opaque substring captured by `find_placeholder_end`; never
  re-parse or normalize it. Watch-for: shell-quoting or YAML-escaping the value
  and corrupting `'...'` / `%(...)` content.
- **Frontmatter is file-scoped but placeholders live per-snippet.** A
  `variables.<name>` spec applies to *every* snippet in the file. Extracting
  from one snippet, then inlining, could change which snippet "owns" the
  behavior. The chosen scope (current file) keeps this coherent, but the inline
  action must pick a deterministic target — the **first** `<@name>` occurrence in
  document order — to match the parser's first-occurrence-wins rule. Watch-for:
  inlining onto a later occurrence and silently changing prompt behavior.
- **`tower_lsp` capability + `code_action` handler wiring.** Adding the provider
  means setting `code_action_provider` in `initialize` and implementing
  `code_action`. The handler receives a `CodeActionParams` with a range, not a
  single position; eligibility must be computed from the range start. This gates
  the entire feature's UX, so it is not low-severity: get the range/marker-root
  gating wrong and actions appear in the wrong place or never appear. Mitigation:
  mirror the existing `find_marker_root` guard used by every other handler, and
  test eligibility at a zero-width cursor, a selection spanning the placeholder,
  and a cursor just outside the span.

## Plan Details

### Decisions (from pre-draft grill, 2026-05-26)

- **Conflict on extract** — when a `variables.<name>` spec already exists and
  *differs* from the inline behavior: offer a *separate* "Extract and overwrite
  frontmatter" action in addition to the safe one. When the existing spec already
  matches, just offer the placeholder-simplification (no frontmatter change /
  no-op on the spec).
- **Edit scope** — current file only. Rewrite all/any matching placeholders
  within the active document; never emit cross-file edits.
- **Inline + suggestions** — if a frontmatter spec carries `suggestions` (which
  inline syntax cannot represent), do **not** offer the inline action at all.
  Inline is only offered for specs reducible to a single `default` or `command`.
- **Inline target for multiple usages** — add the inline source to the **first**
  `<@name>` occurrence in prompt order only; leave later occurrences as plain
  `<@name>`.
- **Cross-snippet usage** — a `variables.<name>` spec applies to *every* snippet
  in the file, so extracting/inlining a name used by more than one snippet is not
  semantically neutral. Still offer both actions, but **title them to flag the
  blast radius** when N>1 snippets use the name, e.g.
  *"Extract `<@branch>` to frontmatter (affects all 3 snippets using it)"*. The
  author decides; the title makes the consequence explicit rather than silent.
- **Config-spec overlay** — when a config-defined `variables.<name>` spec exists
  (resolution order: inline > frontmatter > config), treat it as a **conflict
  signal**: skip the inline action (inlining would shadow the config spec in a
  way the author may not intend) and flag it on extract. This is why
  `compute_code_actions` takes `&config.variables` — the parameter is load-bearing,
  not plumbing.

### Mapping between inline forms and frontmatter

| Inline placeholder        | Frontmatter spec field |
| ------------------------- | ---------------------- |
| `<@name:?default>`        | `default: default`     |
| `<@name:command>`         | `command: command`     |
| `<@name>` (plain)         | (no spec / free-form)  |

`suggestions` has no inline equivalent — relevant only as a *blocker* for the
inline action (see decisions).

### Critical Files

- `src/lsp.rs` — add `code_action_provider` to `initialize` capabilities, add
  the `code_action` trait method, and add `compute_code_actions` plus the two
  edit-builder functions. Reuse `placeholder_at`, `frontmatter_end_line`,
  `find_variable_declaration_line`, `uri_to_path`, `find_marker_root`,
  `line_range`. This is where ~all new code lands.
- `src/parser.rs` — source of truth for placeholder spans
  (`find_placeholder_end`, `parse_variable_inner`) and frontmatter variable
  parsing (`parse_variable_block`). Read-only here; reuse to locate spans and to
  confirm a frontmatter entry's shape, but do not change parsing behavior.
- `src/domain.rs` — `VariableSpec` (default/suggestions/command) and
  `VariableSource` (Free/Command/Default). Used to reason about eligibility.
- `docs/SNIPPET_SYNTAX.md` — authoritative grammar for placeholders, defaults,
  commands, and `<#name>` dependent refs. Update only if behavior visibly
  changes for authors (likely a short note that the LSP offers these actions).

### Gotchas

- `placeholder_at` extracts only the *name* today (splits on `:`); the extract
  action needs the **source substring** too. Either extend it or add a sibling
  that returns the inner source. The inner-source span is bounded by
  `find_placeholder_end` from `src/parser.rs`.
- `<@name:?default>` — the `?` marks a default; the stored frontmatter value is
  everything *after* the `?`. Don't include the `?` in the `default:` value.
- A code action range can be a zero-width cursor or a selection; gate on the
  range **start** position and reuse the existing single-position helpers.
- Frontmatter may be entirely absent. Extract-to-frontmatter must be able to
  *create* the `---\n...\n---` block (and a `variables:` key) at the top of the
  file. Inserting before line 0 with a trailing newline; mind files that start
  with a heading vs. blank line.
- When inlining removes the last sub-key of a `variables.<name>` entry, also
  remove the `name:` line; if that empties the `variables:` block, remove
  `variables:` too. Be careful not to delete a `variables:` block that still has
  other variables.
- LSP positions are 0-based (char) but the parser/lint use 1-based lines in
  places (see `lint_finding_to_diagnostic`). Keep the conversion consistent.
- **YAML scalar quoting is mandatory, not optional.** When emitting a
  `default:`/`command:` value, quote it whenever it would otherwise be ambiguous
  YAML — values containing `:`, `#`, leading/trailing whitespace, or starting
  with a YAML indicator char. `<#ref>` tokens themselves are safe inside a
  double-quoted scalar but must not be escaped/altered. The existing
  `parse_variable_block` uses `strip_quotes` on read, so the writer's quoting must
  round-trip through it.
- **Inline eligibility has guards beyond "single default xor command".** Skip the
  inline action when: the target `<@name>` already carries an inline source
  (would double-specify), the spec carries `suggestions` (already decided), or a
  config-defined spec exists for the name (config-overlay conflict signal).

### Pseudo-code / Sketches

```text
code_action(params):
  if find_marker_root(uri) is None: return None
  content = documents[uri]
  range_start = params.range.start
  actions = []

  # Extract path
  if (name, src_kind, src_value, span) = inline_placeholder_at(content, range_start):
      existing = frontmatter_spec(content, name)
      if existing is None or existing == spec_from(src_kind, src_value):
          actions += extract_action(name, src_kind, src_value, span, create_fm_if_missing)
      else:  # conflict
          actions += extract_overwrite_action(name, src_kind, src_value, span, existing)

  # Inline path
  if name = frontmatter_var_decl_at(content, range_start):
      spec = frontmatter_spec(content, name)
      if spec has suggestions: skip
      elif spec reducible to single default xor command:
          first = first_plain_placeholder(content, name)   # document order
          actions += inline_action(name, spec, first, remove_fm_entry)

  return actions or None

extract_action -> WorkspaceEdit {
  TextEdit: replace placeholder span  "<@name:SRC>" -> "<@name>"
  TextEdit: upsert variables.name spec in frontmatter (create block if missing)
}

inline_action -> WorkspaceEdit {
  TextEdit: replace first "<@name>"   -> "<@name:?default>" | "<@name:command>"
  TextEdit: delete frontmatter entry (and empty parents)
}
```

## Deliverables

### Deliverable 1. Advertise code action capability + handler skeleton

Wire the LSP to advertise and route code actions. Add
`code_action_provider: Some(CodeActionProviderCapability::Simple(true))` to the
`ServerCapabilities` in `initialize`, and implement the `code_action` trait
method on `Backend`. The method mirrors the existing handlers: gate on
`find_marker_root`, read the document from `documents`, and delegate to a new
pure `compute_code_actions(content, range, &config.variables) -> Option<Vec<CodeActionOrCommand>>`
that returns `None` until the later deliverables fill it in. This isolates the
async/IO boundary from the testable pure logic, matching `compute_completions` /
`compute_hover`.

- [x] Add `code_action_provider` to `initialize` capabilities.
- [x] Add `async fn code_action(&self, params: CodeActionParams)` to the `LanguageServer` impl with marker-root + document gating.
- [x] Add `compute_code_actions(...)` stub returning `None`.
- [x] Test: `initialize` response includes `code_action_provider` (assert at the handler boundary, not just the pure fn).
- [x] `cargo build` + existing tests still green; no behavior change yet.

### Deliverable 2. Placeholder/source extraction helper

Provide a helper that, given a line (or content) and a cursor position, returns
the placeholder name, its source kind (default vs. command vs. free), the source
*value* string, and the full byte span of the placeholder. Today `placeholder_at`
returns only `(name, start, end)` and discards the source. Add a sibling
(e.g. `inline_placeholder_at`) that also returns the inner source, reusing
`parser::find_placeholder_end` to get the correct end (so embedded `>` inside
`<#...>` refs and quoted strings don't truncate the span). Free-form `<@name>`
placeholders return a "no source" result so the extract action isn't offered.

- [x] Add `inline_placeholder_at` returning name, source kind, source value, span.
- [x] Reuse `parser::find_placeholder_end` for span end; preserve `<#name>` tokens verbatim.
- [x] Unit tests: default `<@p:?.>`, command `<@b:git ...>`, command containing `<#ref>`, plain `<@x>` (no source), malformed/unterminated (none).

### Deliverable 3. Extract Variable to Frontmatter (existing frontmatter)

Implement the extract action for files that already have a frontmatter block.
Produce a `WorkspaceEdit` with two `TextEdit`s: (1) replace the placeholder span
with the plain `<@name>` form, (2) upsert the `variables.<name>` entry under the
`variables:` block (creating the `variables:` key if frontmatter exists but lacks
it). When an equal spec already exists, only emit edit (1). When a *conflicting*
spec exists, do not emit the safe action here — that is Deliverable 5.

- [x] Build the `<@name>` replacement `TextEdit`.
- [x] Build the frontmatter upsert `TextEdit` (insert `variables:` and/or `  name:` + `    default:`/`    command:` with correct 2-space indentation).
- [x] Map `<@name:?default>` → `default:`, `<@name:command>` → `command:`.
- [x] YAML-quote the emitted value when needed (values with `:`, `#`, leading/trailing space, indicator-char start); preserve `<#ref>` tokens unchanged; ensure it round-trips through `parse_variable_block`/`strip_quotes`.
- [x] When the name is used by >1 snippet in the file, title the action with the affected-snippet count (e.g. "… affects all N snippets using it").
- [x] No-op the frontmatter edit when an identical spec already exists.
- [x] Tests: extract inline default to frontmatter; extract inline command to frontmatter; extract when an identical spec already present (placeholder simplified, no dup spec); extract a default containing `:`/`#`/`<#ref>` produces valid quoted YAML.

### Deliverable 4. Extract creates frontmatter when missing

Extend Deliverable 3 to handle files with no `---` frontmatter block. The action
inserts a new block at the very top of the document containing `variables:` and
the new entry, preserving the rest of the file. Handle the leading-content edge
cases (file starts with `##`, file starts blank).

- [x] Detect absent frontmatter via `frontmatter_end_line` returning `None`.
- [x] Insert `---\nvariables:\n  name:\n    default|command: value\n---\n` at offset 0, applying the same YAML-quoting rule as Deliverable 3.
- [x] Tests: create frontmatter from a file with none (default case and command case); value needing quoting; ensure body and existing first heading are untouched.

### Deliverable 5. Conflict handling — separate overwrite action

When the cursor is on an inline placeholder whose `variables.<name>` spec exists
*and differs*, offer a second, explicitly-labeled action
("Extract and overwrite frontmatter spec for `name`") that replaces the existing
spec's `default`/`command` with the inline value. The safe action from
Deliverable 3 is suppressed in this case to avoid silently clobbering. Title the
two actions distinctly so the editor's code-action menu makes the choice obvious.

- [x] Detect existing-and-differing spec.
- [x] Emit only the overwrite-titled action in the conflict case.
- [x] Overwrite replaces the differing field(s) in place; leaves unrelated fields (e.g. `suggestions`) intact unless they conflict with the inline kind.
- [x] Tests: conflict produces overwrite action (not the plain one); applying it updates the spec; no-op/clean case still produces the plain action.

### Deliverable 6. Inline Variable from Frontmatter

Implement the inline action triggered on a frontmatter variable declaration line.
Eligibility: the spec must be reducible to a single `default` *xor* `command`
and must **not** carry `suggestions` (no action offered otherwise). Produce a
`WorkspaceEdit`: (1) rewrite the **first** `<@name>` occurrence in document order
to `<@name:?default>` or `<@name:command>`, preserving any `<#...>` tokens; (2)
remove the frontmatter entry, and clean up empty `variables:` parent if it
becomes empty. Later `<@name>` occurrences are left as plain placeholders.

- [x] Detect cursor on a `variables.<name>` declaration line (reuse `find_variable_declaration_line` logic / the references-path frontmatter detection).
- [x] Eligibility gate: single default-or-command, no suggestions; **skip** when the target `<@name>` already carries an inline source; **skip** when a config-defined spec exists for the name (overlay conflict signal — uses `&config.variables`).
- [x] Find the first `<@name>` in document order; build the inline-rewrite `TextEdit`, preserving any `<#...>` tokens.
- [x] When the name is used by >1 snippet in the file, title the action with the affected-snippet count.
- [x] Build the frontmatter-entry removal `TextEdit`; remove empty `variables:` block when last entry removed.
- [x] Tests: inline default into first usage; inline command into first usage; multi-usage rewrites only the first; suggestion-bearing spec offers no action; spec with both default and command (ambiguous) offers no action; config spec present → no action; target already has inline source → no action; reversible Extract→Inline yields byte-identical body.

### Deliverable 7. Negative cases, docs, and full acceptance matrix

Close out the issue's acceptance criteria: confirm no action is offered for
malformed/unsupported placeholders or non-eligible positions, and confirm
parse/lint/hover/completion behavior is unchanged. Add a short author-facing note
to `docs/SNIPPET_SYNTAX.md` (and/or README) mentioning the LSP offers these
refactors. Run the full grill/issue test list end to end.

- [x] Test: no action on `<@>` / `<@has space>` / unterminated placeholder.
- [x] Test: no action when cursor is on plain body text or a non-variable frontmatter key.
- [x] Test: edits are returned as `WorkspaceEdit` only (assert nothing is applied server-side — the handler returns edits, never writes files).
- [x] Handler-boundary test: returned `WorkspaceEdit` targets the correct document URI and ranges (not just that the pure fn produced edits).
- [x] Test: eligibility at zero-width cursor, selection spanning the placeholder, and cursor just outside the span.
- [x] Confirm `cargo test` covers every bullet in the issue's "Tests cover" list.
- [x] Brief doc note in `docs/SNIPPET_SYNTAX.md` about the available code actions.
- [x] `cargo fmt`, `cargo clippy -- -D warnings`, `cargo test` all clean.

## Issues

- **2026-05-26 — agent:pi** — Implemented all planned LSP code actions, docs note, and acceptance tests; verification passed with `cargo test` and `cargo clippy -- -D warnings`.
- **2026-05-26 — agent:claude (adversarial review)** — Plan reviewed by 2 adversarial sub-agents (Risks & Assumptions, Completeness & Scope). ~9 findings; 7 merged. Most significant: both reviewers independently flagged that file-scoped frontmatter makes Extract/Inline non-neutral when a name is shared across snippets — resolved via user decision (warning-titled actions + config-overlay conflict gate) and a new top-line risk. Also hardened YAML scalar quoting (was a "watch-for", now a deliverable + gotcha) and added handler-boundary capability/WorkspaceEdit verification.
- **2026-05-26 — agent:claude** — Pre-draft grill resolved four design decisions (conflict→separate overwrite action; scope→current file; suggestions→no inline action; multi-usage→first occurrence only). Recorded under Plan Details › Decisions.
- **2026-05-26 — agent:claude** — Open question deferred: should the extract action also offer to *merge* an inline default with an existing `command` spec (or vice versa) rather than treating any field difference as a conflict? Current plan treats differing field as conflict → overwrite. Revisit if authors find the overwrite too blunt.
