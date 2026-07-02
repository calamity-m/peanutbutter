---
name: Shell utilities
tags:
  - shell
  - files
description: Common shell and file operations.
variables:
  file:
    command: rg . --files
  directory:
    default: .
---

# Shell Utilities

## List directory contents

```bash
ls -lsha <@path:?.>
```

## Watch a file for new lines

```bash
tail -f <@file>
```

## Find files by name pattern

```bash
find <@directory> -name "<@pattern>" -type f
```

## Read and decode base64 content from a file

```bash
cat <@file> | base64 -d
```

## Create a directory and navigate into it

```bash
mkdir -p <@path> && cd <@path>
```

## Copy a file to a timestamped backup

Example output path shown in the picker preview:

```text
notes.md.20240517123000.bak
```

```bash
cp <@file> <@file>.$(date +%Y%m%d%H%M%S).bak
```

## Search for a running process

```bash
ps aux | grep <@process>
```
