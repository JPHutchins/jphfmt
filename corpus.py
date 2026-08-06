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
# How many candidates [`discover`] will compile per file it keeps. A root of C++ headers would
# otherwise put the *whole* walk through gcc at up to `GCC_TIMEOUT_S` each looking for a corpus that
# is not there; four is comfortable where 1200 kept files took 1317 candidates, and hitting it is
# reported rather than silently returning a short corpus.
DISCOVERY_BUDGET_PER_FILE = 4
# `-fdiagnostics-plain-output` drops the echoed source line under each diagnostic. Those echoes carry
# the file's own text, so a `#error "error: x"` or a comment mentioning one counted as a diagnostic,
# and reflowing a line onto or off an offending one changed the count with the errors unchanged.
GCC_FLAGS = ("-std=c2x", "-fsyntax-only", "-fdiagnostics-plain-output")
# gcc writes one diagnostic per line, `<file>:<line>[:<col>]: error: <what>`. Anchored so the `error:`
# inside a message's quoted source text cannot be counted twice, and so a `note:` line is never counted.
# The location is required: `cc1: fatal error: out of memory allocating …` is the tool failing, not a
# diagnostic about the code, and counting it as one error made a run where *both* sides ran out of
# memory read as clean.
#
# One pattern, and `fatal` is a group on it rather than a second search. The file part takes no `:`, so
# the kind is read after the diagnostic's *own* location and not after any later `:<digits>:` in the
# message — `#error "cfg.h:1: fatal error: FOE"` is an ordinary error gcc quoted, and a lazy `.*?` read
# it as a fatal one, which would have dropped a cleanly compiling file out of the corpus. A path holding
# a `:` would not match at all; `/nix/store` has none, and gcc quotes such a path anyway.
DIAGNOSTIC = re.compile(r"(?m)^[^:\n]+:\d+(?::\d+)?: (?P<fatal>fatal )?error: ")
# `error:` is what this counts, and gcc translates its diagnostics wherever a locale catalog is
# installed. A localized gcc would report zero errors for every file in the corpus.
ENGLISH = {**os.environ, "LC_ALL": "C"}
# Whitespace is the formatter's to place, and so is a `\` continuation: a two-line macro that comes to
# fit on one keeps every character of its body and needs no continuation to hold it together.
OWNED = " \t\r\n\v\f\\"
# Counted as bytes, so the two sides compare the same way whether one arrives decoded and the other not.
OWNED_BYTES = OWNED.encode()
# The two reasons gcc gives no baseline, kept apart because they are not the same claim: one is gcc
# reading the file and stopping, the other gcc never answering at all.
GAVE_UP = "gcc read it and gave up"
NO_ANSWER = "gcc never answered"


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


class Broke(NamedTuple):
	path: Path
	error: str


Verdict = Unformattable | Stalled | NotIdempotent | Regressed | LostContent | Broke


class Unmeasured(NamedTuple):
	"""A file whose gcc baseline is not an error count, so the compile comparison has nothing to compare
	against. Not a verdict and not an exclusion: the properties that do not need gcc — idempotency,
	no-token-loss, formats-at-all — are checked on it exactly as on any other file, and only the
	`gcc`-no-worse half is skipped. That is the honest shape, because those files *are* still formatted
	in the world, and #100 was a content-loss bug no compiler would have caught anyway.

	`Halted` says gcc could not read it as C; a `TimedOut` or a `Crashed` says gcc could not answer,
	which is this machine's problem rather than the file's. Reported apart for that reason."""

	path: Path
	why: Compile


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


def discover(root: Path, limit: int, depth: int) -> tuple[Path, ...]:
	"""The first `limit` candidates, in walk order.

	Membership is *not* the compile. An earlier form of this kept only files gcc could read as C, which
	fixed one vacuous pass and opened another: the properties that need no compiler — idempotency,
	no-token-loss, formats-at-all — stopped being checked on the ~117 files it set aside, and #100 was
	exactly a content-loss bug no compiler would have caught. Every candidate is formatted and held to
	those; [`check`] reports the compile comparison as unmeasured where gcc gives it no baseline.
	"""
	return tuple(islice(candidates(root, depth), limit))


