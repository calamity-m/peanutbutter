---
name: snippet name
tags: 
    - a
    - b
description: frontmatter metadata
---

# Snippet Title

Simple example snippets

## Echo something without newline

Echo some output to the terminal without a newline

```
echo -n <@input>
```

## List all files, including hidden

This is more complex example, the default of our input will be ".".

```
ls -lsha <@input:?.>
```

## Cat file and base64 contents, with no newline

Here we have a definition for input, which is the output of "rg . --files".

While we're at it, let's make my description more pretty - **with markdown**

### wheee

some text

```
cat <@input:rg . --files> | base64 -w 0
```

