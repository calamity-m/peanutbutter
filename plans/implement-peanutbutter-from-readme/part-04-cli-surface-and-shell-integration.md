# Part 04 - CLI Surface and Shell Integration

Status: done
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

- [x] Add a CLI parser that exposes `pb`, `pb execute`, `pb add [path]`, `pb del [name]`, and `pb --bash <binding>`.
- [x] Keep bare `pb` non-destructive and aligned with the README help or no-op expectation.
- [x] Implement Bash integration output that invokes `pb execute` and writes the returned command into the shell buffer.
- [x] Implement `pb add` using `$VISUAL` or `$EDITOR`, creating or opening the relevant snippet file in the configured snippet root.
- [x] Implement `pb del` with exact-match or clearly disambiguated deletion semantics so the command is safe.
- [x] Persist frecency updates only after a command has been successfully selected and emitted.
- [x] Add CLI and shell smoke tests and update the README if any behavior needs to be clarified for users.

## Progress Log

- [x] 2026-04-11: Scoped the final wiring work around the README CLI contract and Bash-specific shell integration path.
- [x] 2026-04-11: Added `src/cli.rs` with argument parsing, help text, Bash integration generation, execute persistence, snippet creation/edit launching, and exact-match deletion.
- [x] 2026-04-11: Switched the Part 03 terminal runner to prefer `/dev/tty` output so `pb execute` can be safely captured by Bash command substitution while still drawing the TUI on the terminal.
- [x] 2026-04-11: Added `scripts/smoke-part04-bash.sh` and CLI tests covering Bash script generation, add/delete behavior, and frecency persistence.

## Discoveries

- Observation: The README only explicitly specifies Bash integration, so that should be the first supported shell rather than a generic abstraction for every shell upfront.
  Evidence: `README.md` CLI design section and `AGENTS.md` command examples
- Observation: The deletion contract is underspecified, so v1 needs exact or clearly disambiguated matching to avoid destructive surprises.
  Evidence: `README.md` lists `pb del [name]` but does not define ambiguous-match behavior
- Observation: Shell-buffer integration only works cleanly if `pb execute` keeps stdout reserved for the final command and renders the TUI somewhere else, so the runner now prefers `/dev/tty` for terminal output.
  Evidence: `src/execute.rs` and the 2026-04-11 PTY command-substitution smoke run
- Observation: `pb add [path]` needs a concrete write policy for multi-root setups, so v1 writes relative add targets into the first configured snippet root and appends `.md` when the requested path has no extension.
  Evidence: `src/cli.rs` `resolve_add_target` and `cli::tests::resolve_add_target_defaults_and_appends_markdown_extension`
- Observation: Frecency persistence should not block command insertion after a successful selection, so Part 04 treats post-emission save failures as warnings rather than undoing the emitted command.
  Evidence: `src/cli.rs` `run_execute_command_with`

## Validation

- Command or check: `cargo test cli`
  Result: Passed on 2026-04-11 with 9 CLI-focused tests covering parsing, Bash script generation, add/delete behavior, and execute persistence.
- Command or check: `cargo test`
  Result: Passed on 2026-04-11 with 62 total tests green.
- Command or check: `cargo clippy --all-targets --all-features -- -D warnings`
  Result: Passed on 2026-04-11.
- Command or check: `cargo fmt --check`
  Result: Passed on 2026-04-11.
- Command or check: `./scripts/smoke-part04-bash.sh`
  Result: Passed on 2026-04-11; generated Bash integration and validated it with `bash -n`.
- Command or check: PTY command-substitution smoke run for `out="$(target/debug/pb execute)"`
  Result: Passed on 2026-04-11; the interactive flow completed and the captured stdout ended with the emitted command while the TUI rendered on the terminal device.

## Next Handoff

The README-driven v1 is now functionally complete. Future work can focus on packaging, broader shell integrations, and tuning default snippet-root ergonomics rather than core behavior gaps.
