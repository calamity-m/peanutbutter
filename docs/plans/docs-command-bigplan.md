# BIGPLAN: Expose snippet syntax & config reference from the binary (`pb docs`)

## Plan Overview

Issue #48 asks for user-facing reference material to ship inside the `peanutbutter`
binary. The real motivation is narrower: let an LLM (or offline user) read the
snippet syntax spec locally — `pb docs syntax` — instead of fetching
`docs/SNIPPET_SYNTAX.md` from GitHub. This effort adds a `pb docs <topic>` command
that prints embedded copies of the canonical syntax reference and config example to
clean stdout. The canonical files stay where users and the README expect them
(`docs/SNIPPET_SYNTAX.md`, `examples/config.toml`); a `build.rs` copies them into
`OUT_DIR` at compile time so production code embeds generated assets, while tests
compare those assets against the canonical source files to catch drift. "Done" means
a released-binary user can run `pb docs syntax` and `pb docs config` and get the
exact spec text, with CI guarding against an empty/missing embed. **Manpages are explicitly deferred** to a
follow-up (see Issues); this plan does not fully close #48.

## Risks

- **Packaged-crate build break** — `build.rs` reads `docs/SNIPPET_SYNTAX.md` and
  `examples/config.toml` at compile time, including during `cargo install` from
  crates.io. Both are git-tracked and packaged today (verified via
  `cargo package --list`, no `include`/`exclude` in `Cargo.toml`). A later
  `exclude`/`include` addition that drops `docs/` or `examples/` makes the published
  crate fail to build for everyone. Mitigation: a packaging-guard test asserting both
  paths appear in `cargo package --list`, plus a Gotcha note forbidding a narrowing
  `include`/`exclude` without updating the build.
- **Stale embed after editing a canonical file (local builds)** — `build.rs` output
  is only regenerated when Cargo knows the inputs changed. Without explicit
  `cargo:rerun-if-changed` lines for both source files, editing the syntax doc would
  leave the old text embedded in incremental builds. This is a *local-dev* failure
  only; the published-crate / `cargo install` path always does a clean build with a
  fresh `OUT_DIR`, so it is unaffected by this and is instead protected by the
  packaging guard above. Mitigation: emit `rerun-if-changed` for each source file and
  for `build.rs` itself; the real cross-check is the source-vs-embed test in
  Deliverable 1 (it reads the canonical file directly from the source tree, not the
  `OUT_DIR` copy — see that deliverable), which catches a stale or wrong embed.
- **Cross-platform line-ending drift** — the feature promises byte-exact output, but
  git line-ending normalization can make the canonical files checkout as CRLF on some
  machines and LF on others. Mitigation: add a `.gitattributes` rule pinning
  `docs/SNIPPET_SYNTAX.md` and `examples/config.toml` to LF, and keep the
  source-vs-embed tests as the watch-for signal.
- **Stdout contamination** — peanutbutter is strict about clean fd 1 on the execute
  hotkey path. `pb docs` is not that path, but colorized or prefixed output becomes
  useless to an LLM piping it. Mitigation: write raw embedded bytes to
  stdout with no `owo-colors`, no status text; command-level tests assert exact stdout
  and empty stderr.

## Plan Details

The flow is: `build.rs` copies the two canonical files into `OUT_DIR/assets/` →
a new `src/docs.rs` module embeds them with `include_str!(concat!(env!("OUT_DIR"), …))`
and exposes a `Topic` enum + a `render`/`run` writer function → `cli.rs` gains a
`Command::Docs { topic: Option<Topic> }` variant → `main.rs` dispatches it to stdout.
Bare `pb docs` (no topic) lists available topics so an LLM can discover them.

### Critical Files

- `build.rs` (new) — copies `docs/SNIPPET_SYNTAX.md` → `$OUT_DIR/assets/snippet_syntax.md`
  and `examples/config.toml` → `$OUT_DIR/assets/config.toml`; emits `rerun-if-changed`.
  Build scripts run with CWD = package root, so the relative source paths resolve.
