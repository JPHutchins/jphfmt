# /// script
# requires-python = ">=3.14"
# dependencies = ["camas[mcp]==0.1.22"]
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

# ---- TypeScript: the editors/vscode LSP + client ----
ts_install = Task("npm ci", cwd=VSCODE, when=VSCODE_DIR)
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

# Every read-only validation, maximally parallel across both ecosystems. Compile-validation is
# `cargo test`/`doc` for Rust and `tsc --noEmit` for TypeScript.
check = Parallel(
	rust_fmt_check,
	clippy,
	test,
	doc,
	nix_fmt_check,
	ts_fmt_check,
	eslint,
	ts_typecheck,
	ts_build,
	knip,
	typos,
)

# Every deterministic fixer. The two ecosystems run in parallel; the Rust side is the flake's
# `.#fix` app (cargo fmt then clippy --fix); TypeScript lint-fixes then formats so prettier has the
# last word.
fix = Parallel(
	rust_fix,
	Sequential(eslint_fix, ts_fmt, name="ts_fix"),
)

# Everyday default: fix in place, then validate. CI installs TS deps, then validates check + audit
# (no mutation) — `npm ci` makes a fresh checkout hermetic.
all = Sequential(fix, check)
ci = Sequential(ts_install, Parallel(check, audit, name="validate"))

_ = Config(default_task=all, github_task=ci, agent=Claude(fix=fix, check=check))

if __name__ == "__main__":
	run_cli(globals())
