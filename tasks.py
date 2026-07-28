# /// script
# requires-python = ">=3.14"
# dependencies = ["camas[mcp]==0.1.25"]
# ///
"""Project tasks — the single source of truth for validation, run with ``camas``.

The Rust formatter crate lives here; the TypeScript LSP and VS Code client are a
child project (``vscode = Project("editors/vscode")``), mounted for dotted
dispatch (``camas vscode``, ``camas vscode.lint``) and composed into this file's
``Config``. A ``Project`` binding contributes one node per ``Config`` slot — the
child's own default, github, fix and check — so the entry points that span both
ecosystems are the unnamed ones: bare ``camas`` fixes then checks everything,
bare ``camas`` under GitHub Actions runs the read-only pass plus the audit, and
the agent gate gets the fast variants. The named tasks here carry their scope:
``check`` is the crate and the cross-cutting checkers, ``camas vscode.check`` is
the TypeScript side.

camas runs from inside ``nix develop``. The Rust leaves invoke the flake's
``nix run .#<target>`` apps (crane-backed, cached, sandboxed) rather than raw
cargo, so the toolchain and the +stable/+MSRV matrix are pinned by the flake
(``.#test`` and ``.#test-msrv``) — no rustup. The MSRV lives in Cargo.toml, read
by the flake.

Scoping is positive throughout: a leaf declares the paths it reads (``when``) or
the files it takes (``{paths}``), so a scoped gate run prunes what the change
cannot have affected, and a full run — which never consults ``when`` — still
covers everything.
"""

import runpy
import shlex
from pathlib import Path

import camas
from camas import (
	AgentFormat,
	Claude,
	Config,
	Parallel,
	Project,
	Sequential,
	Task,
	by_suffix,
	run_cli,
)

ROOT = Path(__file__).parent
VSCODE_DIR = "editors/vscode"

# Every path a Rust check reads, as a positive scope rather than a "not the extension" negation:
# the negation dragged clippy and the whole test suite into a README edit, and the gate fires on
# every batch of edits, so that was the common case.
RUST = ("src", "tests", "Cargo.toml", "Cargo.lock", ".cargo", "flake.nix", "flake.lock")

# rustfmt takes files, not directories, so the full-run fallback has to name the crate's sources.
# camas re-executes this file on every invocation, so the glob cannot go stale.
RUST_SOURCES = tuple(
	path.relative_to(ROOT).as_posix()
	for prefix in ("src", "tests")
	for path in ROOT.glob(f"{prefix}/**/*.rs")
)
RS = by_suffix((".rs",), default=RUST_SOURCES)

# `ty` needs camas importable to check this file. Point it at the interpreter that is already
# running it, so what type-checks tasks.py is the camas that executes it — no second pin to drift.
CAMAS_SITE = shlex.quote(str(Path(camas.__file__).parent.parent))


def nix_files(changed: tuple[str, ...]) -> bool:
	return any(c.endswith(".nix") for c in changed)


vscode = Project(VSCODE_DIR)

# ---- Rust: the jphfmt crate ----
rust_fmt_check = Task("nix run .#fmt", when=RUST)
clippy = Task("nix run .#lint", when=RUST)
rust_fix = Task("nix run .#fix", mutates=True, when=RUST)
test = Parallel(
	Task("nix run .#test"),
	Task("nix run .#test-msrv"),
	when=RUST,
)
doc = Task("nix run .#doc", when=RUST)

# Tight inner-loop Rust checks: raw cargo against the dev shell's warm target/, incremental and
# single-toolchain (no crane sandbox rebuild, no MSRV double-build). Same commands the crane apps
# wrap, so the signal matches; the agent gate drives these while `check`/CI keep the crane path.
rust_fmt_check_fast = Task("cargo fmt --check -- {paths}", paths=RS)
clippy_fast = Task("cargo clippy --all-targets --all-features -- -D warnings", when=RUST)
test_fast = Task("cargo nextest run --all-features", when=RUST)
doc_fast = Task(
	"cargo doc --no-deps --all-features",
	env={"RUSTDOCFLAGS": "-D warnings"},
	when=RUST,
)

