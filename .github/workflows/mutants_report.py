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


class Position(msgspec.Struct):
    """One end of a span; cargo-mutants nests both ends, and the report reads the start."""

    line: int
    column: int


class Span(msgspec.Struct):
    """A mutant's position in the shape cargo-mutants writes.

    >>> msgspec.json.decode(
    ...     b'{"start": {"line": 102, "column": 13}, "end": {"line": 102, "column": 40}}', type=Span
    ... )
    Span(start=Position(line=102, column=13), end=Position(line=102, column=40))
    """

    start: Position
    end: Position


class Function(msgspec.Struct):
    """The function a mutant lives in; cargo-mutants writes more keys than the report reads."""

    function_name: str | None = None


class Mutant(msgspec.Struct):
    """One missed mutant's record — the fields the report's entries link by."""

    name: str
    file: str
    span: Span
    function: Function | None = None


class Scenario(msgspec.Struct):
    """An outcome's scenario; a missed mutant's holds the record under the JSON key ``Mutant``."""

    mutant: Mutant | None = msgspec.field(default=None, name="Mutant")


class Outcome(msgspec.Struct):
    """One outcome entry; cargo-mutants writes ``scenario`` as a name string for the simple
    outcomes and as an object for a missed mutant — the report reads only the object ones."""

    summary: str | None = None
    scenario: Scenario | str | None = None


class Totals(msgspec.Struct):
    """The five counts every outcomes shape carries; the decode zeroes an absent field, and a
    present count that is not a whole number fails the decode, which is the format change the
    merge's summary fallback exists for."""

    total_mutants: int = 0
    caught: int = 0
    missed: int = 0
    unviable: int = 0
    timeout: int = 0


class OutcomesDoc(Totals):
    """The whole outcomes.json — the counts, and the entries as raw JSON. A null list decodes
    as None, and each entry decodes on its own, so one malformed entry cannot forfeit the
    readable rest of the document.

    >>> msgspec.json.decode(b'{"total_mutants": 1, "outcomes": null}', type=OutcomesDoc).outcomes is None
    True
    >>> entries(msgspec.json.decode(b'{"outcomes": ["Baseline"]}', type=OutcomesDoc))
    []
    """

    outcomes: list[msgspec.Raw] | None = None


class SummaryCounts(Totals):
    """The counts a sweep.log summary line carries, zero for the fields cargo-mutants omitted."""


class MergedDoc(OutcomesDoc):
    """The merge's output: every shard's outcomes and the summed counts."""




class Cell(msgspec.Struct):
    """One shard cell of the plan before it is keyed: a file and its --shard slice when it has one."""

    file: str
    index: str
    shard: str = ""


# The one file whose whole-file sweep kept losing its runner (#65). A rename of the file must
# rename this too; a plan whose list no longer holds it warns rather than silently degrading to a
# whole-file cell.
SHARDED = "src/reflow/builders.rs"


def plan(files: list[str], shards: int) -> list[Cell]:
    """One cell per file, and SHARDED split into cargo-mutants' own --shard slices.

    >>> plan(["src/a.rs", "src/reflow/builders.rs"], 4)
    [Cell(file='src/a.rs', index='0', shard=''), Cell(file='src/reflow/builders.rs', index='1', shard='0/4'), Cell(file='src/reflow/builders.rs', index='2', shard='1/4'), Cell(file='src/reflow/builders.rs', index='3', shard='2/4'), Cell(file='src/reflow/builders.rs', index='4', shard='3/4')]
    """
    cells: list[Cell] = []
    if SHARDED not in files:
        print(f"::warning::{SHARDED} is not in the plan; it sweeps as a whole file", file=sys.stderr)
    for file in files:
        if file == SHARDED:
            for part in range(shards):
                cells.append(Cell(file=file, index=str(len(cells)), shard=f"{part}/{shards}"))
        else:
            cells.append(Cell(file=file, index=str(len(cells))))
    return cells


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


def loaded[T](path: Path, type: type[T]) -> T:
    try:
        return msgspec.json.decode(path.read_bytes(), type=type)
    except (OSError, msgspec.ValidationError, msgspec.DecodeError) as err:
        raise SystemExit(f"{path}: not a decodable outcomes document ({err})") from err


