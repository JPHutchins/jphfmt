# /// script
# requires-python = ">=3.14"
# ///
"""Release metadata: one version, four files, and the tag that ships it.

``Cargo.toml`` holds the version. ``check`` fails when any other file disagrees;
``ship`` does a whole release in one go — check, rewrite, commit, tag, push
``main`` and the tag, which is what starts CI's publish jobs — refusing before it
touches anything if the tree, the branch, or the version is not fit to release
from. Run through ``camas release``, never by hand.
"""

import json
import re
import subprocess
import sys
import tomllib
from pathlib import Path
from typing import Any, NamedTuple

ENCODING = "utf-8"
ROOT = Path(__file__).parent

# The files a release rewrites, repo-relative — `tasks.py` scopes `version_check` to these.
TRACKED = (
	"Cargo.toml",
	"Cargo.lock",
	"editors/vscode/package.json",
	"editors/vscode/package-lock.json",
)
CARGO_TOML, CARGO_LOCK, PACKAGE_JSON, PACKAGE_LOCK = (ROOT / name for name in TRACKED)

CRATE = "jphfmt"
PUBLISHER = "JPH"
MISSING = "<missing>"


def read(path: Path) -> str:
	return path.read_text(encoding=ENCODING)


type Json = dict[str, Any]


def loaded(path: Path) -> Json:
	"""`path` as JSON, or an empty mapping — a malformed file is drift to report, not a traceback."""
	try:
		value = json.loads(read(path))
	except (OSError, json.JSONDecodeError):
		return {}
	return value if isinstance(value, dict) else {}


def toml_loaded(path: Path) -> Json:
	try:
		with path.open("rb") as f:
			return tomllib.load(f)
	except (OSError, tomllib.TOMLDecodeError):
		return {}


class Found(NamedTuple):
	"""What a file says the release is, for ``check`` to compare."""

	path: Path
	field: str
	value: str


class Rewrite(NamedTuple):
	"""One pattern to replace in one file, applied only once every pattern has matched."""

	path: Path
	pattern: str
	replacement: str


def cargo_version() -> str:
	package = toml_loaded(CARGO_TOML).get("package")
	version = package.get("version") if isinstance(package, dict) else None
	if not isinstance(version, str):
		raise SystemExit(f"{CARGO_TOML.relative_to(ROOT)}: no readable [package] version")
	return version


def cargo_lock_version() -> str | None:
	packages = toml_loaded(CARGO_LOCK).get("package", [])
	if not isinstance(packages, list):
		return None
	for package in packages:
		if package.get("name") == CRATE:
			version = package.get("version")
			return version if isinstance(version, str) else None
	return None


def npm_field(manifest: Json, field: str) -> str | None:
	value = manifest.get(field)
	return value if isinstance(value, str) else None


def npm_lock_root_version(lock: Json) -> str | None:
	packages = lock.get("packages")
	root = packages.get("") if isinstance(packages, dict) else None
	return npm_field(root, "version") if isinstance(root, dict) else None


def or_missing(value: str | None) -> str:
	return MISSING if value is None else value


def found() -> tuple[Found, ...]:
	"""Each file read once: `check` compares five fields across four files."""
	manifest, lock = loaded(PACKAGE_JSON), loaded(PACKAGE_LOCK)
	return (
		Found(CARGO_LOCK, "package.jphfmt.version", or_missing(cargo_lock_version())),
		Found(PACKAGE_JSON, "version", or_missing(npm_field(manifest, "version"))),
		Found(PACKAGE_JSON, "publisher", or_missing(npm_field(manifest, "publisher"))),
		Found(PACKAGE_LOCK, "version", or_missing(npm_field(lock, "version"))),
		Found(PACKAGE_LOCK, "packages..version", or_missing(npm_lock_root_version(lock))),
	)


def check() -> int:
	version = cargo_version()
	expected = {"publisher": PUBLISHER}
	drift = [
		f"  {entry.path.relative_to(ROOT)} {entry.field} = {entry.value!r}, expected "
		f"{expected.get(entry.field, version)!r}"
		for entry in found()
		if entry.value != expected.get(entry.field, version)
	]
	if drift:
		print(f"release metadata disagrees with Cargo.toml version {version!r}:")
		print("\n".join(drift))
		print("\nfix with: uv run release.py sync " + version)
		return 1
	print(f"release metadata agrees: {version} (publisher {PUBLISHER})")
	return 0


def rewrites(version: str) -> tuple[Rewrite, ...]:
	"""Every edit a sync makes. JSON's trailing comma is optional to match, so a field that becomes
	the last in its object still does."""
	return (
		Rewrite(CARGO_TOML, r'^version = ".*"\r?$', f'version = "{version}"'),
		Rewrite(
			CARGO_LOCK,
			rf'^name = "{CRATE}"\r?\nversion = ".*"\r?$',
			f'name = "{CRATE}"\nversion = "{version}"',
		),
		Rewrite(PACKAGE_JSON, r'^(\s*)"version": ".*"(,?)\r?$', rf'\g<1>"version": "{version}"\g<2>'),
		Rewrite(
			PACKAGE_JSON,
			r'^(\s*)"publisher": ".*"(,?)\r?$',
			rf'\g<1>"publisher": "{PUBLISHER}"\g<2>',
		),
		Rewrite(PACKAGE_LOCK, r'^(\s*)"version": ".*"(,?)\r?$', rf'\g<1>"version": "{version}"\g<2>'),
		Rewrite(
			PACKAGE_LOCK,
			rf'^(\s+)"": \{{\r?\n(\s+)"name": "{CRATE}",\r?\n(\s+)"version": ".*"(,?)\r?$',
			rf'\g<1>"": {{\n\g<2>"name": "{CRATE}",\n\g<3>"version": "{version}"\g<4>',
		),
	)


