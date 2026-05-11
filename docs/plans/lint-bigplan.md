# BIGPLAN: Lint command

## Plan Overview

Add a read-only `pb lint` command that checks configured snippet roots for common authoring problems before they surprise users in the picker. The command should be useful both interactively and from scripts: pretty human output by default, JSON output behind a flag, and predictable exit codes. Done means `pb lint` can report the requested normal checks, optionally include stricter style checks via `--strict`, run frecency GC logic in dry-run mode only, and has tests covering both output modes and exit behavior.

## Risks

- **Suggestion commands have side effects** — Lint will execute suggestion commands to prove they work, which can be surprising if snippet collections contain commands with side effects. Mitigate by documenting that lint executes suggestions in the same non-login `bash -c` style as runtime, applying the existing timeout, and leaving mutating remediation out of scope.
- **Parser currently discards malformed detail** — Existing parsing intentionally ignores malformed frontmatter and malformed placeholders, but lint needs diagnostics with file/line context. Mitigate by adding lint-specific validation paths that scan source text for findings instead of trying to infer every issue from the lossy runtime `SnippetFile` model.
- **Markdown validation can expand scope quickly** — Strict-mode “validate markdown” could become a full CommonMark conformance project. Mitigate by defining this as parser-relevant markdown validation for snippet files: fence balance, snippet-section structure, and any existing markdown parser capability if a small dependency is chosen deliberately.

## Plan Details

`pb lint` is a non-interactive command over resolved snippet roots. It should not launch the TUI, write selected commands to stdout, or mutate the frecency store. Pretty output goes to stdout; operational errors go to stderr through existing `main.rs` error handling. Exit code semantics: `0` for no findings, `1` for lint findings, `2` for operational failures (config load failure, unreadable snippet root). When an individual snippet file is unreadable, emit a warning finding and continue rather than exiting `2`; reserve `2` for failures that prevent lint from running at all.

### Finding model

A minimal lint model should carry enough data for both pretty and JSON output without duplicating formatting logic:

```text
LintFinding {
  severity: "error" | "warning",
  code: stable machine-readable string (e.g. "lint/broken-frontmatter"),
  path: snippet source path or state file path,
  line: optional 1-based line number,
  snippet_id: optional id when known,
  message: short user-facing description,
  detail: optional longer context,
}
```

Canonical codes for all checks (define these as constants in `src/lint.rs`):

| Code | Severity | Mode |
|---|---|---|
| `lint/broken-frontmatter` | error | normal |
| `lint/undeclared-variable` | warning | strict |
| `lint/unused-variable` | warning | normal |
| `lint/duplicate-slug` | warning | normal |
| `lint/suggestion-command-failed` | error | normal |
| `lint/suggestion-command-timeout` | error | normal |
| `lint/suggestion-commands-disabled` | warning | normal |
| `lint/gc-orphan-reattachable` | warning | normal |
| `lint/gc-orphan-unresolvable` | warning | normal |
| `lint/static-inline-command` | warning | normal |
| `lint/markdown-structure` | warning | strict |
| `lint/missing-code-language` | warning | strict |
| `lint/frontmatter-override` | warning | strict |

Normal-mode checks should produce findings for broken frontmatter, unused frontmatter/config variables, duplicate slugs, failing suggestion commands, dry-run GC orphans, and inline command variables that should be frontmatter suggestions. Strict mode adds undeclared-variable warnings, markdown validation, missing language tags on snippet-defining code blocks, and frontmatter variables that override broader variable definitions in a confusing way.

### Check interpretation

