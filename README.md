# Peanutbutter

A command-line snippet tool that tries to mould to your own use-cases.

## Quick-Start

```bash
export PEANUTBUTTER_PATH="$PWD/examples"
# peanutbutter --bash C+b
# peanutbutter --bash C+f
# etc.
eval "$(peanutbutter --bash)"
# also installs `pb` as a bash alias for `peanutbutter`
```

## Why

I personally find that other cheatsheet or snippet tools don't adapt to my workflows, and cause me to constantly exit out of my own flow state.
Ideally a snippet/cheatsheet tool should feel natural and complete at "the speed of thought" or whatever the fuck that means - really just, allow me to express myself naturally.
Nothing can do that, but peanutbutter tries to do that through understanding:

- Snippets need to be readable outside of the tool.
- Fuzzy finding is amazing, and allows you to narrow in a command you know exists, or feel out the existence of some shell script; but:
- Sometimes I have no idea what I want, only a feeling. Sometimes I want a structured way to look through my collection.

## Alternatives

There are alternatives to this, as this isn't a unique or new problem. These alternatives probably do this concept better, but they just don't hit every single note I'm looking for.

- `denisidoro/navi` is probably the best, and does most of this tool but better - but has some minor annoyances around searching and the specification for cheats that irks me.
- `fzf` can do this with a bit of bash-fu - frequency/recency is harder to tune. 
- `alexpasmantier/television` with a cheatsheet channel. This can be pretty nice, but I didn't like how restricted I felt in channel definitions
- the "mega" cheatsheet things like tl;dr, but I don't really want 90k commands I'll never use, in formats I don't like.

## Modes

Peanutbutter has a fuzzy find mode, and a structured "file-list" mode. 

- Fuzzy finding is basically like fzf, I just stand on the back of helix's `nucleo-matcher` crate.
- File-list mode lets you walk through your snippets as they're structured via directories and files. I find this useful when I need inspiration on what I want to do. It's hard to explain,
but when you __know__ you need to do something, but you want to explore what you have.

:shrug: maybe this is pointless but oh well.

## AI Disclaimer

This tool has been [VIBED] with direction from myself. It's a tool I don't care enough about to craft myself painstakingly after work and on my weekends, 
but enough that I've thought about it for years.

Code probably shit - but the code would be shit if I wrote every single line myself too. Nothing in the repo would cause a `CVE-999-999` so feel safe to use it,
or burn it at the stake.

## Snippet Specification

Snippets are really just **ANY** markdown file that follows the following structure:

A `##` heading, followed below by some ``` ``` code wrapping block. If multiple code wrapping blocks
are present, only the first will be considered the snippet. Otherwise, anything between the code wrapping block
and the heading is considered description/preview data. 

Here are the following rules:

1. A snippet file can be any markdown file, or directory containing nested markdown files
2. A snippet is defined by a preceeding `##` heading, and some ``` ``` code wrapping block
3. Snippets have variable input syntax of <@AAA:BBB>, where AAA is the name of the input, which
is shown to the user/provide context to what to input, and BBB is the pre-seeded options the user
can select from. A user can enter something completely different however, as BBB is just a hint/options
list.
4. Variable input syntax is extended with `:?`, which denotes a default/pre-populated option.
5. Common variable inputs are setup by default, such as:
    <@file> <-- list of files in the current working directory
    <@directory> <--- list of directories in the current working directory
6. Predefined variable inputs can be defined by the user in peanutbutter's config file

## Configuration

### Snippet Paths

By default, peanutbutter looks for snippets in `~/.config/peanutbutter/snippets/`. Additional directories can be added via the `PEANUTBUTTER_PATH` environment variable, using colon-separated paths (same convention as `$PATH`).

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

A fully commented example config lives at [examples/config.toml](/home/calam/code/peanutbutter/examples/config.toml).

Example:

```toml
[paths]
snippets = [
  "/home/me/work-snippets",
  "/home/me/personal-snippets",
]
state_file = "/home/me/.local/state/peanutbutter/state.tsv"

[ui]
height = 18

[search]
frecency_weight = 250.0

[search.frecency]
half_life_days = 14.0
location_weight = 1.0
frequency_weight = 1.0

[search.fuzzy]
name = 30
tag = 20
frontmatter_name = 15
description = 10
path = 10
body = 8

[theme]
accent = "red"
muted = "dark_gray"
selected_bg = "#30343f"
selected_fg = "white"
prompt_active_fg = "black"
prompt_active_bg = "#f4d35e"
error_fg = "red"

[variables.http_method]
default = "GET"
suggestions = ["GET", "POST", "PUT", "PATCH", "DELETE"]

[variables.kube_context]
command = "kubectl config get-contexts -o name"
```

Notes:

- `paths.snippets` adds extra snippet roots, alongside `PEANUTBUTTER_PATH` and the default XDG snippets directory
- `ui.height` controls the maximum inline TUI height
- `search.frecency_weight` controls how much frecency influences the combined search ordering
- `search.frecency.*` tunes the time/location/frequency balance for ranking
- `search.fuzzy.*` tunes how much each snippet field contributes to fuzzy matching
- `theme.*` currently controls the main selection and prompt colors; colors accept common names like `red`, `dark_gray`, `white`, or `#RRGGBB`
- `variables.<name>` defines reusable inputs for free-form placeholders like `<@http_method>` or `<@kube_context>`
- A configured variable can provide either `suggestions = [...]` or `command = "..."`, and may also provide `default = "..."`

## Peanutbutter CLI

1. `peanutbutter --bash C+b` <-- create bash for ctrl+b hotkey, so I can put into my bashrc eval "$(...)"
2. `peanutbutter add ...` <-- add a snippet, opening the relevant snippet file in their $EDITOR/$VISUAL
3. `peanutbutter del ...` <-- delete a snippet
4. `peanutbutter execute` <-- run the inline tui for people who want to be explicit, and just execute the snippet they complete. doesn't have to go into bash buffer, but could be piped, e.g. peanutbutter execute | grep -i "a"

After `eval "$(peanutbutter --bash)"`, bash also gets a `pb` alias that points at `peanutbutter`.