def sync(version: str, guarded: bool = True) -> int:
	if not re.fullmatch(r"\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?", version):
		raise SystemExit(f"not a semver version: {version!r}")
	# Called on its own this rewrites four tracked files; `ship` has already cleared the tree.
	if guarded:
		refuse_unless_clean()
	# Every pattern is applied in memory first, so one that no longer matches leaves the tree as it
	# was rather than half rewritten.
	patched = {path: read(path) for path in {rewrite.path for rewrite in rewrites(version)}}
	for rewrite in rewrites(version):
		text, count = re.subn(
			rewrite.pattern,
			rewrite.replacement,
			patched[rewrite.path],
			count=1,
			flags=re.MULTILINE,
		)
		if count != 1:
			raise SystemExit(
				f"{rewrite.path.relative_to(ROOT)}: no match for {rewrite.pattern!r}; "
				"nothing was written"
			)
		patched[rewrite.path] = text
	for path, text in patched.items():
		path.write_text(text)
	return check()


def git(*args: str) -> str:
	"""Run git and return its stdout. A warning on stderr is not output — `git status --porcelain`
	is read for emptiness — but a failure reports both streams, since `push` explains itself there."""
	done = subprocess.run(
		("git", *args),
		cwd=ROOT,
		stdout=subprocess.PIPE,
		stderr=subprocess.PIPE,
		text=True,
		encoding=ENCODING,
		check=False,
	)
	if done.returncode != 0:
		said = "\n".join(part for part in (done.stdout.strip(), done.stderr.strip()) if part)
		raise SystemExit(f"git {' '.join(args)} failed:\n{said}")
	return done.stdout.strip()


def git_reporting(*args: str) -> str:
	"""`git` for a command whose report is on stderr, like `push`."""
	done = subprocess.run(
		("git", *args),
		cwd=ROOT,
		stdout=subprocess.PIPE,
		stderr=subprocess.STDOUT,
		text=True,
		encoding=ENCODING,
		check=False,
	)
	if done.returncode != 0:
		raise SystemExit(f"git {' '.join(args)} failed:\n{done.stdout.strip()}")
	return done.stdout.strip()


def bumped(version: str, part: str) -> str:
	major, minor, patch = (int(n) for n in version.split("-")[0].split(".")[:3])
	match part:
		case "major":
			return f"{major + 1}.0.0"
		case "minor":
			return f"{major}.{minor + 1}.0"
		case _:
			return f"{major}.{minor}.{patch + 1}"


def ordered(version: str) -> tuple[int, int, int, int]:
	"""Semver order, coarsely: a release outranks its own pre-releases (1.0.0 follows 1.0.0-rc1)."""
	major, minor, patch = (int(n) for n in version.split("-")[0].split("."))
	return (major, minor, patch, int("-" not in version))


def resolve(spec: str) -> str:
	"""The version `spec` asks for: a bump of the current one, or an explicit X.Y.Z ahead of it."""
	current = cargo_version()
	if spec in ("major", "minor", "patch"):
		return bumped(current, spec)
	if not re.fullmatch(r"\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?", spec):
		raise SystemExit(f"expected major, minor, patch, or X.Y.Z — got {spec!r}")
	if ordered(spec) <= ordered(current):
		raise SystemExit(f"{spec} does not follow {current}")
	return spec


def refuse_unless_clean() -> None:
	if git("status", "--porcelain"):
		raise SystemExit("working tree is dirty; commit or stash first")


def refuse_unless_releasable(tag: str) -> None:
	"""Everything that must hold before a release is allowed to write, commit, or push."""
	if git("rev-parse", "--abbrev-ref", "HEAD") != "main":
		raise SystemExit("release from main, so the default branch carries what was published")
	refuse_unless_clean()
	git("fetch", "--quiet", "origin", "main", "--tags")
	if git("rev-parse", "HEAD") != git("rev-parse", "origin/main"):
		raise SystemExit("main is not level with origin/main; pull or push first")
	if git("tag", "--list", tag):
		raise SystemExit(f"{tag} already exists")
	if git("ls-remote", "--tags", "origin", tag):
		raise SystemExit(f"{tag} already exists on origin")


def ship(spec: str) -> int:
	if check() != 0:
		return 1
	version = resolve(spec)
	tag = f"v{version}"
	refuse_unless_releasable(tag)
	if sync(version, guarded=False) != 0:
		return 1
	print(git("commit", "--all", "--message", f"release {version}"))
	print(git("tag", "--annotate", tag, "--message", tag))
	# One push, and main is named before the tag: a published version whose commit is not on the
	# default branch is how this metadata drifted, and `cargo install --git` builds that branch.
	# Together they succeed or fail as one, leaving no tag pointing at an unpushed commit.
	print(git_reporting("push", "--atomic", "origin", "main", tag))
	print(f"shipped {tag}: CI publishes the crate and the extension from the tag")
	return 0


def main(argv: tuple[str, ...]) -> int:
	match argv:
		case ("check",):
			return check()
		case ("sync", version):
			return sync(version)
		case ("ship", spec):
			return ship(spec)
		case _:
			print(__doc__)
			print("usage: release.py check | sync X.Y.Z | ship (major|minor|patch|X.Y.Z)")
			return 2


if __name__ == "__main__":
	raise SystemExit(main(tuple(sys.argv[1:])))
