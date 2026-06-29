import { spawn } from "node:child_process";

/// The outcome of running jphfmt over a buffer: a tagged union so callers match exhaustively.
export type FormatResult =
  | { readonly kind: "formatted"; readonly text: string }
  | { readonly kind: "failed"; readonly message: string };

/// Run `jphfmt --width <width>` over `source` on stdin and resolve with its result. Never rejects:
/// a spawn error or non-zero exit is surfaced as a `failed` variant.
export const formatSource = (
  binary: string,
  width: number,
  source: string,
): Promise<FormatResult> =>
  new Promise((resolve) => {
    const controller = new AbortController();
    const timeout = setTimeout(() => {
      controller.abort();
    }, 30_000);
    const child = spawn(binary, ["--width", String(width)], {
      signal: controller.signal,
    });
    const stdout: Buffer[] = [];
    const stderr: Buffer[] = [];
    child.stdout.on("data", (chunk: Buffer) => {
      stdout.push(chunk);
    });
    child.stderr.on("data", (chunk: Buffer) => {
      stderr.push(chunk);
    });
    child.stdin.on("error", () => {
      void 0;
    });
    child.on("error", (error) => {
      clearTimeout(timeout);
      if (!child.killed) child.kill();
      resolve({ kind: "failed", message: error.message });
    });
    child.on("close", (code) => {
      clearTimeout(timeout);
      resolve(
        code === 0
          ? { kind: "formatted", text: Buffer.concat(stdout).toString("utf8") }
          : {
              kind: "failed",
              message:
                Buffer.concat(stderr).toString("utf8").trim() ||
                `jphfmt exited with code ${String(code ?? "unknown")}`,
            },
      );
    });
    child.stdin.end(source);
  });
