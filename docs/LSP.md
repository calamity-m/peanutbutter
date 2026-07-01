# peanutbutter LSP

`peanutbutter lsp` starts a Language Server Protocol server over stdio that
gives editors a rich authoring experience for `.md` snippet files.

## Features

| Feature | Trigger |
|---|---|
| Diagnostics | Inline squiggles on open/save — lint findings from `pb lint` |
| Completions | Frontmatter keys (`name`, `description`, `tags`, `variables`); variable spec sub-keys (`default`, `suggestions`, `command`); `<@variable>` placeholders in code blocks |
| Hover | `<@name>` shows the resolved variable spec; frontmatter keys show brief docs |
| Go-to-definition | `<@name>` in a code block → `variables.name:` in frontmatter |
| Find references | Frontmatter variable declaration → every `<@name>` in the file |

## Activation scope

The server only activates for `.md` files that live under a directory tree
containing a **marker file**. Any of the following names is accepted:

| Filename | Style |
|---|---|
| `.peanutbutter.toml` | Hidden, tool-specific |
| `peanutbutter.toml` | Visible config |
| `_peanutbutter.toml` | Visible, underscore-prefixed |

Create a marker file in the root of your snippet directory:

```sh
touch .peanutbutter.toml        # or peanutbutter.toml / _peanutbutter.toml
```

The server walks up from the open file's directory until it finds a marker.
The directory containing the marker becomes the snippet root used for linting.
Files outside any marked tree receive no diagnostics and no completions.

Marker files are TOML. Empty files are valid, and these optional top-level keys
customize LSP behavior for that workspace. If a marker file is not valid TOML,
the LSP treats the workspace as inactive until the marker is fixed:

```toml
# Files or directories the LSP should ignore under this marker root.
ignore = ["archive/**", "generated"]

# If set, the LSP only attaches to matching paths under this marker root.
attach_only = ["snippets/**"]

# Disable lint rules for this workspace. Rule names may include or omit `lint/`.
skip_rules = ["unused-variable", "lint/markdown-structure"]
```

`ignore` and `attach_only` are glob patterns matched against paths relative to
the marker directory, using `/` separators. `*` and `?` match within a single
path segment; use `**` to match across directories. Directory-style patterns
also match files below that directory, so `generated` ignores
`generated/foo.md`.

Workspace marker settings are marker-local; they do not extend the global
`config.toml` schema. Global lint config still applies first, and `skip_rules`
adds workspace-specific disabled rules on top of it. The marker is parsed after
the nearest marker root is found, so when multiple nested markers exist the
nearest ancestor wins and supplies the LSP settings for files under it.

## Neovim setup

### Recommended — Neovim 0.11+ built-in LSP config

Neovim 0.11 added `vim.lsp.config` and `vim.lsp.enable` for LSP server
configuration. This is now the preferred setup path; `require("lspconfig")`
from nvim-lspconfig is deprecated.

Add this to your Neovim config (`init.lua` or a plugin file):

```lua
vim.lsp.config("peanutbutter", {
  cmd = { "peanutbutter", "lsp" },
  filetypes = { "markdown" },
  root_markers = {
    ".peanutbutter.toml",
    "peanutbutter.toml",
    "_peanutbutter.toml",
  },
  workspace_required = true,
})

vim.lsp.enable("peanutbutter")
```

`workspace_required = true` keeps the client gated to marked snippet trees.
`root_markers` does the upward walk; with `workspace_required`, Neovim starts
the client only when that walk finds a marker. Omit it and Neovim falls back to
single-file mode — it attaches to every markdown buffer and the client sits
inert outside a marked tree, rather than not attaching at all.

### Fallback — Neovim 0.10 and older

Older Neovim versions can start the server directly with `vim.lsp.start`.
Add this to your Neovim config (`init.lua` or a plugin file):

```lua
vim.api.nvim_create_autocmd("FileType", {
  pattern = "markdown",
  callback = function()
    local markers = { ".peanutbutter.toml", "peanutbutter.toml", "_peanutbutter.toml" }
    local root = vim.fs.dirname(vim.fs.find(markers, { upward = true })[1])

    if root == nil then
      return
    end

    vim.lsp.start({
      name = "peanutbutter",
      cmd = { "peanutbutter", "lsp" },
      root_dir = root,
    })
  end,
})
```

`vim.lsp.start` is idempotent: if an instance with the same `name` and
`root_dir` is already running, Neovim reuses it rather than spawning a second
process.

### Completions

Neovim's built-in `omnifunc` (`<C-x><C-o>`) works out of the box.  If you use
a completion plugin, no extra configuration is needed — the server advertises
`completionProvider` during the handshake and any LSP-aware plugin will pick it up.

### Verifying the setup

1. Open a `.md` file under a directory that contains a marker file.
2. Run `:checkhealth vim.lsp` or `:lua vim.print(vim.lsp.get_clients())` to confirm the `peanutbutter` client is attached.
3. Introduce a lint error (e.g. add an unused `variables:` entry) and save — a warning squiggle should appear.
4. Type `<@` inside a fenced code block and trigger completions — declared variable names should appear.

## Helix Setup

Add the peanutbutter language server to your Helix language config
(`~/.config/helix/languages.toml`, or `$XDG_CONFIG_HOME/helix/languages.toml`):

```toml
[language-server.peanutbutter]
command = "peanutbutter"
args = ["lsp"]

[[language]]
name = "markdown"
language-servers = ["marksman", "peanutbutter"]
```

The `language-servers` list replaces Helix's default Markdown server list, so
include any Markdown servers you already use. Keeping `marksman` first preserves
Helix's general Markdown behavior while peanutbutter adds snippet-specific LSP
features. If you only want peanutbutter, use
`language-servers = ["peanutbutter"]` instead.

Helix will start `peanutbutter lsp` for Markdown buffers, but the server only
provides diagnostics, completions, hover, and navigation inside directories
with a peanutbutter marker file. See [Activation scope](#activation-scope) for
the accepted marker filenames.

### Verifying the Helix setup

1. Run `hx --health markdown` and confirm `peanutbutter` appears under
   configured language servers.
2. Open a `.md` file under a directory that contains a marker file.
3. Introduce a lint error, such as an unused `variables:` entry, and save — a
   warning should appear.
4. Type `<@` inside a fenced code block and trigger completions — declared
   variable names should appear.