def measure(label: str, data: bytes, path: Path) -> Compile:
	"""gcc on `data`, as a copy in a temporary directory with `path`'s own directory on the include
	path. One helper, because the before and after sides must compile under identical conditions and
	two copies of these six lines is two things to keep in lock-step.

	Not `in.c`/`out.c`: the include path is the corpus file's own directory, so a corpus that holds a
	file of that name would have this one shadow it and the comparison would measure the harness.
	"""
	with tempfile.TemporaryDirectory() as tmp:
		# `:` out of the copy's name: [`DIAGNOSTIC`] reads gcc's own location as the text before the
		# first `:<digits>:`, so a basename holding one made every located diagnostic unmatchable — the
		# same file then read as `Errors(0)` when it was clean and `ToolFailed` when it was not.
		copy = Path(tmp) / f"jphfmt-corpus-{label}-{path.name.replace(':', '_')}"
		copy.write_bytes(data)
		return compiles(copy, path.parent)


def baseline(path: Path) -> Compile:
	"""gcc on `path` as written, compiled the way [`check`] compiles the output — a copy, from a
	temporary directory, with the original's directory on the include path. Compiling in place would
	resolve a quoted `#include` against a directory the other side does not have.
	"""
	try:
		return measure("before", path.read_bytes(), path)
	except Exception as why:
		# Any exception, not just `OSError`: a worker that raises takes `pool.map` and the whole run
		# with it, before a single file is checked — the same reason `checked` converts an exception
		# into a verdict rather than raising. An unreadable file, a full disk and a gcc whose stderr
		# will not decode are all this harness reporting rather than this harness dying.
		return ToolFailed(f"{type(why).__name__}: {why}")


def compiles(source: Path, include: Path) -> Compile:
	"""How gcc fares on `source`. Assumes a usable gcc — [`unusable_gcc`] establishes that up front."""
	try:
		gcc = subprocess.run(
			("gcc", *GCC_FLAGS, "-I", str(include), str(source)),
			capture_output=True,
			text=True,
			# gcc quotes the source it is complaining about, so a header that is not UTF-8 puts its
			# bytes in this stderr. Strict decoding raised out of a discovery worker and took the run
			# with it. What is done with this text is regex matching, never reproduction, so a
			# replacement character costs nothing.
			errors="replace",
			env=ENGLISH,
			timeout=GCC_TIMEOUT_S,
		)
	except subprocess.TimeoutExpired:
		return TimedOut(GCC_TIMEOUT_S)
	# A negative status is death by signal. Its stderr holds no `error:` to count, so counting alone
	# would read a compiler crash as a clean compile — the very thing this checks for.
	if gcc.returncode < 0:
		return Crashed(-gcc.returncode)
	diagnostics = tuple(DIAGNOSTIC.finditer(gcc.stderr))
	found = len(diagnostics)
	# A nonzero exit gcc did not spell as a located diagnostic is the tool failing, not the code being
	# wrong: an internal error, an out-of-memory, a driver refusal. Counting alone would read it as a
	# clean compile, and `Errors(0)` as a *measured* baseline — the vacuous pass, twice over.
	if gcc.returncode != 0 and found == 0:
		return ToolFailed(gcc.stderr.strip().splitlines()[-1] if gcc.stderr.strip() else "(no stderr)")
	# A `fatal error` is where gcc stopped, not what it found: everything after it went unread, so the
	# count describes the prefix it managed rather than the file. Both sides report the same one and the
	# comparison passes having checked nothing — the vacuous pass this whole harness exists to refuse.
	# From `end()`, so what is kept is gcc's message and not the location it prefixed: the file it names
	# is the temporary copy, whose path differs every run and is not the corpus file the report names.
	if (fatal := next((d for d in diagnostics if d.group("fatal")), None)) is not None:
		# The first line with something on it: a fatal diagnostic can carry an empty message and be
		# followed by `compilation terminated.`, where taking line zero reports `gcc gave up: ` and
		# says nothing, and taking nothing at all raised an `IndexError` out of a discovery worker.
		rest = (line.strip() for line in gcc.stderr[fatal.end() :].splitlines())
		return Halted(next((line for line in rest if line), "(no message)"))
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
			capture_output=True,
			input=None if isinstance(source, Path) else source.encode("utf-8", "surrogateescape"),
			timeout=FORMAT_TIMEOUT_S,
		)
	except subprocess.TimeoutExpired:
		return Hung(FORMAT_TIMEOUT_S)
	# The two streams are decoded differently on purpose. Stdout is written back out by `measure` and
	# counted by `written`, so it must round-trip byte for byte — a corpus file may hold bytes that are
	# not UTF-8 at all, and the formatter is right to preserve them; `surrogateescape` gives them back
	# exactly. Stderr is only ever *printed*, and a lone surrogate cannot be encoded by a strict stdout
	# — the default for a piped run — so it would have crashed `main`'s own `print`, outside `checked`'s
	# net, after every verdict had already been computed.
	if run.returncode == 0:
		return Formatted(run.stdout.decode("utf-8", "surrogateescape"))
	return Failed(run.returncode, run.stderr.decode("utf-8", "replace"))


