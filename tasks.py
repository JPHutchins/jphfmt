# /// script
# requires-python = ">=3.14"
# dependencies = ["camas[mcp]==0.1.21"]
# ///
"""Project tasks — the single source of truth for validation, run with ``camas``.

Covers both ecosystems: the Rust formatter crate and the TypeScript LSP/VS Code
client under ``editors/vscode``. ``check`` runs every read-only validation in
parallel; ``fix`` runs every deterministic fixer; ``all`` does fix-then-check.

Each leaf declares its ``paths`` scope (no ``{paths}`` token — cargo and npm are
whole-project tools), so a scoped run (the FileChanged hook's ``camas mcp fix
--paths <file>``, or the gate) drops the ecosystem the change didn't touch:
editing Rust never runs the TS toolchain and vice-versa. Full runs are unaffected.
"""

import tomllib
from pathlib import Path

from camas import Claude, Config, Parallel, Sequential, Task, run_cli

CARGO_TOML = tomllib.loads(Path("Cargo.toml").read_text())
MSRV = CARGO_TOML["package"]["rust-version"]

VSCODE_DIR = "editors/vscode"
VSCODE = Path(VSCODE_DIR)


def rust_paths(changed: tuple[str, ...]) -> tuple[str, ...]:
	"""The Rust crate's scope: every changed file outside the TypeScript project."""
	return tuple(c for c in changed if c != VSCODE_DIR and not c.startswith(VSCODE_DIR + "/"))


# ---- Rust: the jphfmt crate (scoped to everything outside editors/vscode) ----
rust_fmt = Task("cargo fmt --all", mutates=True, paths=rust_paths)
rust_fmt_check = Task("cargo fmt --all -- --check", paths=rust_paths)
clippy = Task("cargo clippy --all-targets --all-features -- -D warnings", paths=rust_paths)
clippy_fix = Task(
	"cargo clippy --fix --allow-dirty --allow-staged --all-targets --all-features",
	mutates=True,
	paths=rust_paths,
)
test = Parallel(
    Task(
        "cargo +{RUST} test --all-features",
        env={"CARGO_TARGET_DIR": str(Path("target") / "{RUST}")},
        paths=rust_paths,
    ),
    matrix={"RUST": ("stable", MSRV)},
    name="test",
)
doc = Task(
	"cargo doc --no-deps --all-features",
	env={"RUSTDOCFLAGS": "-D warnings"},
	paths=rust_paths,
)

# ---- TypeScript: the editors/vscode LSP + client (scoped to that directory) ----
ts_install = Task("npm ci", cwd=VSCODE, paths=VSCODE_DIR)
ts_fmt = Task("npm run format", cwd=VSCODE, mutates=True, paths=VSCODE_DIR)
ts_fmt_check = Task("npm run format:check", cwd=VSCODE, paths=VSCODE_DIR)
eslint = Task("npm run lint", cwd=VSCODE, paths=VSCODE_DIR)
eslint_fix = Task("npm run lint:fix", cwd=VSCODE, mutates=True, paths=VSCODE_DIR)
ts_typecheck = Task("npm run typecheck", cwd=VSCODE, paths=VSCODE_DIR)
ts_build = Task("npm run build", cwd=VSCODE, paths=VSCODE_DIR)
knip = Task("npx --yes knip", cwd=VSCODE, paths=VSCODE_DIR)

# ---- Cross-cutting checkers ----
# typos runs reproducibly via uvx (no install) and covers the whole tree.
typos = Task("uvx typos", paths=".")
# These need their cargo tool installed, so they live outside `check` and get their own CI jobs:
# cargo-audit (RUSTSEC advisories) and cargo-mutants (mutation testing — proves the tests bite).
audit = Task("cargo audit", paths=rust_paths)
mutants = Task("cargo mutants --jobs 8", paths=rust_paths)

# Every read-only validation that runs without a per-tool install, maximally parallel across both
# ecosystems. Compile-validation is `cargo test`/`doc` for Rust and `tsc --noEmit` for TypeScript.
check = Parallel(
	rust_fmt_check,
	clippy,
	test,
	doc,
	ts_fmt_check,
	eslint,
	ts_typecheck,
	ts_build,
	knip,
	typos,
)

# Every deterministic fixer. The two ecosystems run in parallel; each is ordered internally
# (Rust formats then clippy-fixes; TypeScript lint-fixes then formats so prettier has the last word).
# Under a scoped run, the untouched ecosystem's branch prunes to nothing and is dropped.
fix = Parallel(
	Sequential(rust_fmt, clippy_fix, name="rust_fix"),
	Sequential(eslint_fix, ts_fmt, name="ts_fix"),
)

# Everyday default: fix in place, then validate. CI default: install TS deps, then validate
# (no mutation) — `npm ci` makes a fresh checkout hermetic.
all = Sequential(fix, check)
ci = Sequential(ts_install, check)

_ = Config(default_task=all, github_task=ci, agent=Claude(fix=fix, check=check))

if __name__ == "__main__":
	run_cli(globals())
