# /// script
# requires-python = ">=3.14"
# dependencies = ["msgspec==0.21.1", "mypy==2.3.1"]
# ///
"""The mutation report the nightly workflow files.

``cargo mutants`` reports in the order it happened to test things. This reports by where the code
lives, because two hundred survivors are a claim about which *functions* the suite fails to pin
down. A mutant is a ``file:line``, which means nothing a few commits later, so each one links to the
commit it was found at.
"""

import re
import sys
from collections import Counter
from itertools import groupby
from pathlib import Path
from typing import NamedTuple

import msgspec

# GitHub rejects a body past 65536 characters. #65 was cut off just under it, losing the totals that
# sit at the tail; what does not fit here is counted in the report and kept in the run's artifact.
BODY_LIMIT = 60000


class Span(msgspec.Struct):
    """A mutant's position, the ``span.start`` of cargo-mutants' outcomes.json."""

    line: int
    column: int


class Mutant(msgspec.Struct):
    """One missed mutant's record — the fields the report's entries link by."""

    name: str
    file: str
    span: Span
    function: dict[str, str] | None = None


class Scenario(msgspec.Struct):
    """An outcome's scenario; a missed mutant's holds the record under ``Mutant``."""

    Mutant: "Mutant | None" = None


class Outcome(msgspec.Struct):
    """One outcome entry; the report reads only the missed-mutant ones."""

    summary: str
    scenario: Scenario | None = None


class OutcomesDoc(msgspec.Struct):
    """The whole outcomes.json — the counts, and the entries. An absent count decodes as zero, and
    a present count that is not a whole number fails the decode, which is the format change the
    merge's summary fallback exists for."""

    outcomes: list[Outcome] = []
    total_mutants: int = 0
    caught: int = 0
    missed: int = 0
    unviable: int = 0
    timeout: int = 0


class SummaryCounts(msgspec.Struct):
    """The counts a sweep.log summary line carries, zero for the fields cargo-mutants omitted."""

    total_mutants: int
    missed: int = 0
    caught: int = 0
    unviable: int = 0
    timeout: int = 0


class ShardCell(msgspec.Struct):
    """One shard cell of the plan — the same shape mutants_plan.py emits; one spelling would need a
    shared module the doctest runner cannot import."""

    file: str
    index: str
    shard: str = ""


class MergedDoc(msgspec.Struct):
    """The merge's output: every shard's outcomes and the summed counts."""

    outcomes: list[Outcome] = []
    total_mutants: int = 0
    caught: int = 0
    missed: int = 0
    unviable: int = 0
    timeout: int = 0


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


def loaded(path: Path) -> OutcomesDoc:
    try:
        return msgspec.json.decode(path.read_bytes(), type=OutcomesDoc)
    except (OSError, msgspec.ValidationError, msgspec.DecodeError) as err:
        raise SystemExit(f"{path}: not a decodable outcomes document ({err})") from err


