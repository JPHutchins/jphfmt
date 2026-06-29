import * as esbuild from "esbuild";

// VS Code client — runs in the extension host, so 'vscode' is provided.
await esbuild.build({
  entryPoints: ["out/extension.js"],
  bundle: true,
  outfile: "out/extension.js",
  platform: "node",
  external: ["vscode"],
  format: "cjs",
  allowOverwrite: true,
});

// LSP server — standalone process, bundle everything (jphfmt IS imported).
await esbuild.build({
  entryPoints: ["out/server.js"],
  bundle: true,
  outfile: "out/server.js",
  platform: "node",
  format: "cjs",
  allowOverwrite: true,
});