- `src/docs.rs` (new) — embeds the copied files as `&'static str` consts, defines the
  `Topic` value-enum (`Syntax`, `Config`), and a `run<W: Write>(topic, writer)` that
  writes raw bytes. Public items get rustdoc.
- `src/cli.rs` — add `Docs { topic: Option<Topic> }` to the `Command` enum; add a
  parse test alongside `clap_recognizes_expected_commands`.
- `src/main.rs` — dispatch arm calling `docs::run` against `io::stdout()`.
- `src/lib.rs` — `pub mod docs;`.
- `docs/SNIPPET_SYNTAX.md`, `examples/config.toml` — unchanged canonical sources; must
  remain git-tracked and packaged.
- `README.md` and `cli::after_help` — mention `pb docs` for discoverability.
- `.github/workflows/unstable.yml`, `.github/workflows/release.yml` — add the required
  `cargo package --list --allow-dirty` guard to the existing CI jobs.
- `.gitattributes` (new) — pin the two embedded canonical text files to LF so
  byte-exact output does not vary by checkout platform.
- `Cargo.toml` — no `include`/`exclude` today, so both canonical files are packaged
  by default. Adding either key in the future MUST keep `docs/SNIPPET_SYNTAX.md` and
  `examples/config.toml` in the package, or `cargo install` breaks. Flagged here so a
  future editor sees the constraint at the point of change.

### Gotchas

- `build.rs` MUST emit `cargo:rerun-if-changed=docs/SNIPPET_SYNTAX.md`,
  `cargo:rerun-if-changed=examples/config.toml`, and one for `build.rs` itself, or
  edits to the docs won't re-embed on incremental builds.
- `build.rs` must `fs::create_dir_all($OUT_DIR/assets)` before copying — the dir does
  not pre-exist and `fs::copy` will not create it. This is the most likely first-pass
  build.rs bug.
- Byte-exactness can be silently broken by git line-ending normalization. Pin
  `docs/SNIPPET_SYNTAX.md` and `examples/config.toml` to LF in `.gitattributes`; a
  future rule that rewrites either file to CRLF on checkout should trip the
  source-vs-embed test.
- The drift test must compare the embed against the canonical file read **directly
  from the source tree** (`include_str!("../docs/SNIPPET_SYNTAX.md")`, which the
  compiler reads from `docs/`, not `OUT_DIR`). Comparing two `OUT_DIR` reads would
  compare the embed against itself and catch nothing.
- An existing unrelated `src/syntax.rs` / `src/syntax/` module handles snippet
  command-template parsing — it is NOT the doc text. `Topic::Syntax` / `pb docs syntax`
  is pure embedded reference text; do not wire it to the `syntax` module.
