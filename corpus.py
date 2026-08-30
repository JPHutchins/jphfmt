# /// script
# requires-python = ">=3.14"
# dependencies = []
# ///
"""Format the C that is installed on this machine, and hold the output to what the input was.

Two properties no fixture can state, over real headers rather than reductions:

* **idempotent** — formatting the output returns it unchanged;
* **compiles no worse** — ``gcc -fsyntax-only`` reports no more errors on the output than on the
  input, and does not crash on output that the input did not crash it on; for files where gcc
  halts at a fatal error the comparison is by severity — the halt's message and prefix count — not
  the total count;
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
#
# One pattern, and `fatal` is a group on it rather than a second search, read only at the diagnostic's
# own location, which gcc always starts at column zero — the anchor refuses every whitespace prefix,
# a `\f`/`\v`/`\r`-led line is not a shape gcc writes. The file part runs through the colon
# its line number follows — `#line 1 "a:b.c"` puts a `:` inside it and `#line 1 ""` leaves it empty
# but keeps the colon, and either must still measure rather than read as a `ToolFailed` that fails the
# whole run. It is required, so a bare `123: error:` — no file part, no leading colon, not a gcc
# shape — matches nothing, and the line number itself is required, so `cc1: fatal error: …` — the tool
# failing, not a diagnostic — matches nothing either, as it must, and it is ASCII, as gcc writes it
# under `LC_ALL=C`. Stated, not silent: a two-number shape like `1:2: error:` reads as file `1`,
# line 2 — a file of that name and a message spelled that way are indistinguishable — and gcc
# always writes the file part, so no run measures through one. What is given up is a *message* whose
# own line starts location-shaped at column
# zero — a quoted `#error "x\ncfg.h:1: fatal error: FOE"` — which this reads as a fatal diagnostic.
# No corpus file spells one.
DIAGNOSTIC = re.compile(r"(?m)^(?=\S)(?:[^\n]*?:)[0-9]+(?::[0-9]+)?: (?P<fatal>fatal )?error: ")
# `error:` is what this counts, and gcc translates its diagnostics wherever a locale catalog is
# installed. A localized gcc would report zero errors for every file in the corpus. `LANGUAGE`
# overrides `LC_ALL` in glibc — a translated catalog installed makes gcc translate with only
# `LC_ALL` set — so both are scrubbed.
ENGLISH = {**os.environ, "LC_ALL": "C", "LANGUAGE": "C"}
# Whitespace is the formatter's to place, and so is a `\` continuation: a two-line macro that comes to
# fit on one keeps every character of its body and needs no continuation to hold it together.
OWNED = " \t\r\n\v\f\\"
# Counted as bytes, so the two sides compare the same way whether one arrives decoded and the other not.
OWNED_BYTES = OWNED.encode()
# The two ways gcc reports a file without answering, kept apart because they are not the same claim:
# gcc never answering at all, and gcc failing.
NO_ANSWER = "gcc never answered"
FAILED = "gcc failed"


class Errors(NamedTuple):
	counted: int


class Halted(NamedTuple):
	"""gcc stopped at a fatal error, so what it reported is where it gave up rather than how the file
	compiles — plus how many plain errors it counted first, which the halt-vs-halt comparison carries
	because a formatter may introduce one without moving the halt. A missing `#include`, or a header
	that is not C — `simdjson.h` stops at `<cstddef>` — reports exactly one error, which the input and
	the output share, so the comparison passes on the prefix alone: the two sides halted at the same
	place with the same count, and nothing after the halt was read on either side."""

	detail: str
	counted: int


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
	"""A file whose gcc baseline is a machine failure, so the compile comparison has nothing to compare
	against. Not a verdict and not an exclusion: the properties that do not need gcc — idempotency,
	no-token-loss, formats-at-all — are checked on it exactly as on any other file, and only the
	`gcc`-no-worse half is skipped. That is the honest shape, because those files *are* still formatted
	in the world, and #100 was a content-loss bug no compiler would have caught anyway.

	A halted baseline is not this: both sides halt at the same place, which is a comparison that found
	nothing worse. A `TimedOut` or a `Crashed` says gcc could not answer, and a `ToolFailed` says gcc
	failed — this machine's problem rather than the file's. Reported apart for that reason."""

	path: Path
	why: Crashed | TimedOut | ToolFailed


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
		# Truncated by bytes from the left, so the tail — the extension gcc dispatches on — survives:
		# `NAME_MAX` counts bytes, this prefix is 20-21 of them, and a basename already near the limit
		# would make the copy ENAMETOOLONG — a harness failure wearing a verdict about the file.
		stem = (
			path.name.encode("utf-8", "surrogateescape")[-200:]
			.decode("utf-8", "surrogateescape")
		)
		copy = Path(tmp) / f"jphfmt-corpus-{label}-{stem}"
		copy.write_bytes(data)
		return compiles(copy, path.parent)


def first_nonblank(text: str, fallback: str) -> str:
	"""The first line of `text` with something on it, or `fallback` — the one spelling of the rule both
	the tool-failure and the halt details read by. A fatal diagnostic can carry an empty message and be
	followed by `compilation terminated.`, where taking line zero reports nothing and taking nothing at
	all raised an `IndexError` out of a discovery worker.

	>>> first_nonblank("\\ncompilation terminated.\\n", "(none)")
	'compilation terminated.'
	>>> first_nonblank("\\n\\n", "(none)")
	'(none)'
	"""
	return next((line.strip() for line in text.splitlines() if line.strip()), fallback)


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
		return ToolFailed(first_nonblank(gcc.stderr, "(no stderr)"))
	# A `fatal error` is where gcc stopped, not what it found: everything after it went unread, so the
	# count is the prefix it managed rather than the file — and the halt carries it, because output that
	# introduces new plain errors before halting at the same place is the difference the halt-vs-halt
	# comparison must not discard. Both sides report the same one and the comparison passes having
	# checked nothing — the vacuous pass this whole harness exists to refuse.
	# From `end()`, so what is kept is gcc's message and not the location it prefixed: the file it names
	# is the temporary copy, whose path differs every run and is not the corpus file the report names.
	if (fatal := next(((i, d) for i, d in enumerate(diagnostics) if d.group("fatal")), None)) is not None:
		index, fatal_diagnostic = fatal
		return Halted(first_nonblank(gcc.stderr[fatal_diagnostic.end() :], "(no message)"), index)
	return Errors(found)


def severity(result: Compile) -> tuple[int, int]:
	"""How bad a compile is, ordered — a halt is worse than any number of errors, and gcc not
	answering at all is worse than a halt.

	>>> severity(Errors(3)) < severity(Errors(4))
	True
	>>> severity(Errors(999)) < severity(Halted("x.h: No such file", 0))
	True
	>>> severity(Halted("x", 0)) < severity(Halted("x", 1))
	True
	>>> severity(Halted("x", 1)) < severity(Crashed(11))
	True
	>>> severity(Crashed(11)) == severity(TimedOut(30))
	True
	>>> severity(Errors(5)) == severity(Errors(5))
	True
	"""
	match result:
		case Errors(count):
			return (0, count)
		case Halted(_, count):
			return (1, count)
		case Crashed() | TimedOut() | ToolFailed():
			return (2, 0)
		case _:
			raise AssertionError(f"unhandled Compile variant: {type(result).__name__}")


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


def worse_than(after: Compile, before: Compile) -> bool:
	"""Whether `after` outranks `before`: by severity — which for two halts is their prefix error
	counts — or by halting on a different message, the one difference severity cannot see.

	>>> worse_than(Errors(2), Errors(1))
	True
	>>> worse_than(Halted("x", 0), Halted("x", 0))
	False
	>>> worse_than(Halted("x", 1), Halted("x", 0))
	True
	>>> worse_than(Halted("y", 0), Halted("x", 0))
	True
	>>> worse_than(Crashed(11), Halted("x", 0))
	True
	"""
	return severity(after) > severity(before) or (
		isinstance(after, Halted) and isinstance(before, Halted) and after.detail != before.detail
	)


def check(binary: Path, path: Path) -> tuple[tuple[Verdict, ...], Unmeasured | None, bool]:
	"""Every property that fails for `path`, not the first — plus whether its baseline was an error
	count, which is what tells `main` gcc syntax-checked something rather than compared two halts.

	Reporting only the first hid a compile-breaking bug behind a non-idempotency for as long as both
	were present: `sqlite3.c` was unstable, so the `gcc` comparison never ran on it, and the 48 errors
	its formatted form carries surfaced only once the instability was fixed (#109, #112).
	"""
	first = format_with(binary, path)
	if not isinstance(first, Formatted):
		return ((as_verdict(path, first),), None, False)
	once = first.text
	again = format_with(binary, once)
	unstable: tuple[Verdict, ...] = ()
	if not isinstance(again, Formatted):
		unstable = (as_verdict(path, again),)
	elif again.text != once:
		unstable = (NotIdempotent(path, *first_difference(once, again.text)),)
	# Bytes for the original: it is being counted, not interpreted, and a corpus header that is not UTF-8
	# is the formatter's to report rather than this harness's to raise on. Read once and used twice —
	# for the count and for the baseline copy — so the two cannot disagree about a file that changed
	# underneath the run, and the 360k-line amalgamations are not read twice for nothing.
	original = path.read_bytes()
	# Back to the bytes the formatter actually emitted, for both the count and the compile.
	emitted = once.encode("utf-8", "surrogateescape")
	source, output = written(original), written(emitted)
	lost: tuple[Verdict, ...] = ()
	if output < source:
		lost = (LostContent(path, source, output),)
	# `severity` puts a halt above every error count, so a Halted baseline makes `after` unable to
	# outrank it and *every* output read clean — `severity(Errors(48)) > severity(Halted(…))` is False.
	# That is the vacuous pass, and the answer is to compare nothing and say so, not to compare it
	# anyway. gcc halts on 117 of this machine's 1200: a header that is not C, or C whose `#include`
	# this cannot resolve, where both sides report the same one fatal error.
	#
	# The after side still compiles for a halt: a machine failure ranks above it, so output that makes
	# gcc crash or hang where the input merely halted is a `Regressed`, and two halts compare by
	# message — output that halts on a different one changed what the file's includes resolve to.
	# Only a machine-failure baseline skips the comparison — nothing can outrank it.
	before = measure("before", original, path)
	if not isinstance(before, Errors | Halted):
		return (unstable + lost, Unmeasured(path, before), False)
	after = measure("after", emitted, path)
	regressed: tuple[Verdict, ...] = ()
	if worse_than(after, before):
		regressed = (Regressed(path, before, after),)
	return (unstable + lost + regressed, None, isinstance(before, Errors))


def checked(binary: Path, path: Path) -> tuple[tuple[Verdict, ...], Unmeasured | None, bool]:
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
		# Scrubbed of lone surrogates: a traceback naming a path whose bytes are not UTF-8 carries them,
		# and `main` prints a verdict outside this net, where a strict stdout — what a piped run has —
		# cannot encode one. Same crash class the formatter's stderr had, on the path that exists to
		# stop crashes.
		scrubbed = traceback.format_exc().encode("utf-8", "replace").decode("utf-8")
		return ((Broke(path, scrubbed.strip()),), None, False)


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
		case Halted(detail, count):
			error = "error" if count == 1 else "errors"
			return f"gcc gave up: {detail}" if count == 0 else f"gcc gave up: {count} {error}, then {detail}"
		case Crashed(signal):
			return f"killed by signal {signal}"
		case TimedOut(seconds):
			return f"no answer in {seconds}s"
		case ToolFailed(detail):
			return f"gcc failed: {detail}"
		case _:
			raise AssertionError(f"unhandled Compile variant: {type(result).__name__}")


def baseline_why(why: Crashed | TimedOut | ToolFailed) -> str:
	"""Which of the two machine failures `why` is — the label [`describe_unmeasured`] groups and
	`main`'s failing filters partition by. One spelling, so a new [`Compile`] variant is one edit
	(#64's class).

	>>> baseline_why(TimedOut(30))
	'gcc never answered'
	>>> baseline_why(Crashed(11))
	'gcc never answered'
	>>> baseline_why(ToolFailed("cc1: out of memory"))
	'gcc failed'
	"""
	match why:
		case TimedOut() | Crashed():
			return NO_ANSWER
		case ToolFailed():
			return FAILED
		case _:
			raise AssertionError(f"unhandled Compile variant: {type(why).__name__}")


class Failures(NamedTuple):
	"""The two machine-failure groups of a run's unmeasured files, in the order they are reported."""

	never_answered: tuple[Unmeasured, ...]
	failed: tuple[Unmeasured, ...]


def failures(unmeasured: tuple[Unmeasured, ...]) -> Failures:
	"""One partition of the unmeasured files into their machine-failure groups — the single spelling
	both [`describe_unmeasured`] and `main`'s failing filters consume, so the partition and its labels
	cannot drift apart.

	>>> failures(()) == Failures((), ())
	True
	>>> failures((Unmeasured(Path("a.h"), TimedOut(30)),)).never_answered[0].path.name
	'a.h'
	"""
	return Failures(
		never_answered=tuple(u for u in unmeasured if baseline_why(u.why) == NO_ANSWER),
		failed=tuple(u for u in unmeasured if baseline_why(u.why) == FAILED),
	)


def describe_unmeasured(groups: Failures) -> tuple[str, ...]:
	"""Which files got no compile comparison, grouped by cause, and named — never a bare count. Every
	one of them was still formatted and held to the properties that need no compiler.

	A `TimedOut` or a `Crashed` is gcc never answering, and a `ToolFailed` is gcc failing — either way
	this machine, and a different claim.

	>>> describe_unmeasured(Failures((), ()))
	()
	>>> for line in describe_unmeasured(Failures((Unmeasured(Path("a.h"), TimedOut(30)),), ())):
	...     print(line)
	1 file has no gcc baseline — gcc never answered; the format properties still ran
	  a.h: no answer in 30s
	>>> for line in describe_unmeasured(Failures((), (Unmeasured(Path("b.h"), ToolFailed("cc1: out of memory")),))):
	...     print(line)
	1 file has no gcc baseline — gcc failed; the format properties still ran
	  b.h: gcc failed: cc1: out of memory
	"""
	lines: list[str] = []
	for group, why in ((groups.never_answered, NO_ANSWER), (groups.failed, FAILED)):
		if not group:
			continue
		count = f"1 file has" if len(group) == 1 else f"{len(group)} files have"
		lines.append(f"{count} no gcc baseline — {why}; the format properties still ran")
		lines.extend(f"  {printable(u.path)}: {describe(u.why)}" for u in group)
	return tuple(lines)


def report_unmeasured(groups: Failures) -> None:
	"""The coverage lines for `groups`, to stderr — the interrupt path and the success path both report
	them, and must stay one spelling."""
	for line in describe_unmeasured(groups):
		print(line, file=sys.stderr)


def printable(path: Path) -> str:
	"""The path rendered for output: a name whose bytes are not UTF-8 carries lone surrogates, which
	a strict stdout — what a piped run has — cannot encode.

	>>> printable(Path("bad" + chr(0xDCFF) + ".h"))
	'bad\\\\udcff.h'
	"""
	return str(path).encode("utf-8", "backslashreplace").decode("utf-8")


def report(verdict: Verdict) -> str:
	match verdict:
		case Unformattable(path, status, stderr) if status < 0:
			return f"SIGNAL {-status:<3} {printable(path)}\n           {stderr.strip() or '(no stderr)'}"
		case Unformattable(path, status, stderr):
			return f"EXIT {status:<4} {printable(path)}\n           {stderr.strip() or '(no stderr)'}"
		case Stalled(path, seconds):
			return f"HUNG      {printable(path)}\n           no output in {seconds}s"
		case NotIdempotent(path, line, again):
			return f"UNSTABLE  {printable(path)}\n            once: {line!r}\n           twice: {again!r}"
		case Regressed(path, before, after):
			return f"REGRESSED {printable(path)}\n           gcc: {describe(before)} -> {describe(after)}"
		case LostContent(path, before, after):
			return f"LOST      {printable(path)}\n           {before - after} of {before} characters gone"
		case Broke(path, error):
			indented = error.replace("\n", "\n           ")
			return f"HARNESS   {printable(path)}\n           {indented}"
		case _:
			raise AssertionError(f"unhandled Verdict variant: {type(verdict).__name__}")


def main(argv: tuple[str, ...]) -> int:
	if argv == ("--self-test",):
		import doctest

		return doctest.testmod().failed
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
		print(f"no corpus at {printable(args.root)} — pass --root", file=sys.stderr)
		return 1
	if not args.no_build and (unbuilt := unbuildable()) is not None:
		print(unbuilt, file=sys.stderr)
		return 1
	if not args.binary.is_file():
		print(f"no formatter at {printable(args.binary)} — pass --binary", file=sys.stderr)
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
			f"the walk found only {len(files)} {noun} within {args.depth} {levels} of "
			f"{printable(args.root)}, not the {args.limit} asked for",
			file=sys.stderr,
		)

	# Reported as they arrive rather than collected: the largest files take minutes each, and a run that
	# is interrupted — or watched — should have already named everything it found.
	clean = 0
	error_counts = 0
	both = 0
	unmeasured_only = 0
	unmeasured: list[Unmeasured] = []
	# The pool is built inside the `try` so a Ctrl+C in the submission loop reaches the handler, and
	# starts unbound so one delivered mid-construction does not meet the handler with nothing to shut
	# down. The whole tail is inside too — a Ctrl+C during the shutdown, the coverage lines or the
	# headline would otherwise propagate uncaught with none of the accumulated state reported.
	pool: ThreadPoolExecutor | None = None
	try:
		pool = ThreadPoolExecutor(max_workers=args.jobs)
		pending = [pool.submit(checked, args.binary, path) for path in files]
		for done in as_completed(pending):
			verdicts, unmeasured_here, errors_baseline = done.result()
			# A file whose gcc baseline is a machine failure is not clean, even when no property
			# failed on it: the reasons list owns it, and a headline that counted it clean would
			# count the run's own failures as passes.
			clean += not verdicts and unmeasured_here is None
			unmeasured_only += not verdicts and unmeasured_here is not None
			error_counts += errors_baseline
			both += bool(verdicts) and unmeasured_here is not None
			unmeasured.extend(filter(None, (unmeasured_here,)))
			for verdict in verdicts:
				print(report(verdict), flush=True)
		pool.shutdown()
		groups = failures(tuple(unmeasured))
		report_unmeasured(groups)
		# gcc reading a file and giving up is a verdict about the file, and 117 of this machine's 1200
		# are that. gcc never answering, or gcc failing, is this machine failing, and a run that counts
		# those clean reports a pass it did not earn — the shape `Unmeasured` was a hard verdict for
		# before the corpus stopped excluding anything. When not one file produced an error count, gcc
		# never reported a full-file measure — which a clean pass cannot be about either.
		unchecked = error_counts == 0
		short = len(files) < args.limit
		# The headline never claims a clean pass on a failing run, and says which of the reasons failed
		# it. The gate is the same list — a run whose `why` is empty is the clean pass — so the exit
		# code and the headline cannot drift apart. The checks clause excludes the machine-failure
		# files, so the counts sum to the failing files instead of double-counting them.
		checks = len(files) - clean - both - unmeasured_only
		why = (
			[f"{checks} files the checks reported on"] * (checks > 0)
			+ [f"{len(groups.never_answered)} files {NO_ANSWER} for"] * bool(groups.never_answered)
			+ [f"{len(groups.failed)} files {FAILED} for"] * bool(groups.failed)
			+ ["no file was syntax-checked"] * unchecked
			+ [f"a corpus of {len(files)} files where {args.limit} was asked for"] * short
		)
		if not why:
			print(f"{clean} of {len(files)} files clean")
			return 0
		print(f"{clean} of {len(files)} files clean; the run fails on {', '.join(why)}", file=sys.stderr)
		return 1
	except KeyboardInterrupt:
		# `with ThreadPoolExecutor(...)` waits for every queued file on the way out, so a Ctrl+C sat
		# there for the rest of the run — for a corpus whose slowest file takes minutes, long enough to
		# look like a hang. Cancel what has not started and let the running workers go. The handler
		# cannot know which phase the interrupt landed in — the loop, or the success path's shutdown —
		# so it claims no phase, only the state it accumulated.
		if pool is not None:
			pool.shutdown(wait=False, cancel_futures=True)
		report_unmeasured(failures(tuple(unmeasured)))
		print("interrupted", file=sys.stderr)
		return 130


if __name__ == "__main__":
	raise SystemExit(main(tuple(sys.argv[1:])))
