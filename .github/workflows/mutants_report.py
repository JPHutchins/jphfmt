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


def integer(value: object) -> int | None:
	"""`bool` subclasses `int`, and a JSON `true` must not become line 1.

	>>> integer(3), integer(True), integer("3")
	(3, None, None)
	"""
	return value if type(value) is int else None


def listed(value: object) -> list[Any]:
	"""`{"outcomes": null}` is valid JSON and not iterable.

	>>> listed([1]), listed(None), listed("xs")
	([1], [], [])
	"""
	return value if isinstance(value, list) else []


def loaded(path: Path) -> Json:
	try:
		value = json.loads(path.read_bytes())
	except (OSError, ValueError) as err:
		raise SystemExit(f"{path}: not readable as JSON ({err})") from err
	if not isinstance(value, dict):
		raise SystemExit(f"{path}: expected an object")
	return value


def counts(outcomes: Json) -> Counts:
	"""An absent count is nought; a count that is present and not a number is a format change.

	>>> counts({"total_mutants": 3})
	Counts(tested=3, caught=0, missed=0, unviable=0, timeout=0)
	"""

	def tally(field: str) -> int:
		if field not in outcomes:
			return 0
		value = integer(outcomes[field])
		if value is None:
			raise SystemExit(f"{field}: expected a whole number, got {outcomes[field]!r}")
		return value

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
	line, column = integer(start.get("line")), integer(start.get("column"))
	if line is None or column is None:
		return None
	named = function.get("function_name") if isinstance(function, dict) else None
	fn = named if isinstance(named, str) else "(no function)"
	return Survivor(file, fn, line, column, described(name, file, line, column, fn))


