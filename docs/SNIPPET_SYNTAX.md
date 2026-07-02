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
    suggestions:
      - GET
      - POST
      - PUT
      - PATCH
      - DELETE
  git_branch:
    command: git branch --format='%(refname:short)'
```

Each variable spec may include:

- `default` — pre-populates the prompt input.
- `suggestions` — fixed suggestion values.
- `command` — shell command whose stdout lines become suggestions.
- `hint` — ghost text shown while the prompt input is empty; display-only,
  never part of the value.

These specs apply only to snippets in the same markdown file. They do not
apply to other files. The peanutbutter LSP can refactor between inline
`<@name:?default>` / `<@name:command>` placeholders and equivalent
`variables.<name>` frontmatter specs with editor code actions.

Malformed or unsupported frontmatter variable specs are ignored during normal
execution. Run `pb lint` to validate supported frontmatter syntax and variable
expectations before opening the picker. Normal lint warns about variable specs
that no snippet references. `pb lint --strict` additionally warns about style
issues such as confusing file-local variable overrides.

## Snippets

Snippets can be authored manually in any markdown editor, or generated from a
recently-run shell command via `pb new` — see the README's
[`pb new` walkthrough](../README.md#pb-new-walkthrough) for the capture flow.

A snippet is defined by:

1. A level-two markdown heading (`##`).
2. Optional markdown description text below that heading.
3. The first fenced code block below that heading whose language tag is not bare `text`.

Example:

````markdown
## List matching files

Search files in the current repository.

```bash
rg <@pattern> <@file>
```
````

The heading text is the snippet name. The description text is shown in the UI
and participates in search. The first non-`text` fenced code block is the
executable snippet body.

Bare `text` fences are reserved for picker-visible examples in snippet
descriptions. They remain part of the description markdown and are rendered in
the preview, but they are ignored when peanutbutter chooses the executable
snippet body.

````markdown
## Copy one path to another

This example is shown in the picker preview:

```text
source.txt -> destination.txt
```

This is the executable snippet body:

```bash
cp <@source> <@destination>
```
````

