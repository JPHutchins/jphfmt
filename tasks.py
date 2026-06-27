# /// script
# requires-python = ">=3.14"
# dependencies = ["camas"]
# ///
"""Project tasks — run with ``camas``.

This is a Rust project; the leaves are ``cargo`` commands. The ``# /// script``
PEP 723 block above lets a non-Python repo run this file standalone via
``uv run tasks.py <task>`` (building a throwaway env with camas), so the team
needs no project virtualenv. Delete the block and the ``__main__`` guard if you
invoke ``camas`` directly.
"""

from camas import Claude, Config, Parallel, Sequential, Task, run_cli

# The binding name is the task name (this defines `fmt`, `clippy`, …); pass
# name= only to rename or to name a nested anonymous group.
#
# cargo's tooling is crate-scoped, not file-scoped — `cargo fmt`/`clippy`/`test`
# act on whole workspace members, and neither `cargo fmt` nor `rustfmt` accepts a
# directory the way `ruff {paths}` does. So no leaf opts into {paths} scoping;
# the gate runs them whole-workspace and `--under` prunes by recorded timing for
# the inner loop instead.

fmt = Task("cargo fmt --all", mutates=True)
fmt_check = Task("cargo fmt --all -- --check")
clippy = Task("cargo clippy --all-targets --all-features -- -D warnings")
test = Task("cargo test --all-features")
doc = Task("cargo doc --no-deps --all-features", env={"RUSTDOCFLAGS": "-D warnings"})

# Independent and read-only → Parallel (wall-clock max, not sum).
check = Parallel(fmt_check, clippy, test, doc)

# The behavior-preserving auto-fixer the Claude Code plugin's FileChanged hook
# runs on edits (`camas mcp fix`): format first, then clippy's machine-applicable
# fixes. --allow-dirty/--allow-staged let clippy --fix run against the uncommitted
# tree the hook fires on. (cargo clippy --fix recompiles, so this is not free —
# drop clippy_fix from the Sequential if the per-edit latency hurts.)
clippy_fix = Task(
	"cargo clippy --fix --allow-dirty --allow-staged --all-targets --all-features",
	mutates=True,
)
fix = Sequential(clippy_fix, fmt)

# The everyday inner-loop task: fix in place, then run the checks in parallel.
# `check` re-runs fmt_check after fix already formatted — a trivially cheap
# redundancy kept so "the checks" stay a single definition.
all = Sequential(fix, check)

# Config is discovered by type under any binding (here `_`): bare `camas` runs
# default_task (`all`) — or github_task (`check`, no mutation in CI) under GitHub
# Actions. agent= wires the Claude Code plugin — agent.fix is the FileChanged
# autofix node, agent.check is the node the PostToolBatch gate runs (omit it to
# default to default_task).
#
# Structured-output note: to feed the gate machine-readable clippy diagnostics,
# move lint denials into Cargo.toml's [lints] table so the base command needs no
# trailing `-- -D warnings`, then append the format flag — camas adds agent_format
# args at the *end* of the command, which `cargo`'s `--message-format` requires be
# *before* `--`:
#   clippy = Task("cargo clippy --all-targets --all-features",
#                 agent_format=("--message-format=json", "raw"))

_ = Config(default_task=all, github_task=check, agent=Claude(fix=fix, check=check))

if __name__ == "__main__":
	run_cli(globals())