def survivors(outcomes: Json) -> tuple[Survivor, ...]:
	scenarios = (
		outcome.get("scenario")
		for outcome in listed(outcomes.get("outcomes"))
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



def shard_map(shards: list[Any]) -> str:
    """The footer's index-to-file lines, so an artifact name names its source.

    >>> shard_map([{"file": "src/doc.rs", "index": "0"}])
    '- `mutants-out-0` = `src/doc.rs`'
    """
    return "\n".join(
        f"- `mutants-out-{one.get('index')}` = `{one.get('file')}`"
        for one in shards
        if isinstance(one, dict)
    )


def parsed_summary(text: str) -> dict[str, int] | None:
    """The counts cargo-mutants' summary line carries, the only record a timeout-bearing
    sweep leaves when it never writes outcomes.json. cargo-mutants omits a zero-valued
    field rather than printing it (a clean 6-mutant file prints no `missed` at all), so
    every field is optional and an absent one counts zero.

    >>> parsed_summary("612 mutants tested in 4h: 55 missed, 526 caught, 18 unviable, 13 timeouts")
    {'total_mutants': 612, 'missed': 55, 'caught': 526, 'unviable': 18, 'timeout': 13}
    >>> parsed_summary("6 mutants tested in 4m: 5 caught, 1 unviable")
    {'total_mutants': 6, 'missed': 0, 'caught': 5, 'unviable': 1, 'timeout': 0}
    >>> parsed_summary("Found 0 mutants to test") is None
    True
    """
    matched = re.search(
        r"(\d+) mutants tested in [^:]*: "
        r"(?P<fields>(?:\d+ (?:missed|caught|unviable|timeouts)(?:, )?)+)",
        text,
    )
    if not matched:
        return None
    counts = {
        name: int(value)
        for value, name in re.findall(
            r"(\d+) (missed|caught|unviable|timeouts)", matched.group("fields")
        )
    }
    return {
        "total_mutants": int(matched.group(1)),
        "missed": counts.get("missed", 0),
        "caught": counts.get("caught", 0),
        "unviable": counts.get("unviable", 0),
        "timeout": counts.get("timeouts", 0),
    }


def _summary_from_logs(index: str, merged_root: str, prior_root: str) -> dict[str, int] | None:
    """The sweep.log counts at either artifact depth, or the empty dict for a zero-mutant file."""
    for log in (
        Path(f"{merged_root}/mutants-out-{index}/mutants.out/sweep.log"),
        Path(f"{merged_root}/mutants-out-{index}/sweep.log"),
        Path(f"{prior_root}-{index}/mutants.out/sweep.log"),
        Path(f"{prior_root}-{index}/sweep.log"),
    ):
        if not log.exists():
            continue
        try:
            text = log.read_text(encoding="utf-8")
        except (OSError, ValueError):
            continue
        summary = parsed_summary(text)
        if summary is not None:
            return summary
        if "Found 0 mutants to test" in text:
            return {}
    return None


def merge_shards(shards: list[Any], merged_root: str, prior_root: str) -> tuple[Json, list[str]]:
    """One merged outcomes document and the shard files that left no readable one.

    The four branch shapes, exercised end to end in a temp directory:

    >>> import tempfile
    >>> tmp = tempfile.mkdtemp()
    >>> root = Path(tmp)
    >>> _ = (root / "merged/mutants-out-0/mutants.out").mkdir(parents=True)
    >>> _ = (root / "merged/mutants-out-0/mutants.out/outcomes.json").write_text('{"total_mutants": 5, "caught": 3, "outcomes": [{"summary": "CaughtMutant"}]}', encoding="utf-8")
    >>> _ = (root / "merged/mutants-out-0/complete").touch()
    >>> _ = (root / "merged/mutants-out-1/mutants.out").mkdir(parents=True)
    >>> _ = (root / "merged/mutants-out-1/mutants.out/outcomes.json").write_text("{corrupt", encoding="utf-8")
    >>> _ = (root / "merged/mutants-out-2/mutants.out").mkdir(parents=True)
    >>> _ = (root / "merged/mutants-out-2/mutants.out/sweep.log").write_text("612 mutants tested in 4h: 55 missed, 526 caught, 18 unviable, 13 timeouts", encoding="utf-8")
    >>> _ = (root / "merged/mutants-out-2/complete").touch()
    >>> _ = (root / "merged/mutants-out-3/mutants.out").mkdir(parents=True)
    >>> _ = (root / "merged/mutants-out-3/mutants.out/sweep.log").write_text("Found 0 mutants to test", encoding="utf-8")
    >>> _ = (root / "merged/mutants-out-3/complete").touch()
    >>> cells = [{"file": "a", "index": str(i)} for i in range(4)]
    >>> merge_shards(cells, str(root / "merged"), str(root / "prior"))
    ({'outcomes': [{'summary': 'CaughtMutant'}], 'total_mutants': 617, 'caught': 529, 'missed': 55, 'unviable': 18, 'timeout': 13}, ['a'])

    The corrupt outcomes.json is named missing, the summary-only shard contributes its
    counts, and the zero-mutant shard contributes nothing.
    """
    outcomes: list[Any] = []
    totals: dict[str, int] = {}
    missing: list[str] = []
    for shard in shards:
        if not isinstance(shard, dict):
            raise SystemExit(f"shard cell malformed: {shard!r}")
        file, index = shard.get("file"), shard.get("index")
        if not isinstance(file, str) or not isinstance(index, str):
            raise SystemExit(f"shard cell malformed: {shard!r}")
        candidates = (
            Path(f"{merged_root}/mutants-out-{index}/mutants.out/outcomes.json"),
            Path(f"{merged_root}/mutants-out-{index}/outcomes.json"),
            Path(f"{prior_root}-{index}/mutants.out/outcomes.json"),
            Path(f"{prior_root}-{index}/outcomes.json"),
        )
        data = None
        for candidate in candidates:
            if not candidate.exists():
                continue
            try:
                data = loaded(candidate)
                break
            except SystemExit:
                continue
        if data is None:
            summary = _summary_from_logs(index, merged_root, prior_root)
            if summary is None:
                missing.append(file)
                continue
            for field, value in summary.items():
                totals[field] = totals.get(field, 0) + value
            continue
        try:
            tallied = counts(data)
            entries = listed(data.get("outcomes"))
            if tallied.tested == 0 and entries:
                raise SystemExit("reports no mutants tested but holds outcome entries")
            if tallied.missed == 0 and any(
                isinstance(one, dict) and one.get("summary") == "MissedMutant" for one in entries
            ):
                raise SystemExit("reports no missed but holds MissedMutant entries")
            outcomes.extend(one for one in entries if isinstance(one, dict))
            totals["total_mutants"] = totals.get("total_mutants", 0) + tallied.tested
            for field in ("caught", "missed", "unviable", "timeout"):
                totals[field] = totals.get(field, 0) + getattr(tallied, field)
        except SystemExit as err:
            summary = _summary_from_logs(index, merged_root, prior_root)
            if summary is not None:
                for field, value in summary.items():
                    totals[field] = totals.get(field, 0) + value
            else:
                missing.append(f"{file} ({err})")
    return {"outcomes": outcomes, **totals}, missing


def body(
    outcomes: Json,
    tally: Counts,
    repo: str,
    sha: str,
    run: str,
    shards: list[Any] | None = None,
) -> str:
	found = survivors(outcomes)
	commit = f"[`{sha[:7]}`](https://github.com/{repo}/tree/{sha})"
	head = (f"## Mutation testing — {commit}", "", summary_table(tally), "")
	map_lines = f"\n{shard_map(shards)}\n" if shards else ""
	footer = (
		"<sub>Logs and a per-mutant diff for each survivor are in the `mutants-out-<index>` "
		f"artifacts of [the run]({run}) and of the prior completed runs whose shards it resumed; "
		"the issue body names the shards that left no outcomes.</sub>\n" + map_lines
	)
	if not found:
		claim = (
			"Every mutant was caught. Nothing to triage."
			if not tally.missed
			else f"The summary counts {tally.missed} missed, and no `MissedMutant` entry was found "
			"for any of them — read the artifact rather than this, and check whether "
			"cargo-mutants' output format moved."
		)
		return "\n".join((*head, claim, "", footer)) + "\n"
	preamble = (
		*head,
		f"{len(found)} mutants survived the suite — the tests pass with the change applied, so "
		f"nothing pins that code down. Every entry links to its line at {commit}.",
		"",
	)
	note = (
		f"<sub>{{}} further survivors are omitted to fit GitHub's issue body limit; the artifacts "
		f"have all {len(found)}.</sub>\n"
	)
	# Budget the scaffold as rendered, separators included, and the note at its longest — dropping
	# every survivor — so what `fitted` is told is spare really is.
	scaffold = len("\n".join((*preamble, "", note.format(len(found)), footer)))
	shown, dropped = fitted(
		ordered(found), f"https://github.com/{repo}/blob/{sha}", BODY_LIMIT - scaffold
	)
	omitted = (note.format(dropped),) if dropped else ()
	return "\n".join((*preamble, shown, *omitted, footer))


def tested(path: Path) -> tuple[Json, Counts]:
	"""A run that tested nothing is a format change or an aborted run, not a clean sweep."""
	data = loaded(path)
	tally = counts(data)
	if not tally.tested:
		raise SystemExit(f"{path}: reports no mutants tested")
	return data, tally


def main(argv: tuple[str, ...]) -> int:
	match argv:
		case ("report", outcomes, repo, sha, run, out, shards_json):
			data, tally = tested(Path(outcomes))
			Path(out).write_text(
				body(data, tally, repo, sha, run, json.loads(shards_json)), encoding="utf-8"
			)
			print(f"missed={tally.missed}")
			print(f"title={title(tally, sha)}")
		case ("merge", shards_json, merged_root, prior_root, out, missing_txt, empty_txt):
			shards = json.loads(shards_json)
			merged_doc, missing = merge_shards(shards, merged_root, prior_root)
			destination = Path(out)
			destination.parent.mkdir(parents=True, exist_ok=True)
			destination.write_text(json.dumps(merged_doc), encoding="utf-8")
			Path(missing_txt).write_text("\n".join(missing), encoding="utf-8")
			if missing:
				head, *rest = missing
				more = f", ... ({len(rest)} more)" if rest else ""
				print(
					f"::warning::{len(missing)} shards left no outcomes: {head}{more}",
					file=sys.stderr,
				)
			print(f"missing={'true' if missing else 'false'}")
			empty = not merged_doc["outcomes"] and merged_doc.get("total_mutants", 0) == 0
			print(f"empty={'true' if empty else 'false'}")
			print(f"ran={'true' if shards else 'false'}")
			if empty:
				Path(empty_txt).write_text("no shard produced outcomes — the sweep did not run\n", encoding="utf-8")
		case _:
			print(__doc__)
			print("       mutants_report.py merge SHARDS_JSON MERGED_ROOT PRIOR_ROOT OUT MISSING_TXT EMPTY_TXT")
			print("       mutants_report.py report OUTCOMES REPO SHA RUN_URL OUT_FILE SHARDS_JSON")
			return 2
	return 0


if __name__ == "__main__":
	raise SystemExit(main(tuple(sys.argv[1:])))
