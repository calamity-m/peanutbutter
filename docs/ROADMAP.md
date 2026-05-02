# Roadmap

Candidate features to grow peanutbutter from "personal tool" into "tool you'd
recommend to a teammate". Ordered roughly by impact ÷ effort. Nothing here is
committed — this is a backlog, not a plan.

## Completed

- **Zsh + fish integration.** `peanutbutter zsh` / `peanutbutter fish`
  subcommands emitting ZLE / `commandline -i` glue equivalent to the bash
  script. Shipped in v0.5.1.

## Near-term cut

The five items I'd ship first. Together they turn the project from a personal
tool into something shareable.

- ~~**Zsh + fish integration.**~~ Done — see Completed above.
- **Snippet-local variable declarations in frontmatter.** Let a snippet declare
  its own variable specs (suggestions, defaults, command) so the file is
  self-contained and shareable without depending on the reader's `config.toml`.
- **Tag-as-navigation.** First-class tag filter — a `tag:foo` query prefix or a
  dedicated tag list view. Tags are already parsed and weighted; they just have
  no UI.
- **`pb check` / lint.** Surface broken frontmatter, undeclared variables,
  failing suggestion commands, duplicate slugs across files. Force-multiplier
  on the above as the snippet count grows.
- **Suggestion-command timeout + opt-out flag.** Prerequisite for importing
  other people's snippets safely. Today `bash -c` runs unbounded.

## Tier 1 — High leverage, fits the philosophy

- ~~**Zsh + fish integration.**~~ Done — see Completed above.
- **Snippet-local variable declarations.** (See above.)
- **Tag-as-navigation.** (See above.)
- **`pb check` lint.** (See above.)
- **Snippet preview command-output.** Resolve `<@name:cmd>` shadow-style in the
  picker preview so users feel the snippet before committing.

## Tier 2 — Significant UX wins

- **`pb new <name>` capture flow.** Read the last shell history entry (or
  argv), suggest variables for arguments, write to a chosen file. The
  `navi --learn`-equivalent the README implicitly skips.
- **Sync / share story.** Document the "snippets root as a git repo" pattern
  and/or ship a thin `pb sync` wrapper.
- **Conditional / dependent variables.** Let a later variable's suggestion
  command see earlier values via `$variable_name` env vars.
- **Re-execute last snippet.** A "last fully-rendered command" key, separate
  from frecency events.
- **Language-aware snippets.** Surface the fenced-block language in the picker;
  optionally let `pb run` dispatch via the right interpreter.
- **Richer description rendering.** `termimad` is already a dep; lean on it for
  lists/links/sub-headings so snippet files double as a personal wiki.
- **Search query operators.** `name:`, `path:`, `tag:`, `body:` prefixes — lets
  users target a field instead of tuning weights.

## Tier 3 — Stretch / differentiation

- **Cross-machine state.** Merge frecency from a shared source so dotfile-shared
  multi-host setups stay coherent.
- **Snippet collections / namespaces** beyond directories — named bundles,
  optionally remote.
- **Opt-in upstream fallback** (e.g. `??foo` → cheat.sh / tldr) when no local
  match exists. Strictly opt-in; counter to the README's stance otherwise.
- **Sandboxed suggestion commands.** Timeouts, output caps, explicit
  `[security] allow_command_suggestions` toggle. (Partially overlaps with the
  Tier 1 timeout work.)
- **Windows / PowerShell support.** PSReadLine `Insert(...)` integration plus a
  Windows release in CI.
- **Snippet rename / tombstone migration.** `pb gc` to reattach orphaned
  frecency events when snippets are renamed or moved.
- **MCP / local-LLM hooks.** Expose snippets as MCP tools or wire an "explain
  this snippet" action to a local model. Optional, feature-flagged.
- **Pre/post-emit hooks.** Config-driven `bash -c` hooks (`shellcheck` before,
  `wl-copy` after, etc.) — small code, many doors.
