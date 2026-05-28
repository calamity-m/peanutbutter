# AGENTS.md

This provides context for this project. The README.md acts as a specification to follow.

---

## 1. Think Before Coding

**Don't assume. Don't hide confusion. Surface tradeoffs.**

Before implementing:
- State your assumptions explicitly. If uncertain, ask.
- If multiple interpretations exist, present them — don't pick silently.
- If a simpler approach exists, say so. Push back when warranted.
- Don't silently expand scope into wiring, integrations, or adjacent work that wasn't requested. If scope is unclear, ask rather than guessing big.
- If something is unclear, stop. Name what's confusing. Ask.

## 2. Simplicity First

**Minimum code that solves the problem. Nothing speculative.**

- No features beyond what was asked.
- No abstractions for single-use code.
- No "flexibility" or "configurability" that wasn't requested.
- No error handling for impossible scenarios.
- If you write 200 lines and it could be 50, rewrite it.

Ask yourself: "Would a senior engineer say this is overcomplicated?" If yes, simplify.

## 3. Surgical Changes

**Touch only what you must. Clean up only your own mess.**

When editing existing code:
- Don't "improve" adjacent code, comments, or formatting.
- Don't refactor things that aren't broken.
- Match existing style, even if you'd do it differently.
- If you notice unrelated dead code, mention it — don't delete it.

When your changes create orphans:
- Remove imports/variables/functions that YOUR changes made unused.
- Don't remove pre-existing dead code unless asked.

The test: every changed line should trace directly to the user's request.

## 4. Goal-Driven Execution

**Define success criteria. Loop until verified.**

Transform tasks into verifiable goals:
- "Add validation" → "Write tests for invalid inputs, then make them pass"
- "Fix the bug" → "Write a test that reproduces it, then make it pass"
- "Refactor X" → "Ensure tests pass before and after"

For multi-step tasks, state a brief plan:

```text
1. [Step] -> verify: [check]
2. [Step] -> verify: [check]
3. [Step] -> verify: [check]
```

Strong success criteria let you loop independently. Weak criteria ("make it work") require constant clarification.

## 5. In-Code Documentation

**Public API must be documented. Internal logic should explain the why.**

For all public functions, types, structs, traits, and constants:
- Use `///` rustdoc comments
- Describe what the item is for and any non-obvious parameter or return constraints
- If the types alone make everything clear, a one-liner is enough

For internal code, comment the *why*, not the *what*:
- `// retry because the upstream API is eventually consistent` is useful; `// call the API` is not
- Keep it short — one line is usually right

Do not comment what the code already says plainly.

## 6. Pre-commit Hooks

**Prefer pre-commit hooks over repeated "do X after changes" reminders.**

This repo uses [`prek`](https://github.com/calam1/prek) (configured in `prek.toml`). Run `prek install` once per checkout to activate hooks. Configured hooks:
- **pre-commit**: `cargo fmt --check`, `cargo build`, `cargo test`, `cargo clippy -- -D warnings`
- **pre-push to main**: `cargo clippy -- -D warnings`
- **manual**: `cargo tarpaulin --out Html` — run on demand with `prek run --stage manual cargo-coverage`; requires `cargo install cargo-tarpaulin` once

If the user continually asks to run the same check → suggest adding it to `prek.toml` as a pre-commit hook.

## 7. Repository Map

### Key directories

```text
src/           -> library modules (one file per module, new-style Rust)
  main.rs      -> binary entry point, command dispatch
  lib.rs       -> module declarations and crate-level docs
  domain.rs    -> core value types (snippets, variables, ids)
  parser.rs    -> Markdown → SnippetFile
  execute.rs   -> interactive TUI (ratatui + crossterm)
  frecency.rs  -> usage history and location-aware scoring
  index.rs     -> in-memory snippet index
  search.rs    -> combined fuzzy + frecency ranking
tests/         -> integration tests (examples.rs)
docs/          -> specs (SNIPPET_SYNTAX.md)
scripts/       -> git hook helpers (pre-push-clippy.sh)
```

### Entry point

```text
src/main.rs  ->  cargo run  (binary: peanutbutter)
```

### Data flow

```text
CLI args → cli::Cli (clap) → command dispatch
  Execute  → execute TUI (ratatui/crossterm) → stdout (shell buffer write)
  Bash     → bash_integration_for_current_exe → stdout (eval'd by shell)
  Edit     → editor launch → file save
```

## 8. Project Specific Notes

`peanutbutter` is a terminal snippet manager with an inline TUI. Running `peanutbutter --bash` also installs a `pb` bash alias. The core value props:

1. **Location-aware frecency** — snippet rankings factor in the current working directory, not just frequency/recency globally
2. **Two navigation modes** — fuzzy search over snippet names/content/frontmatter, and a file-tree browser with tab-completion
3. **Shell buffer integration** — selected snippets are written into the terminal's input buffer (not executed directly), achieved via shell hotkey setup (`peanutbutter --bash C+b` outputs shell code for eval and installs the `pb` alias)
4. **Plain markdown format** — snippet files are readable without tooling
5. Uses new-style Rust modules (`src/browse.rs`, not `src/browse/mod.rs`) — follow this pattern for new files.
6. TUI is built with `ratatui` + `crossterm`; terminal state is sensitive — always restore raw mode on exit paths.
7. The `execute` subcommand writes output to stdout for shell consumption; don't mix debug prints into that path.

### TUI / shell-integration invariants

The shell hotkey path is the primary way users invoke this tool. It has subtle invariants that are easy to break with well-intentioned refactors:

- **Stdout is captured by `$(...)`** when invoked via the shell hotkey. Anything written to fd 1 during the TUI ends up in the shell's readline buffer, not on screen. The chosen snippet must be the *only* thing left on fd 1 by the time the process exits.
- **Crossterm queries the terminal via fd 1 and reads replies from stdin** — e.g. the cursor-position DSR (`^[[6n`). Routing the Rust-side writer to stderr does NOT redirect these queries; they still go to fd 1. If fd 1 is a pipe, the query bytes leak into the shell buffer and crossterm waits the full timeout (~3s) for a reply that never comes.
- **The fix on Unix is the `StdoutTtyGuard` `dup2` pattern** in `src/execute/terminal.rs`: save fd 1, point it at `/dev/tty` (or stderr if that's a tty), restore on drop. Do not remove this guard or replace it with a Rust-level writer switch — they are not equivalent.
- **Cross-platform support must not regress the primary (Unix) path.** When adding Windows/PowerShell behavior, gate the new code with `#[cfg(windows)]` or `#[cfg(not(unix))]` rather than replacing the existing Unix path. The Unix path is exercised by every shell user; Windows is a smaller surface.
- **Verify TUI changes through the actual hotkey path**, not just `pb execute` in a terminal. The two paths differ in whether fd 1 is a tty. A working `pb execute` says nothing about the hotkey path. Minimum repro: `out=$(peanutbutter execute); echo "[$out]"` from an interactive shell, and confirm no stray escape sequences appear in `$out`.

### Snippet Format

When interacting with, extending, or touching the snippet parsing or logic refer to [the snippet syntax specification](/docs/SNIPPET_SYNTAX.md)

### Commands

```bash
cargo build          # build
cargo run            # run (binary: peanutbutter)
cargo test           # run all tests
cargo test <name>    # run a single test by name
cargo clippy         # lint
cargo fmt            # format
```