# Tight inner-loop Rust fixer: raw cargo, mirroring the flake's `.#fix` app (fmt then clippy --fix)
# without the `nix run` wrapper. rustfmt takes the changed files — a formatter's own repository
# cannot afford `--all` rewriting sources the change never touched.
rust_fmt_fix = Task("cargo fmt -- {paths}", mutates=True, paths=RS)
clippy_fix_fast = Task(
	"cargo clippy --fix --allow-dirty --allow-staged --all-targets --all-features",
	mutates=True,
	when=RUST,
)
rust_fix_fast = Sequential(rust_fmt_fix, clippy_fix_fast)

# ---- Release: one version across four files, and the tag that ships it ----
# `release` is manual and outward-facing, so it stays out of the composed defaults; `version_check`
# joins them, because the drift it catches is invisible until someone installs from the wrong place.
# `TRACKED` comes from release.py itself, so the list is maintained once: runpy rather than import,
# because camas evaluates this file without its own directory on sys.path, and because re-execution
# cannot serve the stale value an import cache would.
RELEASE_FILES = (*runpy.run_path(str(ROOT / "release.py"))["TRACKED"], "release.py")

version_check = Task("uv run release.py check", when=RELEASE_FILES)
# One command does the whole release — check, rewrite the four files, commit, tag, push main and the
# tag. `--VERSION` takes major/minor/patch or an explicit X.Y.Z; bare `camas release` bumps the patch.
release = Sequential(
	Task("uv run release.py ship {VERSION}", mutates=True),
	matrix={"VERSION": ("patch",)},
	help="camas release [--VERSION=major|minor|patch|X.Y.Z]",
)

# ---- Cross-cutting checkers ----
# typos runs reproducibly via uvx (no install) and covers the whole tree, narrowing to the changed
# files on a scoped run; sarif is native, so the gate reads diagnostics instead of prose.
typos = Task("uvx typos {paths}", paths=".", agent_format=AgentFormat("--format sarif", "sarif"))
nix_fmt_check = Task("nix run .#fmt-nix", when=nix_files)
# Both task files as well as the release script: tasks.py is the one file whose breakage takes every
# other task with it.
py_types = Task(
	f"uvx ty check --extra-search-path {CAMAS_SITE} {{paths}}",
	paths=by_suffix((".py",), default=("release.py", "tasks.py", f"{VSCODE_DIR}/tasks.py")),
)
# audit folds into CI, not `check`; mutants (proves the tests bite) is nightly, its own workflow.
audit = Task("nix run .#audit")
mutants = Task("cargo mutants --jobs 8")

# ---- Composition ----
rust_check = Parallel(rust_fmt_check, clippy, test, doc)
rust_check_fast = Parallel(
	rust_fmt_check_fast,
	clippy_fast,
	test_fast,
	doc_fast,
)
rust = Sequential(rust_fix, rust_check)
cross = Parallel(nix_fmt_check, typos, version_check, py_types)

# Read-only, the crate plus the cross-cutting checkers. `check_fast` is the same set with the raw
# cargo leaves, which is what the agent gate validates a scoped change against.
check = Parallel(rust_check, cross)
check_fast = Parallel(rust_check_fast, cross)

# Each slot pulls the child's matching slot, so the extension is fixed when this file fixes and
# checked when it checks — never the wrong one because a name resolved in the wrong context.
_ = Config(
	default_task=Parallel(rust, cross, vscode, name="all"),
	github_task=Parallel(check, audit, vscode, name="validate"),
	agent=Claude(
		fix=Parallel(rust_fix_fast, vscode, name="fix"),
		check=Parallel(check_fast, vscode, name="check_all"),
	),
)

if __name__ == "__main__":
	run_cli(globals())
