---
name: Search
tags:
  - search
  - ripgrep
  - grep
---

# Search

## Search for a pattern in files

```bash
rg <@pattern> <@path:?.>
```

## Search only files of a specific type

```bash
rg -t <@type:?py> <@pattern> <@path:?.>
```

## Search case-insensitively

```bash
rg -i <@pattern> <@path:?.>
```

## Show only the matching portion of each line

```bash
rg -o "<@pattern>" <@path:?.>
```

## Count matches per file

```bash
rg -c <@pattern> <@path:?.>
```

## Search and replace (preview without writing)

```bash
rg <@pattern> --passthru -r "<@replacement>" <@file:rg . --files>
```
