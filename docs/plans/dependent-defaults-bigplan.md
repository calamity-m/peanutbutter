# BIGPLAN: Dependent variable references in `?default` expressions

> Tracks GitHub issue [#47 — feat: allow dependent variable references in `?default` expressions](https://github.com/calamity-m/peanutbutter/issues/47).
>
> Builds on the dependent-variable machinery introduced by
> [dependent-variables-bigplan](dependent-variables-bigplan.md).

## Plan Overview

Today, `<#name>` and `<#name:raw>` dependent references work inside suggestion
commands (`<@name:command>`) but not inside default expressions
(`<@name:?default>`). Authors who want a contextual pre-fill — e.g. an output
path derived from earlier confirmed picks — have to fake it by writing
`<@name:echo ...>`, which spawns `bash -c` to produce a static string and
reads like a command instead of a default. "Done" means: a `?default`
expression accepts the same `<#name>` / `<#name:raw>` token grammar as
suggestion commands, substitution happens *after* the upstream variables are
confirmed, the same declaration-order rules apply (forward and self refs are
lint errors), and snippets that don't use the feature behave exactly as
today.

## Risks

- **Default rendering timing is different from suggestion commands** — A
  suggestion command is rendered *at the moment the user enters the variable*
  (so the latest confirmed values feed it). A default has historically been a
  static string carried on the `Variable` struct from parse time. The new
  behavior must render the default at variable-entry time, not at parse time,
  or substitutions will always be empty. Mitigation: parse the default into
  a `CommandTemplate` once and store the parsed form; render it at the same
  point `load_prompt_state` currently restores `default_input`. Watch-for:
  defaults pre-rendered with empty values because rendering moved to the
  wrong place.
- **Dirty-descendant rule must extend to defaults** — If the user tabs back
  and changes an upstream value, a downstream variable whose default refers to
  that upstream is now stale. The existing dependent-suggestion machinery
  handles this for *suggestion lists* by marking descendants dirty until
  reconfirmed; the default-pre-fill path must respect the same rule, or the
  pre-filled value will silently disagree with the user's later picks.
  Mitigation: treat a default-side `<#name>` reference as a dependency the
  same way a suggestion-command reference is, so the existing dirty-tracking
  in `PromptState` covers it. Watch-for: a previously-typed value in the
  input buffer being overwritten by a re-rendered default on revisit.
- **Unconfirmed upstream behavior must match the suggestion-command path** —
  Decision: default rendering uses the *same* strict semantics as suggestion
  commands. `render` returns `RenderError::MissingConfirmed` when an
  upstream is unconfirmed; the default path treats that as "no pre-fill
  available" and leaves the input buffer empty (callers may still fall back
  to the `default_input` of "" or a frontmatter/config-supplied default if
  one exists for that name — but no `<#...>`-driven splice is attempted).
  Under normal forward navigation, declaration-order rules guarantee the
  upstreams are already confirmed when the user reaches the variable, so
  this path is only exercised when an upstream has been dirtied. When the
  user revisits the upstream and reconfirms, the next entry to the
  downstream variable re-renders the default cleanly. Watch-for: an empty
  input buffer being mistaken for "confirmed empty" — the dirty-descendant
  rule must continue to apply to the downstream variable until the user
  actually confirms its value.
- **Pre-existing snippet corpus must keep parsing identically** —
  `assets/starter_snippets.md` and any user-authored corpora contain
  legacy `<@p:?.>`-style defaults. The variant retyping in Deliverable 1
  must not change their parsed semantics. Mitigation: regression-run the
  starter snippets through parse + render as part of Deliverable 1's
  acceptance tests.
- **LSP / lint surface must cover the default path** — Today
  `<#...>`-validation lint codes (`unknown-variable-reference`,
  `forward-variable-reference`, `self-variable-reference`,
  `invalid-dependent-reference`) only fire against suggestion commands.
  Defaults currently get a no-op at `lint.rs:769`. Risk: authors write a
  forward reference in a default and get no warning. Mitigation: route
  default sources through the same `parse_command_template` +
  reference-validation helpers used by commands.