def add_counts(merged: MergedDoc, totals: Totals) -> None:
    merged.total_mutants += totals.total_mutants
    merged.missed += totals.missed
    merged.caught += totals.caught
    merged.unviable += totals.unviable
    merged.timeout += totals.timeout


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
    fn = mutant.function.function_name if mutant.function else None
    fn = fn if fn else "(no function)"
    return Survivor(
        mutant.file,
        fn,
        mutant.span.start.line,
        mutant.span.start.column,
        described(mutant.name, mutant.file, mutant.span.start.line, mutant.span.start.column, fn),
    )


def readable(outcomes: OutcomesDoc | MergedDoc) -> list[msgspec.Raw]:
    """The raw entries that decode on their own; one malformed entry is skipped, and the merged
    document carries the readable rest verbatim."""
    kept: list[msgspec.Raw] = []
    for one in outcomes.outcomes or []:
        try:
            msgspec.json.decode(one, type=Outcome)
        except (msgspec.ValidationError, msgspec.DecodeError):
            continue
        kept.append(one)
    return kept


def entries(outcomes: OutcomesDoc | MergedDoc) -> list[Outcome]:
    """Each entry decoded on its own; one malformed entry is skipped, the readable rest kept."""
    return [msgspec.json.decode(one, type=Outcome) for one in readable(outcomes)]


def drift_reason(data: OutcomesDoc) -> str | None:
    """The drift guards, in one place: the merge names the shard by the reason, and the sweep
    marker refuses to mark a shard the merge would reject — so a re-dispatch re-runs exactly the
    shards that can be fixed, never the ones that deterministically re-reject."""
    if data.total_mutants == 0 and (data.outcomes or []):
        return "reports no mutants tested but holds outcome entries"
    if data.missed == 0 and any(one.summary == "MissedMutant" for one in entries(data)):
        return "reports no missed but holds MissedMutant entries"
    return None


