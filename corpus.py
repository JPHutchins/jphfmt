# /// script
# requires-python = ">=3.14"
# dependencies = []
# ///
"""Format the C that is installed on this machine, and hold the output to what the input was.

Two properties no fixture can state, over real headers rather than reductions:

* **idempotent** — formatting the output returns it unchanged;
* **compiles no worse** — ``gcc -fsyntax-only`` reports no more errors on the output than on the
  input.

The second is the only check that has ever caught jphfmt's compile-breaking bugs (#88, #90, #93,
#95, #100), and it needs real headers to be worth running: #100 corrupted 478 of 1200 corpus files
while every fixture in the suite stayed green, because the character it dropped was whitespace and
its output was a fixpoint.

Both files go through gcc from the *same* directory with the *same* flags. Compiling the input
where it lives and the output somewhere else resolves ``#include`` differently, which reports
failures that are the harness's rather than the formatter's.
"""

import argparse
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from typing import NamedTuple


class Ok(NamedTuple):
	path: Path


class Unformattable(NamedTuple):
	path: Path
	status: int


class NotIdempotent(NamedTuple):
	path: Path
	line: str
	again: str


class Regressed(NamedTuple):
	path: Path
	before: int
	after: int


Verdict = Ok | Unformattable | NotIdempotent | Regressed


def discover(root: Path, limit: int) -> tuple[Path, ...]:
	"""The first `limit` C files under `root`, in whatever order the filesystem gives them.

	A store path is content-addressed, so the same `limit` files come back run to run.
	"""
	found = (p for p in root.glob("*/*/*/*") if p.suffix in (".c", ".h") and p.is_file())
	return tuple(p for _, p in zip(range(limit), found))


def errors(source: Path, include: Path) -> int:
	"""How many errors gcc reports, or 0 if it cannot be run at all.

	>>> errors(Path("/nonexistent-4a1c/x.c"), Path("/"))
	1
	"""
	gcc = subprocess.run(
		("gcc", "-std=c2x", "-fsyntax-only", "-I", str(include), str(source)),
		capture_output=True,
		text=True,
	)
	return gcc.stderr.count("error:")


def first_difference(before: str, after: str) -> tuple[str, str]:
	"""The first line the two disagree on.

	>>> first_difference("a\\nb\\n", "a\\nc\\n")
	('b', 'c')
	>>> first_difference("a\\n", "a\\n")
	('', '')
	"""
	pairs = zip(before.splitlines(), after.splitlines())
	return next((pair for pair in pairs if pair[0] != pair[1]), ("", ""))


def format_with(binary: Path, source: Path | str) -> tuple[int, str]:
	run = subprocess.run(
		(str(binary), str(source) if isinstance(source, Path) else "/dev/stdin"),
		input=None if isinstance(source, Path) else source,
		capture_output=True,
		text=True,
	)
	return run.returncode, run.stdout


def check(binary: Path, path: Path) -> Verdict:
	status, once = format_with(binary, path)
	if status != 0:
		return Unformattable(path, status)
	_, twice = format_with(binary, once)
	if twice != once:
		return NotIdempotent(path, *first_difference(once, twice))
	with tempfile.TemporaryDirectory() as tmp:
		original, formatted = Path(tmp) / "in.c", Path(tmp) / "out.c"
		original.write_bytes(path.read_bytes())
		formatted.write_text(once)
		before, after = errors(original, path.parent), errors(formatted, path.parent)
	return Ok(path) if after <= before else Regressed(path, before, after)


def report(verdict: Verdict) -> str:
	match verdict:
		case Ok(path):
			return f"ok        {path}"
		case Unformattable(path, status):
			return f"EXIT {status:<4} {path}"
		case NotIdempotent(path, line, again):
			return f"UNSTABLE  {path}\n            once: {line!r}\n           twice: {again!r}"
		case Regressed(path, before, after):
			return f"REGRESSED {path}\n           gcc errors: {before} -> {after}"


def main(argv: tuple[str, ...]) -> int:
	cli = argparse.ArgumentParser(description=__doc__)
	cli.add_argument("--root", type=Path, default=Path("/nix/store"))
	cli.add_argument("--limit", type=int, default=1200)
	cli.add_argument("--jobs", type=int, default=8)
	cli.add_argument("--binary", type=Path, default=Path("target/release/jphfmt"))
	cli.add_argument("--no-build", action="store_true")
	args = cli.parse_args(argv)

	if not args.no_build:
		subprocess.run(("cargo", "build", "--release", "--quiet"), check=True)

	files = discover(args.root, args.limit)
	if not files:
		print(f"no .c/.h files under {args.root} — nothing to check", file=sys.stderr)
		return 1

	with ThreadPoolExecutor(max_workers=args.jobs) as pool:
		verdicts = tuple(pool.map(lambda p: check(args.binary, p), files))

	failures = tuple(v for v in verdicts if not isinstance(v, Ok))
	for verdict in failures:
		print(report(verdict))
	print(f"{len(files) - len(failures)} of {len(files)} files clean")
	return 1 if failures else 0


if __name__ == "__main__":
	raise SystemExit(main(tuple(sys.argv[1:])))
