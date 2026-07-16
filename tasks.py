# /// script
# requires-python = ">=3.14"
# dependencies = ["camas[mcp]==0.1.25"]
# ///
"""Project tasks — the single source of truth for validation, run with ``camas``.

Covers both ecosystems: the Rust formatter crate and the TypeScript LSP/VS Code
client under ``editors/vscode``. ``check`` runs every read-only validation in
parallel; ``fix`` runs every deterministic fixer; ``all`` does fix-then-check.

camas runs from inside ``nix develop``. The Rust leaves invoke the flake's
``nix run .#<target>`` apps (crane-backed, cached, sandboxed) rather than raw
cargo, so the toolchain and the +stable/+MSRV matrix are pinned by the flake
(``.#test`` and ``.#test-msrv``) — no rustup. The MSRV lives in Cargo.toml, read
by the flake.

Each leaf's ``when`` scopes it to its ecosystem: a scoped gate run skips the
toolchain the change didn't touch (a full run runs everything).
"""

from pathlib import Path

from camas import Claude, Config, Parallel, Sequential, Task, run_cli

VSCODE_DIR = "editors/vscode"
VSCODE = Path(VSCODE_DIR)


def outside_vscode(changed: tuple[str, ...]) -> bool:
	return any(c != VSCODE_DIR and not c.startswith(VSCODE_DIR + "/") for c in changed)


def nix_files(changed: tuple[str, ...]) -> bool:
	return any(c.endswith(".nix") for c in changed)


# ---- Rust: the jphfmt crate ----
rust_fmt_check = Task("nix run .#fmt", when=outside_vscode)
clippy = Task("nix run .#lint", when=outside_vscode)
rust_fix = Task("nix run .#fix", mutates=True, when=outside_vscode)
test = Parallel(
	Task("nix run .#test"),
	Task("nix run .#test-msrv"),
	name="test",
	when=outside_vscode,
)
doc = Task("nix run .#doc", when=outside_vscode)

# Tight inner-loop Rust checks: raw cargo against the dev shell's warm target/, incremental and
# single-toolchain (no crane sandbox rebuild, no MSRV double-build). Same commands the crane apps
# wrap, so the signal matches; the agent gate drives these while `check`/`ci` keep the crane path.
rust_fmt_check_fast = Task("cargo fmt --all --check", when=outside_vscode)
clippy_fast = Task("cargo clippy --all-targets --all-features -- -D warnings", when=outside_vscode)
test_fast = Task("cargo nextest run --all-features", when=outside_vscode)
doc_fast = Task(
	"cargo doc --no-deps --all-features",
	env={"RUSTDOCFLAGS": "-D warnings"},
	when=outside_vscode,
)

# Tight inner-loop Rust fixer: raw cargo, mirroring the flake's `.#fix` app (fmt then clippy --fix)
# without the `nix run` wrapper.
rust_fmt_fix = Task("cargo fmt --all", mutates=True, when=outside_vscode)
clippy_fix_fast = Task(
	"cargo clippy --fix --allow-dirty --allow-staged --all-targets --all-features",
	mutates=True,
	when=outside_vscode,
)
rust_fix_fast = Sequential(rust_fmt_fix, clippy_fix_fast, name="rust_fix_fast")

# ---- TypeScript: the editors/vscode LSP + client ----
# node_modules is supplied hermetically by the flake dev shell (importNpmLock),
# so there is no install step here — the tools are already present.
ts_fmt = Task("npm run format", cwd=VSCODE, mutates=True, when=VSCODE_DIR)
ts_fmt_check = Task("npm run format:check", cwd=VSCODE, when=VSCODE_DIR)
eslint = Task("npm run lint", cwd=VSCODE, when=VSCODE_DIR)
eslint_fix = Task("npm run lint:fix", cwd=VSCODE, mutates=True, when=VSCODE_DIR)
ts_typecheck = Task("npm run typecheck", cwd=VSCODE, when=VSCODE_DIR)
ts_build = Task("npm run build", cwd=VSCODE, when=VSCODE_DIR)
knip = Task("npx --yes knip", cwd=VSCODE, when=VSCODE_DIR)

# ---- Cross-cutting checkers ----
# typos runs reproducibly via uvx (no install) and covers the whole tree.
typos = Task("uvx typos")
nix_fmt_check = Task("nix run .#fmt-nix", when=nix_files)
# audit folds into `ci`, not `check`; mutants (proves the tests bite) is nightly, its own workflow.
audit = Task("nix run .#audit")
mutants = Task("cargo mutants --jobs 8")

# Read-only validation, grouped per ecosystem so each group waits only on its own fixers.
# Compile-validation is `cargo test`/`doc` for Rust and `tsc --noEmit` for TypeScript.
rust_check = Parallel(rust_fmt_check, clippy, test, doc, name="rust_check")
ts_check = Parallel(ts_fmt_check, eslint, ts_typecheck, ts_build, knip, name="ts_check")

# Every read-only validation, maximally parallel across both ecosystems and the cross-cutting checkers.
check = Parallel(rust_check, ts_check, nix_fmt_check, typos)

# The agent gate's tight inner loop: raw-cargo Rust checks (warm target/, no MSRV) in place of the
# crane apps; the TS and cross-cutting leaves are already raw and stay as-is.
rust_check_fast = Parallel(rust_fmt_check_fast, clippy_fast, test_fast, doc_fast, name="rust_check_fast")
check_fast = Parallel(rust_check_fast, ts_check, nix_fmt_check, typos)

# Every deterministic fixer, grouped per ecosystem. The Rust side is the flake's `.#fix` app (cargo
# fmt then clippy --fix); TypeScript lint-fixes then formats so prettier has the last word.
ts_fix = Sequential(eslint_fix, ts_fmt, name="ts_fix")
fix = Parallel(rust_fix, ts_fix)

# The agent gate's fast fixer: raw-cargo Rust fixer in place of the `.#fix` app; TS side unchanged.
fix_fast = Parallel(rust_fix_fast, ts_fix)

# Per-ecosystem fix-then-check: each ecosystem's checks wait only on its own fixers.
rust = Sequential(rust_fix, rust_check)
ts = Sequential(ts_fix, ts_check)

# Everyday default: fix in place, then validate — the two ecosystem pipelines and the cross-cutting
# checkers (which wait on no fixer) all run in parallel, so neither ecosystem ever blocks the other.
all = Parallel(rust, ts, nix_fmt_check, typos)
ci = Parallel(check, audit, name="validate")

_ = Config(default_task=all, github_task=ci, agent=Claude(fix=fix_fast, check=check_fast))

if __name__ == "__main__":
	run_cli(globals())
