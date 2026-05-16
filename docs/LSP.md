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

### Recommended — nvim-lspconfig

Use [nvim-lspconfig](https://github.com/neovim/nvim-lspconfig) if you have it
installed. It gives better `:LspInfo` output, handles root detection cleanly,
and makes executable/path problems easier to diagnose than a hand-written
autocmd.

Register peanutbutter as a custom server:

```lua
local lspconfig = require("lspconfig")
local lspconfig_configs = require("lspconfig.configs")

if not lspconfig_configs.peanutbutter then
  lspconfig_configs.peanutbutter = {
    default_config = {
      cmd = { "peanutbutter", "lsp" },
      filetypes = { "markdown" },
      root_dir = require("lspconfig.util").root_pattern(
        ".peanutbutter.toml",
        "peanutbutter.toml",
        "_peanutbutter.toml"
      ),
      single_file_support = false,
    },
  }
end

lspconfig.peanutbutter.setup({})
```

### Fallback — built-in `vim.lsp` autocmd

If you really do not want to use nvim-lspconfig, you can start the server
directly with Neovim's built-in LSP client. Prefer the nvim-lspconfig setup
above unless you have a specific reason not to add that dependency.

Add this to your Neovim config (`init.lua` or a plugin file):

```lua
vim.api.nvim_create_autocmd("FileType", {
  pattern = "markdown",
  callback = function()
    vim.lsp.start({
      name = "peanutbutter",
      cmd = { "peanutbutter", "lsp" },
      -- root_dir tells Neovim which directory to treat as the project root.
      -- Here we walk up looking for a marker file, mirroring the server's own logic.
      root_dir = (function()
        local markers = { ".peanutbutter.toml", "peanutbutter.toml", "_peanutbutter.toml" }
        return vim.fs.dirname(vim.fs.find(markers, { upward = true })[1])
      end)(),
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
2. Run `:LspInfo` (nvim-lspconfig) or `:lua vim.print(vim.lsp.get_clients())` to confirm the `peanutbutter` client is attached.
3. Introduce a lint error (e.g. add an unused `variables:` entry) and save — a warning squiggle should appear.
4. Type `<@` inside a fenced code block and trigger completions — declared variable names should appear.
