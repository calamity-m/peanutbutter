const vscode = require("vscode");
const {
  LanguageClient,
  TransportKind,
} = require("vscode-languageclient/node");

let client;

function activate() {
  const config = vscode.workspace.getConfiguration("peanutbutter");
  const command = config.get("path", "peanutbutter");

  const serverOptions = {
    command,
    args: ["lsp"],
    transport: TransportKind.stdio,
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
