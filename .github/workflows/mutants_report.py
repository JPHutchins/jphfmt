# /// script
# requires-python = ">=3.14"
# ///
"""The mutation report the nightly workflow files.

``cargo mutants`` reports in the order it happened to test things. This reports by where the code
lives, because two hundred survivors are a claim about which *functions* the suite fails to pin
down. A mutant is a ``file:line``, which means nothing a few commits later, so each one links to the
commit it was found at.
"""

import json
import re
import sys
from collections import Counter
from itertools import groupby
from pathlib import Path
from typing import Any, NamedTuple

# GitHub rejects a body past 65536 characters. #65 was cut off just under it, losing the totals that
# sit at the tail; what does not fit here is counted in the report and kept in the run's artifact.
BODY_LIMIT = 60000

type Json = dict[str, Any]


class Counts(NamedTuple):
	tested: int
	caught: int
	missed: int
	unviable: int
	timeout: int

	def percent(self, part: int) -> str:
		return f"{round(100 * part / self.tested)}%" if self.tested else "—"


class Survivor(NamedTuple):
	file: str
	function: str
	line: int
	column: int
	change: str


def loaded(path: Path) -> Json:
	try:
		value = json.loads(path.read_bytes())
	except (OSError, ValueError) as err:
		raise SystemExit(f"{path}: not readable as JSON ({err})") from err
	if not isinstance(value, dict):
		raise SystemExit(f"{path}: expected an object")
	return value


def counts(outcomes: Json) -> Counts:
	def tally(field: str) -> int:
		value = outcomes.get(field)
		return value if isinstance(value, int) else 0

	return Counts(
		tested=tally("total_mutants"),
		caught=tally("caught"),
		missed=tally("missed"),
		unviable=tally("unviable"),
		timeout=tally("timeout"),
	)


def described(name: str, file: str, line: int, column: int, function: str) -> str:
	"""What is left of a mutant's name once the report's own grouping has said the rest.

	>>> described("src/doc.rs:87:29: replace += with -= in render", "src/doc.rs", 87, 29, "render")
	'replace += with -='
	>>> described("a.rs:1:1: replace f -> usize with 0", "a.rs", 1, 1, "f")
	'replace f -> usize with 0'
	"""
	return name.removeprefix(f"{file}:{line}:{column}: ").removesuffix(f" in {function}")


def code(text: str) -> str:
	"""A mutant's name is Rust source, where `*`, `|` and `_` are all live markdown.

	>>> code("replace + with *")
	'`replace + with *`'
	>>> code("delete `x`")
	'`` delete `x` ``'
	"""
	longest = max((len(run) for run in re.findall(r"`+", text)), default=0)
	fence, pad = "`" * (longest + 1), " " if longest else ""
	return f"{fence}{pad}{text}{pad}{fence}"


def survivor(mutant: Json, name: str) -> Survivor | None:
	span, function, file = mutant.get("span"), mutant.get("function"), mutant.get("file")
	start = span.get("start") if isinstance(span, dict) else None
	if not isinstance(start, dict) or not isinstance(file, str) or not isinstance(name, str):
		return None
	line, column = start.get("line"), start.get("column")
	if not isinstance(line, int) or not isinstance(column, int):
		return None
	named = function.get("function_name") if isinstance(function, dict) else None
	fn = named if isinstance(named, str) else "(no function)"
	return Survivor(file, fn, line, column, described(name, file, line, column, fn))


def survivors(outcomes: Json) -> tuple[Survivor, ...]:
	scenarios = (
		outcome.get("scenario")
		for outcome in outcomes.get("outcomes", ())
		if isinstance(outcome, dict) and outcome.get("summary") == "MissedMutant"
	)
	mutants = (scenario.get("Mutant") for scenario in scenarios if isinstance(scenario, dict))
	return tuple(
		one
		for mutant in mutants
		if isinstance(mutant, dict) and (one := survivor(mutant, mutant.get("name", ""))) is not None
	)


def ordered(found: tuple[Survivor, ...]) -> tuple[Survivor, ...]:
	"""Densest first, so a prefix cut to fit the body limit is still the most useful prefix."""
	files = Counter(one.file for one in found)
	functions = Counter((one.file, one.function) for one in found)
	return tuple(
		sorted(
			found,
			key=lambda one: (
				-files[one.file],
				one.file,
				-functions[one.file, one.function],
				one.function,
				one.line,
				one.column,
				one.change,
			),
		)
	)


