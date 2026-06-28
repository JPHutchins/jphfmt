# /// script
# requires-python = ">=3.14"
# dependencies = ["camas"]
# ///
"""Project tasks — the single source of truth for validation, run with ``camas``.

Covers both ecosystems: the Rust formatter crate and the TypeScript LSP/VS Code
client under ``editors/vscode``. ``check`` runs every read-only validation in
parallel; ``fix`` runs every deterministic fixer; ``all`` does fix-then-check.
"""

from pathlib import Path

from camas import Claude, Config, Parallel, Sequential, Task, run_cli

VSCODE = Path("editors/vscode")

# ---- Rust: the cfmt crate ----
rust_fmt = Task("cargo fmt --all", mutates=True)
rust_fmt_check = Task("cargo fmt --all -- --check")
clippy = Task("cargo clippy --all-targets --all-features -- -D warnings")
clippy_fix = Task(
	"cargo clippy --fix --allow-dirty --allow-staged --all-targets --all-features",
	mutates=True,
)
test = Task("cargo test --all-features")
doc = Task("cargo doc --no-deps --all-features", env={"RUSTDOCFLAGS": "-D warnings"})

# ---- TypeScript: the editors/vscode LSP + client (run from its own directory) ----
ts_install = Task("npm ci", cwd=VSCODE)
ts_fmt = Task("npm run format", cwd=VSCODE, mutates=True)
ts_fmt_check = Task("npm run format:check", cwd=VSCODE)
eslint = Task("npm run lint", cwd=VSCODE)
eslint_fix = Task("npm run lint:fix", cwd=VSCODE, mutates=True)
ts_typecheck = Task("npm run typecheck", cwd=VSCODE)
ts_build = Task("npm run build", cwd=VSCODE)

# Every read-only validation, maximally parallel across both ecosystems. Compile-validation is
# `cargo test`/`doc` for Rust and `tsc --noEmit` (typecheck) for TypeScript.
check = Parallel(
	rust_fmt_check,
	clippy,
	test,
	doc,
	ts_fmt_check,
	eslint,
	ts_typecheck,
)

# Every deterministic fixer. The two ecosystems run in parallel; each is ordered internally
# (Rust formats then clippy-fixes; TypeScript lint-fixes then formats so prettier has the last word).
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
