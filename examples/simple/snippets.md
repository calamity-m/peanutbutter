---
name: Example Snippet Name Metadata
tags: 
    - example tag
    - example tag 2
description: frontmatter metadata, this stuff gets fuzzy searched so use at your own peril. Helpful, but beyond tags ehhh - do as you see fit.
---

# Example Snippets

This file has example snippets that show the basic syntax for peanutbutter. The README specifies the specification better,
but examples are easiest to grep.

## Echo something without newline

This is a description, which we can use some pretty markdown in. 
Our first wrapped code block (backticks) will be considered the
"snippet"

For instance, the following snippet takes an input variable, and
echoes it back to the terminal with no newline.

```bash
echo -n <@input>
```

## List all files, including hidden

Here, we have a more complex example of user input. `:?` sets a default value
for the user input.

```bash
ls -lsha <@input:?.>
```

## Cat file and base64 contents, with no newline

Sometimes we want to populate suggestions, and have those suggestions come from
some shell command. This can be done by just specifying the command after your colon.

Here we have a definition for input, which is the output of "rg . --files".

It's also worth pointing out that the "description" here is markdown that gets rendered nicely.
Sometimes you have a snippet that needs context displayed with it, instructions or just a primer.

### wheeeeeeeee

1. you
2. so
3. fine

Why use this snippet? Idk.

```
cat <@input:rg . --files> | base64 -w 0
```

