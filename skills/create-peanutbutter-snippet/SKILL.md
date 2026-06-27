---
name: create-peanutbutter-snippet
description: Author or revise Peanutbutter markdown snippets for the pb snippet manager. Use when the user asks to create, clean up, lint, organize, or explain pb snippets.
---

# Create Peanutbutter Snippet

Help users create executable Markdown snippets for Peanutbutter (`pb`). Keep snippets readable enough to share with people who do not know the tool, while preserving the exact syntax `pb` parses.

## First: Load the Syntax Reference

Before writing or changing snippets, prefer the canonical syntax reference from an installed `pb`:

```bash
pb docs syntax
```

Treat that reference as the source of truth. If this skill conflicts with it, follow the reference.

## Workflow

1. **Understand the command goal.** Identify the command, required inputs, safe defaults, target shell, and any assumptions about the current directory or tools.
2. **Choose the snippet location.** If the user has an existing snippet file or category, add to it. Otherwise suggest a small topical Markdown file such as `git.md`, `docker.md`, or `project.md`.
3. **Start new snippet files with frontmatter when useful.** Add file-level `name`, `description`, `tags`, and shared `variables` so the file searches well and repeated prompts have suggestions/defaults.
4. **Write one `##` section per executable snippet.** Use a short action-oriented heading.
5. **Add concise description text.** Explain when to use it, important assumptions, and any risky side effects.
6. **Use `text` fences only for preview examples.** A bare `text` fence is never the executable body.
7. **Put the executable command in the first non-`text` fenced block.** Use the user's shell for the fence and command syntax when known (`bash`, `zsh`, `fish`, `pwsh`, etc.); otherwise use `sh` for portable POSIX-style commands.
8. **Add placeholders for values the user should decide at execution time.** Use Peanutbutter placeholders instead of hardcoding local-only values.
9. **Validate with `pb lint`.** Use `pb lint --strict` when updating a reusable snippet collection.

## New File Shape With Frontmatter

For a new snippet file, include concise YAML frontmatter when it improves search, organization, or prompt reuse:

````markdown
---
name: Git helpers
description: Commands for everyday Git repository work
tags: [git, repo]
variables:
  branch:
    command: git branch --format='%(refname:short)'
  remote:
    default: origin
---

## Push branch

Push a local branch to a remote and set upstream tracking.

```bash
git push -u <@remote> <@branch>
```
````

Frontmatter reminders:

- `name`, `description`, and `tags` help search and display.
- `variables` defines file-local prompt behavior for placeholders used by snippets in the same file.
- Variable specs can provide `default`, fixed `suggestions`, or a shell `command` whose stdout lines become suggestions.
- Prefer file-local `variables` when several snippets share the same placeholder behavior.

## Snippet Shape

Use this basic form for each executable snippet:

````markdown
## Short action title

One or two sentences explaining when to use this command.

```bash
command --flag <@value>
```
````

For preview examples, keep the `text` block before the executable block:

````markdown
## Copy one path to another

Example input shown in the picker preview:

```text
source.txt -> destination.txt
```

```bash
cp <@source> <@destination>
```
````

## Syntax Reminders

- Only `##` headings start snippets.
- The snippet body is the first fenced code block below the `##` heading whose language is not bare `text`.
- Sections with only `text` fences are ignored as executable snippets.
- Placeholder names use ASCII letters, digits, `_`, and `-`.
- Free-form placeholder: `<@name>`
- Default value: `<@name:?default>`
- Suggestion command: `<@name:command>`
- Use dependent references only when the syntax reference says they are valid for that placeholder source.
- Optional YAML frontmatter can define file metadata and file-local variable specs.

## Authoring Guidelines

- Prefer boring, copyable shell over clever aliases or functions.
- Make commands location-aware only when that is useful; otherwise avoid absolute paths.
- Include safety flags for destructive commands where appropriate, or clearly state the risk in the description.
- Do not hide important side effects in the command body.
- Keep placeholder labels meaningful: `<@branch>`, `<@container>`, `<@pattern>`, not `<@x>`.
- Prefer frontmatter variable specs when several snippets in one file share the same suggestions or defaults.
- Do not invent syntax. Check `pb docs syntax` first when `pb` is available.

## Validation

When possible, run lint against the snippet path:

```bash
pb lint --strict
```

Before finalizing, check that:

- Every intended snippet has a `##` heading and an executable non-`text` fence.
- Preview-only examples use `text` fences.
- Placeholder syntax is valid.
- Descriptions are useful in the picker preview.
- Commands do not depend on private machine-specific aliases unless the user requested that.