- **`:raw` in defaults is the *common* case, not the exception** — Path-style
  defaults (the motivating use case in #47) almost always want `:raw` to
  avoid shell-quote noise like `'ns'.'sec'.'key'.out`. But a `:raw` splice
  in a default pre-fills an editable buffer that the user will likely
  accept with Enter — closer to execution than a `:raw` splice into a
  suggestion list. If the upstream is free-form (user-typeable), an
  attacker-influenced or accidentally-pasted value with shell
  metacharacters reaches the shell once the user accepts the pre-fill.
  Mitigation: introduce a dedicated lint code,
  `raw-default-untrusted-upstream`, that warns when a `<#name:raw>` inside
  a `?default` references an upstream variable whose own source is `Free`
  (no suggestion command, no fixed suggestion list, no frontmatter/config
  spec that constrains its values). Document the rule alongside the
  existing `:raw` security note. Watch-for: false positives on legitimately
  trusted free-form variables — confirm the existing `[lint.<code>]`
  suppression knob works for the new code.

## Plan Details

The mechanism for parsing `<#name>` / `<#name:raw>` already exists in
`src/command_template.rs`. The work is to thread it into the default-rendering
path and to keep lint/LSP in sync.

The change has three layers:

1. **Parsing** — `VariableSource::Default(String)` becomes
   `VariableSource::Default(CommandTemplate)` (or a parallel pre-parsed
   field). `parse_variable_inner` in `src/parser.rs` runs the default string
   through `parse_command_template`. The outer placeholder parser already
   handles nested `<#...>` correctly, so no parser-grammar change is needed
   beyond this re-typing.
2. **Substitution** — Wherever the code currently treats the default as a
   literal `String` and feeds it into the prompt input buffer (see
   `execute/app.rs:322` `default_input`), substitute through the new
   `render` helper using the same `confirmed: &BTreeMap<String, String>`
   snapshot that the suggestion path uses.
3. **Validation + dirty tracking** — `referenced_names` on a default
   template feeds the same `PromptState` dirty-descendant tracking as
   suggestion commands; the same lint codes apply.

### Critical Files

- `src/domain.rs` — `VariableSource::Default` variant. Decide whether to
  keep `Default(String)` and parse lazily, or change to
  `Default(CommandTemplate)`. Recommended: change the variant so the parsed
  form is the single source of truth (mirrors how commands are stored after
  parsing today).
- `src/parser.rs` — `parse_variable_inner` (line ~603). Parse the `?default`
  branch through `parse_command_template`. Surface parse errors the same
  way suggestion-command parse errors surface today (left literal /
  recoverable lint).
- `src/execute/app.rs` — `default_input` for `VariableSource::Default`
  (line ~322). Must accept `confirmed` and call `render`. Same recoverable-
  error path as the suggestion side.
- `src/execute/prompt.rs` — `load_prompt_state`. When restoring the input
  buffer from a default on first entry, render the default template against
  the current confirmed snapshot. The existing input-buffer preservation
  rule still wins for revisits — only first-entry / re-dirtied entries use
  the rendered default.
- `src/lint.rs` — extend the `VariableSource::Default(_)` arm (line ~769)
  to validate `<#...>` references with the same code set used for
  commands.
- `src/lsp.rs` — `dependent_ref_at` and friends must also scan default
  source ranges (currently scoped to command sources). Hover / go-to /
  references / completions inside `<@name:?...>` should behave the same as
  inside `<@name:command>`.
- `docs/SNIPPET_SYNTAX.md` — "Default Value" subsection plus the
  "Dependent Variables" section need a worked default example and a note
  that the same ordering and lint rules apply.

### Gotchas

- **Parsed-default re-typing has wide blast radius.** `VariableSource::Default`
  is matched in several places (parser, lint, execute/app, execute/prompt).
  Changing the inner type touches all of them in one PR. Alternative: keep
  the variant as `String` and lazily parse at use sites — fewer call-site
  edits but two sources of truth for the parsed form. Recommend the
  one-shot retyping.
- **Empty-template fast path.** A default with no `<#...>` references must
  cost no more than today (a single string clone). Keep the rendering call
  cheap when `is_dependent(&template)` is false.
- **The `?` strip happens before template parsing.** `parse_variable_inner`
  strips the `?` prefix and passes the remainder to the default parser; the
  template parser sees no leading `?`.
- **Default values are pre-filled into the input buffer, not auto-confirmed.**
  Rendering a default with substitution does not confirm the variable on
  the user's behalf. The user still presses Enter/Tab to confirm. This
  matters for the chained-defaults case: `<@b:?<#a:raw>>` followed by
  `<@c:?<#b:raw>>` only substitutes a `b` into `c`'s default if the user
  confirmed `b` (even if `b` was identical to its rendered default).
- **Escape syntax stays the same.** `\<#name>` renders as literal `<#name>`
  inside a default and does not count as a reference, identical to the
  command path. No new escape rule.
- **`<@output:?<#a>.<#b>>` rendering quotes by default.** The default
  splice form `<#a>` shell-single-quotes the value. For path-construction
  use cases authors will almost certainly want `<#a:raw>`. Document this
  prominently — the worked example in the issue uses `:raw` for exactly
  this reason.
- **`:raw` in defaults reaches the shell via the user's Enter, not via
  `bash -c`.** Unlike `:raw` in suggestion commands (executed in a
  controlled sub-shell with the rendered string), a `:raw` splice in a
  default lands in the user's input buffer; the user is the one who
  decides to send it to the shell. This shifts the threat model — see the
  `raw-default-untrusted-upstream` lint in Deliverable 3.
- **Cycles via defaults can't form under the declaration-order rule.** A
  default may only reference variables that appear earlier in prompt
  order; the existing forward- and self-reference lint codes already
  forbid the only ways to construct a cycle (`<@a:?<#b>>` followed by
  `<@b:?<#a>>` would fire `forward-variable-reference` on the first
  default). No new cycle-detection pass is needed, but the lint coverage
  in Deliverable 3 must include a test that demonstrates this.
- **Dirty-descendant tracking is upstream-name-keyed, not source-kind-
  keyed.** Per [[dependent-variables-bigplan]], the existing dirty rule
  reads `referenced_names(template)` regardless of whether the template
  came from a suggestion command or anywhere else. Extending it to
  defaults is a wiring change — make sure the function that builds the
  per-variable upstream set considers `VariableSource::Default`'s
  template as well as `VariableSource::Command`'s.

### Pseudo-code / Sketches

```text
// domain.rs
enum VariableSource {
    Free,
    Command(CommandTemplate),   // already migrated for the command path
    Default(CommandTemplate),   // NEW: was Default(String)
}

// parser.rs (parse_variable_inner)
if let Some(default_src) = rest.strip_prefix('?') {
    let template = parse_command_template(default_src)
        .unwrap_or_else(|_| vec![Fragment::Literal(default_src.to_string())]);
    VariableSource::Default(template)
}

// execute/app.rs (default_input)
VariableSource::Default(tpl) => {
    // Strict render. Same semantics as the suggestion-command path:
    // if any upstream <#name> is unconfirmed, no pre-fill is offered.
    // The dirty-descendant rule guarantees this only happens after the
    // user has dirtied an upstream — under normal forward navigation,
    // upstreams are always confirmed first per the declaration-order rule.
    render(tpl, confirmed).ok()
}

// lint.rs
VariableSource::Default(tpl) => {
    validate_dependent_refs(tpl, &declaration_order, current_name);
    // Same code set: unknown-variable-reference, forward-variable-reference,
    // self-variable-reference, invalid-dependent-reference.
}
```

## Deliverables

### Deliverable 1. Parse `?default` as a `CommandTemplate`

Re-type `VariableSource::Default` to carry a `CommandTemplate` and route
default parsing through the existing `parse_command_template` helper. No
behavior change yet for snippets that don't use `<#...>` in defaults.

- [x] Change `VariableSource::Default(String)` to
      `VariableSource::Default(CommandTemplate)` in `src/domain.rs`.
- [x] In `src/parser.rs::parse_variable_inner`, parse the post-`?` remainder
      via `parse_command_template`. On parse error, fall back to a single
      `Fragment::Literal` containing the raw text so authors aren't blocked
      from running the snippet — lint surfaces the parse error separately.
- [x] Update every `match` on `VariableSource::Default` in `lint.rs`,
      `execute/app.rs`, `execute/prompt.rs`, and tests to bind the
      template form. For non-dependent defaults, the existing behavior
      (return the literal string) must be preserved bit-for-bit.
- [x] Add a parser test: `<@out:?<#a:raw>.<#b:raw>.out>` parses with the
      full inner string preserved (relies on the existing
      `find_placeholder_end` nested-ref handling).
- [x] Add a parser test: `<@p:?plain>` parses to a single
      `Fragment::Literal("plain")`, confirming the no-ref fast path.
- [x] **Enumerate every read site of `VariableSource::Default`** before
      changing the variant. Grep for `VariableSource::Default` and
      `Default(` (with manual filtering) across the workspace; produce a
      list in this deliverable's commit description. Known sites today:
      `src/parser.rs`, `src/lint.rs`, `src/execute/app.rs` (×3),
      `src/execute/prompt.rs`. Verify none are missed — in particular
      check picker preview / hover rendering, bash export, and the
      `pb new` capture path. Each site decides between "render leniently
      for preview" and "show the raw template text."
- [x] Verify `find_placeholder_end` (src/parser.rs:575) already treats
      nested `<#...>` inside `<@...:?...>` as opaque (it appears to, from
      reading the source — assert it with a dedicated test rather than
      relying on inspection).
- [x] Regression: run `assets/starter_snippets.md` and any starter corpus
      through the parser pre- and post-change; produced `Snippet` ASTs
      must be byte-identical for entries that don't use `<#...>` in
      defaults.

### Deliverable 2. Render dependent defaults at variable-entry time

Substitute `<#...>` references in defaults using the current confirmed-values
snapshot, at the same point the input buffer is populated for first entry.
Reuse the strict `render` from the suggestion path and define the missing-
upstream behavior consistently.

*Depends on Deliverable 1.*

- [x] Reuse the existing strict `render(&CommandTemplate, &confirmed)`
      from `src/command_template.rs`. Do NOT add a lenient variant: a
      default whose render fails (missing confirmed upstream) yields "no
      pre-fill", and the input buffer falls back to empty (or whatever
      non-`<#...>` default text exists, e.g. a frontmatter/config-supplied
      static default for the same variable name).
