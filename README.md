# Peanutbutter

[![Unstable](https://github.com/calamity-m/peanutbutter/actions/workflows/unstable.yml/badge.svg)](https://github.com/calamity-m/peanutbutter/actions/workflows/unstable.yml)
[![Release](https://github.com/calamity-m/peanutbutter/actions/workflows/release.yml/badge.svg)](https://github.com/calamity-m/peanutbutter/actions/workflows/release.yml)
[![Latest Release](https://img.shields.io/github/v/release/calamity-m/peanutbutter)](https://github.com/calamity-m/peanutbutter/releases/latest)

A friendly terminal snippet management tool.

## Quick-Start

**bash** — add to `~/.bashrc`:

```bash
export PEANUTBUTTER_PATH="$PWD/examples"
eval "$(peanutbutter bash)"
# then press Ctrl + b and have fun.
# also installs `pb` as a bash alias for `peanutbutter`
```

**zsh** — add to `~/.zshrc`:

```zsh
export PEANUTBUTTER_PATH="$PWD/examples"
eval "$(peanutbutter zsh)"
```

**fish** — add to `~/.config/fish/config.fish`:

```fish
set -x PEANUTBUTTER_PATH "$PWD/examples"
peanutbutter fish | source
```

**PowerShell** — add to `$PROFILE`:

```powershell
$env:PEANUTBUTTER_PATH = "$PWD/examples"
peanutbutter powershell | Invoke-Expression
```

## Why

I personally find that other cheatsheet or snippet tools don't adapt to my workflows, and cause me to constantly exit out of my own flow state.
Ideally a snippet/cheatsheet tool should feel natural and complete at "the speed of thought" or whatever the fuck that means - really just, allow me to express myself naturally.
Nothing can do that sadly, but peanutbutter tries to get close by understanding:

- Snippets need to be readable outside of the tool.
- Fuzzy finding is amazing, and allows you to narrow in a command you know exists, or feel out the existence of some shell script;
- But sometimes I have no idea what I want, just knowledge that I have something in my personal journal with a bajillion pages. Fuzzy finding through that is just going to frustrate me.

## Modes

Peanutbutter has fuzzy find, structured "file-list", and tag modes. `Ctrl+T` cycles through them in the picker: fuzzy -> file-list -> tags -> fuzzy.

- Fuzzy finding is basically like fzf, I just stand on the back of helix's `nucleo-matcher` crate.
- Fuzzy mode supports these field operators for precise searches: `name:`, `path:`, `tag:`, and `snippet:`. Operators can be mixed with ordinary fuzzy text, so `tag:docker logs` means docker-tagged snippets that also match `logs` somewhere in the normal searchable text. `snippet:` searches the executable snippet code; `body:` is accepted as a compatibility alias.
- File-list mode lets you walk through your snippets as they're structured via directories and files. I find this useful when I need inspiration on what I want to do. It's hard to explain, but when you **know** you need to do something, but you want to explore what you have.
- Tags mode lists frontmatter tags with snippet counts. Type to filter tags, press Enter to drill into snippets for the highlighted tag, then type again to filter that snippet list by name. In the drilled list, Backspace clears the snippet filter before returning to tags; Esc returns to tags immediately. Snippets with no tags appear in `(untagged)`, and snippets with multiple tags count once under each tag.

## Curating, Editing and Maintaining Snippets

I personally hate having to interact with snippet/cheatsheet tools, I want the easiest and lowest cost way to edit, delete, add, or whatever, my snippets. I consider this curating them.

- In the picker, `Ctrl+E` opens the selected snippet in `$VISUAL` or `$EDITOR` at its heading line. When the editor exits, peanutbutter reloads snippets and returns to the picker.
- You can add snippets via the cli - `pb edit <tab-complete>`.
- After running a command you want to save, run `pb new [name]` — it harvests the last 50 entries from the shell's in-memory history, lets you pick one, suggests which arguments should become variables, and appends a snippet to `<first-root>/snippets.md`.

### `pb new` walkthrough

```text
$ ssh root@10.0.0.4 'systemctl restart nginx'
$ pb new deploy
┌─ pb new: pick a command ───────────────────────────────┐
│ ▸ ssh root@10.0.0.4 'systemctl restart nginx'          │
│   docker run -e API_TOKEN=xyz... my/image              │
│   ...                                                  │
└────────────────────────────────────────────────────────┘
↑↓/jk move   enter pick   type to filter   esc cancel

┌─ pb new ───────────────────────────────────────────────┐
│ Name: deploy                                           │
│ Preview:                                               │
│   ssh root@<@host> '<@value>'                          │
│ Tokens:                                                │
│   ▸ [x] 10.0.0.4                → host                 │
│     [x] systemctl restart nginx → value                │
└────────────────────────────────────────────────────────┘
space toggle   e rename   n name   enter accept   b back   esc cancel
```

`pb new` requires the shell integration to be sourced (`eval "$(peanutbutter bash C+b)"` or its zsh/fish/PowerShell equivalent) — that's what populates the history list. You can skip the history step entirely by passing a command after `--`:

```bash
pb new deploy -- ssh root@host 'systemctl restart nginx'
```

#### Privacy

`pb new` reads the parent shell's in-memory history list (not `$HISTFILE`). Anything you accept in the confirm screen is written verbatim into the target snippet file. Heuristics try to flag likely secrets (`--token=...`, `--password=...`, `Authorization: Bearer ...`, long base64-shaped tokens) and select them on by default so you notice; a warning fires if you accept a snippet with a flagged secret still literal. Even so: avoid typing real secrets on the command line.

#### v1 limits

- Writes to `<first snippet root>/snippets.md` only.
- History capture is line-oriented; multi-line commands (heredocs, backslash continuations) only round-trip cleanly when supplied via `--`.
- History payload is byte-capped (~64 KiB) before exec; very large entries may be dropped oldest-first.
- Two concurrent `pb new` runs racing on the same file are last-writer-wins.

:shrug: maybe this is pointless but oh well.

## Alternatives

There are alternatives to this, as this isn't a unique or new problem. These alternatives probably do this concept better, but they just don't hit every single note I'm looking for.

- `denisidoro/navi` is probably the best, and does most of this tool but better - but has some minor annoyances around searching and the specification for cheats that irks me.
- `fzf` can do this with a bit of bash-fu - frequency/recency is harder to tune.
- `alexpasmantier/television` with a cheatsheet channel. This can be pretty nice, but I didn't like how restricted I felt in channel definitions
- the "mega" cheatsheet things like tl;dr, but I don't really want 90k commands I'll never use, in formats I don't like.

## AI Disclaimer

This tool has been [VIBED] with direction from myself. It's a tool I don't care enough about to craft myself painstakingly after work and on my weekends,
but enough that I've thought about it for years.

Code probably shit - but the code would be shit if I wrote every single line myself too. Use it like I do, or burn it at the stake. You have free-will right? ;)

## Snippet Specification

For a stricter syntax reference, see [docs/SNIPPET_SYNTAX.md](docs/SNIPPET_SYNTAX.md).

Snippets are really just **ANY** markdown file that follows the following structure:

A `##` heading, followed below by some ` ` code wrapping block. If multiple code wrapping blocks
are present, only the first will be considered the snippet. Otherwise, anything between the code wrapping block
and the heading is considered description/preview data.

I recommend just [just reading through the examples](/examples/simple/snippets.md)

This snippet syntax lets you show your snippets to random coworkers, friends or what have you without asking them to understand much - the input variable syntax is fairly simple and close to self explanatory.

## Configuration

### Snippet Paths

By default, peanutbutter looks for snippets in `~/.config/peanutbutter/snippets/`. Additional directories can be added via the `PEANUTBUTTER_PATH` environment variable, using colon-separated paths (same convention as `$PATH`). [Snippet paths can also be added via configuration](#config-file)

For example, to also include the bundled `examples/` directory from this repo:

```bash
export PEANUTBUTTER_PATH="/path/to/peanutbutter/examples"
```

Or to try out the examples without moving any files, add this to your `~/.bashrc` or `~/.zshrc`:

```bash
export PEANUTBUTTER_PATH="$PWD/examples"
```

The XDG default (`~/.config/peanutbutter/snippets/`) is always included and doesn't need to be listed explicitly.

### Config File

Peanutbutter reads config from `~/.config/peanutbutter/config.toml` by default. You can override that path with `PB_CONFIG_FILE=/path/to/config.toml`.

This file is optional. If it doesn't exist, peanutbutter uses built-in defaults.

A fully commented example config lives at [examples/config.toml](examples/config.toml).

Notes:

- `paths.snippets` adds extra snippet roots, alongside `PEANUTBUTTER_PATH` and the default XDG snippets directory
- `ui.height` controls the maximum inline TUI height
- `search.frecency_weight` controls how much frecency influences the combined search ordering
- `search.frecency.*` tunes the time/location/frequency balance for ranking
- `search.fuzzy.*` tunes how much each snippet field contributes to fuzzy matching
- `theme.name` selects a built-in theme (`default`, `gruvbox`, `catppuccin`, `nord`, or `monochrome`); `theme.*` color values override that base and accept common names like `red`, `dark_gray`, `white`, or `#RRGGBB`. Use `--theme <name>` to select a clean named theme from the CLI.
- `variables.<name>` defines reusable inputs for free-form placeholders like `<@http_method>` or `<@kube_context>`
- A configured variable can provide either `suggestions = [...]` or `command = "..."`, and may also provide `default = "..."`
- Suggestion commands (both inline `<@name:cmd>` and `[variables.name] command = "..."`) run under non-login, non-interactive `bash -c`. They inherit `$PATH` from peanutbutter's parent process but do **not** source `~/.bash_profile` or `~/.bashrc`, so they can't use shell aliases or functions defined there. This is deliberate: a login shell's startup output (e.g. `Agent pid NNNN` from ssh-agent) would otherwise leak into the suggestion list, and any interactive prompt it triggers (e.g. an ssh-add passphrase) would hang the TUI.
- `suggestion_commands.timeout_ms` caps how long any suggestion command may run (default `2000` ms); commands that exceed it are killed and the variable falls back to manual input
- `suggestion_commands.allow_commands` set to `false` disables all suggestion command execution globally — variables fall back to static suggestions or manual input (useful when importing untrusted snippet collections)
- `lint.<code>` config can disable or suppress specific lint findings. Use the lint code without the `lint/` prefix, e.g. `[lint.suggestion-command-failed]`. `disable = true` drops that lint entirely, `ignore_file` matches snippet paths relative to their snippet root, and `ignore_command` matches command text for command-backed lint findings. `ignore_file` and `ignore_command` accept either one glob string or a list of glob strings.

```toml
[lint.suggestion-command-failed]
ignore_command = "*rg*"
ignore_file = ["test*", "fixtures/*"]
disable = false
```

## Peanutbutter CLI

1. `peanutbutter bash [C+b]` — emit bash integration script (eval it in `~/.bashrc`)
2. `peanutbutter zsh [C+b]` — emit zsh integration script (eval it in `~/.zshrc`)
3. `peanutbutter fish [C+b]` — emit fish integration script (source it in `config.fish`)
4. `peanutbutter powershell [C+b]` — emit PowerShell/PSReadLine integration script (add it to `$PROFILE`)
5. `peanutbutter edit ...` — edit a snippet file in `$EDITOR`/`$VISUAL`
6. `peanutbutter lint [--strict] [--json]` — check configured snippet roots for authoring problems
7. `peanutbutter execute [--theme <name>]` — run the inline TUI; output can be piped, e.g. `peanutbutter execute | grep foo`
8. `peanutbutter lsp` — start a Language Server Protocol server over stdio for editor integration (diagnostics, completions, hover, go-to-definition). See [docs/LSP.md](docs/LSP.md) for setup.

All shell integrations install a `pb` alias and wire up `pb edit <TAB>` plus `--theme <TAB>` completion.

`pb lint` is read-only. It reports broken frontmatter, unused frontmatter/config variables, duplicate snippet slugs, suggestion-command failures, dry-run frecency GC orphans, and obvious static inline suggestion commands. Bare manual placeholders like `<@value>` are valid in normal mode. `--strict` adds style/structure checks such as undeclared manual placeholders, unbalanced fences, missing code-fence language tags, and confusing file-local variable overrides. Pretty output is written to stdout by default; `--json` writes a parseable object with stable `findings[].code` fields. Exit codes are `0` for no findings, `1` for lint findings, and `2` for operational failures that prevent lint from running.

Important caveat: lint executes suggestion commands to verify them, using the same non-login `bash -c` behavior and configured timeout as runtime. If `suggestion_commands.allow_commands = false`, lint skips those commands and reports warning findings instead. Frecency GC checks are dry-run only and never reattach, purge, save, or write backups.

Pretty output example:

```text
/path/to/snippets.md
  warning:3 lint/unused-variable: frontmatter variable 'env' is not referenced by any snippet in this file
```

JSON output example:

```json
{
  "findings": [
    {
      "severity": "warning",
      "code": "lint/unused-variable",
      "path": "/path/to/snippets.md",
      "line": 3,
      "message": "frontmatter variable 'env' is not referenced by any snippet in this file"
    }
  ]
}
```
