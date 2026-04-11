# Part 03 - Inline TUI Execution and Variable Resolution

Status: planned
Parent: [`README.md`](README.md)
Context: [`PLAN_CONTEXT.md`](../../PLAN_CONTEXT.md)

This part file is a living working document. Keep the checklist, progress log, discoveries, validation, and handoff current while the work is active.

## Outcome

`pb execute` can drive an inline terminal flow that shows ranked snippets, lets users browse or search, previews snippet context, collects variable values, and renders the final command text without executing it.

## Scope

- In scope: choose the terminal stack; implement the event loop and state machine for list, browse, preview, and variable-entry states; render the current query and result set; support backtracking without dropping user context; parse and resolve snippet variables; implement built-in `file` and `directory` providers plus command-backed suggestion sources; render the final command string for stdout.
- Out of scope: shell hotkey generation, snippet creation and deletion commands, and support for shells beyond the first targeted integration.

## Dependencies

- Needs: Part 01 for parsed snippets and variable tokens, and Part 02 for ranked and browsable result sets.
- Unblocks: Part 04 can wrap the execute flow with CLI wiring and shell integration once the interactive behavior is stable.

## Checklist

- [ ] Pick the v1 terminal libraries and define a TUI state machine that does not leak shell-specific behavior into the UI layer.
- [ ] Render the current query, ranked results, browse path, and snippet preview in an inline terminal view.
- [ ] Support switching between fuzzy and tree navigation without losing the user's narrowed state.
- [ ] Implement variable prompting for free-form values, defaults, built-in file and directory sources, and command-backed suggestion lists.
- [ ] Render the completed command string exactly as shell-ready text and exit without running it.
- [ ] Add automated tests for variable parsing and command rendering, plus a manual smoke script for terminal interaction.

## Progress Log

- [x] 2026-04-11: Defined the interactive execution slice around the README requirement that snippet selection writes into the shell buffer instead of executing directly.

## Discoveries

- Observation: Variable syntax includes plain placeholders, defaulted values, built-in providers, and external command-backed suggestion sources, so the prompt flow needs more than a single text input.
  Evidence: `README.md`, `AGENTS.md`, and `examples/simple/snippets.md`
- Observation: The final action is command emission, not command execution, which should keep the TUI independent from shell semantics.
  Evidence: `README.md` CLI design section and `AGENTS.md` shell buffer integration description

## Validation

- Command or check: `cargo test variables`
  Result: Pending until implementation.
- Command or check: unit tests for placeholder parsing and rendered command output.
  Result: Pending until implementation.
- Command or check: manual smoke check of `pb execute` in a real terminal against `examples/`.
  Result: Pending until implementation.

## Next Handoff

Do not wire shell integration until the pure `pb execute` experience is stable and can render the correct command text on its own.
