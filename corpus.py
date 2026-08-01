# /// script
# requires-python = ">=3.14"
# dependencies = []
# ///
"""Format the C that is installed on this machine, and hold the output to what the input was.

Two properties no fixture can state, over real headers rather than reductions:

* **idempotent** — formatting the output returns it unchanged;
* **compiles no worse** — ``gcc -fsyntax-only`` reports no more errors on the output than on the
  input, and does not crash on output that the input did not crash it on.

The second is the only check that has ever caught jphfmt's compile-breaking bugs (#88, #90, #93,
#95, #100), and it needs real headers to be worth running: #100 corrupted 478 of 1200 corpus files
while every fixture in the suite stayed green, because the character it dropped was whitespace and
its output was a fixpoint.

Both files go through gcc from the *same* directory with the *same* flags. Compiling the input
where it lives and the output somewhere else resolves ``#include`` differently, which reports
failures that are the harness's rather than the formatter's.

Nothing here is skipped quietly. A missing compiler, an unreadable corpus and a gcc that dies on a
signal each end the run or name the file, because a check that passes vacuously is worse than one
that is not run at all.
"""

import argparse
import shutil
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor
from itertools import islice
from pathlib import Path
from typing import NamedTuple

GCC_TIMEOUT_S = 30


class Errors(NamedTuple):
	count: int


class Crashed(NamedTuple):
	signal: int


class TimedOut(NamedTuple):
	seconds: float


Compile = Errors | Crashed | TimedOut


class Unformattable(NamedTuple):
	path: Path
	status: int
	stderr: str


class NotIdempotent(NamedTuple):
	path: Path
	line: str
	again: str


class Regressed(NamedTuple):
	path: Path
	before: Compile
	after: Compile


Verdict = Unformattable | NotIdempotent | Regressed


def discover(root: Path, limit: int, depth: int) -> tuple[Path, ...]:
	"""The first `limit` C files at most `depth` levels under `root`, shallowest first.

	Depth-capped rather than recursive: `/nix/store` is deep enough that an unbounded walk does not
	finish. Shallowest first because that is where the headers worth compiling are —
	`<hash>-glibc-dev/include/stdio.h` is three levels down and a fixed four-level glob misses it.
	"""
	levels = (root.glob("/".join(["*"] * d)) for d in range(1, depth + 1))
	found = (
		p
		for level in levels
		for p in level
		if p.suffix in (".c", ".h") and p.is_file()
	)
	return tuple(islice(found, limit))


def compiles(source: Path, include: Path) -> Compile:
	"""How gcc fares on `source`. Assumes gcc is on PATH — [`main`] checks that once, up front."""
	try:
		gcc = subprocess.run(
			("gcc", "-std=c2x", "-fsyntax-only", "-I", str(include), str(source)),
			capture_output=True,
			text=True,
			timeout=GCC_TIMEOUT_S,
		)
	except subprocess.TimeoutExpired:
		return TimedOut(GCC_TIMEOUT_S)
	# A negative status is death by signal. Its stderr holds no `error:` to count, so counting alone
	# would read a compiler crash as a clean compile — the very thing this checks for.
	if gcc.returncode < 0:
		return Crashed(-gcc.returncode)
	return Errors(gcc.stderr.count("error:"))


def severity(result: Compile) -> tuple[int, int]:
	"""How bad a compile is, ordered — a crash or a timeout is worse than any number of errors.

	>>> severity(Errors(3)) < severity(Errors(4))
	True
	>>> severity(Errors(999)) < severity(Crashed(11))
	True
	>>> severity(Crashed(11)) == severity(TimedOut(30))
	True
	>>> severity(Errors(5)) == severity(Errors(5))
	True
	"""
	match result:
		case Errors(count):
			return (0, count)
		case Crashed() | TimedOut():
			return (1, 0)


def first_difference(before: str, after: str) -> tuple[str, str]:
	"""The first line the two disagree on, counting a line one of them does not have.

	>>> first_difference("a\\nb\\n", "a\\nc\\n")
	('b', 'c')
	>>> first_difference("a\\n", "a\\n")
	('', '')
	>>> first_difference("a\\nb\\n", "a\\n")
	('b', '<no line>')
	>>> first_difference("a\\n", "a\\nb\\n")
	('<no line>', 'b')
	"""
	missing = "<no line>"
	lines = zip(
		before.splitlines() + [missing] * len(after.splitlines()),
		after.splitlines() + [missing] * len(before.splitlines()),
	)
	return next((pair for pair in lines if pair[0] != pair[1]), ("", ""))


