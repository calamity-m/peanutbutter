## BIGPLAN: Conditional / Dependent Variables

> Tracks GitHub issue [#8 — feat: conditional / dependent variables](https://github.com/calamity-m/peanutbutter/issues/8).

## Plan Overview

Allow a variable's suggestion command to reference values the user has already
confirmed for earlier variables, via explicit `<#name>` / `<#name:raw>`
placeholders inside the suggestion command string. The motivating case is
"pick a kubernetes secret, then pick a key from *that* secret's `data`" —
today each variable is resolved in isolation, so this is impossible. "Done"
means: snippets can declare chains of dependent suggestion commands, the TUI
re-fetches a downstream suggestion list when (and only when) any upstream
value it depends on has changed, the dependence is *explicit* (parsed, not
heuristic), and snippets that don't use the feature behave exactly as before.

The design deliberately avoids env-var injection — the original sketch
injected every confirmed value into the suggestion sub-shell as an env var,
which led to env-name shadowing, dash-in-name issues, leakage of unrelated
values, and a fragile substring heuristic for "does this command depend on
that variable?". Explicit `<#var>` placeholders make dependence visible in
the snippet, in the AST, in the cache key, and in the lint — all from one
source of truth.

## Risks

- **Tab-cycling UX feels wrong** — Today the user can Tab/Shift+Tab freely
  between variables; if they tab back and change an upstream value,
  downstream suggestion lists become stale. The reopen comment on #8 flags
  this as *the* hard part. Mitigation: keep previously-typed downstream
  values, mark dependent descendants dirty until reconfirmed, and re-run the
  downstream suggestion command only when the upstream snapshot it depends on
  has changed (see "Suggestion cache" in Plan Details). Watch-for: confusing
  flicker, lost user input, stale confirmed values being consumed by later
  variables, or transient prompt state (selected suggestion row, scroll,
  fuzzy filter input) being reset on cache hits.
- **`load_prompt_state` is a single point of failure** — The change reshapes
  this function (new "preserve typed value vs. fall back to default" rule,
  implicit cache lookup, dependence-driven re-render). A regression here
  breaks every snippet, not just dependent ones. Mitigation: add explicit
  characterization tests for the non-dependent path (Deliverable 1) before
  changing behavior; treat any diff in those as a blocker.
- **Inline command parsing can break on nested refs** — Inline command sources
  such as `<@key:aws s3 ls s3://<#bucket>/...>` contain an inner `>` before
  the outer `<@...>` placeholder is complete. Mitigation: update the inline
  placeholder parser to treat `<#...>` fragments inside command sources as
  nested syntax, and test that the outer command source is not truncated.
- **Tab cycling becomes per-keystroke expensive without caching** — Without
  the suggestion cache, every Tab/Shift+Tab re-spawns `bash -c` for the
  destination variable, fine for `echo` but visible latency for
  `kubectl get ...`. Mitigation: Deliverable 2 introduces an upstream-snapshot
  cache for dependent commands keyed on the confirmed-values fingerprint, so
  unchanged upstream → no re-spawn. Non-dependent commands intentionally keep
  today's re-fetch-on-revisit behavior. Watch-for: cache invalidation bugs
  (cached suggestions winning after an upstream change).
- **`raw` splice is a footgun** — `<#var:raw>` substitutes the confirmed
  value verbatim into the command string with no quoting. A value containing
  spaces, `;`, `$(...)`, or shell metacharacters can change the semantics of
  the suggestion command — including arbitrary command execution if the
  upstream value comes from an attacker-influenced suggestion list.
  Mitigation: keep the default (`<#var>`) shell-quoted; document `:raw` as
  the explicit opt-out for command-construction patterns and warn that
  upstream values for `:raw`-consumed variables should come from a trusted
  source. Watch-for: snippets in the wild using `:raw` against
  user-typeable upstreams.
- **Lint can't faithfully exercise dependent commands** — `pb lint` runs
  suggestion commands without any user input, so a downstream command with
  `<#bucket>` can't be executed (there is no confirmed bucket). Mitigation:
  detect dependence from the parsed AST (no heuristic — `<#name>` is a
  syntactic token) and skip those commands at lint time with a dedicated
  lint code that can be suppressed via the existing `[lint.<code>]` config.
- **Runtime template errors must not panic** — Lint should catch malformed,
  missing, forward, or self references, but users can still execute snippets
  that have not been linted or whose lint findings were ignored. Mitigation:
  make template parse/render failures recoverable prompt errors, never
  `confirmed[name]` panics.
- **Lint/LSP drift creates conflicting editor feedback** — The LSP publishes
  lint diagnostics but has separate hover/go-to/reference helpers for
  placeholders. Mitigation: dependent-reference diagnostics should come from
  the shared lint/template parser path, while LSP navigation should reuse the
  same reference parser so `<#...>` validity and editor behavior agree.

## Plan Details

Today, variable resolution is independent per variable (see
[`src/execute/app.rs:190`](../../src/execute/app.rs) for the
`SuggestionProvider` trait and
[`src/execute/prompt.rs:272`](../../src/execute/prompt.rs) for the loader).
`command_suggestions` spawns `bash -c <cmd>` in `cwd` with the parent's env
untouched.

The change has three layers:

1. **Parsing** — `<#name>` and `<#name:raw>` become recognized tokens inside
   any suggestion-command source: `<@name:cmd>` inline form, frontmatter
   `variables.<name>.command`, and config-level `variable_inputs.<name>.command`.
   The inline `<@...>` parser must handle nested `<#...>` refs without
   truncating the outer command at the inner `>`. Authors can write a literal
   `<#...>` by escaping the opener as `\<#...>`. The parsed form is a
   sequence of `Literal(String)` and `Ref { name, raw }` fragments.
2. **Substitution** — Before invoking `bash -c`, the runtime walks the
   fragments and produces a final command string by substituting each `Ref`:
   shell-single-quote-escape the confirmed value for default form; splice
   verbatim for `:raw`.
3. **Caching + re-render** — Suggestions for variable `V` depend on the set
   of upstream names referenced by `V`'s command. Dependent commands re-fetch
   only when the confirmed values of *those* names have changed since the
   last successful fetch. Non-dependent commands keep today's re-fetch-on-
   revisit behavior.

### Critical Files

- `src/parser.rs` — currently parses the snippet body's `<@name:source>`
  placeholders. Extend the parser used for the *source* portion of a
  suggestion command (or a new parser at the substitution layer) to
  recognize `<#name>` / `<#name:raw>` tokens and return a fragment vec plus
  referenced-name set. The same helper should be reused by runtime rendering,
  prompt cache-key construction, and lint. Frontmatter
  `variables.<name>.command` strings also need parsing.
- `src/execute/app.rs` — `SuggestionProvider` trait. `suggestions(...)` gains
  a `confirmed: &BTreeMap<String, String>` argument. `SystemSuggestionProvider`
  substitutes before calling `command_suggestions`.
- `src/execute/prompt.rs` — `PromptState`, `load_prompt_state`,
  `cycle_prompt_variable`, `command_suggestions`. Adds the suggestion cache,
  dirty-descendant tracking, and the input-buffer preservation rule.
  `command_suggestions` itself does *not* learn about dependence — it still
  takes a fully-rendered command string.
- `src/execute/tests.rs` — `TestProvider` impl signature changes; add coverage
  for the dependent flow, re-fetch-on-change, raw vs. quoted substitution,
  cache hits on revisit-without-change.
- `src/lint.rs` — replace any planned substring heuristic with AST-driven
  dependence detection. Add `dependent-suggestion-skipped` lint code. Add
  validations specific to `<#var>`: undefined ref, forward ref, self-ref.
- `src/lsp.rs` — diagnostics already flow from lint on open/change. Extend
  placeholder helpers so `<#name>` / `<#name:raw>` references inside command
  sources support go-to-definition, references, hover, and diagnostics ranges
  that point at the invalid dependent reference itself.
- `src/config.rs` — config-level `[variables.<name>]` / variable input command
  overrides flow through `VariableInputConfig`; ensure these commands use the
  same template parsing/rendering path as inline and frontmatter commands.
- `docs/SNIPPET_SYNTAX.md` — document `<#name>` and `<#name:raw>`, the
  quoting contract, ordering rule, escape syntax, and lint codes.

### Gotchas

- **Order is prompt variable order**, not display order. A `<#name>` may only
  reference variables that appear earlier in the snippet's deduplicated
  prompt order (the same order the TUI tabs through). Frontmatter/config
  specs can override or supplement variables, but they do not create a
  separate dependency order. Forward refs and self-refs are lint errors.
- **Confirmed vs. typed value.** Only values the user has *confirmed* (moved
  past with Enter/Tab) feed substitution. In-flight input buffer is not
  visible to downstream commands. If an upstream value changes, dependent
  descendant values become dirty: their text is preserved for editing, but
  they are not treated as confirmed for later substitutions until the user
  revisits and confirms them again.
- **Shell-quote = single-quote-escape.** The default `<#name>` substitution
  wraps the value in `'...'` and escapes embedded `'` as `'\''`. This is
  POSIX-safe and matches what `printf '%q'` would produce for most values
  on bash. `:raw` substitutes the literal value verbatim — no quoting.
- **Literal dependent-ref syntax is escaped with backslash.** `\<#name>` in a
  suggestion command renders as literal `<#name>` and does not count as a
  dependency. The parser consumes only the escape backslash for this syntax.
- **`load_prompt_state` always re-evaluates dependence**, but the cache makes
  re-fetch a no-op when the upstream snapshot is unchanged. Don't try to
  optimize by skipping `load_prompt_state` on revisit — the input-buffer
  preservation rule lives there.
- **Suggestion caching today is per-cycle.** `load_prompt_state` re-fetches
  every revisit. The new cache is used only for dependent commands and is
  keyed on `(variable name, BTreeMap<upstream-name → confirmed-value>)`,
  where the upstream-name set is the parsed `<#...>` references in the
  variable's command. Non-dependent commands bypass the new cache and keep
  today's re-fetch-on-revisit behavior.
- **Input-buffer preservation rule.** On entering a variable via
  `load_prompt_state`: if the variable name has a confirmed or dirty entry in
  prompt state, restore that text into the input buffer; otherwise use
  `default_input`. "Confirmed empty" counts as confirmed. On a dependent
  cache hit, preserve transient suggestion state (highlighted row, scroll,
  fuzzy filter). On a cache miss, reset transient suggestion state because
  suggestions may differ; the input buffer still survives.
- **TestProvider must update in lockstep** with the trait signature change
  or the build breaks.
- **LSP navigation is narrower than lint.** Lint must validate all command
  sources (inline, frontmatter, config). LSP go-to/hover/references can only
  navigate source text that exists in the open Markdown document: inline
  command refs and frontmatter command refs. Config-level command refs still
  get lint diagnostics where applicable, but do not have an in-document target
  span unless the config file itself gets LSP support later.

### Pseudo-code / Sketches

```text
// Parsed form of a suggestion command source
enum Fragment {
    Literal(String),
    Ref { name: String, raw: bool },
}
type CommandTemplate = Vec<Fragment>;

fn parse_command_template(src: &str) -> Result<CommandTemplate, ParseError> { ... }
fn referenced_names(template: &CommandTemplate) -> BTreeSet<String> { ... }

// Substitution before exec
fn render(
    template: &CommandTemplate,
    confirmed: &BTreeMap<String, String>,
) -> Result<String, RenderError> {
    template.iter().map(|f| match f {
        Fragment::Literal(s) => Ok(s.clone()),
        Fragment::Ref { name, raw: false } => confirmed
            .get(name)
            .map(|value| shell_single_quote(value))
            .ok_or_else(|| RenderError::MissingConfirmed(name.clone())),
        Fragment::Ref { name, raw: true } => confirmed
            .get(name)
            .cloned()
            .ok_or_else(|| RenderError::MissingConfirmed(name.clone())),
    }).collect()
}

fn shell_single_quote(v: &str) -> String {
    // 'value' with embedded ' escaped as '\''
    format!("'{}'", v.replace('\'', r"'\''"))
}

// SuggestionProvider
fn suggestions(
    &self,
    variable: &Variable,
    cwd: &Path,
    local_variables: &BTreeMap<String, VariableSpec>,
    confirmed: &BTreeMap<String, String>,  // NEW
) -> io::Result<Vec<String>>;

// Cache key on PromptState
struct CacheKey {
    variable: String,
    // Only upstream names that appear in the parsed template (sorted for
    // stable hashing). Non-dependent commands do not use this cache.
    upstream_snapshot: BTreeMap<String, String>,
}

// Tab-back path
//   cycle_prompt_variable re-calls load_prompt_state with new `confirmed`.
//   load_prompt_state:
//     1. If the parsed template has refs, compute cache key from current
//        confirmed snapshot; otherwise bypass cache and preserve old behavior.
//     2. Dependent cache hit → reuse suggestions, no bash spawn, preserve
//        transient suggestion state.
//     3. Dependent cache miss → render template, run command_suggestions,
//        store successful suggestions only. Parse/render failures become
//        prompt errors and are not cached.
//     4. Restore input buffer from confirmed/dirty prompt text if present,
//        else default_input. Reset transient suggestion state only on miss.
```

## Deliverables

### Deliverable 1. Parser + substitution mechanics

Add `<#name>` and `<#name:raw>` to the suggestion-command grammar; thread
`confirmed` through the `SuggestionProvider` trait; substitute before
spawning `bash -c`. This is the minimum that lets a snippet author write a
dependent command and have it work; no UX changes yet beyond "the second
variable can see the first."

- [x] Define `Fragment` enum and `CommandTemplate` type alias. Implement
      `parse_command_template(&str) -> Result<CommandTemplate, ParseError>`
      and `referenced_names(&CommandTemplate) -> BTreeSet<String>` in
      `src/parser.rs` (or a new module if it stays local to the substitution
      path). Reuse these helpers for runtime rendering, prompt cache keys,
      and lint.
- [x] Implement `shell_single_quote(&str) -> String` (single-quote wrap with
      `'` escaped as `'\''`). Unit test against values containing `'`, spaces,
      `$`, `;`, backslashes, newlines, empty string.
- [x] Implement
      `render(&CommandTemplate, &BTreeMap<String, String>) -> Result<String, RenderError>`
      with recoverable errors for missing confirmed values.
- [x] Add `confirmed: &BTreeMap<String, String>` param to
      `SuggestionProvider::suggestions` in `src/execute/app.rs`.
- [x] In `SystemSuggestionProvider::suggestions`, parse the command source
      into a template (cache parse result if cheap) and call `render` before
      passing to `command_suggestions`.
- [x] Apply template-driven parsing to all three sources of suggestion
      commands: inline `<@name:cmd>`, frontmatter `variables.<name>.command`,
      config `variable_inputs.<name>.command`. For inline commands, update
      the outer `<@...>` parser so nested `<#...>` refs do not terminate the
      placeholder early.
- [x] Update `TestProvider` in `src/execute/tests.rs` for the new signature.
- [x] Update `load_prompt_state` / `cycle_prompt_variable` call sites to pass
      only currently confirmed, non-dirty values into rendering.
- [x] Unit test: template with `<#bucket>` renders correctly with confirmed
      `bucket=foo` (becomes `'foo'`); with `bucket=O'Brien's` (becomes
      `'O'\''Brien'\''s'`).
- [x] Unit test: template with `<#verb:raw>` renders verbatim — confirmed
      `verb=get pods` becomes `get pods` (no quotes).
- [x] Unit test: escaped literal `\<#bucket>` renders as `<#bucket>` and does
      not appear in `referenced_names`.
- [x] Integration test: snippet with `<@bucket:...>` then
      `<@key:aws s3 ls s3://<#bucket>/...>` resolves with the chosen bucket
      visible to the second command (using `TestProvider`), proving the
      inline parser keeps the full outer command source despite the nested
      `<#bucket>` delimiter.
- [x] Integration test: command-as-variable pattern using `:raw`. e.g.
      `<@verb:?get pods>` then `<@target:kubectl <#verb:raw> -o name>`
      executes as a single multi-word kubectl invocation.
- [x] Regression test (characterization): snippets with a single free-form
      variable and snippets with two independent (non-dependent) variables
      behave identically pre- and post-change — including that
      `default_input` is restored on first entry to a never-confirmed
      variable and independent command suggestions still re-fetch on revisit.
      Guards the "no change for snippets that don't use the feature" promise
      against `load_prompt_state` reshuffling.

### Deliverable 2. Tab-back UX: preserve typed values, cache by upstream snapshot

When the user Tabs back and changes an upstream value, dependent descendants
become dirty: their text is preserved for editing, but later variables cannot
consume those descendant values until the user reconfirms them. Returning to a
downstream variable must re-run its suggestion command (with the new value
substituted) while keeping the preserved text in the input buffer. Conversely,
if the user Tabs back without changing the upstream, the downstream command
must *not* re-spawn — cache dependent suggestions keyed on the relevant
upstream snapshot.

- [x] In `load_prompt_state`, apply the input-buffer rule from Plan Details
      (preserve confirmed or dirty prompt text if present — including
      "confirmed empty"; else `default_input`). Preserve transient suggestion
      state on cache hit; reset highlighted row, scroll, and fuzzy filter on
      cache miss.
- [x] Add a per-`PromptState` suggestion cache for dependent commands keyed
      on `(variable name, BTreeMap<upstream-name → confirmed-value>)` where
      the upstream-name set is the parsed `<#...>` references in the
      variable's command. Non-dependent commands bypass this cache.
- [x] `load_prompt_state` consults the cache before calling
      `provider.suggestions(...)` for dependent commands. On miss, populates
      the cache with successful results only; failed commands are retryable on
      revisit so transient `kubectl`/network errors do not trap the user.
- [x] Test: fill `bucket=A`, fill `key=k1`, Tab back to `bucket`, change to
      `B`, Tab forward — `key` is dirty, `key`'s suggestions reflect bucket
      `B`, and the typed `k1` is still in the input buffer.
- [x] Test: Tab back to `bucket` without changing, Tab forward — `key`'s
      suggestion command was NOT re-spawned (assert via `TestProvider` call
      counter). Input buffer and transient suggestion state are unchanged.
- [x] Test: "confirmed empty" upstream value persists across revisit (input
      buffer comes back empty, not reset to `default_input`).
- [x] Test: a later variable that references dirty `key` cannot render until
      `key` is revisited and reconfirmed; the TUI shows a recoverable prompt
      error rather than using stale data.
- [x] Test: failed dependent suggestion commands are not cached; revisiting
      the variable retries the provider command.

### Deliverable 3. Lint: skip and validate dependent commands

`pb lint` runs suggestion commands to validate them. Commands that contain
`<#...>` references can't be executed faithfully without user input, so skip
them at lint time. Also surface structural problems with `<#...>` usage
(undefined name, forward reference, self-reference).

- [x] Detect dependence by checking whether the shared parsed command
      template contains any `Fragment::Ref`. No heuristic, no substring
      matching, and escaped `\<#...>` literals do not count.
- [x] If dependent, skip execution and emit `dependent-suggestion-skipped`
      at info/warn level.
- [x] Verify the existing `[lint.<code>] ignore_command = "..."` config
      pattern works for the new code; if it doesn't, surface that as an
      Issues entry rather than silently adding plumbing.
- [x] Emit `unknown-variable-reference` when `<#foo>` references a name not
      declared in the snippet's placeholder set or frontmatter.
- [x] Emit `forward-variable-reference` when `<#foo>` references a name
      declared *after* the current variable in declaration order.
- [x] Emit `self-variable-reference` when a variable's own command references
      itself.
- [x] Test: dependent suggestion command is not executed during lint and the
      new code appears in output.
- [x] Test: non-dependent command continues to execute as before.
- [x] Test: each of unknown / forward / self ref produces the expected lint
      code.
- [x] Test: escaped literal `\<#not_a_ref>` is not reported as an unknown
      variable reference and does not cause command execution to be skipped.

### Deliverable 4. Documentation

Capture the new behavior in `docs/SNIPPET_SYNTAX.md` so authors can rely on
it. Cover both substitution forms, quoting semantics, declaration-order
rules, lint codes, and the latency caveat.

- [x] Add a "Dependent Variables" subsection under "Variable Placeholders"
      in `docs/SNIPPET_SYNTAX.md` with the k8s-secret end-to-end example.
- [x] Document `<#name>` (shell-quoted) vs `<#name:raw>` (literal splice)
      with worked examples — including the command-as-variable pattern that
      motivates `:raw` — and document `\<#name>` as the literal escape form.
- [x] Document prompt variable order = dependency order; frontmatter/config
      specs do not create a separate order, and forward/self-refs are lint
      errors.
- [x] Document that only confirmed upstream values are substituted; in-flight
      input is not visible, and descendants dirtied by an upstream change must
      be reconfirmed before later variables can use them.
- [x] Document the lint codes: `dependent-suggestion-skipped`,
      `unknown-variable-reference`, `forward-variable-reference`,
      `self-variable-reference`.
- [x] Document the latency / timeout-compounding caveat: each dependent step
      is bounded by the per-command timeout (default 2000ms); recommend
      keeping dependent commands cheap.
- [x] Document the `:raw` security note: values substituted via `:raw` are
      not quoted, so authors should restrict `:raw` consumption to variables
      whose upstream values come from a trusted source (suggestion lists they
      control), not free-form user input.
- [x] Failure UX (no new code): a dependent suggestion command that exits
      non-zero shows the error in the prompt status line (existing behavior)
      and lets the user type a value or Shift+Tab back to fix the upstream.
      Failed dependent commands are not cached, so revisiting retries them.

### Deliverable 5. LSP support for dependent references

Extend `peanutbutter lsp` so dependent references behave like first-class
symbols in snippet files. Diagnostics for invalid `<#...>` references should
come from lint, while editor navigation should let users jump from a
`<#name>` / `<#name:raw>` inside a suggestion command to the corresponding
frontmatter variable definition when one exists, and find usages across both
`<@name>` placeholders and `<#name>` dependent references.

- [x] Ensure lint findings for `unknown-variable-reference`,
      `forward-variable-reference`, and `self-variable-reference` include
      precise ranges on the offending `<#...>` reference so LSP diagnostics
      underline the dependent ref, not the whole command or snippet.
- [x] Extend LSP token detection beyond `placeholder_at` so cursor positions
      inside `<#name>` and `<#name:raw>` references are recognized in inline
      command sources and frontmatter `command:` strings.
- [x] Go-to-definition: from `<#name>` / `<#name:raw>`, jump to
      `variables.<name>` in frontmatter when present; if the variable is
      inline-only with no frontmatter declaration, return no definition rather
      than jumping to an arbitrary `<@name>` use.
- [x] References: from a frontmatter variable declaration or a `<#name>` ref,
      include both `<@name>` prompt placeholders and `<#name>` / `<#name:raw>`
      dependent refs in the Markdown document.
- [x] Hover: on `<#name>` / `<#name:raw>`, show the same variable spec hover
      used for `<@name>`, plus whether the ref is shell-quoted or raw.
- [x] Completion: when typing `<#` inside a suggestion-command source,
      suggest only earlier prompt variables, matching the forward-reference
      rule. Do not offer dependent-ref completions outside command sources.
- [x] Tests in `src/lsp.rs`: diagnostics ranges for invalid dependent refs;
      go-to-definition from `<#...>`; references include `<@...>` and
      `<#...>` uses; hover distinguishes quoted vs. raw; completion excludes
      later variables.

## Issues

- **2026-05-17 — agent:claude (implementation complete)** — All 5
  deliverables landed in one pass. Net additions: new
  `src/command_template.rs` module; `<#name>` / `<#name:raw>` token
  parsing, shell-quoting, and render path; `SuggestionProvider` trait
  gained `confirmed` and `command_source`; nested `<#...>` parsing
  inside `<@...>` placeholders; `PromptState` grew `dirty:
  BTreeSet<String>` and `suggestion_cache: HashMap<...>` (forced boxing
  of `PromptState` in `Screen::Prompt` to satisfy
  `clippy::large_enum_variant`); five new lint codes wired into both
  `run` and `lint_file`; LSP `dependent_ref_at` token detection plus
  hover/goto/references/completion for `<#...>`; precise `(col_start,
  col_end)` spans on `LintFinding` for invalid dependent refs; new
  "Dependent Variables" doc section. 325 lib tests pass; existing
  `[lint.<code>] ignore_command` suppression plumbing works for
  `dependent-suggestion-skipped` (covered by a new test). Pre-existing
  `terminal.rs` clippy warnings unchanged.
- **2026-05-17 — agent:claude (scope update)** — Added LSP coverage as
  Deliverable 5. Lint already covered invalid dependent-reference validation,
  but the plan was missing editor behavior: precise diagnostic ranges,
  go-to-definition, references, hover, and completions for `<#...>` refs.
- **2026-05-17 — agent:claude (adversarial review)** — Plan reviewed by 2
  adversarial sub-agents (Risks & Assumptions, Completeness & Scope). 11
  findings; 11 merged into plan. Most significant changes: dependent-command
  caching no longer changes non-dependent snippet behavior, upstream changes
  dirty descendants until reconfirmed, and inline nested `<#...>` parsing plus
  `\<#...>` literal escaping are now explicit.
- **2026-05-17 — agent:claude (design pivot)** — Plan rewritten around
  explicit `<#var>` / `<#var:raw>` reference syntax (replacing the original
  env-injection approach). The pivot removes whole categories of risk:
  env-name shadowing, dash-in-name handling, the `$name` substring
  dependence heuristic, and incidental leakage of unrelated confirmed
  values into every suggestion sub-shell. Dependence is now visible in the
  snippet text, in the parsed AST, in the cache key, and in lint — one
  source of truth. Deliverable count dropped from 5 to 4 and the lint
  surface simplified.
- **2026-05-17 — agent:claude (adversarial review)** — Plan reviewed by 2
  adversarial sub-agents (Risks & Assumptions, Completeness & Scope) on the
  prior env-injection draft. Findings that still apply have been carried
  forward: upstream-snapshot cache for tab-back latency, regression tests for
  the non-dependent `load_prompt_state` path, precise input-buffer
  preservation rule. Findings made moot by the `<#var>` redesign:
  env-shadow lint, dash-in-name handling, dependence-heuristic fragility,
  env leakage into descendants.
- **2026-05-17 — agent:claude (open)** — Verify the existing
  `[lint.<code>] ignore_command = "..."` config plumbing actually exists
  before relying on it from Deliverable 3. If it doesn't, that adds
  unplanned work; convert into its own deliverable or sub-task.
- **2026-05-17 — agent:claude (watch-for)** — From Risks, carry into
  implementation review:
  (1) cache invalidation bugs (stale suggestions winning after upstream
  change),
  (2) transient prompt state (highlighted row, scroll, fuzzy filter) being
  unexpectedly reset on revisit,
  (3) `:raw` usage against user-typeable upstream variables (security
  smell — may motivate a future lint).
- **2026-05-17 — agent:claude** — Initial plan drafted from issue #8 + its
  comments + targeted pre-draft questions. Skipped full `grill-me` because
  the acceptance criteria are explicit and the comments narrowed the open
  questions to a small, enumerable set that was resolved inline.
