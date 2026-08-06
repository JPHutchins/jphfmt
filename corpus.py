# /// script
# requires-python = ">=3.14"
# dependencies = []
# ///
"""Format the C that is installed on this machine, and hold the output to what the input was.

Two properties no fixture can state, over real headers rather than reductions:

* **idempotent** — formatting the output returns it unchanged;
* **compiles no worse** — ``gcc -fsyntax-only`` reports no more errors on the output than on the
  input, and does not crash on output that the input did not crash it on;
* **keeps what was written** — the output holds at least as many characters that are neither
  whitespace nor ``\\`` as the input did. Without it a formatter that emits nothing reads as clean on
  every file, since empty output is both a fixpoint and free of compiler errors.

The second is the only check that has ever caught jphfmt's compile-breaking bugs (#88, #90, #93,
#95, #100), and it needs real headers to be worth running: #100 corrupted 478 of 1200 corpus files
while every fixture in the suite stayed green, because the character it dropped was whitespace and
its output was a fixpoint.

Both files go through gcc from the *same* directory with the *same* flags. Compiling the input
where it lives and the output somewhere else resolves ``#include`` differently, which reports
failures that are the harness's rather than the formatter's.

Nothing here is skipped quietly, and nothing here passes on nothing. A missing compiler, a gcc that
rejects these flags, an unreadable corpus and a gcc that dies on a signal each end the run or name
the file, because a check that passes vacuously is worse than one that is not run at all.
"""

import argparse
import os
import re
import shutil
import subprocess
import sys
import tempfile
import traceback
from concurrent.futures import ThreadPoolExecutor, as_completed
from collections.abc import Iterator
from itertools import islice
from pathlib import Path
from typing import NamedTuple

GCC_TIMEOUT_S = 30
# Generous: the largest corpus file is a 360k-line amalgamation, and a run that reports the wrong
# file as hung is worse than one that waits.
FORMAT_TIMEOUT_S = 60
BUILD_TIMEOUT_S = 300
# `-fdiagnostics-plain-output` drops the echoed source line under each diagnostic. Those echoes carry
# the file's own text, so a `#error "error: x"` or a comment mentioning one counted as a diagnostic,
# and reflowing a line onto or off an offending one changed the count with the errors unchanged.
GCC_FLAGS = ("-std=c2x", "-fsyntax-only", "-fdiagnostics-plain-output")
# gcc writes one diagnostic per line, `<file>:<line>[:<col>]: error: <what>`. Anchored so the `error:`
# inside a message's quoted source text cannot be counted twice, and so a `note:` line is never counted.
# The location is required: `cc1: fatal error: out of memory allocating …` is the tool failing, not a
# diagnostic about the code, and counting it as one error made a run where *both* sides ran out of
# memory read as clean.
ERROR_LINE = re.compile(r"(?m)^\S.*?:\d+(?::\d+)?: (?:fatal )?error: ")
# The subset that ends the compilation. Anchored the same way, so a `fatal error:` quoted inside a
# message is not mistaken for one gcc raised.
FATAL_LINE = re.compile(r"(?m)^\S.*?:\d+(?::\d+)?: fatal error: ")
# `error:` is what this counts, and gcc translates its diagnostics wherever a locale catalog is
# installed. A localized gcc would report zero errors for every file in the corpus.
ENGLISH = {**os.environ, "LC_ALL": "C"}
# Whitespace is the formatter's to place, and so is a `\` continuation: a two-line macro that comes to
# fit on one keeps every character of its body and needs no continuation to hold it together.
OWNED = " \t\r\n\v\f\\"
# Counted as bytes, so the two sides compare the same way whether one arrives decoded and the other not.
OWNED_BYTES = OWNED.encode()


class Errors(NamedTuple):
	count: int


class Halted(NamedTuple):
	"""gcc stopped at a fatal error, so what it reported is where it gave up rather than how the file
	compiles. A missing `#include`, or a header that is not C — `simdjson.h` stops at `<cstddef>` and
	reports exactly one error, which the input and the output share, so the comparison passes having
	syntax-checked nothing."""

	detail: str


class Crashed(NamedTuple):
	signal: int


class TimedOut(NamedTuple):
	seconds: float


class ToolFailed(NamedTuple):
	detail: str


Compile = Errors | Halted | Crashed | TimedOut | ToolFailed


class Formatted(NamedTuple):
	text: str


class Failed(NamedTuple):
	status: int
	stderr: str


class Hung(NamedTuple):
	seconds: float


Formatting = Formatted | Failed | Hung


