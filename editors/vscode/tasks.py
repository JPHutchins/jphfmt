# /// script
# requires-python = ">=3.14"
# dependencies = ["camas[mcp]==0.1.29"]
# ///
"""The VS Code extension's tasks — run with ``camas``, or from the root as ``camas vscode``."""

from camas import Claude, Config, Parallel, Sequential, Task, run_cli

fmt = Task("npm run format", mutates=True)
fmt_check = Task("npm run format:check")
lint = Task("npm run lint")
lint_fix = Task("npm run lint:fix", mutates=True)
typecheck = Task("npm run typecheck")
build = Task("npm run build")
knip = Task("npx --yes knip")
# Not `camas --check`: ty cannot resolve camas from the flake's store path (JPHutchins/camas#277).
types = Task("uv run tasks.py --check", when="tasks.py")

check = Parallel(fmt_check, lint, typecheck, build, knip, types)
fix = Sequential(lint_fix, fmt)
all = Sequential(fix, check)

_ = Config(default_task=all, github_task=check, agent=Claude(fix=fix, check=check))

if __name__ == "__main__":
	run_cli(globals())
