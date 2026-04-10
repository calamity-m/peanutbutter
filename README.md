# Peanutbutter

## User Experience

The idea behind peanutbutter is for it to feel natural at thought, requiring as little input/nudging to work as possible. This is basically impossible, but peanutbutter tries to get as close as it can:
    -> no snippet tools have a nice inline tui, with fuzzy/frecency algo searching
    -> snippet tools generally force users into obscure formats that can't be plainly read by others
    -> snippet tools need to understand there are multiple use-cases:
        1. A user knows what they want to do, but doesn't remember the command
        2. The user has no idea what they want to do, but they think they might have run a similar command before
        3. The user has an inclination of what they want to do, but are unsure how to go about it.

This leads me to believe peanutbutter should try to find a middleground of all three, rather than hyper-focusing on one. A user should be able to hit the hot-key in their terminal, e.g. ctrl + b and naturally arrive to where/what they want. As an example:

```
user@user:/home/working/development$___
-> I need to check git history, I know I have commands but I don't know which one off the top of my head. I could ctrl + r and search history. 
-> Ehh, nothing relevant.
-> User hits ctrl + b, and sees based on their location, frequency common commands
user@user:/home/working/development$___
> ...
| git-snippet1
| git-snippet2
| code-build-snippet1

-> Ahh ehh, I want compat -ish

user@user:/home/working/development$___
> compat g

| git-snippet1
| git-snippet5

-> o i want that one
-> user selects it, populates variables

user@user:/home/working/development$git-snippet1

-> presses enter, and it executes

```

Versus:

```
user@user:/home/working/development$___
-> idk, I need to convert a file here to something else.
-> ctrl + r has nothing, let me try ctrl + b
user@user:/home/working/development$___
> ...
| git-snippet1
| git-snippet2
| code-build-snippet1
-> nothing here is what i want... idk what i want. what do I even have? let me use a file style...
-> some hotkey change
> ...
| git/git-snippet1
|    /git-snippet2
| docker/docker-snippet-1
|       /compose/
|               /compose-snippet-1
| files/file-snippet1
|      /file-snippet2 
-> uhh i need files
-> the user types "f", and hits tab to autocomplete - they dont want to type
> f<tab> = files/
| file-snippet1
| file-snippet2
-> user selects, etc. and so on
```

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

## Peanutbutter CLI

The peanutbutter CLI by default, calling `pb` shouldn't really do anything. This is because we need the hot-key setup and shell completion to put the result into the terminal buffer for the user to execute.

Instead, `pb` should help the user with their usage of peanutbutter. For instance, we would want the following:

1. `pb --bash C+b` <-- create bash for ctrl+b hotkey, so I can put into my bashrc eval "$(...)"
2. `pb add ...` <-- add a snippet, opening the relevant snippet file in their $EDITOR/$VISUAL
3. `pb del ...` <-- delete a snippet
4. `pb execute` <-- run the inline tui for people who want to be explicit, and just execute the snippet they complete. doesn't have to go into bash buffer, but could be piped, e.g. pb execute | grep -i "a"