class Unformattable(NamedTuple):
	path: Path
	status: int
	stderr: str


class Stalled(NamedTuple):
	path: Path
	seconds: float


class NotIdempotent(NamedTuple):
	path: Path
	line: str
	again: str


class Regressed(NamedTuple):
	path: Path
	before: Compile
	after: Compile


class LostContent(NamedTuple):
	path: Path
	before: int
	after: int


class Unmeasured(NamedTuple):
	path: Path
	before: Compile
	after: Compile


class Broke(NamedTuple):
	path: Path
	error: str


Verdict = Unformattable | Stalled | NotIdempotent | Regressed | LostContent | Unmeasured | Broke


class Corpus(NamedTuple):
	"""What [`discover`] settled on: the files to check, and the candidates gcc could not read as C."""

	files: tuple[Path, ...]
	skipped: tuple[Path, ...]


def candidates(root: Path, depth: int) -> Iterator[Path]:
	"""Every `.c`/`.h` file at most `depth` levels under `root`, shallowest first.

	Depth-capped rather than recursive: `/nix/store` is deep enough that an unbounded walk does not
	finish. Shallowest first because that is where the headers worth compiling are —
	`<hash>-glibc-dev/include/stdio.h` is three levels down and a fixed four-level glob misses it.

	Each level is sorted, because `glob` yields a directory in whatever order the filesystem does: two
	runs would otherwise check different files and an A/B against another binary would compare two
	different corpora.
	"""
	levels = (sorted(root.glob("/".join(["*"] * d))) for d in range(1, depth + 1))
	return (p for level in levels for p in level if p.suffix in (".c", ".h") and p.is_file())


def discover(root: Path, limit: int, depth: int, jobs: int = 8) -> Corpus:
	"""The first `limit` candidates gcc can measure as C.

	A suffix is not a language. `/nix/store` is full of `.h` files that are C++ (`simdjson.h`), and of
	C that needs an include path this has no way to reconstruct; gcc stops at the first `#include` it
	cannot resolve and reports one `fatal error`. Both sides then report the same one, `severity` finds
	no regression, and the file reads clean having been syntax-checked *not at all* — 117 of 1200 on
	this machine, counted as passes for as long as this check has existed.

	So membership is the compile itself: a candidate is corpus if gcc gets through it, and the ones it
	does not are named by [`main`] rather than dropped quietly. jphfmt formats C, so a header it cannot
	read as C is not evidence about jphfmt either way.
	"""
	stream = candidates(root, depth)
	kept: list[Path] = []
	skipped: list[Path] = []
	with ThreadPoolExecutor(max_workers=jobs) as pool:
		while len(kept) < limit:
			# In order, in batches, so the set is the same on two runs and an A/B compares one corpus.
			batch = tuple(islice(stream, jobs * 8))
			if not batch:
				break
			for path, ok in zip(batch, pool.map(measurable, batch)):
				(kept if ok else skipped).append(path)
	return Corpus(tuple(kept[:limit]), tuple(skipped))


def measurable(path: Path) -> bool:
	"""Whether gcc gets through `path` as C — the membership test [`discover`] applies."""
	return isinstance(compiles(path, path.parent), Errors)


def compiles(source: Path, include: Path) -> Compile:
	"""How gcc fares on `source`. Assumes a usable gcc — [`unusable_gcc`] establishes that up front."""
	try:
		gcc = subprocess.run(
			("gcc", *GCC_FLAGS, "-I", str(include), str(source)),
			capture_output=True,
			text=True,
			env=ENGLISH,
			timeout=GCC_TIMEOUT_S,
		)
	except subprocess.TimeoutExpired:
		return TimedOut(GCC_TIMEOUT_S)
	# A negative status is death by signal. Its stderr holds no `error:` to count, so counting alone
	# would read a compiler crash as a clean compile — the very thing this checks for.
	if gcc.returncode < 0:
		return Crashed(-gcc.returncode)
	found = len(ERROR_LINE.findall(gcc.stderr))
	# A nonzero exit gcc did not spell as a located diagnostic is the tool failing, not the code being
	# wrong: an internal error, an out-of-memory, a driver refusal. Counting alone would read it as a
	# clean compile, and `Errors(0)` as a *measured* baseline — the vacuous pass, twice over.
	if gcc.returncode != 0 and found == 0:
		return ToolFailed(gcc.stderr.strip().splitlines()[-1] if gcc.stderr.strip() else "(no stderr)")
	# A `fatal error` is where gcc stopped, not what it found: everything after it went unread, so the
	# count describes the prefix it managed rather than the file. Both sides report the same one and the
	# comparison passes having checked nothing — the vacuous pass this whole harness exists to refuse.
	if (fatal := FATAL_LINE.search(gcc.stderr)) is not None:
		return Halted(gcc.stderr[fatal.start() :].splitlines()[0].strip())
	return Errors(found)