- [x] Update `SuggestionProvider::default_input` (or its caller in
      `execute/app.rs:322`) to accept the same `confirmed` argument the
      suggestion path already receives, and render the default template
      through strict `render`. On `RenderError::MissingConfirmed`, return
      `None` (or empty) rather than panicking.
- [x] Ensure `load_prompt_state` re-renders the default on first entry to a
      never-confirmed variable, after upstream values are available. Do NOT
      overwrite a preserved input buffer on revisit — the existing input-
      buffer preservation rule wins.
- [x] **Extend the dirty-descendant graph to read default templates.**
      Locate the function in `src/execute/prompt.rs` that builds each
      variable's `upstream-name set` from its `CommandTemplate` (the
      function `dependent-variables-bigplan` introduced for the suggestion
      path). Verify whether it is keyed off `VariableSource::Command`
      specifically or off a source-agnostic accessor; if the former, lift
      it so it also reads `VariableSource::Default`'s template. Failing
      that, this becomes its own sub-deliverable — flag in `## Issues`
      rather than absorbing the refactor silently.
- [x] Unit test: default `<@out:?<#namespace:raw>.<#secret:raw>.<#key:raw>.out>`
      with all three upstreams confirmed renders to `ns.sec.key.out` in
      the input buffer.
- [x] Unit test: same default with `secret` unconfirmed yields no
      pre-fill (input buffer empty); after confirming `secret` and
      re-entering the variable, the default renders correctly.
