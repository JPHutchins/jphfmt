import { spawn } from "node:child_process";

/// The outcome of running cfmt over a buffer: a tagged union so callers match exhaustively.
export type FormatResult =
  | { readonly kind: "formatted"; readonly text: string }
  | { readonly kind: "failed"; readonly message: string };

/// Run `cfmt --width <width>` over `source` on stdin and resolve with its result. Never rejects:
/// a spawn error or non-zero exit is surfaced as a `failed` variant.
export const formatSource = (
  binary: string,
  width: number,
  source: string,
): Promise<FormatResult> =>
  new Promise((resolve) => {
    const child = spawn(binary, ["--width", String(width)]);
    const stdout: Buffer[] = [];
    const stderr: Buffer[] = [];
    child.stdout.on("data", (chunk: Buffer) => {
      stdout.push(chunk);
    });
    child.stderr.on("data", (chunk: Buffer) => {
      stderr.push(chunk);
    });
    child.on("error", (error) => {
      resolve({ kind: "failed", message: error.message });
    });
    child.on("close", (code) => {
      resolve(
        code === 0
          ? { kind: "formatted", text: Buffer.concat(stdout).toString("utf8") }
          : {
              kind: "failed",
              message:
                Buffer.concat(stderr).toString("utf8").trim() ||
                `cfmt exited with code ${String(code ?? "unknown")}`,
            },
      );
    });
    child.stdin.end(source);
  });
