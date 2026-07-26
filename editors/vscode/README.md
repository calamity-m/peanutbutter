# Peanutbutter for VS Code

Minimal language client for [Peanutbutter](https://github.com/calamity-m/peanutbutter)
snippet repositories. It starts `peanutbutter lsp` and connects VS Code to it —
nothing more.

## Prerequisites

The extension does not bundle or download the CLI. Install `peanutbutter` first
and make sure it is on your `PATH` (see the
[main README](https://github.com/calamity-m/peanutbutter#quick-start)).

## Usage

1. Install the CLI.
2. Install this extension.
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

## Packaging and publishing

Publishing is manual and infrequent. From `editors/vscode`:

```bash
npm version patch

npx @vscode/vsce package

npx @vscode/vsce publish

npx ovsx publish peanutbutter-<version>.vsix
```

This produces a `.vsix` and publishes the same extension to the Visual Studio
Marketplace and Open VSX.
