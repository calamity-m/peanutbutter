# Peanutbutter

Rust terminal snippet manager for people who keep executable Markdown snippets and want fast, location-aware shell-buffer insertion via `pb`.

## 1. Think Before Coding

**Don't assume. Don't hide confusion. Surface tradeoffs.**

Before implementing:

- State assumptions explicitly; if the behavior can be read two ways, ask or present both options.
- Keep the change scoped to the requested outcome; mention adjacent cleanup separately instead of doing it silently.
- Prefer the smallest design that preserves the shell/TUI invariants below; push back on speculative abstraction.
- Treat `README.md`, `docs/SNIPPET_SYNTAX.md`, and `docs/LSP.md` as user-facing specs, not marketing copy.

## 2. Guidelines

- Keep the shell hotkey path stdout-clean. During `execute`, fd 1 is the command payload captured by the shell; status, warnings, and TUI drawing must not leak into it.
- Parser, lint, LSP, examples, and docs describe the same snippet language. A syntax change usually touches `src/parser/`, `src/lint/`, `src/lsp/`, `docs/SNIPPET_SYNTAX.md`, and `tests/examples.rs` together.
- Preserve terminal cleanup paths. `RawModeGuard`, `StdoutTtyGuard`, event draining, and `cleanup_terminal` protect users' interactive shells; do not replace them with a Rust-level writer swap, and gate Windows/PowerShell changes so the Unix hotkey path does not regress.
- Keep persistent ids stable. `SnippetId` is `relative/path.md#heading-slug`; frecency state, GC, index lookup, and LSP navigation depend on that exact separator and slug model.
- Ranking changes belong behind `SearchConfig` / `FrecencyConfig` and need focused tests. The frecency store is intentionally a hand-editable TSV, not a database.
- Use the configured hooks instead of inventing new check lists: `prek install` once, then `prek run` or the targeted cargo commands below.

## 3. Goal-Driven Execution

**Define success criteria. Loop until verified.**

Transform tasks into verifiable goals:

- "Add validation" -> "Write tests for invalid snippets, then make them pass"
- "Fix the bug" -> "Write a test or repro that fails before the fix, then make it pass"
- "Refactor X" -> "Run the relevant tests before and after and keep public behavior unchanged"

For multi-step tasks, state a brief plan:

```text
1. [Step] -> verify: [check]
2. [Step] -> verify: [check]
3. [Step] -> verify: [check]
```

Useful checks:

```bash
cargo fmt --check
cargo test
cargo clippy -- -D warnings
```

For snippet-language changes, lint the bundled examples with user config isolated:

```bash
tmp=$(mktemp -d); PB_CONFIG_FILE="$tmp/config.toml" PEANUTBUTTER_PATH="$PWD/examples" XDG_CONFIG_HOME="$tmp/config" XDG_STATE_HOME="$tmp/state" cargo run --quiet -- lint --strict --json; rm -rf "$tmp"
```

For shell/TUI changes, also verify a captured-stdout invocation from an interactive shell, e.g. `out=$(cargo run --quiet -- execute); printf '[%s]\n' "$out"`, and confirm no escape sequences or status text appear in `out`.

## 4. In-Code Documentation

**Public API must be documented. Internal logic should explain the why.**

For public Rust items, use rustdoc (`///` for items, `//!` for modules). Describe the role, non-obvious constraints, and user-visible side effects; a one-liner is enough when types make the rest clear.

Comment the invariants that are easy to break when editing blind:

- fd/TTY behavior in `src/tui/terminal.rs` and `src/execute/terminal.rs`.
- `SnippetId` construction and any code that parses or persists ids.
- Snippet grammar edge cases: bare `text` fences, `##` heading boundaries, frontmatter variable specs, and dependent placeholder ordering.
- Frecency scoring constants and path-affinity tradeoffs in `src/frecency.rs` / `src/search.rs`.
- LSP activation via `MARKER_FILENAMES` and the full-sync document store in `src/lsp.rs`.

## 5. Key Decisions

- `src/main.rs` is only the binary shell: it loads config, parses `cli::Cli`, dispatches to `cli::run_*` functions, and writes the selected command only after `execute` completes.
- `config::AppConfig` and `config::Paths` are the runtime wiring point: config file, XDG defaults, env overrides, snippet roots, state file, UI height, theme, search weights, variable overrides, and suggestion-command policy meet there.
- Snippet ingestion flows `discovery::discover_markdown_files` -> `parser::parse_file` -> `index::SnippetIndex`. `parser` returns `SnippetFile` / `Snippet`; `index::IndexedSnippet` adds path and frontmatter context for search, display, tags, and editing.
- `execute::terminal::run_execute_with_provider` owns the ratatui/crossterm loop around `ExecutionApp`; tests can inject a `SuggestionProvider`, while production uses `SystemSuggestionProvider` for builtins, config/frontmatter suggestions, and shell commands.
- Picker navigation is explicitly three-mode: `NavigationMode::Fuzzy`, `NavigationMode::Browse`, and `NavigationMode::Tags`, cycled with `Ctrl+T`. Keep render, state, and help text in sync when changing modes.
- `FrecencyStore` is append-only usage history persisted as TSV. `FrecencyStore::score` combines recency, cwd path affinity, and frequency; `search::rank` adds fuzzy scores and sorts by combined score.
- `lsp::run_lsp_server` creates a tokio runtime for tower-lsp `Backend`. The LSP only activates for `.md` files under a marker from `MARKER_FILENAMES` (`.peanutbutter.toml`, `peanutbutter.toml`, `_peanutbutter.toml`).
