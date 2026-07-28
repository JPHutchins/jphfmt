"""The VS Code extension's tasks — jphfmt's TypeScript half, run with ``camas``.

Mounted by the root ``tasks.py`` as ``Project("editors/vscode")``: every node's
``cwd`` and change-scope rebase to this directory, so camas runs these from here
and a change touching nothing under it prunes them all. ``node_modules`` comes
from the flake dev shell (``importNpmLock``), so there is no install step.

The npm scripts stay the entry point rather than ``{paths}``-scoped tool calls:
prettier's and eslint's file sets are declared in ``package.json``, and this
directory is small enough that narrowing them would cost more in duplication
than it saves in time. Neither has a structured output format to hand the agent
gate — ESLint 9 moved its ``junit`` formatter out of core — so no leaf here sets
``agent_format``.
"""

from camas import Claude, Config, Parallel, Sequential, Task

fmt = Task("npm run format", mutates=True)
fmt_check = Task("npm run format:check")
lint = Task("npm run lint")
lint_fix = Task("npm run lint:fix", mutates=True)
typecheck = Task("npm run typecheck")
build = Task("npm run build")
knip = Task("npx --yes knip")

# Compile-validation is `tsc --noEmit` plus the real bundle — what the marketplace package ships.
check = Parallel(fmt_check, lint, typecheck, build, knip)
# lint-fix first so prettier has the last word.
fix = Sequential(lint_fix, fmt)
all = Sequential(fix, check)

_ = Config(default_task=all, github_task=check, agent=Claude(fix=fix, check=check))
