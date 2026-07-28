import { existsSync } from "node:fs";
import { join } from "node:path";
import { workspace, type ExtensionContext } from "vscode";
import {
  LanguageClient,
  TransportKind,
  type LanguageClientOptions,
  type ServerOptions,
} from "vscode-languageclient/node";

let client: LanguageClient | undefined;

/// The formatter to spawn: the `jphfmt.path` setting when it is set, otherwise the binary this
/// build bundles, otherwise `jphfmt` on `PATH`.
///
/// A platform-specific package carries `bin/`; the universal one — what a platform with no build
/// of its own installs — does not, and falls back to `PATH` as every release before this did. The
/// setting wins either way, because people build their own.
const formatter = (context: ExtensionContext, configured: unknown): string => {
  // `settings.json` is user-authored, so the declared `string` type is a claim rather than a fact.
  if (typeof configured === "string" && configured) return configured;
  const bundled = context.asAbsolutePath(
    join("bin", process.platform === "win32" ? "jphfmt.exe" : "jphfmt"),
  );
  return existsSync(bundled) ? bundled : "jphfmt";
};

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
      path: formatter(context, config.get<unknown>("path", "")),
      width: config.get<number>("width", 100),
    },
  };
  client = new LanguageClient("jphfmt", "jphfmt", serverOptions, clientOptions);
  void client.start();
};

export const deactivate = (): Thenable<void> | undefined => client?.stop();