- Broken frontmatter means a top-of-file `---` block is unterminated or cannot be parsed according to the supported frontmatter subset. Runtime may continue to ignore it, but lint should report it.
- Undeclared variables means a free-form placeholder such as `<@name>` has no inline source, no file-local frontmatter variable spec, no config-defined variable spec, and is not a built-in variable (`file`, `directory`). Inline defaults and inline commands are self-declared. This is a strict-mode warning only because bare manual placeholders are valid when human context is required.
- Unused variables means a file-local frontmatter variable spec is not referenced by any snippet in that file, or a config-defined variable is not referenced by any snippet across configured roots.
- Duplicate slugs means two `##` snippets in the same relative path slugify to the same base slug; runtime currently appends numeric suffixes, but lint should report the collision so authors can rename headings deliberately.
- Failing suggestion commands includes inline `<@name:command>` and frontmatter/config-backed `command = “...”` paths used by a snippet. Commands run with the snippet root as cwd (not the user's invocation cwd), under non-login `bash -c`, inheriting the parent `PATH`, with the existing timeout. When `allow_commands = false` is set in config, lint does not execute commands but emits a `lint/suggestion-commands-disabled` warning for each snippet that has command-backed variables so the user knows they were skipped.
- “Invokes gc” means lint calls extracted orphan detection logic from `gc.rs` (not `run_with` directly, which mixes output with discovery) and reports orphaned frecency events as findings. It must not prompt, reattach, purge, save, or write a backup. Use two codes matching GC's own distinction: `lint/gc-orphan-reattachable` for orphans where GC would find a candidate snippet to reattach to, and `lint/gc-orphan-unresolvable` for orphans with no candidate. Put the candidate snippet ID in the `detail` field for reattachable orphans.
- Inline command variables like `<@input:echo "a\nb\c">` should be reported when the command appears to be a static suggestion list better represented as a frontmatter `variables.<name>.suggestions` declaration. Keep the heuristic simple: obvious `echo`/`printf` static lists only, not arbitrary shell analysis.
- Strict overriding checks should detect file-local frontmatter variables that reuse a name already configured globally and change suggestions or command behavior. The finding should suggest renaming when the local meaning is different; identical overlays or adding a default only can stay quiet.

### Critical Files

- `src/cli.rs` — add `Command::Lint`, CLI flags, command runner, and CLI-level tests.
- `src/main.rs` — dispatch `lint`, print output, and map lint findings to the agreed exit code.
- `src/parser.rs` — existing snippet/frontmatter parser; lint should reuse what is safe but add source-aware validation where runtime parsing is intentionally permissive.
- `src/domain.rs` — may need public lint structs if the lint module exposes typed results.
- `src/gc.rs` — extract or reuse dry-run orphan detection without mutating state or relying on human-formatted GC output.
- `src/execute/prompt.rs` and `src/execute/app.rs` — existing suggestion command behavior and timeout semantics to mirror for lint.
- `src/lib.rs` — declare a new `lint` module if implemented as `src/lint.rs`.
- `docs/SNIPPET_SYNTAX.md` and `README.md` — document `pb lint`, strict mode, JSON output, and the fact that suggestion commands execute during lint.
- `tests/` — integration coverage for CLI behavior and sample snippet roots.

### Gotchas

- Do not mix debug/status output into stdout for JSON mode; stdout must be parseable JSON when `--json` is set.
- `serde` is available with `derive`, but `serde_json` is not currently in `Cargo.toml`; JSON output will need a deliberate dependency addition or a small manual serializer. Prefer adding `serde_json` if implementing real JSON.
- Frontmatter parsing is currently custom and permissive, not general YAML. Lint should validate the supported subset unless the implementation intentionally introduces a real YAML parser.
- Runtime duplicate slug handling makes IDs unique by suffixing; lint’s duplicate-slug check should look at the base slug before suffixing.
- Suggestion commands run under non-login, non-interactive `bash -c` and inherit the parent `PATH`; lint should not source shell profiles either.
- `gc::run_with` mixes orphan detection with formatted output and only returns aggregate counts — it cannot be called directly to produce per-orphan findings. Extract a pure `gc::collect_orphans(paths) -> Vec<GcOrphan>` function (or equivalent) that returns typed orphan records without writing output. The `GcOrphan` record should carry the event's snippet path/ID and the best reattachment candidate (if any) so lint can emit the two `gc-orphan-*` codes correctly.
- `builtin_suggestions` (and the built-in variable name list) lives in `src/execute/prompt.rs` as `pub(crate)`. The lint module needs to know which variable names are built-ins (`file`, `directory`) to implement the undeclared-variable check. Either expose `is_builtin_variable(name: &str) -> bool` as `pub` from `execute/prompt.rs`, or move the constant set to `src/domain.rs`.

### Pseudo-code / Sketches

```text
run_lint(paths, options, writer):
  config = load app config
  findings = []

  for root in paths.snippet_roots:
    for markdown file under root:
      content = read file or return operational error
      parsed = parse_file(file, root, content)
      findings += lint_frontmatter_source(content)
      findings += lint_duplicate_base_slugs(content, relative_path)
      findings += lint_variables(parsed, config.variables)
      findings += lint_suggestion_commands(parsed, config, cwd, timeout)
      findings += lint_static_inline_suggestion_commands(parsed)
      if options.strict:
        findings += lint_markdown_structure(content)
        findings += lint_missing_code_languages(parsed/source ranges)
        findings += lint_frontmatter_overrides(parsed.frontmatter, config.variables)

  findings += lint_gc_dry_run(paths)

  if options.json:
    write JSON array/object to stdout
  else:
    write grouped pretty findings to stdout

  return has_findings(findings)
```

## Deliverables

### Deliverable 1. CLI contract and output model

This deliverable defines the user-facing command shape before implementing individual checks. It adds `pb lint` with `--strict` and `--json`, a shared `LintFinding` model, pretty rendering that lists findings with path and description, JSON rendering for scripts, and exit-code handling. Acceptance criteria: no findings exits `0`, findings exit `1`, operational failures (config load failure, unreadable snippet root) exit `2`, and JSON mode produces parseable machine output with stable codes.

- [x] Add a `Lint` variant to `src/cli.rs` with `--strict` and `--json` flags.
- [x] Add a `src/lint.rs` module with documented public option/result/finding types.
- [x] Implement pretty output grouped or sorted by path with code, severity, line when available, and description.
- [x] Implement JSON output with stable field names and no extra stdout text.
- [x] Dispatch `pb lint` from `src/main.rs` and map outcomes to exit codes `0`, `1`, and operational error handling.
- [x] Add CLI tests for argument parsing, output mode selection, and exit semantics where practical.

### Deliverable 2. Normal lint checks

This deliverable makes default `pb lint` useful for real snippet maintenance. It reports broken frontmatter, undeclared variables, duplicate base slugs, failing suggestion commands, dry-run GC findings, and static inline suggestion commands that should be frontmatter variables. The implementation should reuse existing parser, config, discovery, suggestion-command, and GC behavior where doing so stays simple, but add lint-specific source scanning where runtime parsing is too permissive.

- [x] Validate top-of-file frontmatter delimiters and supported fields well enough to report broken frontmatter with file and line context.
- [x] Report undeclared free-form variables that are not inline-sourced, file-local, config-defined, or built in.
- [x] Report duplicate base slugs within the same relative snippet file before runtime numeric suffixing.
- [x] Execute inline and resolved suggestion commands using the snippet root as cwd, with the existing timeout semantics; report failures/timeouts as findings. When `allow_commands = false`, emit `lint/suggestion-commands-disabled` per affected snippet instead of running commands.
- [x] Extract `gc::collect_orphans` (or equivalent) from `gc.rs` returning typed per-orphan records without writing output. Call from lint; emit `lint/gc-orphan-reattachable` or `lint/gc-orphan-unresolvable` per orphan with candidate ID in `detail` when applicable.
- [x] Detect obvious static inline command suggestions and suggest moving them into frontmatter `variables` suggestions.
- [x] Add unit tests for each normal check, including at least one multi-root duplicate-id or GC-related case. For suggestion-command checks, test with portable commands (`echo`, `true`, `false`) and a command that sleeps past the timeout to verify timeout behavior. Add a test verifying lint runs against the explicitly configured snippet root(s), not a hardcoded default.

### Deliverable 3. Strict-mode checks

This deliverable adds opt-in stricter authoring checks without making the default command noisy. `pb lint --strict` should include every normal check plus markdown/snippet-structure validation, missing language tags on code fences that define snippets, and confusing local variable overrides. Strict findings follow the same output and exit-code rules as normal findings, but only appear when `--strict` is set.

- [x] Define the concrete strict markdown validation scope in tests before implementation.
- [x] Report unbalanced fences or malformed snippet-defining sections that the runtime parser would otherwise ignore.
- [x] Report snippet-defining fenced code blocks with no language tag.
- [x] Report file-local frontmatter variables that override config-defined variables with a different suggestion source and suggest renaming.
- [x] Ensure strict-only findings are absent without `--strict` and present with `--strict`.

### Deliverable 4. Documentation and examples

This deliverable updates user-facing docs so snippet authors understand how to run lint and how to interpret its findings. It should document the command name, flags, exit behavior, output modes, strict mode, and the important safety caveat that suggestion commands are executed during lint while GC stays dry-run only.

- [x] Update `README.md` CLI documentation to include `peanutbutter lint [--strict] [--json]`.
- [x] Update `docs/SNIPPET_SYNTAX.md` to replace the future `pb check` wording with `pb lint` and describe the linted frontmatter/variable expectations.
- [x] Add or update examples showing a pretty finding and representative JSON output.
- [x] Run `cargo fmt --check`, `cargo test`, and `cargo clippy -- -D warnings -A dead_code` after implementation.

## Issues

- **2026-05-11 — agent:pi** — Adjusted variable linting after design review: undeclared variables are now strict warnings only, and normal lint reports unused frontmatter/config variable definitions.

- **2026-05-11 — agent:pi** — Implemented `pb lint` end-to-end: CLI flags/output model, normal and strict checks, GC dry-run orphan collection, docs, and verification with `cargo fmt --check`, `cargo test`, and `cargo clippy -- -D warnings -A dead_code`.

- **2026-05-11 — agent:claude (adversarial review, round 2)** — Plan reviewed by 2 adversarial sub-agents (Risks & Assumptions, Completeness & Scope). 11 findings; 10 merged into plan. Most significant changes: added canonical code registry to finding model, `gc::run_with` extraction made an explicit task, `allow_commands=false` behavior defined (skip+warn), two GC orphan codes to match GC's own distinction, suggestion-command cwd fixed to snippet root, exit code `2` committed as a hard spec, `builtin_suggestions` visibility gap called out as a Gotcha.

- **2026-05-11 — agent:claude (adversarial review)** — Plan reviewed by 2 adversarial perspectives (Risks & Assumptions, Completeness & Scope). 4 findings; 4 merged into plan. Most significant changes: clarified lint side effects from suggestion-command execution, narrowed strict markdown validation scope, and made exit/output behavior an explicit deliverable.
