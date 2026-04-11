# Part 01 - Core Foundations and Snippet Parsing

Status: planned
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

- [ ] Replace `src/main.rs`-only layout with a small internal module tree that leaves room for parser, ranking, TUI, CLI, and persistence code.
- [ ] Define core domain structs for snippet files, snippets, snippet IDs, parsed variables, and frontmatter metadata.
- [ ] Decide and document the v1 policy for config, snippet-root, and state file locations.
- [ ] Implement filesystem discovery for markdown files under configured snippet roots.
- [ ] Implement a line-oriented parser that handles optional YAML frontmatter, `##` snippet headings, markdown description text, and first fenced code blocks.
- [ ] Add parser fixtures and tests for every file under `examples/`, including malformed or out-of-spec behavior.
- [ ] Decide whether out-of-spec examples should be fixed in-repo or deliberately ignored by the parser in v1.

## Progress Log

- [x] 2026-04-11: Drafted the implementation slice and captured the current parser-related repo discoveries.
- [x] 2026-04-11: Fixed `examples/nested/root.md` to use a `##` snippet heading so the example set matches the README parser contract.

## Discoveries

- Observation: The codebase is still at a `Hello, world!` starting point, so there is no legacy architecture that the parser or config model must preserve.
  Evidence: `src/main.rs`
- Observation: The earlier heading mismatch in `examples/nested/root.md` has been resolved, so the example corpus now matches the README rule that snippets are defined by `##` headings.
  Evidence: `examples/nested/root.md` and the `README.md` snippet specification section

## Validation

- Command or check: `cargo test parser`
  Result: Pending until implementation.
- Command or check: parser and discovery tests cover every current file in `examples/`.
  Result: Pending until implementation.
- Command or check: manual review confirms invalid sections are either ignored or reported consistently.
  Result: Pending until implementation.

## Next Handoff

Implement snippet discovery and parsing first, then decide whether the out-of-spec nested example should be fixed or intentionally excluded from v1 behavior.