def survivors(outcomes: OutcomesDoc | MergedDoc) -> tuple[Survivor, ...]:
    """The missed-mutant entries, pinned by a record in the shape a real outcomes.json carries.

    >>> doc = msgspec.json.decode(
    ...     b'{"total_mutants": 1, "missed": 1, "outcomes": [{"summary": "MissedMutant", "scenario": {"Mutant": {"name": "src/doc.rs:102:13: delete match arm in last_line_width", "file": "src/doc.rs", "span": {"start": {"line": 102, "column": 13}, "end": {"line": 102, "column": 40}}, "function": {"function_name": "last_line_width"}}}}]}',
    ...     type=OutcomesDoc,
    ... )
    >>> survivors(doc)
    (Survivor(file='src/doc.rs', function='last_line_width', line=102, column=13, change='delete match arm'),)
    """
    return tuple(
        one
        for outcome in entries(outcomes)
        if outcome.summary == "MissedMutant"
        and isinstance(outcome.scenario, Scenario)
        and outcome.scenario.mutant is not None
        and (one := survivor(outcome.scenario.mutant)) is not None
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


def shard_map(shards: list[Cell]) -> str:
    """The footer's index-to-file lines, so an artifact name names its source.

    >>> shard_map([Cell(file="src/doc.rs", index="0")])
    '- `mutants-out-0` = `src/doc.rs`'
    >>> shard_map([Cell(file="src/doc.rs", index="0", shard="1/4")])
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
    SummaryCounts(total_mutants=612, caught=526, missed=55, unviable=18, timeout=13)
    >>> parsed_summary("6 mutants tested in 4m: 5 caught, 1 unviable")
    SummaryCounts(total_mutants=6, caught=5, missed=0, unviable=1, timeout=0)
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


ZERO_MUTANTS = "Found 0 mutants to test"


def sweep_marked(outcomes_dir: Path, log: Path) -> bool:
    """A shard counts as analyzed iff its outcomes file decodes, its log names a zero-mutant
    file, or its log carries the parseable summary — the same evidence the merge's fallback
    reads, so the marker and the fallback cannot disagree.

    >>> from tempfile import TemporaryDirectory
    >>> with TemporaryDirectory() as tmp:
    ...     _ = Path(tmp, "sweep.log").write_text("6 mutants tested in 4m: 5 caught, 1 unviable", encoding="utf-8")
    ...     sweep_marked(Path(tmp), Path(tmp, "sweep.log"))
    True
    >>> with TemporaryDirectory() as tmp:
    ...     _ = Path(tmp, "sweep.log").write_text(ZERO_MUTANTS, encoding="utf-8")
    ...     sweep_marked(Path(tmp), Path(tmp, "sweep.log"))
    True
    >>> with TemporaryDirectory() as tmp:
    ...     _ = Path(tmp, "sweep.log").write_text("baseline failed to build", encoding="utf-8")
    ...     sweep_marked(Path(tmp), Path(tmp, "sweep.log"))
    False
    >>> with TemporaryDirectory() as tmp:
    ...     _ = Path(tmp, "outcomes.json").write_text("{corrupt", encoding="utf-8")
    ...     _ = Path(tmp, "sweep.log").write_text("baseline failed to build", encoding="utf-8")
    ...     sweep_marked(Path(tmp), Path(tmp, "sweep.log"))
    False
    """
    if (outcomes_dir / "outcomes.json").is_file():
        # Decodable is analyzed, drift-failing included: the reject is deterministic — a
        # re-run cannot change it — and the merge's guard-failure fallback recovers the
        # counts from the uploaded log. Only the no-evidence class re-runs.
        try:
            loaded(outcomes_dir / "outcomes.json", OutcomesDoc)
        except SystemExit:
            pass
        else:
            return True
    try:
        text = log.read_text(encoding="utf-8")
    except (OSError, ValueError):
        return False
    return log_evidence(text) is not None


def log_evidence(text: str) -> SummaryCounts | None:
    """The counts a sweep log carries: a parseable summary, or the zero-mutant shape — the one
    classification the shard marker and the merge's fallback share."""
    if ZERO_MUTANTS in text:
        return SummaryCounts(total_mutants=0)
    return parsed_summary(text)


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
        summary = log_evidence(text)
        if summary is not None:
            return summary
    return None


def merge_shards(
    shards: list[Cell], merged_root: str, prior_root: str
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
    >>> _ = (root / "merged/mutants-out-4/mutants.out").mkdir(parents=True)
    >>> _ = (root / "merged/mutants-out-4/mutants.out/outcomes.json").write_text('{"total_mutants": 0, "outcomes": [{"summary": "CaughtMutant"}]}', encoding="utf-8")
    >>> cells = [Cell(file="a", index=str(i)) for i in range(5)]
    >>> merged_doc, missing = merge_shards(cells, str(root / "merged"), str(root / "prior"))
    >>> (merged_doc.total_mutants, merged_doc.missed, missing)
    (617, 55, ['a', 'a: reports no mutants tested but holds outcome entries'])
    >>> [msgspec.json.decode(one, type=Outcome) for one in merged_doc.outcomes]
    [Outcome(summary='CaughtMutant', scenario=None)]
    >>> import shutil
    >>> shutil.rmtree(tmp)

    The corrupt outcomes.json is named missing, the summary-only shard contributes its counts, the
    zero-mutant shard contributes nothing, and the shard that fails a drift guard is named missing
    with its reason instead of aborting the merge.
    """
    merged = MergedDoc(outcomes=[])
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
                data = loaded(candidate, OutcomesDoc)
                break
            except SystemExit:
                continue
        if data is None:
            summary = _summary_from_logs(cell.index, merged_root, prior_root)
            if summary is None:
                missing.append(name)
                continue
            add_counts(merged, summary)
            continue
        try:
            if (reason := drift_reason(data)) is not None:
                raise SystemExit(f"{name}: {reason}")
        except SystemExit as err:
            summary = _summary_from_logs(cell.index, merged_root, prior_root)
            if summary is None:
                missing.append(str(err))
                continue
            add_counts(merged, summary)
            continue
        if merged.outcomes is None:
            merged.outcomes = []
        merged.outcomes.extend(readable(data))
        add_counts(merged, data)
    return merged, missing


def body(
    outcomes: MergedDoc,
    tally: Counts,
    repo: str,
    sha: str,
    run: str,
    shards: list[Cell],
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
    data = loaded(path, MergedDoc)
    tally = counts(data)
    if not tally.tested:
        raise SystemExit(f"{path}: reports no mutants tested")
    return data, tally


def main(argv: tuple[str, ...]) -> int:
    """The merge mode's stdout is the workflow's output contract, pinned here — the yaml gates
    read these lines, and a desynced `key=value` shape or title would garble the rolling issue.

    >>> from contextlib import redirect_stdout
    >>> import io
    >>> from tempfile import TemporaryDirectory
    >>> with TemporaryDirectory() as tmp:
    ...     with redirect_stdout(io.StringIO()) as captured:
    ...         code = main(("merge", "[]", tmp, tmp, f"{tmp}/out.json", f"{tmp}/missing.txt", f"{tmp}/empty.txt"))
    ...     (code, captured.getvalue(), Path(tmp, "empty.txt").read_text(encoding="utf-8"))
    (0, 'missing=false\\nran=false\\npartial=false\\ntitle=Mutation sweep did not run\\n', 'The sweep did not run: the plan left no shard jobs.')

    The report mode's stdout drives the rolling and retirement gates; the all-missing branch
    of the merge drives the no-outcomes title and body.

    >>> import json
    >>> with TemporaryDirectory() as tmp:
    ...     _ = Path(tmp, "merged.json").write_bytes(msgspec.json.encode(MergedDoc(total_mutants=3, missed=1)))
    ...     with redirect_stdout(io.StringIO()) as captured:
    ...         code = main(("report", f"{tmp}/merged.json", "JPHutchins/jphfmt", "abc1234", "run-url", f"{tmp}/report.md", "[]"))
    ...     (code, captured.getvalue())
    (0, 'missed=1\\ntitle=Mutation testing: 1 surviving mutants at abc1234\\n')
    >>> with TemporaryDirectory() as tmp:
    ...     with redirect_stdout(io.StringIO()) as captured:
    ...         code = main(("merge", json.dumps([{"file": "a.rs", "index": "0"}]), tmp, tmp, f"{tmp}/out.json", f"{tmp}/missing.txt", f"{tmp}/empty.txt"))
    ...     (code, captured.getvalue(), Path(tmp, "empty.txt").read_text(encoding="utf-8"), Path(tmp, "missing.txt").read_text(encoding="utf-8"))
    (0, 'missing=true\\nran=true\\npartial=false\\ntitle=Mutation sweep: no shard produced outcomes\\n', 'Every shard left no outcomes — the shards below are the whole sweep.', 'a.rs')
    """
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
                    msgspec.json.decode(shards_json, type=list[Cell]),
                ),
                encoding="utf-8",
            )
            print(f"missed={tally.missed}")
            print(f"title={title(tally, sha)}")
        case ("merge", shards_json, merged_root, prior_root, out, missing_txt, empty_txt):
            shards = msgspec.json.decode(shards_json, type=list[Cell])
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
            empty = not merged_doc.outcomes and not any(
                (merged_doc.total_mutants, merged_doc.missed, merged_doc.caught, merged_doc.unviable, merged_doc.timeout)
            )
            print(f"ran={'true' if shards else 'false'}")
            all_missing = bool(shards) and len(missing) == len(shards)
            print(f"partial={'true' if missing and not empty else 'false'}")
            if empty:
                print(
                    "title="
                    + (
                        "Mutation sweep did not run"
                        if not shards
                        else "Mutation sweep: no shard produced outcomes"
                        if all_missing
                        else "Mutation sweep: some shards left no outcomes"
                        if missing
                        else "Mutation sweep: every shard completed with zero mutants"
                    )
                )
                Path(empty_txt).write_text(
                    (
                        "The sweep did not run: the plan left no shard jobs."
                        if not shards
                        else "Every shard left no outcomes — the shards below are the whole sweep."
                        if all_missing
                        else "The shards below left no outcomes; the rest completed and none found a mutant."
                        if missing
                        else "Every shard completed and none found a mutant."
                    ),
                    encoding="utf-8",
                )
        case ("plan",):
            files = sys.stdin.read().splitlines()
            print(msgspec.json.encode(plan(files, 4)).decode())
        case ("sweep-marked", outcomes_dir, log):
            return 0 if sweep_marked(Path(outcomes_dir), Path(log)) else 1
        case ("decodable", path):
            try:
                loaded(Path(path), OutcomesDoc)
            except SystemExit:
                return 1
            return 0
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
            print("       mutants_report.py plan < FILES_LIST")
            print("       mutants_report.py sweep-marked OUTCOMES_DIR SWEEP_LOG")
            print("       mutants_report.py decodable OUTCOMES_PATH")
            print("       mutants_report.py --self-test | --self-check FILES...")
            return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main(tuple(sys.argv[1:])))
