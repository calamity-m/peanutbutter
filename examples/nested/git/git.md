---
name: Git
tags:
  - git
  - vcs
---

# Git

## Commit staged changes

```bash
git commit -m "<@message>"
```

## Stage path and commit

```bash
git add <@path:?.> && git commit -m "<@message>"
```

## Create and switch to a new branch

```bash
git switch -c <@branch>
```

## Switch to an existing branch

```bash
git switch <@branch:git branch --format='%(refname:short)'>
```

## Push branch and set upstream

```bash
git push -u <@remote:?origin> <@branch:git branch --show-current>
```

## Stash changes with a description

```bash
git stash push -m "<@description>"
```

## Cherry-pick a commit

```bash
git cherry-pick <@commit>
```

## Amend the last commit message

```bash
git commit --amend -m "<@message>"
```

## View log with graph

```bash
git log --oneline --graph --decorate <@ref:?HEAD>
```

## Soft reset to previous commit

Moves HEAD back but leaves changes staged.

```bash
git reset --soft <@commit:?HEAD~1>
```
