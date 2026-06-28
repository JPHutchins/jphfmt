# jphfmt for editors

A Language Server Protocol implementation that formats C with
[`jphfmt`](../../README.md), plus a thin VS Code client. The server is a plain
stdio LSP, so it works in any LSP-capable editor — VS Code, Neovim, Emacs,
Helix — not just VS Code.

It implements one capability, `textDocument/formatting`: it pipes the document
through the `jphfmt` binary and returns a single full-document edit (or none, when
the file is already formatted). Formatting that fails surfaces as an editor
error notification; it never returns a partial edit.

## Build

```sh
npm install
npm run build      # tsc → out/
npm run check      # strict type-check only
```

`jphfmt` must be on `PATH` (or set `jphfmt.path`). Build it from the repo root with
`cargo build --release`; the binary is `target/release/jphfmt`.

## VS Code

Open this folder in VS Code and press F5 (Extension Development Host), or package
with `npx vsce package` and install the `.vsix`. Settings:

- `jphfmt.path` — path to the `jphfmt` executable (default `jphfmt`).
- `jphfmt.width` — column limit (default `100`).

Format with the usual *Format Document* command; enable *Format on Save* to run
it automatically.

## Other editors (standalone server)

Run the server over stdio and point your client at it:

```sh
node out/server.js --stdio
```

Pass settings via the client's `initializationOptions`, e.g. `{ "path":
"/usr/local/bin/jphfmt", "width": 100 }`. Example for Neovim's built-in LSP:

```lua
vim.lsp.start({
  name = "jphfmt",
  cmd = { "node", "/path/to/out/server.js", "--stdio" },
  init_options = { path = "jphfmt", width = 100 },
  filetypes = { "c" },
})
```

## Layout

- `src/jphfmt.ts` — the `jphfmt` subprocess wrapper (a tagged `FormatResult`).
- `src/server.ts` — the LSP server (`textDocument/formatting`).
- `src/extension.ts` — the VS Code client that launches the server.