def severity(result: Compile) -> tuple[int, int]:
	"""How bad a compile is, ordered — a crash or a timeout is worse than any number of errors.

	>>> severity(Errors(3)) < severity(Errors(4))
	True
	>>> severity(Errors(999)) < severity(Crashed(11))
	True
	>>> severity(Errors(999)) < severity(Halted("x.h:1:10: fatal error: y.h: No such file"))
	True
	>>> severity(Crashed(11)) == severity(TimedOut(30))
	True
	>>> severity(Errors(5)) == severity(Errors(5))
	True
	"""
	match result:
		case Errors(count):
			return (0, count)
		case Halted() | Crashed() | TimedOut() | ToolFailed():
			return (1, 0)


def first_difference(before: str, after: str) -> tuple[str, str]:
	"""The first line the two disagree on, counting a line one of them does not have.

	Line endings are kept, so a difference that is only a CRLF or a trailing newline names itself
	instead of reporting two identical-looking lines.

	>>> first_difference("a\\nb\\n", "a\\nc\\n")
	('b\\n', 'c\\n')
	>>> first_difference("a\\n", "a\\n")
	('', '')
	>>> first_difference("a\\nb\\n", "a\\n")
	('b\\n', '<no line>')
	>>> first_difference("a\\n", "a\\nb\\n")
	('<no line>', 'b\\n')
	>>> first_difference("a\\r\\n", "a\\n")
	('a\\r\\n', 'a\\n')
	>>> first_difference("a\\n", "a")
	('a\\n', 'a')
	"""
	missing = "<no line>"
	lines = zip(
		before.splitlines(keepends=True) + [missing] * len(after.splitlines()),
		after.splitlines(keepends=True) + [missing] * len(before.splitlines()),
	)
	return next((pair for pair in lines if pair[0] != pair[1]), ("", ""))


def written(text: str | bytes) -> int:
	"""How many of `text`'s bytes the formatter does not own — everything but [`OWNED`].

	Counted as bytes so the two sides compare the same way when one arrives decoded and the other does
	not: the original is read raw, because it is being counted rather than interpreted, and a corpus
	header that is not UTF-8 is the formatter's to report rather than this harness's to raise on.

	>>> written("a b\\tc\\n")
	3
	>>> written("#define M(x) \\\\\\n\\tx\\n")
	12
	>>> written("   \\n\\t\\\\\\n")
	0
	>>> written(b"a b\\tc\\n")
	3
	>>> written("\\u00e9;") == written("\\u00e9;".encode())
	True
	"""
	data = text if isinstance(text, bytes) else text.encode()
	return sum(1 for byte in data if byte not in OWNED_BYTES)


def format_with(binary: Path, source: Path | str) -> Formatting:
	"""Format `source`, which is a path to read or the text to pipe.

	Timed out, because `ThreadPoolExecutor` cannot cancel a blocked worker: one formatter that never
	returns would stall the whole run with no diagnostic — not even which file. A formatter that hangs
	is exactly the class of bug this exists to find, so it has to be reportable.
	"""
	try:
		run = subprocess.run(
			(str(binary), str(source) if isinstance(source, Path) else "/dev/stdin"),
			input=None if isinstance(source, Path) else source,
			capture_output=True,
			# The formatter reads and writes UTF-8; `text=True` alone would decode with the process
			# locale, so a corpus header's non-ASCII bytes would fail to decode under a C locale and
			# report a file the formatter handled fine.
			encoding="utf-8",
			timeout=FORMAT_TIMEOUT_S,
		)
	except subprocess.TimeoutExpired:
		return Hung(FORMAT_TIMEOUT_S)
	return Formatted(run.stdout) if run.returncode == 0 else Failed(run.returncode, run.stderr)


def as_verdict(path: Path, outcome: Failed | Hung) -> Verdict:
	match outcome:
		case Failed(status, stderr):
			return Unformattable(path, status, stderr)
		case Hung(seconds):
			return Stalled(path, seconds)