- [x] Unit test: `<#name>` (quoted form) inside a default with confirmed
      `name=O'Brien's` renders as `'O'\''Brien'\''s'` — same quoting
      contract as commands.
- [x] Test: Tab back, change upstream, Tab forward to a downstream variable
      whose default references the upstream — the downstream is dirty, the
      user's typed value (if any) is preserved per the existing rule; if
      no typed value exists, the default is re-rendered with the new
      upstream.
- [x] Test: empty-input-buffer-from-failed-render is *not* treated as
      "confirmed empty" by downstream variables — the variable still
      requires explicit confirmation.
- [x] Regression test (characterization): a snippet using `<@p:?.>` (no
      dependent refs) behaves identically pre- and post-change — first
      entry pre-fills `.`, revisit preserves user input.

### Deliverable 3. Lint validation for defaults

*Depends on Deliverable 1 (shared parsed template).*

Run the same `<#...>` validations against default templates that already
run against command templates. Same lint codes, same suppression plumbing.

- [x] Extract the `<#...>`-validation walk from the command-source path in
      `src/lint.rs` into a helper that takes a `CommandTemplate` plus the
      surrounding declaration order, and call it from both the
      `VariableSource::Command(_)` and `VariableSource::Default(_)` arms.
- [x] Emit `unknown-variable-reference`, `forward-variable-reference`, and
      `self-variable-reference` for default-side refs with precise spans on
      the `<#...>` token.