- `pb docs ... | head` will close the pipe early. Handle broken-pipe / write errors so
  the tool exits quietly instead of printing a Rust panic/backtrace to stderr (which
  would contaminate an LLM's capture). clap's `ValueEnum` already rejects an unknown
  topic with a clean message at parse time.
- `docs/SNIPPET_SYNTAX.md` and `examples/config.toml` must stay in the published
  package. Do not add a narrowing `include`/`exclude` to `Cargo.toml` without also
  re-checking `build.rs`. The packaging-guard test exists to catch this.
- `config.rs:908` already does `include_str!("../examples/config.toml")` in a test.
  Leave it on that direct canonical-file path; add a comment noting it intentionally
  reads the same file as the docs embed, so a future reader does not treat it as a
  divergent second source.
- Keep `pb docs` output raw: no color even when stdout is a TTY (an LLM shell capture
  can still be a TTY). Pipe-friendliness beats prettiness here.
- `Topic` should derive clap's `ValueEnum` so `pb docs syntax|config` parses and
  `--help` lists the topics for free.

### Pseudo-code / Sketches

```text
build.rs:
  out = env OUT_DIR; mkdir out/assets
  copy docs/SNIPPET_SYNTAX.md -> out/assets/snippet_syntax.md
  copy examples/config.toml   -> out/assets/config.toml
  println! cargo:rerun-if-changed=docs/SNIPPET_SYNTAX.md
  println! cargo:rerun-if-changed=examples/config.toml
  println! cargo:rerun-if-changed=build.rs

src/docs.rs:
  const SNIPPET_SYNTAX = include_str!(concat!(env!("OUT_DIR"),"/assets/snippet_syntax.md"))
  const CONFIG_EXAMPLE = include_str!(concat!(env!("OUT_DIR"),"/assets/config.toml"))
  enum Topic { Syntax, Config }   // ValueEnum
  fn run<W: Write>(topic: Option<Topic>, w) -> io::Result<()>:
     match topic:
       Some(Syntax) => w.write_all(SNIPPET_SYNTAX)
       Some(Config) => w.write_all(CONFIG_EXAMPLE)
       None         => write topic list ("syntax\nconfig\n") to w

cli.rs Command:  Docs { topic: Option<Topic> }
main.rs:         Command::Docs{topic} => docs::run(topic, &mut io::stdout())
```

## Deliverables

### Deliverable 1. Build-time embedding pipeline + `pb docs syntax`

The core value: a `build.rs` that copies the canonical files into `OUT_DIR/assets`
with `rerun-if-changed` directives, and a `src/docs.rs` module that embeds them and
prints the snippet syntax reference verbatim. Wire `Command::Docs { topic }` into
`cli.rs` and dispatch in `main.rs`, so `pb docs syntax` writes the exact contents of
`docs/SNIPPET_SYNTAX.md` to clean stdout. Acceptance: `pb docs syntax` output is
byte-identical to the canonical file; editing the canonical file and rebuilding
changes the output.

- [x] Add `build.rs` copying both canonical files into `$OUT_DIR/assets/` and emitting
      `rerun-if-changed` for both sources and `build.rs`.
- [x] Add `src/docs.rs` with `SNIPPET_SYNTAX`/`CONFIG_EXAMPLE` consts, `Topic` enum
      (derive `ValueEnum`), and `run<W: Write>`; rustdoc the public items.
- [x] `pub mod docs;` in `src/lib.rs`.
- [x] Add `Command::Docs { topic: Option<Topic> }` to `cli.rs` with a clap parse test.
- [x] Dispatch the `Docs` arm in `main.rs` to `io::stdout()`; handle broken-pipe /
      write errors by exiting quietly (no panic/backtrace to stderr).
- [x] Test (source-vs-embed drift guard): `docs::SNIPPET_SYNTAX` equals
      `include_str!("../docs/SNIPPET_SYNTAX.md")` — the latter is read by the compiler
      from the source tree, not `OUT_DIR`, so this actually catches divergence.
- [x] Test: the syntax embed is non-empty (length floor) so a zero-byte `OUT_DIR` copy
      from a build.rs bug fails loudly.
- [x] CLI test: invoking the binary as `pb docs syntax` writes byte-identical
      `docs/SNIPPET_SYNTAX.md` content to stdout and writes nothing to stderr.

### Deliverable 2. `pb docs config` topic

Add the `config` topic so `pb docs config` prints `examples/config.toml` verbatim,
reusing the Deliverable 1 pipeline. This gives the binary the "print a config example"
acceptance criterion from #48. Acceptance: `pb docs config` is byte-identical to
`examples/config.toml`.

- [x] Map `Topic::Config` to `CONFIG_EXAMPLE` in `docs::run` (handled by D1's match,
      verify it is wired).
- [x] Test (source-vs-embed): `docs::CONFIG_EXAMPLE` equals
      `include_str!("../examples/config.toml")` read from the source tree.
- [x] CLI test: invoking the binary as `pb docs config` writes byte-identical
      `examples/config.toml` content to stdout and writes nothing to stderr.
- [x] Leave `config.rs:908` on its existing direct `include_str!("../examples/config.toml")`
      path (decided: do not repoint it at `docs::CONFIG_EXAMPLE`). Add a one-line comment
      that it intentionally reads the *same canonical file* as the docs embed, so the two
      are not a divergent second source. (Repointing would couple a config test to the
      `OUT_DIR` build pipeline for no benefit.)

### Deliverable 3. Discovery, drift/packaging guards, and doc wiring

Make the feature discoverable and self-protecting. Bare `pb docs` lists the available
topics (so an LLM can enumerate them); CI guards prevent a gutted or unpackaged embed.
Update user-facing help and the README. Acceptance: `pb docs` lists `syntax` and
`config`; `.github/workflows/unstable.yml` and `.github/workflows/release.yml` fail if
either canonical file leaves the published crate; `.gitattributes` pins the embedded
source files to LF; README and `--help` mention the command.

- [x] `pb docs` with no topic lists topics on stdout (`syntax`, `config`); CLI test
      asserts exact stdout (`syntax\nconfig\n`) and empty stderr.
- [x] Content-marker test: syntax output contains a known heading from the spec; config
      output contains a known table/key — catches a truncated embed.
- [x] Packaging guard as a required CI step in `.github/workflows/unstable.yml` and
      `.github/workflows/release.yml` (not a unit test, which can't cleanly shell out):
      run `cargo package --list --allow-dirty` and assert the output contains the exact
      paths `docs/SNIPPET_SYNTAX.md` and `examples/config.toml`. `--allow-dirty`
      avoids failing on an unrelated dirty worktree; exact-path match avoids substring
      false positives.
- [x] Add `.gitattributes` with LF rules for `docs/SNIPPET_SYNTAX.md` and
      `examples/config.toml`.
- [x] Update both `cli::after_help` and `README.md` to mention `pb docs syntax|config`.

## Issues

- **2026-06-18 — agent:codex (adversarial review)** — Plan reviewed by 2 adversarial
  sub-agents (Risks & Assumptions, Completeness & Scope). 5 distinct findings; all
  merged. Most significant: added command-level stdout/stderr tests for the actual
  binary, made the packaging guard target concrete in both GitHub Actions workflows,
  resolved the line-ending hedge with a `.gitattributes` task, required both README
  and CLI help updates, and removed the unresolved/nonexistent `CLAUDE.md` checklist
  item from scope.
- **2026-06-16 — agent:claude (adversarial review)** — Plan reviewed by 2 adversarial
  sub-agents (Risks & Assumptions, Completeness & Scope). 10 findings; all merged. Most
  significant: the byte-for-byte test must read the canonical file directly from the
  source tree (not the `OUT_DIR` copy) or it compares the embed against itself and
  catches no drift — the plan's headline guarantee. Also added: broken-pipe handling for
  piped output, `create_dir_all` for the OUT_DIR copy, line-ending/`.gitattributes`
  watch-for, `Cargo.toml` as a Critical File, a non-empty embed assertion, a concrete
  `cargo package --list --allow-dirty` CI guard, a resolved decision to leave
  `config.rs:908` on its direct path, and a note disambiguating the unrelated
  `src/syntax.rs` module.
- **2026-06-16 — agent:claude** — Manpages (issue #48 scope item) are deferred from
  this plan per the user's scoping decision. #48 cannot be closed by this work alone;
  a follow-up issue/sub-issue should cover `clap_mangen` generation + packaging. Filed
  here so it is not silently dropped. blocks: nothing in this plan.
- **2026-06-16 — agent:claude** — Verified packaging facts before drafting: `Cargo.toml`
  has no `include`/`exclude`; `cargo package --list` includes `docs/SNIPPET_SYNTAX.md`
  and `examples/config.toml`. The `build.rs` OUT_DIR copy approach was chosen by the
  user over direct `include_str!` from `docs/`/`examples/` and over moving canonical
  files into `assets/`.
