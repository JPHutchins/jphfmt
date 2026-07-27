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
from typing import NamedTuple

ROOT = Path(__file__).parent
CARGO_TOML = ROOT / "Cargo.toml"
CARGO_LOCK = ROOT / "Cargo.lock"
PACKAGE_JSON = ROOT / "editors/vscode/package.json"
PACKAGE_LOCK = ROOT / "editors/vscode/package-lock.json"

CRATE = "jphfmt"
PUBLISHER = "JPH"
MISSING = "<missing>"


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
	with CARGO_TOML.open("rb") as f:
		version = tomllib.load(f).get("package", {}).get("version")
	if not isinstance(version, str):
		raise SystemExit(f"{CARGO_TOML.relative_to(ROOT)}: no [package] version")
	return version


def cargo_lock_version() -> str | None:
	with CARGO_LOCK.open("rb") as f:
		packages = tomllib.load(f).get("package", [])
	if not isinstance(packages, list):
		return None
	for package in packages:
		if package.get("name") == CRATE:
			version = package.get("version")
			return version if isinstance(version, str) else None
	return None


def npm_field(path: Path, field: str) -> str | None:
	value = json.loads(path.read_text()).get(field)
	return value if isinstance(value, str) else None


def npm_lock_root_version() -> str | None:
	root = json.loads(PACKAGE_LOCK.read_text()).get("packages", {}).get("", {})
	version = root.get("version") if isinstance(root, dict) else None
	return version if isinstance(version, str) else None


def or_missing(value: str | None) -> str:
	return MISSING if value is None else value


def found() -> tuple[Found, ...]:
	return (
		Found(CARGO_LOCK, "package.jphfmt.version", or_missing(cargo_lock_version())),
		Found(PACKAGE_JSON, "version", or_missing(npm_field(PACKAGE_JSON, "version"))),
		Found(PACKAGE_JSON, "publisher", or_missing(npm_field(PACKAGE_JSON, "publisher"))),
		Found(PACKAGE_LOCK, "version", or_missing(npm_field(PACKAGE_LOCK, "version"))),
		Found(PACKAGE_LOCK, "packages..version", or_missing(npm_lock_root_version())),
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
		Rewrite(CARGO_TOML, r'^version = ".*"$', f'version = "{version}"'),
		Rewrite(
			CARGO_LOCK,
			rf'^name = "{CRATE}"\nversion = ".*"$',
			f'name = "{CRATE}"\nversion = "{version}"',
		),
		Rewrite(PACKAGE_JSON, r'^(\s*)"version": ".*"(,?)$', rf'\g<1>"version": "{version}"\g<2>'),
		Rewrite(
			PACKAGE_JSON,
			r'^(\s*)"publisher": ".*"(,?)$',
			rf'\g<1>"publisher": "{PUBLISHER}"\g<2>',
		),
		Rewrite(PACKAGE_LOCK, r'^(\s*)"version": ".*"(,?)$', rf'\g<1>"version": "{version}"\g<2>'),
		Rewrite(
			PACKAGE_LOCK,
			rf'^(\s+)"": \{{\n(\s+)"name": "{CRATE}",\n(\s+)"version": ".*"(,?)$',
			rf'\g<1>"": {{\n\g<2>"name": "{CRATE}",\n\g<3>"version": "{version}"\g<4>',
		),
	)


def sync(version: str) -> int:
	if not re.fullmatch(r"\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?", version):
		raise SystemExit(f"not a semver version: {version!r}")
	# Every pattern is applied in memory first, so one that no longer matches leaves the tree as it
	# was rather than half rewritten.
	patched = {path: path.read_text() for path in {rewrite.path for rewrite in rewrites(version)}}
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
	"""Run git, returning what it said on either stream — `push` reports on stderr."""
	done = subprocess.run(
		("git", *args),
		cwd=ROOT,
		stdout=subprocess.PIPE,
		stderr=subprocess.STDOUT,
		text=True,
		check=False,
	)
	if done.returncode != 0:
		raise SystemExit(f"git {' '.join(args)} failed:\n{done.stdout.strip()}")
	return done.stdout.strip()


def bumped(version: str, part: str) -> str:
	major, minor, patch = (int(n) for n in version.split(".")[:3])
	match part:
		case "major":
			return f"{major + 1}.0.0"
		case "minor":
			return f"{major}.{minor + 1}.0"
		case _:
			return f"{major}.{minor}.{patch + 1}"


def ordered(version: str) -> tuple[int, ...]:
	return tuple(int(n) for n in version.split("-")[0].split("."))


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


def refuse_unless_releasable(tag: str) -> None:
	"""Everything that must hold before a release is allowed to write, commit, or push."""
	if git("rev-parse", "--abbrev-ref", "HEAD") != "main":
		raise SystemExit("release from main, so the default branch carries what was published")
	if git("status", "--porcelain"):
		raise SystemExit("working tree is dirty; the release commit must hold only the version")
	git("fetch", "--quiet", "origin", "main", "--tags")
	if git("rev-parse", "HEAD") != git("rev-parse", "origin/main"):
		raise SystemExit("main is not level with origin/main; pull or push first")
	if git("tag", "--list", tag):
		raise SystemExit(f"{tag} already exists")


def ship(spec: str) -> int:
	if check() != 0:
		return 1
	version = resolve(spec)
	tag = f"v{version}"
	refuse_unless_releasable(tag)
	if sync(version) != 0:
		return 1
	print(git("commit", "--all", "--message", f"release {version}"))
	print(git("tag", "--annotate", tag, "--message", tag))
	# main before the tag: a published version whose commit is not on the default branch is how this
	# metadata drifted in the first place, and `cargo install --git` builds that branch.
	print(git("push", "origin", "main"))
	print(git("push", "origin", tag))
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