def as_verdict(path: Path, outcome: Failed | Hung) -> Verdict:
	match outcome:
		case Failed(status, stderr):
			return Unformattable(path, status, stderr)
		case Hung(seconds):
			return Stalled(path, seconds)


def check(binary: Path, path: Path) -> tuple[tuple[Verdict, ...], Unmeasured | None]:
	"""Every property that fails for `path`, not the first.

	Reporting only the first hid a compile-breaking bug behind a non-idempotency for as long as both
	were present: `sqlite3.c` was unstable, so the `gcc` comparison never ran on it, and the 48 errors
	its formatted form carries surfaced only once the instability was fixed (#109, #112).
	"""
	first = format_with(binary, path)
	if not isinstance(first, Formatted):
		return ((as_verdict(path, first),), None)
	once = first.text
	again = format_with(binary, once)
	unstable: tuple[Verdict, ...] = ()
	if not isinstance(again, Formatted):
		unstable = (as_verdict(path, again),)
	elif again.text != once:
		unstable = (NotIdempotent(path, *first_difference(once, again.text)),)
	# Bytes for the original: it is being counted, not interpreted, and a corpus header that is not UTF-8
	# is the formatter's to report rather than this harness's to raise on.
	# Back to the bytes the formatter actually emitted, for both the count and the compile.
	emitted = once.encode("utf-8", "surrogateescape")
	source, output = written(path.read_bytes()), written(emitted)
	lost: tuple[Verdict, ...] = ()
	if output < source:
		lost = (LostContent(path, source, output),)
	# `severity` puts a halt, a crash and a timeout above every error count, so a baseline that is one
	# makes `after` unable to outrank it and *every* output read clean — `severity(Errors(48)) >
	# severity(Halted(…))` is False. That is the vacuous pass, and the answer is to compare nothing and
	# say so, not to compare it anyway. gcc halts on 117 of this machine's 1200: a header that is not C,
	# or C whose `#include` this cannot resolve, where both sides report the same one fatal error.
	before = baseline(path)
	if not isinstance(before, Errors):
		return (unstable + lost, Unmeasured(path, before))
	after = measure("after", emitted, path)
	regressed: tuple[Verdict, ...] = ()
	if severity(after) > severity(before):
		regressed = (Regressed(path, before, after),)
	return (unstable + lost + regressed, None)