def check(binary: Path, path: Path) -> tuple[Verdict, ...]:
	"""Every property that fails for `path`, not the first.

	Reporting only the first hid a compile-breaking bug behind a non-idempotency for as long as both
	were present: `sqlite3.c` was unstable, so the `gcc` comparison never ran on it, and the 48 errors
	its formatted form carries surfaced only once the instability was fixed (#109, #112).
	"""
	first = format_with(binary, path)
	if not isinstance(first, Formatted):
		return (as_verdict(path, first),)
	once = first.text
	again = format_with(binary, once)
	unstable: tuple[Verdict, ...] = ()
	if not isinstance(again, Formatted):
		unstable = (as_verdict(path, again),)
	elif again.text != once:
		unstable = (NotIdempotent(path, *first_difference(once, again.text)),)
	# Bytes for the original: it is being counted, not interpreted, and a corpus header that is not UTF-8
	# is the formatter's to report rather than this harness's to raise on.
	source, output = written(path.read_bytes()), written(once)
	lost: tuple[Verdict, ...] = ()
	if output < source:
		lost = (LostContent(path, source, output),)
	with tempfile.TemporaryDirectory() as tmp:
		# Not `in.c`/`out.c`: the include path is the corpus file's own directory, so a corpus that holds
		# a file of that name would have this one shadow it and the comparison would measure the harness.
		original = Path(tmp) / f"jphfmt-corpus-before-{path.name}"
		formatted = Path(tmp) / f"jphfmt-corpus-after-{path.name}"
		original.write_bytes(path.read_bytes())
		formatted.write_text(once, encoding="utf-8")
		before = compiles(original, path.parent)
		after = compiles(formatted, path.parent)
	# `severity` puts a crash or a timeout above every error count, so a baseline that is one makes
	# `after` unable to outrank it and every output read clean — `severity(Errors(48)) > severity(
	# TimedOut(30))` is False. An input gcc could not measure gives no baseline to compare against, and
	# saying so is the only honest verdict.
	regressed: tuple[Verdict, ...] = ()
	if not isinstance(before, Errors):
		regressed = (Unmeasured(path, before, after),)
	elif severity(after) > severity(before):
		regressed = (Regressed(path, before, after),)
	return unstable + lost + regressed


def checked(binary: Path, path: Path) -> tuple[Verdict, ...]:
	"""[`check`], with whatever it raises reported against `path` instead of ending the run.

	`ThreadPoolExecutor.map` re-raises a worker's exception as its results are consumed, so one
	unreadable file would discard every verdict already computed for the other 1199 — a run that did
	the work and reported on none of it.
	"""
	try:
		return check(binary, path)
	except Exception:
		# The traceback, not just the message: a `Broke` is a bug in this file, and the whole point of
		# reporting one rather than raising is that the run finishes — so it has to name its own site.
		return (Broke(path, traceback.format_exc().strip()),)


def unusable_gcc() -> str | None:
	"""Why this machine's gcc cannot do the check, or `None` if it can.

	Both compiles take the same flags, so flags gcc rejects fail them identically, [`severity`] finds
	no regression, and all 1200 files read clean — the vacuous pass, wearing the same face as a run
	that proved something.
	"""
	if shutil.which("gcc") is None:
		return "gcc is not on PATH — this check needs a C compiler"
	try:
		probe = subprocess.run(
			("gcc", *GCC_FLAGS, "-x", "c", "/dev/null"),
			capture_output=True,
			text=True,
			env=ENGLISH,
			timeout=GCC_TIMEOUT_S,
		)
	except subprocess.TimeoutExpired:
		return f"gcc did not answer an empty file in {GCC_TIMEOUT_S}s"
	if probe.returncode == 0:
		return None
	# The flags are the likely cause and the only one this can name; a broken install (a missing `cc1`)
	# lands here too, and refusing is right either way.
	return f"gcc rejects {' '.join(GCC_FLAGS)}: {probe.stderr.strip() or '(no stderr)'}"


def unbuildable() -> str | None:
	"""Why the release binary could not be built, or `None` if it was.

	Timed out like every other subprocess here: a cargo stalled on a network fetch or a held lock would
	hang the run before it names a single file. `camas corpus` carries no `when=` guard and this repo
	ships no rustup, so a machine with gcc and a corpus but no cargo is the expected one, not the
	exceptional one — and it must say so rather than raise.
	"""
	if shutil.which("cargo") is None:
		return "cargo is not on PATH — build the binary elsewhere and pass --binary --no-build"
	try:
		build = subprocess.run(("cargo", "build", "--release", "--quiet"), timeout=BUILD_TIMEOUT_S)
	except subprocess.TimeoutExpired:
		return f"cargo build did not finish in {BUILD_TIMEOUT_S}s"
	return None if build.returncode == 0 else f"cargo build failed with status {build.returncode}"