If a section only has `text` fences, peanutbutter ignores it as an executable
snippet. This is deliberate: `text` is not a body language in peanutbutter, it
is the reserved language for preview-only examples. Existing executable snippets
that used ````text` should be migrated to an untagged fence or a different
language tag.

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

The source variants at a glance:

| Form                | Meaning            | Behavior when accepted without typing        |
| ------------------- | ------------------ | -------------------------------------------- |
| `<@name>`           | free-form input    | empty value                                  |
| `<@name:@hint>`     | ghost hint         | empty value; hint is display-only            |
| `<@name:?default>`  | editable pre-fill  | the default becomes the value                |
| `<@name:command>`   | suggestion command | selected suggestion (or empty with none)     |

Inside defaults and suggestion commands, `<#name>` and `<#name:raw>` reference
earlier confirmed values — see [Dependent Variables](#dependent-variables).

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

### Hint

```text
<@name:@hint>
```

Shows `hint` as ghost text in the prompt while the input is empty. The hint is
guidance only: as soon as the user types, the typed input replaces it, and
clearing the input back to empty shows it again. Accepting the prompt without
typing substitutes an **empty** value — the hint never reaches the rendered
command.

Example:

```sh
echo "<@input:@hello> world"
```

Accepting `input` without typing renders `echo " world"`. Compare with the
default form below, where accepting `<@input:?hello>` renders
`echo "hello world"`.

Hints can also come from a reusable spec (`variables.<name>.hint` in
frontmatter or `[variables.<name>] hint = "..."` in config) for free-form
placeholders. An inline hint takes precedence over reusable specs. `hint` is
independent from `default`: if both are configured, the default pre-fills the
editable buffer and the hint only becomes visible if the user clears it. Hints
are not suggestions and never appear as selectable suggestion rows.

### Default Value

```text
<@name:?default>
```

Pre-populates the prompt with `default`. The user can accept it or type a
different value. Defaults may include dependent references (`<#name>` and
`<#name:raw>`) using the same grammar, ordering rules, and lint checks as
suggestion commands; see [Dependent Variables](#dependent-variables).

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
still lets the user type a value manually. `pb lint` does not execute suggestion
commands; it only validates their static syntax and dependent-variable
references. Specific lint findings can be suppressed in config with
`[lint.<code>]` tables; for example,
`[lint.invalid-dependent-reference] ignore_command = "*rg*"` suppresses invalid
reference findings for matching command text.

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

1. Inline placeholder source (`<@target:?world>`, `<@target:@hint>`, or
   `<@target:command>`).
2. File-local frontmatter spec for that variable.
3. Config-defined variable spec.
4. Built-in suggestions for `file` and `directory`.

An inline hint only supplies the ghost text; suggestions and defaults for that
placeholder still resolve through the frontmatter/config/built-in chain as if
it were free-form.

File-local specs overlay config-defined specs by field. For example, if
frontmatter defines only `default` and config defines `suggestions`, the prompt
uses the frontmatter default and config suggestions. If frontmatter defines
either `suggestions` or `command`, that frontmatter suggestion source is used
instead of config suggestions or command.

### Dependent Variables

A suggestion command or default value may reference values the user has already
confirmed for earlier variables, using `<#name>` (shell-quoted) or
`<#name:raw>` (literal splice) tokens inside the command/default source. This
lets one variable's suggestion list or pre-filled default depend on another.

Worked example — pick a Kubernetes secret, then pick a key from that secret:

```sh
kubectl get secret <@secret:kubectl get secret -o name | sed 's#secret/##'> -o jsonpath="{.data.<@key:kubectl get secret <#secret> -o jsonpath='{.data}' | jq -r 'keys[]'>}"
```

When the user confirms `secret`, the `key` variable's command is re-evaluated
with `<#secret>` substituted, so its suggestions are the data keys of the
chosen secret.

Worked default example — pre-fill an output path from earlier confirmed picks:

```sh
kubectl get secret <@secret> -o jsonpath='{.data.<@key>}' | tee <@output:?<#namespace:raw>.<#secret:raw>.<#key:raw>.out>
```

`:raw` is natural for path-style construction because the shell-quoted form
would render pieces like `'<namespace>'.'<secret>'.out` instead of a plain file
name. Use it only when the upstream values are trusted or constrained.

**Quoting forms.**

- `<#name>` — substitutes the confirmed value shell-single-quoted. Embedded
  `'` characters become `'\''`. Safe against spaces, `;`, `$(...)`, and other
  shell metacharacters. This is the default and should be used whenever the
  value is an argument to a command.
- `<#name:raw>` — substitutes the confirmed value verbatim, with no quoting.
  Use this when the value itself is meant to provide shell syntax — typically
  the "command-as-variable" pattern:

  ```sh
  <@verb:?get pods> <@target:kubectl <#verb:raw> -o name>
  ```

  Here `verb=get pods` splices as two words into the `kubectl` invocation.

**Escaping a literal `<#name>`.** Prefix the opener with a backslash:
`\<#name>` renders as the literal text `<#name>` and does not count as a
dependent reference.

**Declaration order = dependency order.** A `<#name>` reference may only point
at a variable that appears *earlier* in the snippet's deduplicated prompt
order (the order the TUI tabs through). Frontmatter and config variable specs
do not create a separate dependency order — they overlay specs for variables
that appear in the body. Forward references and self-references are lint
errors.

**Confirmed values only.** Only values the user has confirmed (moved past with
Enter or Tab) are substituted. The in-flight input buffer is not visible to
downstream commands or defaults. If a default references an upstream that is
not currently confirmed, no dependent pre-fill is offered and the input buffer
is left empty. If the user revisits an upstream variable and changes its value,
dependent descendants become *dirty*: their text is preserved for editing, but
they are not treated as confirmed for later substitutions until the user
revisits and reconfirms them.

**Latency.** Each dependent step is bounded by the per-command timeout
(default 2000ms). Suggestion lists are cached per upstream snapshot, so
tabbing back to a downstream variable without changing the upstream does not
re-run the command. Even so, keep dependent commands cheap — chained slow
commands compound.

**Security caveat for `:raw`.** Because `<#name:raw>` does not quote, a value
containing shell metacharacters can change the semantics of the suggestion
command — including arbitrary command execution if the upstream value comes
from untrusted input. In defaults, `:raw` lands in the editable input buffer;
if the user accepts it, the raw text reaches the shell. Restrict `:raw`
consumption to variables whose upstream values come from a trusted or
constrained source (defaults, suggestion lists, or commands you control), not
free-form user input.

**Failure UX.** If a dependent suggestion command exits non-zero, the error
appears in the prompt status line and the user can still type a value or
Shift+Tab back to fix the upstream. Failed dependent commands are not
cached — revisiting the variable retries them.

**Lint codes.**

- `lint/unknown-variable-reference` — `<#name>` points at a name that is not
  declared in the snippet, frontmatter, or config.
- `lint/forward-variable-reference` — `<#name>` points at a variable that
  comes later in prompt order.
- `lint/self-variable-reference` — a variable's command or default references
  itself.
- `lint/invalid-dependent-reference` — the `<#...>` token cannot be parsed
  (unterminated, empty name, unknown modifier).
- `lint/raw-default-untrusted-upstream` — a default uses `<#name:raw>` to
  splice a free-form upstream value. Constrain the upstream, use `<#name>` for
  shell quoting, or suppress the lint if the raw default is intentional.

Existing command-based workarounds such as `<@name:echo <#a:raw>>` still work,
but `<@name:?<#a:raw>>` is preferred for defaults: it avoids spawning a
sub-shell and reads as a pre-fill rather than a suggestion command.

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