- [x] Emit `invalid-dependent-reference` for malformed defaults
      (unterminated, empty name, unknown modifier) when the template
      parse fell back to literal in Deliverable 1.
- [x] Note: `dependent-suggestion-skipped` does NOT apply to defaults
      (defaults are not executed). Confirm with a test that linting a
      dependent default does not emit that code.
- [x] Test: `<@b:?<#nope>>` emits `unknown-variable-reference` pointing at
      `<#nope>`.
- [x] Test: `<@a:?<#b>> ... <@b>` emits `forward-variable-reference` on
      `<#b>`.
- [x] Test: `<@a:?<#a>>` emits `self-variable-reference`.
- [x] Test: escaped `\<#nope>` inside a default is NOT reported.
- [x] **Add new lint code `raw-default-untrusted-upstream`.** Fires when a
      `<#name:raw>` inside a `?default` references a variable whose
      effective source (after frontmatter and config overlay) is `Free`
      (i.e. no suggestion command, no fixed suggestions, no default).
      Severity: warning. Suppressible via `[lint.raw-default-untrusted-upstream]`
      using the same plumbing as other lint codes. Document in the lint
      table and in `docs/SNIPPET_SYNTAX.md`.
- [x] Test: `<@a> ... <@b:?<#a:raw>>` (a is free-form) emits
      `raw-default-untrusted-upstream`.