def summary_table(tally: Counts) -> str:
	return "\n".join(
		(
			"| outcome | count | |",
			"| --- | --: | --: |",
			f"| **caught** | {tally.caught} | {tally.percent(tally.caught)} |",
			f"| **missed** | {tally.missed} | {tally.percent(tally.missed)} |",
			f"| unviable | {tally.unviable} | |",
			f"| timeout | {tally.timeout} | |",
			f"| **tested** | **{tally.tested}** | |",
		)
	)


def entry(one: Survivor, link: str) -> str:
	name = one.file.rsplit("/", 1)[-1]
	return f"- [`{name}:{one.line}`]({link}/{one.file}#L{one.line}) — {code(one.change)}"


def function_lines(function: str, items: tuple[Survivor, ...], link: str) -> tuple[str, ...]:
	return (f"**`{function}`** ({len(items)})", "", *(entry(one, link) for one in items), "")


def file_section(file: str, items: tuple[Survivor, ...], link: str) -> str:
	return "\n".join(
		(
			f"<details><summary><b>{file}</b> — {len(items)} survived</summary>",
			"",
			*(
				line
				for function, group in groupby(items, key=lambda one: one.function)
				for line in function_lines(function, tuple(group), link)
			),
			"</details>",
			"",
		)
	)


def inventory(shown: tuple[Survivor, ...], link: str) -> str:
	"""Grouped by iteration, which holds because [`ordered`] leaves each file's entries adjacent — so
	a prefix renders as exactly the head of the whole."""
	return "\n".join(
		file_section(file, tuple(group), link)
		for file, group in groupby(shown, key=lambda one: one.file)
	)


def fitted(order: tuple[Survivor, ...], link: str, spare: int) -> tuple[str, int]:
	"""Bisects on the entry count, which is sound because rendered length only grows with it. Per
	entry rather than per section, because one file's section can exceed the whole budget."""
	whole = inventory(order, link)
	if len(whole) <= spare:
		return whole, 0
	fits, over = 0, len(order)
	while over - fits > 1:
		half = (fits + over) // 2
		fits, over = (half, over) if len(inventory(order[:half], link)) <= spare else (fits, half)
	return inventory(order[:fits], link), len(order) - fits


def title(tally: Counts, sha: str) -> str:
	if not tally.missed:
		return f"Mutation testing: all {tally.tested} mutants caught at {sha[:7]}"
	return f"Mutation testing: {tally.missed} surviving mutants at {sha[:7]}"


def body(outcomes: Json, repo: str, sha: str, run: str) -> str:
	tally, found = counts(outcomes), survivors(outcomes)
	commit = f"[`{sha[:7]}`](https://github.com/{repo}/tree/{sha})"
	head = (f"## Mutation testing — {commit}", "", summary_table(tally), "")
	if not found:
		return "\n".join((*head, "Every mutant was caught. Nothing to triage.")) + "\n"
	preamble = (
		*head,
		f"{len(found)} mutants survived the suite — the tests pass with the change applied, so "
		f"nothing pins that code down. Every entry links to its line at {commit}.",
		"",
	)
	footer = (
		"<sub>Logs and a per-mutant diff for every one of these are in the `mutants-out` artifact "
		f"of [the run]({run}).</sub>\n"
	)
	shown, dropped = fitted(
		ordered(found),
		f"https://github.com/{repo}/blob/{sha}",
		BODY_LIMIT - len("\n".join(preamble)) - len(footer),
	)
	omitted = (
		f"<sub>{dropped} further survivors are omitted to fit GitHub's issue body limit; the "
		f"artifact has all {len(found)}.</sub>\n",
	)
	return "\n".join((*preamble, shown, *(omitted if dropped else ()), footer))


def main(argv: tuple[str, ...]) -> int:
	match argv:
		case ("body", outcomes, repo, sha, run):
			print(body(loaded(Path(outcomes)), repo, sha, run), end="")
		case ("title", outcomes, sha):
			print(title(counts(loaded(Path(outcomes))), sha))
		case ("missed", outcomes):
			print(counts(loaded(Path(outcomes))).missed)
		case _:
			print(__doc__)
			print("usage: mutants_report.py body OUTCOMES REPO SHA RUN_URL | title OUTCOMES SHA")
			print("       mutants_report.py missed OUTCOMES")
			return 2
	return 0


if __name__ == "__main__":
	raise SystemExit(main(tuple(sys.argv[1:])))
