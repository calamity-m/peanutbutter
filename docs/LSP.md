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

Create an empty marker file in the root of your snippet directory:

```sh
touch .peanutbutter.toml        # or peanutbutter.toml / _peanutbutter.toml
```

The server walks up from the open file's directory until it finds a marker.
The directory containing the marker becomes the snippet root used for linting.
Files outside any marked tree receive no diagnostics and no completions.

When multiple nested markers exist the nearest ancestor wins, so monorepos
with independent snippet roots work correctly.

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
})

vim.lsp.enable("peanutbutter")
```

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
