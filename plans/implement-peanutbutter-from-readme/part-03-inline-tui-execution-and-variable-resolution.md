# Part 03 - Inline TUI Execution and Variable Resolution

Status: done
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

- [x] Pick the v1 terminal libraries and define a TUI state machine that does not leak shell-specific behavior into the UI layer.
- [x] Render the current query, ranked results, browse path, and snippet preview in an inline terminal view.
- [x] Support switching between fuzzy and tree navigation without losing the user's narrowed state.
- [x] Implement variable prompting for free-form values, defaults, built-in file and directory sources, and command-backed suggestion lists.
- [x] Render the completed command string exactly as shell-ready text and exit without running it.
- [x] Add automated tests for variable parsing and command rendering, plus a manual smoke script for terminal interaction.

## Progress Log

- [x] 2026-04-11: Defined the interactive execution slice around the README requirement that snippet selection writes into the shell buffer instead of executing directly.
- [x] 2026-04-11: Implemented `src/execute.rs` with a pure selector/preview/prompt state machine, command rendering, built-in and command-backed suggestion providers, and a ratatui/crossterm terminal runner.
- [x] 2026-04-11: Added a minimal `pb execute` entrypoint in `src/main.rs`, execution-focused tests, and `scripts/smoke-part03-execute.sh` for manual terminal validation.

## Discoveries

- Observation: Variable syntax includes plain placeholders, defaulted values, built-in providers, and external command-backed suggestion sources, so the prompt flow needs more than a single text input.
  Evidence: `README.md`, `AGENTS.md`, and `examples/simple/snippets.md`
- Observation: The final action is command emission, not command execution, which should keep the TUI independent from shell semantics.
  Evidence: `README.md` CLI design section and `AGENTS.md` shell buffer integration description
- Observation: Ratatui's dedicated inline viewport path depends on cursor-position probing that was unreliable in PTY smoke testing, so the v1 runner uses the standard terminal viewport to keep `pb execute` stable.
  Evidence: 2026-04-11 PTY smoke run of `./scripts/smoke-part03-execute.sh`
- Observation: The README's command-backed variable example uses literal `\n` escapes inside `echo`, so suggestion parsing needs to split both real lines and escaped newline sequences to match the documented UX.
  Evidence: `README.md`, `examples/complex/complex.md`, and `execute::tests::command_suggestions_split_literal_backslash_n_sequences`

## Validation

- Command or check: `cargo test execute`
  Result: Passed on 2026-04-11 with 10 execution-flow tests covering command rendering, prompt flow, browse/search backtracking, built-in providers, and command-backed suggestions.
- Command or check: `cargo test`
  Result: Passed on 2026-04-11 with 53 total tests green.
- Command or check: `cargo clippy --all-targets --all-features -- -D warnings`
  Result: Passed on 2026-04-11.
- Command or check: `cargo fmt --check`
  Result: Passed on 2026-04-11.
- Command or check: `./scripts/smoke-part03-execute.sh`
  Result: Launched successfully in a PTY on 2026-04-11 against the `examples/` corpus and cancelled cleanly with `Esc`.

## Next Handoff

Part 04 should replace the temporary one-command `main.rs` wiring with the full CLI grammar, then layer frecency persistence and Bash hotkey integration on top of the now-stable execute flow.
