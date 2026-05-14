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
variables:
  container:
    command: docker ps --format '{{.Names}}'
  environment:
    default: dev
    suggestions: [dev, stage, prod]
---
```

Supported frontmatter fields:

- `name` — optional display/search metadata for the file.
- `description` — optional display/search metadata for the file.
- `tags` — optional searchable tags, either inline (`[a, b]`) or as a YAML
  block list.
- `variables` — optional file-local variable specs keyed by placeholder name.

Unknown frontmatter keys are ignored.

### File-local Variable Specs

Frontmatter can declare reusable input behavior for free-form placeholders in
that file:

```yaml
variables:
  http_method:
    suggestions: [GET, POST, PUT, PATCH, DELETE]
  git_branch:
    command: git branch --format='%(refname:short)'
```

Each variable spec may include:

- `default` — pre-populates the prompt input.
- `suggestions` — fixed suggestion values.
- `command` — shell command whose stdout lines become suggestions.

These specs apply only to snippets in the same markdown file. They do not
apply to other files.

Malformed or unsupported frontmatter variable specs are ignored during normal
execution. Run `pb lint` to validate supported frontmatter syntax and variable
expectations before opening the picker. Normal lint warns about variable specs
that no snippet references. `pb lint --strict` additionally warns about style
issues such as undeclared manual placeholders and confusing file-local variable
overrides.

## Snippets

Snippets can be authored manually in any markdown editor, or generated from a
recently-run shell command via `pb new` — see the README's
[`pb new` walkthrough](../README.md#pb-new-walkthrough) for the capture flow.

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
attached to the placeholder. This is valid for values that need human context;
normal `pb lint` does not require every free-form placeholder to have a matching
frontmatter or config definition.

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

If a suggestion command fails or times out, peanutbutter shows the error but
still lets the user type a value manually. `pb lint` also executes suggestion
commands to verify them, using the configured timeout; when command execution is
disabled, lint reports skipped command-backed variables as warnings instead.
Specific lint findings can be suppressed in config with `[lint.<code>]` tables;
for example, `[lint.suggestion-command-failed] ignore_command = "*rg*"` ignores
expected `rg` failures for that lint.

#### Timeouts and opt-out

Two `[suggestion_commands]` config options control command execution:

```toml
[suggestion_commands]
# Maximum time a suggestion command may run (milliseconds). Default: 2000.
# Commands that exceed this limit are killed; the variable falls back to
# manual input.
timeout_ms = 2000

# Set to false to disable all suggestion command execution. Variables will
# fall back to their static suggestions or manual input.
allow_commands = true
```

### Built-in Variable Names

Some free-form variable names have built-in suggestions:

- `<@file>` — files in the current working directory.
- `<@directory>` — directories in the current working directory.

Config-defined variables can also provide defaults or suggestions for
free-form placeholders such as `<@http_method>`.

Frontmatter-defined variables can also provide defaults or suggestions for
free-form placeholders in the same file. Resolution order is:

1. Inline placeholder source (`<@target:?world>` or `<@target:command>`).
2. File-local frontmatter spec for that variable.
3. Config-defined variable spec.
4. Built-in suggestions for `file` and `directory`.

File-local specs overlay config-defined specs by field. For example, if
frontmatter defines only `default` and config defines `suggestions`, the prompt
uses the frontmatter default and config suggestions. If frontmatter defines
either `suggestions` or `command`, that frontmatter suggestion source is used
instead of config suggestions or command.

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