def describe(result: Compile) -> str:
	match result:
		case Errors(count):
			return f"{count} errors"
		case Halted(detail):
			return f"gcc gave up: {detail}"
		case Crashed(signal):
			return f"killed by signal {signal}"
		case TimedOut(seconds):
			return f"no answer in {seconds}s"
		case ToolFailed(detail):
			return f"gcc failed: {detail}"


def report(verdict: Verdict) -> str:
	match verdict:
		case Unformattable(path, status, stderr) if status < 0:
			return f"SIGNAL {-status:<3} {path}\n           {stderr.strip() or '(no stderr)'}"
		case Unformattable(path, status, stderr):
			return f"EXIT {status:<4} {path}\n           {stderr.strip() or '(no stderr)'}"
		case Stalled(path, seconds):
			return f"HUNG      {path}\n           no output in {seconds}s"
		case NotIdempotent(path, line, again):
			return f"UNSTABLE  {path}\n            once: {line!r}\n           twice: {again!r}"
		case Regressed(path, before, after):
			return f"REGRESSED {path}\n           gcc: {describe(before)} -> {describe(after)}"
		case LostContent(path, before, after):
			return f"LOST      {path}\n           {before - after} of {before} characters gone"
		case Unmeasured(path, before, after):
			return (
				f"UNMEASURED {path}\n           gcc on the input: {describe(before)}"
				f"\n           on the output: {describe(after)}, with nothing to compare it to"
			)
		case Broke(path, error):
			indented = error.replace("\n", "\n           ")
			return f"HARNESS   {path}\n           {indented}"


def main(argv: tuple[str, ...]) -> int:
	cli = argparse.ArgumentParser(description=__doc__)
	cli.add_argument("--root", type=Path, default=Path("/nix/store"))
	cli.add_argument("--limit", type=int, default=1200)
	cli.add_argument("--depth", type=int, default=4)
	cli.add_argument("--jobs", type=int, default=8)
	cli.add_argument("--binary", type=Path, default=Path("target/release/jphfmt"))
	cli.add_argument("--no-build", action="store_true")
	args = cli.parse_args(argv)

	for name, value in (("jobs", args.jobs), ("limit", args.limit), ("depth", args.depth)):
		if value < 1:
			print(f"--{name} must be at least 1, not {value}", file=sys.stderr)
			return 1

	if (unusable := unusable_gcc()) is not None:
		print(unusable, file=sys.stderr)
		return 1
	if not args.root.is_dir():
		print(f"no corpus at {args.root} — pass --root", file=sys.stderr)
		return 1
	if not args.no_build and (unbuilt := unbuildable()) is not None:
		print(unbuilt, file=sys.stderr)
		return 1
	if not args.binary.is_file():
		print(f"no formatter at {args.binary} — pass --binary", file=sys.stderr)
		return 1

	corpus = discover(args.root, args.limit, args.depth, args.jobs)
	files = corpus.files
	if not files:
		print(f"no .c/.h files gcc reads as C within {args.depth} levels of {args.root}", file=sys.stderr)
		return 1
	# Named, not dropped quietly: a corpus that shrank because gcc stopped reading is a corpus that
	# checks less, and a run that does not say so reports coverage it does not have.
	if corpus.skipped:
		print(f"{len(corpus.skipped)} candidates skipped — gcc cannot read them as C, so neither side")
		print("is measurable and a comparison would pass having checked nothing. First few:")
		for path in corpus.skipped[:5]:
			print(f"  {path}")

	# Reported as they arrive rather than collected: the largest files take minutes each, and a run that
	# is interrupted — or watched — should have already named everything it found.
	clean = 0
	pool = ThreadPoolExecutor(max_workers=args.jobs)
	pending = [pool.submit(checked, args.binary, p) for p in files]
	try:
		for done in as_completed(pending):
			verdicts = done.result()
			clean += not verdicts
			for verdict in verdicts:
				print(report(verdict), flush=True)
	except KeyboardInterrupt:
		# `with ThreadPoolExecutor(...)` waits for every queued file on the way out, so a Ctrl+C sat
		# there for the rest of the run — for a corpus whose slowest file takes minutes, long enough to
		# look like a hang. Cancel what has not started and let the running workers go.
		pool.shutdown(wait=False, cancel_futures=True)
		print(f"interrupted after {clean} clean of {len(files)}", file=sys.stderr)
		return 130
	pool.shutdown()
	print(f"{clean} of {len(files)} files clean")
	return 0 if clean == len(files) else 1


if __name__ == "__main__":
	raise SystemExit(main(tuple(sys.argv[1:])))
