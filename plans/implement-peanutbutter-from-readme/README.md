# Implement Peanutbutter from README

Status: done
Review status: blocked-no-subagents
Part count: 4
Context: [`PLAN_CONTEXT.md`](../../PLAN_CONTEXT.md)

This task plan is a living document. Update this file when part status, blockers, sequencing, or acceptance changes. Update the active part file after material implementation progress.

## Objective

Implement the first working version of `pb` around the README-defined experience: markdown-backed snippets, cwd-aware frecency, fuzzy and tree-style navigation, variable prompting, and shell-safe command emission.

When this task is complete, a user should be able to manage snippets with the CLI, invoke the inline selector with `pb execute`, and install Bash hotkey integration with `pb --bash C+b` so the chosen command lands in the shell buffer instead of executing immediately.

## Constraints

- `README.md` is the authoritative spec; the codebase currently contains only `src/main.rs` and example snippet files.
- Keep v1 as a single binary crate with internal modules rather than introducing multiple crates or a background service.
- Follow the snippet format from the README: optional YAML frontmatter, `##` headings, first fenced code block wins, and markdown description content between heading and code block.
- Preserve the CLI contract from the README and `AGENTS.md`: `pb` with no args should remain help or no-op, and actual execution remains the shell's job.
- Favor simple file-backed persistence for config and frecency data unless implementation proves that insufficient.

## Part Tracker

| Part | File | Status | Notes |
| ---- | ---- | ------ | ----- |
| 01 | [Core Foundations and Snippet Parsing](part-01-core-foundations-and-snippet-parsing.md) | done | Crate layout, snippet discovery, line-oriented parser with frontmatter + variable extraction, 17 tests covering every `examples/` file. |
| 02 | [Ranking, Search, and Tree Navigation](part-02-ranking-search-and-tree-navigation.md) | done | SnippetIndex, nucleo-backed fuzzy scoring, ratatui-aware fuzzy/browse state, file-backed frecency store with location+recency+frequency scoring, ranked search combining them. 37 unit + 6 integration tests green. |
| 03 | [Inline TUI Execution and Variable Resolution](part-03-inline-tui-execution-and-variable-resolution.md) | done | `pb execute` now runs the selector, preview, and variable prompts; v1 uses the standard ratatui viewport after inline probing proved unreliable in PTY smoke testing. |
| 04 | [CLI Surface and Shell Integration](part-04-cli-surface-and-shell-integration.md) | done | Added the CLI parser, Bash integration output, safe add/delete commands, post-emission frecency persistence, and shell-safe execute wiring. |

## Global Progress Log

- [x] 2026-04-11: Created the initial four-part implementation bundle from `README.md`, `AGENTS.md`, `Cargo.toml`, `src/main.rs`, and `examples/`.
- [x] 2026-04-11: Recorded that the full four-pass review loop could not be run because this session does not permit sub-agent use.
- [x] 2026-04-11: Aligned `examples/nested/root.md` with the README snippet-heading rule so the examples now consistently use `##` for snippets.
- [x] 2026-04-11: Completed Part 01 — module tree, discovery, parser, and 17 tests (11 unit + 6 integration) green under `cargo test`, `cargo clippy --all-targets -- -D warnings`, and `cargo fmt --check`.
- [x] 2026-04-11: Completed Part 02 — added `nucleo-matcher` + `ratatui` deps; introduced `index`, `fuzzy`, `browse`, `frecency`, and `search` modules. Frecency formula blends time decay, path affinity, and a sublinear frequency boost. 37 unit + 6 integration tests green under all three checks.
- [x] 2026-04-11: Completed Part 03 — added `src/execute.rs`, minimal `pb execute` wiring, variable resolution and command rendering, 10 new execution tests, and a PTY smoke script. `cargo test`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo fmt --check` all pass.
- [x] 2026-04-11: Completed Part 04 — added `src/cli.rs`, full `pb`/`pb execute`/`pb --bash`/`pb add`/`pb del` dispatch, safe snippet deletion, post-emission frecency persistence, and Bash smoke coverage. The TUI now prefers `/dev/tty` output so shell capture keeps stdout clean for the emitted command.

## Review Findings

- Review: Architecture alignment, downgraded from `ARCHITECTURE.md` review because no such file exists in this repo.
  Finding: The strongest architecture evidence is `README.md`, `Cargo.toml`, `src/main.rs`, and the example snippet files. They point to a small single-binary application rather than multiple layers or services.
  Action: Accepted. The plan keeps one crate and separates parser, ranking, TUI, and CLI concerns with modules rather than process or crate boundaries.
- Review: Extensibility and prototype integration.
  Finding: The required sub-agent pass was not run. The main durability risk is accidentally mixing shell integration, TUI state, and parsing into one interface.
  Action: Accepted. The parts explicitly isolate snippet ingestion, ranking, interactive execution, and shell-facing CLI glue.
- Review: Simplicity.
  Finding: The required sub-agent pass was not run. A database, plugin system, or multi-crate split would be unjustified for the current repo size and implementation maturity.
  Action: Accepted. The plan uses file-backed state and a single binary crate.
- Review: Related work explorer.
  Finding: The required sub-agent pass was not run. Existing related work is limited to README examples and sample snippet fixtures; there are no competing internal code patterns yet.
  Action: Accepted. `examples/` should be promoted into the first parser and browse fixtures during implementation.

## Decision Log

- Decision: Split the work into four implementation parts.
  Why: The README scope naturally breaks into foundations, ranking and navigation, interactive execution, and shell-facing CLI work. That split keeps each part independently verifiable without turning the plan into ceremony.
  Date/Author: 2026-04-11 / Codex
- Decision: Treat the README as authoritative and treat example-file mismatches as plan discoveries, not silent parser exceptions.
  Why: The repo does not yet have implementation code or an architecture document that would override the README.
  Date/Author: 2026-04-11 / Codex
- Decision: Mark the plan as `blocked-no-subagents` for review status.
  Why: The skill requires a four-pass sub-agent review loop for substantive plans, and this session does not permit spawning those reviewers.
  Date/Author: 2026-04-11 / Codex

## Acceptance

- The plan bundle stays structurally valid under `validate_plan_bundle.py`.
- Implementation guided by this plan yields a working `pb` that can discover markdown snippets, rank them with cwd-aware frecency, support fuzzy and tree navigation, prompt for variables, and emit shell-ready command text.
- The final implementation validates with `cargo fmt --check`, `cargo test`, and manual smoke checks for `pb --bash`, `pb execute`, `pb add`, and `pb del`.

## Risks and Dependencies

- Shell buffer integration depends on terminal and Bash behavior that is harder to automate than pure Rust logic.
- The README does not yet specify exact config, snippet-root, or frecency-store paths, so v1 needs a clear XDG or home-directory policy.
- The snippet parser must handle markdown rules without overfitting to inconsistent example files.
- Frecency weighting will likely need real-world tuning after the initial implementation exists.

## Notes

This plan did not receive the full required sub-agent review loop. If session policy changes later, run the four formal review passes and update this file before treating the plan as fully reviewed.
