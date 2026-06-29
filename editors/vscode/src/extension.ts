import { join } from "node:path";
import { workspace, type ExtensionContext } from "vscode";
import {
  LanguageClient,
  TransportKind,
  type LanguageClientOptions,
  type ServerOptions,
} from "vscode-languageclient/node";

let client: LanguageClient | undefined;

export const activate = (context: ExtensionContext): void => {
  const module = context.asAbsolutePath(join("out", "server.js"));
  const config = workspace.getConfiguration("jphfmt");
  const serverOptions: ServerOptions = {
    run: { module, transport: TransportKind.ipc },
    debug: { module, transport: TransportKind.ipc },
  };
  const clientOptions: LanguageClientOptions = {
    documentSelector: [
      { scheme: "file", language: "c" },
      { scheme: "untitled", language: "c" },
    ],
    initializationOptions: {
      path: config.get<string>("path", "jphfmt"),
      width: config.get<number>("width", 100),
    },
  };
  client = new LanguageClient("jphfmt", "jphfmt", serverOptions, clientOptions);
  void client.start();
};

export const deactivate = (): Thenable<void> | undefined => client?.stop();
