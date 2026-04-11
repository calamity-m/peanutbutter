# Part 01 - Core Foundations and Snippet Parsing

Status: done
Parent: [`README.md`](README.md)
Context: [`PLAN_CONTEXT.md`](../../PLAN_CONTEXT.md)

This part file is a living working document. Keep the checklist, progress log, discoveries, validation, and handoff current while the work is active.

## Outcome

The crate has a durable internal foundation for the rest of the project: module boundaries are defined, snippet roots can be resolved from config, markdown files can be discovered recursively, and each valid snippet can be parsed into structured data with stable identifiers and tests.

## Scope

- In scope: choose the initial module layout inside `src/`; define core types for snippets, metadata, variables, and snippet identifiers; decide the first-pass config and path policy; implement recursive markdown discovery under configured snippet roots; parse optional frontmatter, `##` headings, descriptions, and the first fenced code block; add parser and discovery tests using `examples/`.
- Out of scope: frecency scoring, fuzzy matching, tree navigation, interactive terminal UI, and shell integration.

## Dependencies

- Needs: `README.md`, `AGENTS.md`, and the existing example snippet files.
- Unblocks: Part 02 can build ranking and navigation on top of structured snippet data, and Parts 03-04 can rely on stable snippet and variable models.

## Checklist

- [x] Replace `src/main.rs`-only layout with a small internal module tree that leaves room for parser, ranking, TUI, CLI, and persistence code.
- [x] Define core domain structs for snippet files, snippets, snippet IDs, parsed variables, and frontmatter metadata.
- [x] Decide and document the v1 policy for config, snippet-root, and state file locations.
- [x] Implement filesystem discovery for markdown files under configured snippet roots.
- [x] Implement a line-oriented parser that handles optional YAML frontmatter, `##` snippet headings, markdown description text, and first fenced code blocks.
- [x] Add parser fixtures and tests for every file under `examples/`, including malformed or out-of-spec behavior.
- [x] Decide whether out-of-spec examples should be fixed in-repo or deliberately ignored by the parser in v1.

## Progress Log

- [x] 2026-04-11: Drafted the implementation slice and captured the current parser-related repo discoveries.
- [x] 2026-04-11: Fixed `examples/nested/root.md` to use a `##` snippet heading so the example set matches the README parser contract.
- [x] 2026-04-11: Introduced the `lib.rs` + module split (`config`, `discovery`, `domain`, `parser`) and moved the binary entry to a thin `main.rs` that reports resolved paths.
- [x] 2026-04-11: Implemented zero-dep markdown discovery, a line-oriented snippet parser (frontmatter subset + state machine + variable extraction), and 17 passing tests covering every file under `examples/`. Validated with `cargo test`, `cargo clippy --all-targets -- -D warnings`, and `cargo fmt --check`.

## Discoveries

- Observation: The codebase is still at a `Hello, world!` starting point, so there is no legacy architecture that the parser or config model must preserve.
  Evidence: `src/main.rs`
- Observation: The earlier heading mismatch in `examples/nested/root.md` has been resolved, so the example corpus now matches the README rule that snippets are defined by `##` headings.
  Evidence: `examples/nested/root.md` and the `README.md` snippet specification section
- Decision: No external crates in v1 — frontmatter uses a hand-rolled YAML subset that handles the fields the current examples need (`name`, `description`, `tags` as inline or block list). Cost: fragile if frontmatter grows. Benefit: zero deps, full control over error shape, fast iteration. Revisit if Part 02+ needs richer metadata.
  Evidence: `src/parser.rs::parse_yaml_subset`
- Decision: Sections without a fenced code block are silently dropped rather than errored. This matches the README's `##`-plus-fence rule and lets description-only sections coexist with real snippets in the same file.
  Evidence: `src/parser.rs::parse_snippets` (State::InSection path on re-entering a heading or EOF)
- Decision: The parser state machine matches the fence it opened with so e.g. `````` inside a `` ``` `` block would survive — today the README only shows triple backticks, but we keep length-matching because it is the standard CommonMark rule and costs nothing.
  Evidence: `src/parser.rs::parse_fence_open` and `is_fence_close`
- Decision: `SnippetId` is a `(relative_path, heading_slug)` pair serialised as `"path/to/file.md#slug"`. Duplicate slugs within a single file are disambiguated with `-1`, `-2`, ... so identity survives reorderings inside a file. This ID is the key the Part 02 frecency store will use.
  Evidence: `src/domain.rs::SnippetId` and `src/parser.rs::build_snippet`
- Decision: v1 path policy — snippet roots default to `$XDG_CONFIG_HOME/peanutbutter/snippets` (falling back to `$HOME/.config/...`) and may be overridden colon-separated via `PB_SNIPPET_ROOTS`. State file defaults to `$XDG_STATE_HOME/peanutbutter/state.json` and may be overridden via `PB_STATE_FILE`. The config is pure path resolution for now; no on-disk config format is committed yet.
  Evidence: `src/config.rs`
- Decision: All current `examples/` files parse cleanly under the current rules, so no in-repo example fixes or deliberate exclusions are needed in v1. The integration test `tests/examples.rs::every_example_file_produces_at_least_one_snippet` guards against regressions.
  Evidence: `tests/examples.rs`
- Decision: `PEANUTBUTTER_PATH` extras take priority over the XDG default — resolution order is `[...extras, xdg_default]`. When the same snippet heading appears in multiple roots, the leftmost (extra) root wins. This means a repo-local `snips/` directory can shadow a personal snippet with the same name. Part 02's index loader must walk roots left-to-right and respect this ordering when handling duplicates.
  Evidence: `src/config.rs::resolve_snippet_roots`

## Validation

- Command or check: `cargo test`
  Result: 17 passed (11 unit in `discovery`/`parser`, 6 integration in `tests/examples.rs`), 2026-04-11.
- Command or check: parser and discovery tests cover every current file in `examples/`.
  Result: Covered by `tests/examples.rs::every_example_file_produces_at_least_one_snippet` (walks the discovered set) plus per-file assertions for `simple/`, `complex/`, and every `nested/` file.
- Command or check: `cargo clippy --all-targets -- -D warnings`
  Result: Clean, 2026-04-11.
- Command or check: `cargo fmt --check`
  Result: Clean, 2026-04-11.
- Command or check: manual review confirms invalid sections are either ignored or reported consistently.
  Result: `parse_snippets` silently drops heading-only sections; asserted by `parser::tests::sections_without_code_are_discarded`.

## Next Handoff

Part 02 (Ranking, Search, and Tree Navigation) can now build on top of `SnippetFile`/`Snippet`/`SnippetId` directly. Concrete starting points for Part 02:
- Introduce an in-memory index type that owns `Vec<SnippetFile>` loaded from `discover_markdown_files` + `parse_file`, keyed by `SnippetId`.
- Decide the cwd-aware frecency store shape and where it serialises (the `state_file` path from `config::default_paths()` is already resolved).
- The fuzzy matcher will need to score across `Snippet::name`, `Snippet::body`, and the parent `SnippetFile::frontmatter` — all three are already structured data, no re-parsing required.
