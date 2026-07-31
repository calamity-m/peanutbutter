const vscode = require("vscode");
const { LanguageClient } = require("vscode-languageclient/node");

let client;

function activate() {
  const config = vscode.workspace.getConfiguration("peanutbutter");
  const command = config.get("path", "peanutbutter");

  // Deliberately no `transport: TransportKind.stdio`. For a command executable
  // the client appends a literal `--stdio` argument for that transport, which
  // `peanutbutter lsp` rejects as an unknown argument and exits before the
  // handshake. Omitting transport still spawns over stdio pipes, just without
  // the extra flag.
  const serverOptions = {
    command,
    args: ["lsp"],
  };

  // The server only responds for .md files under a peanutbutter marker file,
  // so the selector can stay broad without affecting other Markdown projects.
  const clientOptions = {
    documentSelector: [{ scheme: "file", language: "markdown" }],
  };

  client = new LanguageClient(
    "peanutbutter",
    "Peanutbutter",
    serverOptions,
    clientOptions,
  );

  client.start().catch((error) => {
    vscode.window.showErrorMessage(
      `Could not start Peanutbutter. Make sure "${command}" is installed and on PATH, or set "peanutbutter.path".`,
    );

    console.error(error);
  });
}

async function deactivate() {
  await client?.stop();
}

module.exports = {
  activate,
  deactivate,
};
