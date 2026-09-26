# /// script
# requires-python = ">=3.14"
# dependencies = ["camas[mcp]==0.1.29"]
# ///
"""The workflow scripts' checks — run with ``camas``, or from the root as ``camas github``."""

from camas import Config, Parallel, Task, run_cli

REPORT = "workflows/mutants_report.py"

py_types = Task(
	f"uv run --python 3.14 --script {REPORT} --self-check {{paths}}",
	paths=REPORT,
)
py_doctest = Task(f'uv run --python 3.14 --script {REPORT} --self-test', when=REPORT)
# Not `camas --check` (JPHutchins/camas#277).
types = Task("uv run tasks.py --check", when="tasks.py")

check = Parallel(py_types, py_doctest, types)

_ = Config(default_task=check, github_task=check)

if __name__ == "__main__":
	run_cli(globals())
