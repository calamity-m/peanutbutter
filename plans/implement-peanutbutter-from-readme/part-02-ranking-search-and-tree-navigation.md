# Part 02 - Ranking, Search, and Tree Navigation

Status: planned
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

- [ ] Define the search index shape for snippet metadata, content, and relative path information.
- [ ] Implement fuzzy matching across snippet heading text, snippet body, frontmatter fields, and description text.
- [ ] Implement a browse-tree model that supports path narrowing, tab completion, and backspacing through directory levels.
- [ ] Preserve search or browse context so a user can back out of a snippet and continue from the same narrowed state.
- [ ] Design and implement a simple frecency store keyed by snippet ID and cwd context, with enough structure to weight nearby paths higher.
- [ ] Add tests that prove location weighting, recency influence, and high-frequency override behavior.
- [ ] Decide when usage events become durable, such as only after the user confirms emission of a command.

## Progress Log

- [x] 2026-04-11: Captured the ranking and navigation slice directly from the README user-experience narrative.

## Discoveries

- Observation: The README requires two distinct but connected navigation modes rather than a single fuzzy list with optional grouping.
  Evidence: `README.md` user-experience examples for fuzzy search and file-style navigation
- Observation: The README also expects search context to survive backing out of a chosen snippet so users can try the next result quickly.
  Evidence: `README.md` paragraph describing backspacing to return to the previous snippet list state

## Validation

- Command or check: `cargo test ranking`
  Result: Pending until implementation.
- Command or check: unit tests for cwd weighting, recency decay, and frequency override.
  Result: Pending until implementation.
- Command or check: unit or snapshot tests for fuzzy-mode and tree-mode transitions.
  Result: Pending until implementation.

## Next Handoff

Keep ranking and browse state as pure application logic so the TUI can consume it without reimplementing search rules.
