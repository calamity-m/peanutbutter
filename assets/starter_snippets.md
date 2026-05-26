---
tags: [starter, git, files, examples]
description: A curated starter set — real commands, plus a tour of peanutbutter's placeholder syntax. Edit, extend, or replace freely.
variables:
  type:
    suggestions:
      - feat
      - fix
      - chore
      - docs
      - refactor
      - test
  branch:
    command: git branch --format='%(refname:short)'
  remote:
    command: git remote
---

# Peanutbutter starter snippets

These are real commands, not toys — they're meant to be useful on day one and to show off what peanutbutter can do. Edit, delete, or replace this file as you build your own collection. See `docs/SNIPPET_SYNTAX.md` for the full syntax reference, and run `pb edit` to jump straight into editing.

## Conventional commit

Commits with a `type(scope): subject` prefix. The `type` placeholder is backed by a frontmatter suggestion list — open the picker and you'll see the choices. `scope` and `subject` are free-form.

```bash
git commit -m "<@type>(<@scope>): <@subject>"
```

## Switch to an existing branch

The `branch` placeholder is backed by a frontmatter command (`git branch ...`), so the picker shows your real local branches. You can still type a new branch name to create one.

```bash
git switch <@branch>
```

## Delete local branches whose remote is gone

Prunes stale tracking refs, then deletes any local branch whose upstream has been removed. Handy after merging PRs that auto-delete their source branch.

```bash
git fetch --prune
git branch -vv | awk '/: gone]/{print $1 == "*" ? $2 : $1}' | xargs -r git branch -D
```

## Fetch main into local main without switching

Updates your local `main` from `<@remote>` while you stay on your feature branch. No checkout, no stash dance.

```bash
git fetch <@remote> main:main
```

## Decode a Kubernetes secret value

The kind of multi-step lookup peanutbutter is built for, using **dependent
variables** (`<#name>`). Each picker narrows the next:

- `<@namespace>` is picked from a live `kubectl get ns` list.
- `<@secret>` lists only the secrets in the chosen namespace, because its
  command references `<#namespace>`.
- `<@key>` lists only the data keys of the chosen secret, because its command
  references both `<#namespace>` and `<#secret>`.
- `<@output>` pre-fills a save path using `<#name:raw>` splices of all three
  upstream values — e.g. `prod.db-creds.password.out`. The decoded value is
  written to that file via `tee` and also printed to the terminal.

Default `<#name>` substitution shell-single-quotes the upstream value, so
names with unusual characters are safe.

```bash
kubectl get secret \
  -n <@namespace:kubectl get ns --no-headers -o custom-columns=NAME:.metadata.name> \
  <@secret:kubectl get secret -n <#namespace> --no-headers -o custom-columns=NAME:.metadata.name> \
  -o jsonpath='{.data.<@key:kubectl get secret -n <#namespace> <#secret> -o go-template='{{range $k, $_ := .data}}{{$k}}{{"\n"}}{{end}}'>}' \
  | base64 -d \
  | tee <@output:?<#namespace:raw>.<#secret:raw>.<#key:raw>.out>
echo
```

## Find files modified recently

Lists files under `<@path>` modified in the last `<@days>` days. `path` defaults to the current directory; `days` defaults to `7`.

```bash
find <@path:?.> -type f -mtime -<@days:?7> -not -path '*/.git/*'
```

## Open a file in your editor

Built-in `file` suggestions list files in the current directory. Pick one or type a path.

```sh
${EDITOR:-vi} <@file>
```

## Make a script executable and run it

Two-step: chmod, then run. Both occurrences of `<@script>` share the same value — peanutbutter prompts once per unique name.

```bash
chmod +x <@script>
./<@script>
```

## Serve the current directory over HTTP

Quick static server on `<@port:?8000>`. Useful for sharing a build folder or previewing static HTML.

```bash
python3 -m http.server <@port:?8000>
```

## Placeholder syntax cheat sheet

This snippet uses a `text` fence as a preview-only example. Peanutbutter shows it in the picker preview but picks the shell fence below as the runnable body — so selecting it runs `pb edit`, not the cheatsheet text.

```text
<@name>             prompt for a value
<@name:?default>    prompt with a pre-filled default
<@name:command>     prompt with suggestions from a shell command

inside a suggestion command, reference an earlier prompt's value:
  <#name>           shell-quoted splice of the confirmed value
  <#name:raw>       verbatim splice (no quoting — use for shell syntax)
  \<#name>          literal <#name>, not a reference

frontmatter variables (above):
  type:    suggestions: [...]   fixed picker list
  branch:  command: ...         dynamic picker list
```

```sh
pb edit
```
