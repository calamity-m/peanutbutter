# AGENTS.md

This provides context for this project. The README.md acts as a specification to follow.

## Commands

```bash
cargo build          # build
cargo run            # run (binary: peanutbutter)
cargo test           # run all tests
cargo test <name>    # run a single test by name
cargo clippy         # lint
cargo fmt            # format
```

## What This Is

`peanutbutter` is a terminal snippet manager with an inline TUI. Running `peanutbutter --bash` also installs a `pb` bash alias. The core value props:

1. **Location-aware frecency** — snippet rankings factor in the current working directory, not just frequency/recency globally
2. **Two navigation modes** — fuzzy search over snippet names/content/frontmatter, and a file-tree browser with tab-completion
3. **Shell buffer integration** — selected snippets are written into the terminal's input buffer (not executed directly), achieved via shell hotkey setup (`peanutbutter --bash C+b` outputs shell code for eval and installs the `pb` alias)
4. **Plain markdown format** — snippet files are readable without tooling

## Snippet Format

Snippets live in markdown files with optional YAML frontmatter:

```markdown
---
name: optional title
tags: [a, b]
description: shown in UI
---

## Snippet Name

Optional description text (markdown supported).

```shell
the-command <@var_name:options_cmd> --flag <@other:?default_value>
```
```

Rules:
- `##` heading = snippet name; first code block below it = the snippet
- A file can contain multiple snippets (multiple `##` sections)
- Files can be nested in directories; the directory tree is the browsable hierarchy
- See `examples/` for reference

### Variable Syntax `<@name:source>`

| Syntax | Meaning |
|---|---|
| `<@name>` | Free-form input, no suggestions |
| `<@name:cmd>` | Run `cmd` to populate suggestions list |
| `<@name:?default>` | Pre-populated default value |
| `<@file>` | Built-in: files in cwd |
| `<@directory>` | Built-in: directories in cwd |

## CLI Design

```
peanutbutter --bash C+b     # emit shell integration for ctrl+b hotkey (eval "$(...)")
peanutbutter add [path]     # open snippet file in $EDITOR/$VISUAL
peanutbutter del [name]     # delete a snippet
peanutbutter execute        # run the TUI inline; outputs selected command to stdout
```

`peanutbutter` with no args should be a no-op / help — the TUI is only useful inside the shell hotkey flow where the result gets written into the terminal buffer. After `--bash`, `pb` is available as a shorthand alias.

## Frecency Algorithm Intent

The ranking algorithm should weight:
1. **Location** — cwd context (e.g. snippets used in `~/my-repo` rank higher there)
2. **Recency** — more recent use scores higher
3. **Frequency** — high-use snippets (e.g. `git`) can override location weighting
