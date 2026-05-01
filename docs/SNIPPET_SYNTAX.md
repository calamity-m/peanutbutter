# Snippet Syntax

This document specifies the markdown snippet format understood by
peanutbutter. The README describes the tool at a higher level; this file is the
more precise reference for writing snippets.

## Files

Snippet files are markdown files. A snippet path can point at one markdown file
or at a directory tree containing nested markdown files.

Files may include optional YAML frontmatter at the top:

```markdown
---
name: Docker helpers
tags: [docker, containers]
description: Commands for local container workflows
---
```

Supported frontmatter fields:

- `name` — optional display/search metadata for the file.
- `description` — optional display/search metadata for the file.
- `tags` — optional searchable tags, either inline (`[a, b]`) or as a YAML
  block list.

Unknown frontmatter keys are ignored.

## Snippets

A snippet is defined by:

1. A level-two markdown heading (`##`).
2. Optional markdown description text below that heading.
3. The first fenced code block below that heading.

Example:

````markdown
## List matching files

Search files in the current repository.

```bash
rg <@pattern> <@file>
```
````

The heading text is the snippet name. The description text is shown in the UI
and participates in search. The first fenced code block is the executable
snippet body.

### Multiple Snippets Per File

A single markdown file can contain multiple snippets. Each `##` section with a
fenced code block becomes one snippet.

````markdown
## First snippet

```sh
echo first
```

## Second snippet

```sh
echo second
```
````

Sections without a fenced code block are ignored as executable snippets.

### Heading Levels

Only `##` headings start snippets. Other heading levels are ordinary markdown
content.

```markdown
# File title

## Real snippet

### Description subheading
```

In the example above, `Real snippet` starts a snippet. `File title` and
`Description subheading` do not.

## Variable Placeholders

Snippet bodies can include placeholders with this general form:

```text
<@name>
<@name:source>
```

The placeholder name is shown to the user as the prompt label. Valid names may
contain ASCII letters, ASCII digits, `_`, and `-`.

Examples:

```sh
echo <@message>
grep <@pattern> <@file>
```

### Free-form Input

```text
<@name>
```

Prompts the user for a value. No inline default or inline suggestion command is
attached to the placeholder.

Example:

```sh
echo <@input>
```

### Default Value

```text
<@name:?default>
```

Pre-populates the prompt with `default`. The user can accept it or type a
different value.

Example:

```sh
ls -lsha <@path:?.>
```

### Suggestion Command

```text
<@name:command>
```

Runs `command` to populate the prompt's suggestion list. The user can pick a
suggestion or type a different value.

Example:

```sh
cat <@file:rg . --files>
```

Suggestion commands are executed with non-login, non-interactive `bash -c` in
the current working directory. Output is split into suggestions by real
newlines and literal `\n` sequences. Blank suggestions are ignored.

If a suggestion command fails, peanutbutter shows the error but still lets the
user type a value manually.

### Built-in Variable Names

Some free-form variable names have built-in suggestions:

- `<@file>` — files in the current working directory.
- `<@directory>` — directories in the current working directory.

Config-defined variables can also provide defaults or suggestions for
free-form placeholders such as `<@http_method>`.

Inline placeholder sources take precedence over config-defined variables. For
example, `<@target:?world>` uses the inline default rather than a configured
default for `target`.

### Multi-line Values

When filling in a variable, the value can span multiple lines:

- Press **Alt+Enter** (or **Ctrl+J** on terminals that don't deliver Alt+Enter
  as a distinct key) to insert a literal newline into the current value.
- **Paste** preserves newlines: pasting a multi-line block from the clipboard
  inserts it verbatim as a single multi-line value.
- Plain **Enter** still submits the current value and advances to the next
  variable.

This is useful for prompt-template snippets where a variable holds a
multi-paragraph prompt.

## Reusing Variable Values

Placeholders with the same name are prompted once. Every occurrence of that
name is rendered with the same user-provided value.

Example:

```sh
echo "<@input>, how are you? <@input> is your name."
```

The user is prompted for `input` once. If they enter `Sam`, the rendered command
is:

```sh
echo "Sam, how are you? Sam is your name."
```

This behavior is name-based. Peanutbutter does not currently have separate
"define" and "reference" placeholder syntax. Repeating `<@input>` means "reuse
the same value for `input`", not "ask for a new value named `input` again".

Example:

```sh
echo "<@input> hello <@input>"
```

The user is still prompted once for `input`.

If duplicate placeholders with the same name use different sources, the first
occurrence determines the prompt behavior.

Example:

```sh
echo "<@target:?world> <@target:printf 'prod\nstage\n'>"
```

The user is prompted once for `target`, using the first placeholder's default
value. Both occurrences render with the chosen value.

## Rendering Rules

When a snippet is selected:

1. Peanutbutter parses placeholders from the snippet body from left to right.
2. It deduplicates placeholders by name, preserving the first occurrence.
3. It prompts once per unique variable name.
4. It replaces every resolved placeholder occurrence with the stored value for
   that name.
5. Any unresolved or malformed placeholder text is left unchanged.

Malformed placeholders include empty names or names with unsupported
characters.

Examples of unsupported placeholder names:

```text
<@>
<@has space>
<@has/slash>
```

## Complete Example

````markdown
---
name: Git snippets
tags:
  - git
---

## Commit all changes

Stage all changes and commit with a message.

```sh
git add <@path:?.> && git commit -m "<@message>"
```

## Search tracked files

```sh
git grep <@pattern> -- <@file>
```
````

The first snippet prompts for `path` with `.` as the default, then prompts for
`message`. The second snippet prompts for `pattern`, then prompts for `file`
with built-in file suggestions from the current working directory.
