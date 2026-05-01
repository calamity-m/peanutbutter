---
name: Shell utilities
tags:
  - shell
  - files
description: Common shell and file operations.
---

# Shell Utilities

## List directory contents

```bash
ls -lsha <@path:?.>
```

## Watch a file for new lines

```bash
tail -f <@file:rg . --files>
```

## Find files by name pattern

```bash
find <@directory:?.> -name "<@pattern>" -type f
```

## Read and decode base64 content from a file

```bash
cat <@file:rg . --files> | base64 -d
```

## Create a directory and navigate into it

```bash
mkdir -p <@path> && cd <@path>
```

## Copy a file to a timestamped backup

```bash
cp <@file:rg . --files> <@file:rg . --files>.$(date +%Y%m%d%H%M%S).bak
```

## Search for a running process

```bash
ps aux | grep <@process>
```