- [x] Test: `<@a:?host> ... <@b:?<#a:raw>>` (a has a static default → no
      longer "free-form" for this lint's purposes) does NOT emit the
      warning. If we decide to keep the lint strict (only suggestions or
      command lists count as "trusted"), capture that in an Issues entry
      and adjust the test accordingly.
- [x] Test: `<@a:?host>` followed by a default that uses the shell-quoted
      form `<#a>` (not `:raw`) does NOT emit the warning regardless of
      upstream trust.
- [x] Test: cycle attempt `<@a:?<#b>> ... <@b:?<#a>>` emits
      `forward-variable-reference` on the first default, confirming the
      declaration-order rule already prevents cycles (no separate cycle
      detection needed).

### Deliverable 4. LSP coverage for default-side dependent refs

*Depends on Deliverable 3 (precise spans on default-side refs).*

Per [[dependent-variables-bigplan]] Deliverable 5, the LSP reverse index
("references from a frontmatter variable declaration include `<#...>` uses")
already exists for command sources. This deliverable extends the *scan
surface* to include default sources; it does not build a new reverse index.

Editor behaviors (hover, go-to-definition, references, completion,
diagnostic ranges) that already work for `<#...>` inside command sources
must also work inside default sources.

- [x] Extend `dependent_ref_at` (and any sibling helpers in `src/lsp.rs`)
      to recognize cursor positions inside default sources, not only
      command sources.
- [x] References from a frontmatter variable declaration must include
      `<#...>` uses inside default expressions, alongside the existing
      `<@...>` and command-side `<#...>` uses.
- [x] Completion: when typing `<#` inside a `<@name:?...>` default,
      suggest only variables that appear earlier in prompt order — same
      forward-ref rule as commands.
- [x] Hover on `<#name>` / `<#name:raw>` inside a default shows the same
      variable spec hover (with the quoted-vs-raw note) as the command
      path.
- [x] Tests in `src/lsp.rs` covering diagnostic ranges, go-to, references,
      hover, and completion for `<#...>` inside defaults.

### Deliverable 5. Documentation

Update `docs/SNIPPET_SYNTAX.md` so authors can discover and rely on
dependent defaults.

- [x] In the "Default Value" subsection, note that `?default` may contain
      `<#name>` / `<#name:raw>` references with the same grammar as
      suggestion commands, and link to the "Dependent Variables" section.
- [x] In the "Dependent Variables" section, add a worked default example
      from the issue:
      `| tee <@output:?<#namespace:raw>.<#secret:raw>.<#key:raw>.out>`
      and explain why `:raw` is the natural choice for path-style
      construction.
- [x] Document the unconfirmed-upstream behavior chosen in Deliverable 2
      ("missing splices render as empty").
- [x] Confirm the existing ordering, escape, and lint sections in the doc
      already cover defaults once the grammar is shared — only adjust the
      surrounding prose if it currently implies these only apply to
      commands.
- [x] Document the new `raw-default-untrusted-upstream` lint code in the
      lint table.
- [x] Add a short migration note for authors of existing
      `<@name:echo <#a:raw>>` workarounds: this still works, but the
      `<@name:?<#a:raw>>` form is now preferred (no sub-shell spawn,
      reads as a default rather than a command).

## Issues

- **2026-05-26 — agent:pi** — Implemented dependent defaults: `VariableSource::Default` now stores a parsed template, defaults render against confirmed upstreams at prompt-entry time, dirty-descendant tracking includes default refs, lint/LSP scan default-side `<#...>` refs, `raw-default-untrusted-upstream` is documented and tested, and `cargo test` passes.
- **2026-05-26 — agent:claude (adversarial review)** — Plan reviewed by 2
  adversarial sub-agents (Risks & Assumptions, Completeness & Scope). 11
  findings; 9 merged directly, 2 raised to the user for decision. Most
  significant changes: unconfirmed-upstream behavior locked to *strict
  render = no pre-fill* (consistent with the suggestion path) rather than
  a lenient empty-splice renderer; new `raw-default-untrusted-upstream`
  lint code added to Deliverable 3 to address the `:raw`-in-default
  footgun; Deliverable 1 grew an explicit "enumerate all
  `VariableSource::Default` read sites" task to prevent silent regressions
  in preview/hover/export paths; dirty-descendant graph extension was
  promoted from a single bullet to a verification step that may split out
  if the existing graph isn't source-agnostic.
- **2026-05-26 — agent:claude (watch-for)** — Carry into implementation
  review: (1) the dirty-descendant graph wiring (whether it's already
  source-agnostic or needs lifting); (2) `raw-default-untrusted-upstream`
  false-positive rate on `<@a:?static_default>` upstreams — may need to
  refine the "trusted" definition; (3) picker preview / hover / bash
  export rendering of a dependent default outside the TUI context (no
  `confirmed` snapshot exists there — likely render as the raw template
  text or as the literal `<#name>` tokens).
- **2026-05-26 — agent:claude** — Initial plan drafted from issue #47. Brief
  is explicit; skipped full `grill-me` because acceptance criteria,
  constraints, and the unconfirmed-upstream fallback are all spelled out in
  the issue. Reuses the parser/lint/LSP machinery already shipped by
  [[dependent-variables-bigplan]] — most of the work here is plumbing the
  existing helpers into the default code path, not new design.