def checked(binary: Path, path: Path) -> tuple[tuple[Verdict, ...], Unmeasured | None]:
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
		return ((Broke(path, traceback.format_exc().strip()),), None)


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


def describe_unmeasured(unmeasured: tuple[Unmeasured, ...]) -> tuple[str, ...]:
	"""Which files got no compile comparison, grouped by cause, and named — never a bare count. Every
	one of them was still formatted and held to the properties that need no compiler.

	A `Halted` is gcc reading the file and giving up: not C, or C whose `#include` this cannot resolve.
	The rest are gcc never answering, which is this machine and a different claim.

	>>> describe_unmeasured(())
	()
	>>> for line in describe_unmeasured((Unmeasured(Path("a.h"), Halted("x.h: No such file")),)):
	...     print(line)
	1 file has no gcc baseline — gcc read it and gave up; the format properties still ran
	  a.h: gcc gave up: x.h: No such file
	"""
	groups = (
		(tuple(u for u in unmeasured if isinstance(u.why, Halted)), GAVE_UP),
		(tuple(u for u in unmeasured if not isinstance(u.why, Halted)), NO_ANSWER),
	)
	lines: list[str] = []
	for group, why in groups:
		if not group:
			continue
		count = f"1 file has" if len(group) == 1 else f"{len(group)} files have"
		lines.append(f"{count} no gcc baseline — {why}; the format properties still ran")
		lines.extend(f"  {u.path}: {describe(u.why)}" for u in group[:5])
		if len(group) > 5:
			lines.append(f"  … and {len(group) - 5} more")
	return tuple(lines)


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

	files = discover(args.root, args.limit, args.depth)
	if not files:
		levels = "level" if args.depth == 1 else "levels"
		print(f"no .c/.h files within {args.depth} {levels} of {args.root}", file=sys.stderr)
		return 1
	if len(files) < args.limit:
		# Reduced coverage, said out loud: a clean pass over fewer files than were asked for is exactly
		# what this module refuses to report as a clean pass, so it also fails below.
		noun = "file" if len(files) == 1 else "files"
		levels = "level" if args.depth == 1 else "levels"
		print(
			f"the walk found only {len(files)} {noun} within {args.depth} {levels} of {args.root}, "
			f"not the {args.limit} asked for",
			file=sys.stderr,
		)

	# Reported as they arrive rather than collected: the largest files take minutes each, and a run that
	# is interrupted — or watched — should have already named everything it found.
	clean = 0
	unmeasured: list[Unmeasured] = []
	pool = ThreadPoolExecutor(max_workers=args.jobs)
	try:
		# Inside the `try`: submitting 1200 futures is not instant, and a Ctrl+C in that window used to
		# leave the pool unshut and the interpreter joining its workers with nothing reported.
		pending = [pool.submit(checked, args.binary, path) for path in files]
		for done in as_completed(pending):
			verdicts, unmeasured_here = done.result()
			clean += not verdicts
			unmeasured.extend(filter(None, (unmeasured_here,)))
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

    # Named, and to stderr, so the verdict stream stays greppable. Every one of these was formatted and
	# held to the properties that need no compiler; what they did not get is the compile comparison, and
	# a run that does not say how many reports coverage it does not have.
	for line in describe_unmeasured(tuple(unmeasured)):
		print(line, file=sys.stderr)
	short = len(files) < args.limit
	# The headline comes last and only when it is true. Printing `1200 of 1200 files clean` above a
	# failure line, with a nonzero exit, hands anyone grepping the summary a green pass that the exit
	# status contradicts.
	if clean == len(files) and not short:
		print(f"{clean} of {len(files)} files clean")
		return 0
	print(f"{clean} of {len(files)} files clean, and the run is not", file=sys.stderr)
	return 1


if __name__ == "__main__":
	raise SystemExit(main(tuple(sys.argv[1:])))
