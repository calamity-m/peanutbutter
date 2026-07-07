# Peanutbutter

[![Unstable](https://github.com/calamity-m/peanutbutter/actions/workflows/unstable.yml/badge.svg)](https://github.com/calamity-m/peanutbutter/actions/workflows/unstable.yml)
[![Release](https://github.com/calamity-m/peanutbutter/actions/workflows/release.yml/badge.svg)](https://github.com/calamity-m/peanutbutter/actions/workflows/release.yml)
[![Latest Release](https://img.shields.io/github/v/release/calamity-m/peanutbutter)](https://github.com/calamity-m/peanutbutter/releases/latest)

A friendly terminal snippet management tool based on markdown, finally.

![Peanutbutter demo: search for a snippet, fill a variable, and insert the command into the shell buffer](assets/imgs/demo.gif)

<details>
<summary>More demos</summary>

### Find snippets three ways

Fuzzy search highlights matches across names, tags, paths, and command blocks; `Ctrl+t` switches to file and tag browsing.

![Peanutbutter demo: fuzzy search, file browsing, and tag browsing](assets/imgs/demo-modes.gif)

### Smarter prompts

Prompts can use suggestions, dependent values, and editable defaults.

![Peanutbutter demo: suggestions and dependent prompt defaults](assets/imgs/demo-prompts.gif)

### Save a command as a snippet

`pb new` opens a TUI over recent shell history, lets you choose placeholder tokens, and writes the snippet.

![Peanutbutter demo: capture a recent command with pb new](assets/imgs/demo-new.gif)

### Edit in flow

`Ctrl+E` opens the selected snippet in `$VISUAL` or `$EDITOR`, then returns to the refreshed picker.

![Peanutbutter demo: edit the selected snippet from the picker](assets/imgs/demo-edit.gif)

</details>

## Quick-Start

Install the latest release (Linux x86_64; installs to `~/.local/bin`):

```sh
curl -fsSL https://raw.githubusercontent.com/calamity-m/peanutbutter/main/scripts/install.sh | sh
```

Pin a version or change the destination with `PB_VERSION` / `PB_INSTALL_DIR`. Alternatively, grab a binary from [GitHub releases](https://github.com/calamity-m/peanutbutter/releases) directly or via a tool like mise:

```sh
mise use -g github:calamity-m/peanutbutter@latest
```

New here? Run `pb init` once to scaffold starter snippets at the default XDG location.

**bash** — add to `~/.bashrc`:

```bash
eval "$(peanutbutter completions bash)"
# then press Ctrl + b and have fun.
# also installs `pb` as a bash alias for `peanutbutter`
```

**zsh** — add to `~/.zshrc`:

```zsh
eval "$(peanutbutter completions zsh)"
```

**fish** — add to `~/.config/fish/config.fish`:

```fish
peanutbutter completions fish | source
```

**PowerShell** — add to `$PROFILE`:

```powershell
peanutbutter completions powershell | Invoke-Expression
```

## Why

I personally find that other cheatsheet or snippet tools don't adapt to my workflows, and cause me to constantly exit out of my own flow state.
Ideally a snippet/cheatsheet tool should feel natural and complete at "the speed of thought" or whatever the fuck that means - really just, allow me to express myself naturally.
Nothing can do that sadly, but peanutbutter tries to get close by understanding:

- Snippets need to be readable outside of the tool.
- Fuzzy finding is amazing, and allows you to narrow in a command you know exists, or feel out the existence of some shell script;
- But sometimes I have no idea what I want, just knowledge that I have something in my personal journal with a bajillion pages. Fuzzy finding through that is just going to frustrate me.
- Creating, updating and removing snippets should be considered as important as using them - so many snippet/cheatsheet managers are a bitch to create things in mid-workflow, or post-event when I'm already zonked out and braindead.

:shrug: maybe this is pointless but oh well.

## Modes

In an attempt to help address the above understandings, peanutbutter has three search modes for selecting snippets:

1. Fuzzy (Default) - basically like fzf/nucleo, just fuzzy matching - there is query syntax and overloading you can use once you're comfortable, as detailed in [FUZZY](/docs/FUZZY.md).
2. File-based - essentially a dumb file tree, which is counterintuitively, at least to me, sometimes more efficient for finding particular commands to run
3. Tag mode - tags are fun, we tag everything - why the fuck does nobody seem to make it possible to list and search by tags in a tree-like view?

In all modes, the basic concept of being able to backout is maintained - cycle with `Ctrl+T` safely, enter into snippets, half-fill them and press escape to go back to the viewer. You won't get stuck and can always go back.

## Curating, Editing and Maintaining Snippets

I personally hate having to interact with snippet/cheatsheet tools, I want the easiest and lowest cost way to edit, delete, add, or whatever, my snippets. I consider this curating them.

- In the picker, `Ctrl+E` opens the selected snippet in `$VISUAL` or `$EDITOR` at its heading line. When the editor exits, peanutbutter reloads snippets and returns to the picker.
- You can add snippets via the cli - `pb edit <tab-complete>`.
- After running a command you want to save, run `pb new [name]` — it harvests the last 50 entries from the shell's in-memory history, lets you pick one, suggests which arguments should become variables, and appends a snippet to `<first-root>/snippets.md`.
- If your snippets live in git repositories, `pb repo` opens a manager TUI listing every repo discovered under your snippet roots. From there `s` syncs (commit, pull --rebase, push), `p` pushes, `u` pulls, and `Enter` jumps into the repo with `$VISUAL`/`$EDITOR`. A snippet root with no git repository on or above it still appears so you can `Enter` to jump into it; sync/push/pull are disabled for it.

## LSP

Peanutbutter provides a bundled LSP to make life easier - [LSP](/docs/LSP.md) - setup for neovim is first-class, and at somepoint a shitty vscode extension may be created.

Preview (personal neovim setup):

![LSP Setup](assets/imgs/lsp-nvim.png)

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

There is a bundled skill in this repository that I personally use to get my AI to help me refine complex (really complex) snippets: [skills/create-peanutbutter-snippet/SKILL.md](skills/create-peanutbutter-snippet/SKILL.md).

## Snippet Specification

For the complete specification, see [docs/SNIPPET_SYNTAX.md](docs/SNIPPET_SYNTAX.md) or run `pb docs syntax`.

Snippets are really just **ANY** markdown file that follows the following structure:

A `##` heading, followed below by some fenced code wrapping block.

I recommend just [just reading through the examples](/examples/starter_snippets.md). The snippet syntax is fairly intuitive and you'll just pick it up by reading it. Benefit from that is you can show snippets to random coworkers/friends/whatever without asking them to use a new tool.

This snippet syntax lets you show your snippets to random coworkers, friends or what have you without asking them to understand much - the input variable syntax is fairly simple and close to self explanatory.

## Configuration

### Config File

Peanutbutter reads config from `~/.config/peanutbutter/config.toml` by default. You can override that path with `PB_CONFIG_FILE=/path/to/config.toml`.

This file is optional. If it doesn't exist, peanutbutter uses built-in defaults.

A fully commented example config lives at [examples/config.toml](examples/config.toml).

### Keybinds

The keys driving the `pb execute`, `pb settings`, and `pb new` TUIs are remappable through `pb settings` → `keybinds` or by editing `[keybinds.execute.*]`, `[keybinds.settings.*]`, and `[keybinds.new.*]` tables in the config file. Each screen's help line reflects your remaps, invalid bindings show as a non-fatal warning inside the TUI, and `Ctrl+C` always cancels regardless of config. In the interactive editor, `Esc` cancels key capture and `Ctrl+C` quits, so bare `esc` can only be restored via reset-to-default or TOML, and `ctrl+c` remains reserved. The commented reference in [examples/config.toml](examples/config.toml) (also shown by `pb docs config`) lists every action and its defaults.

### Snippet Paths

By default, peanutbutter looks for snippets in `~/.config/peanutbutter/snippets/`. Additional directories can be added via the `PEANUTBUTTER_PATH` environment variable, using colon-separated paths (same convention as `$PATH`). Snippet paths can also be added via configuration

For example, to also include the bundled `examples/` directory from this repo:

```bash
export PEANUTBUTTER_PATH="/path/to/peanutbutter/examples"
```

The XDG default (`~/.config/peanutbutter/snippets/`) is always included and doesn't need to be listed explicitly.
