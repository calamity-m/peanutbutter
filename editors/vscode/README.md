# Peanutbutter for VS Code

Minimal language client for [Peanutbutter](https://github.com/calamity-m/peanutbutter)
snippet repositories. It starts `peanutbutter lsp` and connects VS Code to it —
nothing more.

## Prerequisites

The extension does not bundle or download the CLI. Install `peanutbutter` first
and make sure it is on your `PATH` (see the
[main README](https://github.com/calamity-m/peanutbutter#quick-start)).

## Install

This extension is not published to the Visual Studio Marketplace or Open VSX.
Build the `.vsix` and install it yourself:

```bash
cd editors/vscode
npm install
npx @vscode/vsce package
code --install-extension peanutbutter-*.vsix
```

Reload VS Code afterwards. Repeat the last two commands to pick up changes;
`npm install` is only needed the first time or after dependency bumps. Add
`--force` when reinstalling — without it VS Code sees the same version number
already installed and skips the update instead of failing.

## Usage

1. Install the CLI.
2. Install this extension (above).
3. Open a folder containing a peanutbutter marker file (`.peanutbutter.toml`,
   `peanutbutter.toml`, or `_peanutbutter.toml`).
4. Done — Markdown snippet files get diagnostics, completions, hover,
   navigation, and semantic highlighting.

The extension only activates when a marker file exists somewhere in the
workspace, and the server itself only responds for `.md` files under a marker.
See [docs/LSP.md](https://github.com/calamity-m/peanutbutter/blob/main/docs/LSP.md)
for the activation scope and per-workspace marker configuration.

## Configuration

| Setting             | Default        | Description                             |
| ------------------- | -------------- | --------------------------------------- |
| `peanutbutter.path` | `peanutbutter` | Path to the Peanutbutter executable.    |

Only needed when VS Code does not inherit your shell's `PATH` (Nix, custom
builds, Windows install locations, etc.).

## Local development

```bash
cd editors/vscode
npm install
```

Open `editors/vscode` in VS Code and press `F5` (Run Extension), or launch
directly against a snippet repo:

```bash
code --extensionDevelopmentPath="$PWD/editors/vscode" /path/to/snippet-repo
```

## Publishing

There is no published release, and distribution is the locally built `.vsix`
described in [Install](#install). Marketplace and Open VSX publishing needs
publisher credentials that are not set up; if that ever changes, the flow is
`npm version patch`, then `npx @vscode/vsce publish` and
`npx ovsx publish peanutbutter-<version>.vsix`.
