# Part 04 - CLI Surface and Shell Integration

Status: planned
Parent: [`README.md`](README.md)
Context: [`PLAN_CONTEXT.md`](../../PLAN_CONTEXT.md)

This part file is a living working document. Keep the checklist, progress log, discoveries, validation, and handoff current while the work is active.

## Outcome

The user-facing CLI matches the README: `pb` is help or no-op by default, `pb execute` runs the interactive selector, `pb --bash C+b` emits Bash integration, and `pb add` and `pb del` manage snippet files safely around the established snippet-root policy.

## Scope

- In scope: define and implement the CLI grammar; generate Bash integration output; wire `pb execute` to the TUI flow; implement snippet-file creation and deletion behavior; persist frecency updates after successful selection; add help text, smoke checks, and any README touch-ups needed to match implemented behavior.
- Out of scope: additional shell integrations beyond the first Bash path unless they fall out almost for free, and any packaging or release automation not described by the README.

## Dependencies

- Needs: Parts 01-03, especially stable config paths, parsed snippets, ranking, and `pb execute`.
- Unblocks: a complete README-driven v1 that a user can install and try from their shell.

## Checklist

- [ ] Add a CLI parser that exposes `pb`, `pb execute`, `pb add [path]`, `pb del [name]`, and `pb --bash <binding>`.
- [ ] Keep bare `pb` non-destructive and aligned with the README help or no-op expectation.
- [ ] Implement Bash integration output that invokes `pb execute` and writes the returned command into the shell buffer.
- [ ] Implement `pb add` using `$VISUAL` or `$EDITOR`, creating or opening the relevant snippet file in the configured snippet root.
- [ ] Implement `pb del` with exact-match or clearly disambiguated deletion semantics so the command is safe.
- [ ] Persist frecency updates only after a command has been successfully selected and emitted.
- [ ] Add CLI and shell smoke tests and update the README if any behavior needs to be clarified for users.

## Progress Log

- [x] 2026-04-11: Scoped the final wiring work around the README CLI contract and Bash-specific shell integration path.

## Discoveries

- Observation: The README only explicitly specifies Bash integration, so that should be the first supported shell rather than a generic abstraction for every shell upfront.
  Evidence: `README.md` CLI design section and `AGENTS.md` command examples
- Observation: The deletion contract is underspecified, so v1 needs exact or clearly disambiguated matching to avoid destructive surprises.
  Evidence: `README.md` lists `pb del [name]` but does not define ambiguous-match behavior

## Validation

- Command or check: `cargo test cli`
  Result: Pending until implementation.
- Command or check: `cargo test`
  Result: Pending until implementation.
- Command or check: manual Bash smoke test with `eval "$(pb --bash C+b)"` and a real snippet selection flow.
  Result: Pending until implementation.

## Next Handoff

Implement CLI glue last so deletion, shell wiring, and frecency persistence all sit on top of already-tested core behavior rather than dictating it.