def counts(outcomes: OutcomesDoc | MergedDoc) -> Counts:
    """The typed document's own counts; the decode already enforced that a present count is a whole
    number.

    >>> counts(OutcomesDoc(total_mutants=3))
    Counts(tested=3, caught=0, missed=0, unviable=0, timeout=0)
    """
    return Counts(
        tested=outcomes.total_mutants,
        caught=outcomes.caught,
        missed=outcomes.missed,
        unviable=outcomes.unviable,
        timeout=outcomes.timeout,
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


def survivor(mutant: Mutant) -> Survivor | None:
    if not mutant.name or not mutant.file:
        return None
    named = mutant.function.get("function_name") if mutant.function else None
    fn = named if named else "(no function)"
    return Survivor(
        mutant.file,
        fn,
        mutant.span.line,
        mutant.span.column,
        described(mutant.name, mutant.file, mutant.span.line, mutant.span.column, fn),
    )


def survivors(outcomes: OutcomesDoc | MergedDoc) -> tuple[Survivor, ...]:
    return tuple(
        one
        for outcome in outcomes.outcomes
        if outcome.summary == "MissedMutant"
        and outcome.scenario is not None
        and outcome.scenario.Mutant is not None
        and (one := survivor(outcome.scenario.Mutant)) is not None
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


def shard_map(shards: list[ShardCell]) -> str:
    """The footer's index-to-file lines, so an artifact name names its source.

    >>> shard_map([ShardCell(file="src/doc.rs", index="0")])
    '- `mutants-out-0` = `src/doc.rs`'
    >>> shard_map([ShardCell(file="src/doc.rs", index="0", shard="1/4")])
    '- `mutants-out-0` = `src/doc.rs` [shard 1/4]'
    """
    return "\n".join(
        f"- `mutants-out-{one.index}` = `{one.file}`"
        f"{f' [shard {one.shard}]' if one.shard else ''}"
        for one in shards
    )


def parsed_summary(text: str) -> SummaryCounts | None:
    """The counts cargo-mutants' summary line carries, the only record a timeout-bearing sweep
    leaves when it never writes outcomes.json. cargo-mutants omits a zero-valued field rather than
    printing it (a clean 6-mutant file prints no `missed` at all), so every field is optional and an
    absent one counts zero.

    >>> parsed_summary("612 mutants tested in 4h: 55 missed, 526 caught, 18 unviable, 13 timeouts")
    SummaryCounts(total_mutants=612, missed=55, caught=526, unviable=18, timeout=13)
    >>> parsed_summary("6 mutants tested in 4m: 5 caught, 1 unviable")
    SummaryCounts(total_mutants=6, missed=0, caught=5, unviable=1, timeout=0)
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
    counts_by_name = {
        name: int(value)
        for value, name in re.findall(
            r"(\d+) (missed|caught|unviable|timeouts)", matched.group("fields")
        )
    }
    return SummaryCounts(
        total_mutants=int(matched.group(1)),
        missed=counts_by_name.get("missed", 0),
        caught=counts_by_name.get("caught", 0),
        unviable=counts_by_name.get("unviable", 0),
        timeout=counts_by_name.get("timeouts", 0),
    )


def _summary_from_logs(index: str, merged_root: str, prior_root: str) -> SummaryCounts | None:
    """The sweep.log counts at either artifact depth, or the zero-mutant shape for that text."""
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
            return SummaryCounts(total_mutants=0)
    return None


def merge_shards(
    shards: list[ShardCell], merged_root: str, prior_root: str
) -> tuple[MergedDoc, list[str]]:
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
    >>> cells = [ShardCell(file="a", index=str(i)) for i in range(4)]
    >>> merge_shards(cells, str(root / "merged"), str(root / "prior"))
    (MergedDoc(outcomes=[Outcome(summary='CaughtMutant', scenario=None)], total_mutants=617, caught=529, missed=55, unviable=18, timeout=13), ['a'])

    The corrupt outcomes.json is named missing, the summary-only shard contributes its counts, and
    the zero-mutant shard contributes nothing.
    """
    merged = MergedDoc()
    missing: list[str] = []
    for cell in shards:
        name = f"{cell.file} [shard {cell.shard}]" if cell.shard else cell.file
        candidates = (
            Path(f"{merged_root}/mutants-out-{cell.index}/mutants.out/outcomes.json"),
            Path(f"{merged_root}/mutants-out-{cell.index}/outcomes.json"),
            Path(f"{prior_root}-{cell.index}/mutants.out/outcomes.json"),
            Path(f"{prior_root}-{cell.index}/outcomes.json"),
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
            summary = _summary_from_logs(cell.index, merged_root, prior_root)
            if summary is None:
                missing.append(name)
                continue
            merged.total_mutants += summary.total_mutants
            merged.missed += summary.missed
            merged.caught += summary.caught
            merged.unviable += summary.unviable
            merged.timeout += summary.timeout
            continue
        if data.total_mutants == 0 and data.outcomes:
            raise SystemExit(f"{name}: reports no mutants tested but holds outcome entries")
        if data.missed == 0 and any(one.summary == "MissedMutant" for one in data.outcomes):
            raise SystemExit(f"{name}: reports no missed but holds MissedMutant entries")
        merged.outcomes.extend(data.outcomes)
        merged.total_mutants += data.total_mutants
        merged.missed += data.missed
        merged.caught += data.caught
        merged.unviable += data.unviable
        merged.timeout += data.timeout
    return merged, missing


def body(
    outcomes: MergedDoc,
    tally: Counts,
    repo: str,
    sha: str,
    run: str,
    shards: list[ShardCell],
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


def tested(path: Path) -> tuple[MergedDoc, Counts]:
    """A run that tested nothing is a format change or an aborted run, not a clean sweep."""
    try:
        data = msgspec.json.decode(path.read_bytes(), type=MergedDoc)
    except (OSError, msgspec.ValidationError, msgspec.DecodeError) as err:
        raise SystemExit(f"{path}: not a decodable merged outcomes document ({err})") from err
    tally = counts(data)
    if not tally.tested:
        raise SystemExit(f"{path}: reports no mutants tested")
    return data, tally


def main(argv: tuple[str, ...]) -> int:
    match argv:
        case ("report", outcomes, repo, sha, run, out, shards_json):
            data, tally = tested(Path(outcomes))
            Path(out).write_text(
                body(
                    data,
                    tally,
                    repo,
                    sha,
                    run,
                    msgspec.json.decode(shards_json, type=list[ShardCell]),
                ),
                encoding="utf-8",
            )
            print(f"missed={tally.missed}")
            print(f"title={title(tally, sha)}")
        case ("merge", shards_json, merged_root, prior_root, out, missing_txt, empty_txt):
            shards = msgspec.json.decode(shards_json, type=list[ShardCell])
            merged_doc, missing = merge_shards(shards, merged_root, prior_root)
            destination = Path(out)
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(msgspec.json.encode(merged_doc))
            Path(missing_txt).write_text("\n".join(missing), encoding="utf-8")
            if missing:
                head, *rest = missing
                more = f", ... ({len(rest)} more)" if rest else ""
                print(
                    f"::warning::{len(missing)} shards left no outcomes: {head}{more}",
                    file=sys.stderr,
                )
            print(f"missing={'true' if missing else 'false'}")
            empty = not merged_doc.outcomes and merged_doc.total_mutants == 0
            print(f"empty={'true' if empty else 'false'}")
            print(f"ran={'true' if shards else 'false'}")
            all_missing = bool(shards) and len(missing) == len(shards)
            print(f"all_missing={'true' if all_missing else 'false'}")
            if empty:
                message = (
                    "the sweep did not run — the plan left no shard jobs\n"
                    if not shards
                    else "no shard produced outcomes — every shard left nothing\n"
                    if all_missing
                    else "some shards left no outcomes; the rest found zero mutants\n"
                    if missing
                    else "every shard completed with zero mutants\n"
                )
                Path(empty_txt).write_text(message, encoding="utf-8")
        case ("--self-test",):
            import doctest

            return doctest.testmod().failed
        case ("--self-check", *paths):
            from mypy import api

            stdout, stderr, status = api.run(["--strict", *paths])
            sys.stdout.write(stdout)
            sys.stderr.write(stderr)
            return status
        case _:
            print(__doc__)
            print("       mutants_report.py merge SHARDS_JSON MERGED_ROOT PRIOR_ROOT OUT MISSING_TXT EMPTY_TXT")
            print("       mutants_report.py report OUTCOMES REPO SHA RUN_URL OUT_FILE SHARDS_JSON")
            print("       mutants_report.py --self-test | --self-check FILES...")
            return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main(tuple(sys.argv[1:])))
