# BIGPLAN: migrate CLI parsing to clap

## Plan Overview

The current hand-rolled argument parser in `src/cli.rs` (`parse_args` + `help_text`) is brittle to extend: every new flag or subcommand requires manually wiring argument counts, writing error strings, and maintaining a separate help formatter. This effort replaces that machinery with `clap` (derive API), letting clap own argument parsing, `--help` output, and error formatting. The business logic functions (`run_execute_command`, `run_del_command`, `run_add_command`, `bash_integration_*`) are unchanged — only the parsing and dispatch layer changes. Done means `parse_args`, `CliCommand`, and `help_text` are gone, replaced by a `#[derive(Parser)]` struct, and `main.rs` dispatches through it. `--bash` is renamed to the `bash` subcommand.

## Risks

- **help_text includes dynamic runtime content** — the current `help_text` function appends the resolved snippet roots, config file path, and state file path at runtime. Clap's built-in `--help` is static (build-time). The extra paths info will need to be printed separately. This requires building the command manually with `Cli::command().after_help(dynamic_string)` and then calling `.get_matches()` and `Cli::from_arg_matches(&matches)` — `Cli::parse()` alone cannot inject runtime content into help output.
- **test surface for `parse_args`** — `cli.rs` has comprehensive parsing tests against the public `parse_args` function. These tests will need rewriting against the new clap interface (or removed and superseded by clap's own validation). The `help_text` test (`help_text_prefers_peanutbutter_and_mentions_pb_alias`) must also be deleted since `help_text` itself is deleted.
- **clap exits the process on parse errors** — clap calls `process::exit` directly on parse failures, bypassing any cleanup in `main`. The TUI only starts after dispatch, so there is no raw-mode leak in the current design. Future subcommands that do terminal setup before argument processing must not rely on `main`'s cleanup path running on parse errors.
- **clap generates `--version` / `-V` by default** — without an explicit `version` attribute on the derive struct, clap emits the version from `Cargo.toml`. This is acceptable behaviour; add `#[command(version)]` to opt in explicitly and make it intentional rather than accidental.
- **binary size** — adding clap (derive feature) is the largest common dependency addition for a Rust CLI. Build times and binary size will increase measurably. Acceptable given the maintenance trade-off, but not free.

## Plan Details

### Critical Files

- `src/cli.rs` — contains `parse_args`, `CliCommand`, and `help_text`; the main target of this migration
- `src/main.rs` — calls `parse_args` and dispatches on `CliCommand`; will be rewritten to use the clap-derived struct directly
- `Cargo.toml` — needs `clap` added as a dependency (with `features = ["derive"]`)

### Gotchas

- **`after_help` requires manual command building** — to inject runtime path data into help, use `Cli::command().after_help(dynamic_string).get_matches()` then `Cli::from_arg_matches(&matches).unwrap_or_else(|e| e.exit())`. Do not use `Cli::parse()` if dynamic help content is needed; it has no injection point.
- **Do not use `arg_required_else_help = true`** — it exits with code 2 on bare invocation, breaking the current exit-0 contract. Instead, make `command: Option<Command>` in the `Cli` struct and handle `None` explicitly in `main`: call `Cli::command().print_help()` and `process::exit(0)`.
- **Use `Cli::parse()`, not `Cli::parse_from(skipped_args)`** — `parse_from` expects the full argv including the program name as element 0. The current `main.rs` strips the program name before passing args. Using `parse_from` with the already-skipped args will misinterpret the first subcommand as the binary name. Use `Cli::parse()` which reads `std::env::args_os()` directly.
- **Add `#[command(version)]` explicitly** — clap emits `--version` by default. Adding the attribute makes the intent visible in code and avoids surprising future readers.
- Clap's `--help` exits with code 0 and prints to stdout. The current code prints to stderr on error and exits with code 2. Clap handles this automatically; don't duplicate the exit logic in `main.rs`.
- `run_execute_command_with` is a seam used by tests to inject a fake runner — preserve it unchanged.
- The `is_execute` flag in `main.rs` (controls "execute failed" error message) needs to survive the rewrite.
- `--bash` is renamed to the `bash` subcommand. The README and any shell rc snippets in `examples/` that reference `--bash` will need updating.
- The `DEFAULT_BASH_BINDING` constant (`"C+b"`) moves into the clap `default_value` attribute on the `binding` argument.
- **`Del` argument label** — the `del` subcommand accepts either a snippet name or a `file#slug` id. Add `#[arg(value_name = "NAME_OR_ID")]` to the field so clap's generated usage shows `<NAME_OR_ID>` rather than the misleading `<NAME>`.

### Pseudo-code / Sketches

```text
// Cargo.toml
clap = { version = "4", features = ["derive"] }

// src/cli.rs (new shape)
#[derive(Parser)]
#[command(name = BINARY_NAME, about = "terminal snippet manager")]
#[command(version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,  // Option so bare invocation returns None (exit 0)
}

#[derive(Subcommand)]
pub enum Command {
    /// Run the interactive TUI and insert the selected command into readline.
    Execute,
    /// Open $EDITOR on a snippet file.
    Add { path: Option<PathBuf> },
    /// Delete a snippet by name or id.
    Del {
        #[arg(value_name = "NAME_OR_ID")]
        name: String,
    },
    /// Emit bash shell integration code (eval in .bashrc / .zshrc).
    Bash {
        #[arg(default_value = "C+b")]
        binding: String,
    },
}

// src/main.rs (new shape)
fn main() {
    let app_config = config::load()...;
    // Build command with runtime path info injected into after_help.
    let after = format!("snippet roots:\n  ...\nconfig: {}\nstate: {}",
        paths.config_file.display(), paths.state_file.display());
    let matches = Cli::command().after_help(after).get_matches();
    let cli = Cli::from_arg_matches(&matches).unwrap_or_else(|e| e.exit());
    let Some(command) = cli.command else {
        Cli::command().print_help().unwrap();
        process::exit(0);
    };
    let is_execute = matches!(command, Command::Execute);
    match command {
        Command::Execute => { ... }
        Command::Add { path } => { ... }
        Command::Del { name } => { ... }
        Command::Bash { binding } => { ... }
    }
}
```

## Deliverables

### Deliverable 1. Add clap dependency

Add `clap` to `Cargo.toml` with the `derive` feature. Confirm `cargo build` passes cleanly.

- [x] Add `clap = { version = "4", features = ["derive"] }` to `[dependencies]` in `Cargo.toml`
- [x] Run `cargo build` and verify it compiles with no new warnings

### Deliverable 2. Replace `CliCommand` and `parse_args` with clap derive

Rewrite the parsing layer in `src/cli.rs` using `#[derive(Parser)]` and `#[derive(Subcommand)]`. Remove `parse_args`, `CliCommand`, and `help_text`. Keep all business logic functions untouched.

- [x] Define `Cli` struct with `command: Option<Command>` and `#[command(version)]`
- [x] Define `Command` enum with clap derive attributes and per-subcommand `about` strings
- [x] Model `bash` as a proper subcommand with an optional `binding` argument defaulting to `"C+b"`
- [x] Add `#[arg(value_name = "NAME_OR_ID")]` to the `del` subcommand's argument
- [x] Handle runtime path info (snippet roots, config file, state file) via `Cli::command().after_help(...)` + `Cli::from_arg_matches()`
- [x] Delete `parse_args`, `CliCommand`, `help_text`, and the `DEFAULT_BASH_BINDING` constant
- [x] Confirm `cargo clippy` is clean

### Deliverable 3. Rewrite `main.rs` dispatch

Replace the `parse_args` call and `CliCommand` match in `main.rs` with the clap-derived struct and a match on the new `Command` enum.

- [x] Build the command with runtime `after_help` content using `Cli::command().after_help(...)` before calling `.get_matches()`
- [x] Resolve the `Cli` struct using `Cli::from_arg_matches(&matches).unwrap_or_else(|e| e.exit())`
- [x] Handle `cli.command == None` (bare invocation) by printing help and exiting 0
- [x] Update the `match` arms to use the new `Command` variants
- [x] Use `Cli::parse()` if dynamic `after_help` is not needed for a subcommand path — never use `Cli::parse_from` with pre-skipped args
- [x] Preserve the `is_execute` special-case error message
- [x] Remove the now-redundant manual help printing (`cli::help_text`)

### Deliverable 4. Update tests and docs

Remove or rewrite the `parse_args`-specific tests and the `help_text` test. Update README and any example references to `--bash`.

- [x] Remove tests that directly call `parse_args`
- [x] Remove `help_text_prefers_peanutbutter_and_mentions_pb_alias` (function deleted)
- [x] Rewrite coverage as tests against the clap-derived struct using `Cli::try_parse_from`
- [x] Confirm `bash_integration_script` and `run_execute_command_with` tests are unaffected
- [x] Update README / examples that reference `--bash` to use `bash` subcommand spelling
- [x] Run full test suite and confirm green

## Issues

- **2026-04-27 — user** — `--bash` flag → `bash` subcommand: decided to use `bash` as a proper subcommand (breaking change accepted; cleaner interface). Commit introducing this rename will carry a `!` breaking-change marker.
- **2026-04-28 — review** — `arg_required_else_help` exits code 2 (not 0); resolved by using `Option<Command>` + explicit help print. `Cli::parse_from` with pre-skipped args is a footgun; resolved by always using `Cli::parse()` or the manual `command().get_matches()` path. `after_help` dynamic injection requires `Cli::command()` builder pattern, not `Cli::parse()`. `Del` value_name corrected to `NAME_OR_ID`. `help_text` test added to deletion checklist. `--version` flag kept via explicit `#[command(version)]`.
