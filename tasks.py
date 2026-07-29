# /// script
# requires-python = ">=3.14"
# dependencies = ["camas[mcp]==0.1.29"]
# ///
"""Project tasks — run with ``camas``. The TypeScript half is a child project."""

import runpy
from pathlib import Path

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

RUST = ("src", "tests", "Cargo.toml", "Cargo.lock", ".cargo", "flake.nix", "flake.lock")

# rustfmt takes files, not directories, so a full run has to name them.
RS = by_suffix(
	(".rs",),
	default=tuple(
		sorted(
			path.relative_to(ROOT).as_posix()
			for prefix in ("src", "tests")
			for path in ROOT.glob(f"{prefix}/**/*.rs")
		)
	),
)


def nix_files(changed: tuple[str, ...]) -> bool:
	return any(c.endswith(".nix") for c in changed)


vscode = Project("editors/vscode")

# The crane apps pin the toolchain and the +stable/+MSRV matrix in the flake, so there is no rustup.
rust_fmt_check = Task("nix run .#fmt", when=RUST)
clippy = Task("nix run .#lint", when=RUST)
rust_fix = Task("nix run .#fix", mutates=True, when=RUST)
test = Parallel(Task("nix run .#test"), Task("nix run .#test-msrv"), when=RUST)
doc = Task("nix run .#doc", when=RUST)
audit = Task("nix run .#audit")
mutants = Task("cargo mutants --jobs 8")

# Raw cargo, ~1s/leaf cheaper than the crane app it mirrors: the agent gate's inner loop.
rust_fmt_check_fast = Task("cargo fmt --check -- {paths}", paths=RS)
clippy_fast = Task("cargo clippy --all-targets --all-features -- -D warnings", when=RUST)
test_fast = Task("cargo nextest run --all-features", when=RUST)
doc_fast = Task(
	"cargo doc --no-deps --all-features",
	env={"RUSTDOCFLAGS": "-D warnings"},
	when=RUST,
)
rust_fmt_fix = Task("cargo fmt -- {paths}", mutates=True, paths=RS)
clippy_fix_fast = Task(
	"cargo clippy --fix --allow-dirty --allow-staged --all-targets --all-features",
	mutates=True,
	when=RUST,
)

# runpy rather than import: camas evaluates this file without its own directory on sys.path.
RELEASE_FILES = (*runpy.run_path(str(ROOT / "release.py"))["TRACKED"], "release.py")

version_check = Task("uv run release.py check", when=RELEASE_FILES)
release = Sequential(
	Task("uv run release.py ship {VERSION}", mutates=True),
	matrix={"VERSION": ("patch",)},
	help="camas release [--VERSION=major|minor|patch|X.Y.Z]",
)

typos = Task("uvx typos {paths}", paths=".", agent_format=AgentFormat("--format sarif", "sarif"))
nix_fmt_check = Task("nix run .#fmt-nix", when=nix_files)
py_types = Task("uvx ty check {paths}", paths="release.py")
# Not `camas --check` (JPHutchins/camas#277); the header above exists only to make this work.
task_types = Task("uv run tasks.py --check", when="tasks.py")

rust_check = Parallel(rust_fmt_check, clippy, test, doc)
rust_check_fast = Parallel(rust_fmt_check_fast, clippy_fast, test_fast, doc_fast)
rust_fix_fast = Sequential(rust_fmt_fix, clippy_fix_fast)
fix_fast = Parallel(rust_fix_fast, vscode)
rust = Sequential(rust_fix, rust_check)
cross = Parallel(nix_fmt_check, typos, version_check, py_types, task_types)

check = Parallel(rust_check, cross, vscode)
check_fast = Parallel(rust_check_fast, cross, vscode)
ci = Parallel(check, audit)
all = Parallel(rust, cross, vscode)

_ = Config(
	default_task=all,
	github_task=ci,
	agent=Claude(fix=fix_fast, check=check_fast),
)

if __name__ == "__main__":
	run_cli(globals())