def format_with(binary: Path, source: Path | str) -> tuple[int, str, str]:
	run = subprocess.run(
		(str(binary), str(source) if isinstance(source, Path) else "/dev/stdin"),
		input=None if isinstance(source, Path) else source,
		capture_output=True,
		text=True,
	)
	return run.returncode, run.stdout, run.stderr


def check(binary: Path, path: Path) -> tuple[Verdict, ...]:
	"""Every property that fails for `path`, not the first.

	Reporting only the first hid a compile-breaking bug behind a non-idempotency for as long as both
	were present: `sqlite3.c` was unstable, so the `gcc` comparison never ran on it, and the 48 errors
	its formatted form carries surfaced only once the instability was fixed (#109, #112).
	"""
	status, once, stderr = format_with(binary, path)
	if status != 0:
		return (Unformattable(path, status, stderr),)
	again_status, twice, again_stderr = format_with(binary, once)
	unstable: tuple[Verdict, ...] = ()
	if again_status != 0:
		unstable = (Unformattable(path, again_status, again_stderr),)
	elif twice != once:
		unstable = (NotIdempotent(path, *first_difference(once, twice)),)
	with tempfile.TemporaryDirectory() as tmp:
		original, formatted = Path(tmp) / "in.c", Path(tmp) / "out.c"
		original.write_bytes(path.read_bytes())
		formatted.write_text(once)
		before = compiles(original, path.parent)
		after = compiles(formatted, path.parent)
	regressed: tuple[Verdict, ...] = ()
	if severity(after) > severity(before):
		regressed = (Regressed(path, before, after),)
	return unstable + regressed


def describe(result: Compile) -> str:
	match result:
		case Errors(count):
			return f"{count} errors"
		case Crashed(signal):
			return f"killed by signal {signal}"
		case TimedOut(seconds):
			return f"no answer in {seconds}s"


def report(verdict: Verdict) -> str:
	match verdict:
		case Unformattable(path, status, stderr):
			return f"EXIT {status:<4} {path}\n           {stderr.strip() or '(no stderr)'}"
		case NotIdempotent(path, line, again):
			return f"UNSTABLE  {path}\n            once: {line!r}\n           twice: {again!r}"
		case Regressed(path, before, after):
			return f"REGRESSED {path}\n           gcc: {describe(before)} -> {describe(after)}"


def main(argv: tuple[str, ...]) -> int:
	cli = argparse.ArgumentParser(description=__doc__)
	cli.add_argument("--root", type=Path, default=Path("/nix/store"))
	cli.add_argument("--limit", type=int, default=1200)
	cli.add_argument("--depth", type=int, default=4)
	cli.add_argument("--jobs", type=int, default=8)
	cli.add_argument("--binary", type=Path, default=Path("target/release/jphfmt"))
	cli.add_argument("--no-build", action="store_true")
	args = cli.parse_args(argv)

	if shutil.which("gcc") is None:
		print("gcc is not on PATH — this check needs a C compiler", file=sys.stderr)
		return 1
	if not args.root.is_dir():
		print(f"no corpus at {args.root} — pass --root", file=sys.stderr)
		return 1
	if not args.no_build:
		subprocess.run(("cargo", "build", "--release", "--quiet"), check=True)

	files = discover(args.root, args.limit, args.depth)
	if not files:
		print(f"no .c/.h files within {args.depth} levels of {args.root}", file=sys.stderr)
		return 1

	with ThreadPoolExecutor(max_workers=args.jobs) as pool:
		per_file = tuple(pool.map(lambda p: check(args.binary, p), files))

	for verdict in (v for verdicts in per_file for v in verdicts):
		print(report(verdict))
	clean = sum(1 for verdicts in per_file if not verdicts)
	print(f"{clean} of {len(files)} files clean")
	return 0 if clean == len(files) else 1


if __name__ == "__main__":
	raise SystemExit(main(tuple(sys.argv[1:])))
