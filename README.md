# Peanutbutter

## User Experience

The idea behind peanutbutter is for it to feel natural at thought, requiring as little input/nudging to work as possible. This is basically impossible, but peanutbutter tries to get as close as it can:
    -> no snippet tools have a nice inline tui, with fuzzy/frecency algo searching
    -> the frecency algorithm should take location into account, which nobody seems to do
        1. for example, snippets run in ~/Downloads is probably different to /tmp, or a common /../my-repo/. Weighting this may make life easier.
        2. Frequency should still be an overrider - e.g. git may show up due to high frequency if we have a snippet for git we constantly use, same with recency.
        3. Fuzzy finding should be over the name of the snippet (The `##` heading), the snippet itself, and then the snippet file' frontmatter.
    -> snippet tools generally force users into obscure formats that can't be plainly read by others
    -> snippet tools need to understand there are multiple use-cases:
        1. A user knows what they want to do, but doesn't remember the command
        2. The user has no idea what they want to do, but they think they might have run a similar command before
        3. The user has an inclination of what they want to do, but are unsure how to go about it.

This leads me to believe peanutbutter should try to find a middleground of all three, rather than hyper-focusing on one. A user should be able to hit the hot-key in their terminal, e.g. ctrl + b and naturally arrive to where/what they want.

This essentially creates two modes - similar to fzf where you might have a "file find", versus a "content fuzzy match". In my opinion an ergonomic snippet tool in the terminal handles both, easily swapping between the two. A user should be able to fumble around until they arrive at something they are familar with, at which point they can take control and find exactly what they want. For example, tab completion/backspacing until they find relevant areas, then quickly honing in on the snippet they want. Another example may be fuzzy searching different terms, until they narrow it down to the sinppet they want - but after selecting it they don't like it, so backspacing enough should take them back to the snippet list TUI with their search at the same spot before they entered the snippet, so they can select the down arrow and try the second, etc.

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
export PEANUTBUTTER_PATH="$HOME/code/peanutbutter/examples"
```

The XDG default (`~/.config/peanutbutter/snippets/`) is always included and doesn't need to be listed explicitly.

## Peanutbutter CLI

The peanutbutter CLI by default, calling `pb` shouldn't really do anything. This is because we need the hot-key setup and shell completion to put the result into the terminal buffer for the user to execute.

Instead, `pb` should help the user with their usage of peanutbutter. For instance, we would want the following:

1. `pb --bash C+b` <-- create bash for ctrl+b hotkey, so I can put into my bashrc eval "$(...)"
2. `pb add ...` <-- add a snippet, opening the relevant snippet file in their $EDITOR/$VISUAL
3. `pb del ...` <-- delete a snippet
4. `pb execute` <-- run the inline tui for people who want to be explicit, and just execute the snippet they complete. doesn't have to go into bash buffer, but could be piped, e.g. pb execute | grep -i "a"
