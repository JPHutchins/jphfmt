# /// script
# requires-python = ">=3.14"
# ///
"""Release metadata: one version, four files, and the tag that ships it.

``Cargo.toml`` holds the version; ``check`` fails when any other file disagrees,
``sync X.Y.Z`` rewrites them all, and ``send`` tags that version and pushes it,
which is what starts CI's publish jobs. Run through ``camas`` (``version_check``,
``version_sync``, ``release``), never by hand.
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


class Found(NamedTuple):
	"""What a file says the release is, for ``check`` to compare."""

	path: Path
	field: str
	value: str


def cargo_version() -> str:
	with CARGO_TOML.open("rb") as f:
		version = tomllib.load(f)["package"]["version"]
	assert isinstance(version, str)
	return version


def cargo_lock_version() -> str | None:
	with CARGO_LOCK.open("rb") as f:
		packages = tomllib.load(f)["package"]
	assert isinstance(packages, list)
	for package in packages:
		if package["name"] == CRATE:
			version = package["version"]
			assert isinstance(version, str)
			return version
	return None


def npm_field(path: Path, field: str) -> str | None:
	value = json.loads(path.read_text())[field]
	return value if isinstance(value, str) else None


def npm_lock_versions() -> tuple[str | None, str | None]:
	manifest = json.loads(PACKAGE_LOCK.read_text())
	root = manifest["packages"][""]
	return npm_field(PACKAGE_LOCK, "version"), root.get("version")


def found() -> tuple[Found, ...]:
	lock_top, lock_root = npm_lock_versions()
	return (
		Found(CARGO_LOCK, "package.jphfmt.version", str(cargo_lock_version())),
		Found(PACKAGE_JSON, "version", str(npm_field(PACKAGE_JSON, "version"))),
		Found(PACKAGE_JSON, "publisher", str(npm_field(PACKAGE_JSON, "publisher"))),
		Found(PACKAGE_LOCK, "version", str(lock_top)),
		Found(PACKAGE_LOCK, "packages..version", str(lock_root)),
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


def replace_first(path: Path, pattern: str, replacement: str) -> None:
	text = path.read_text()
	patched, count = re.subn(pattern, replacement, text, count=1, flags=re.MULTILINE)
	if count != 1:
		raise SystemExit(f"{path.relative_to(ROOT)}: no match for {pattern!r}")
	path.write_text(patched)


def sync(version: str) -> int:
	if not re.fullmatch(r"\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?", version):
		raise SystemExit(f"not a semver version: {version!r}")
	replace_first(CARGO_TOML, r'^version = ".*"$', f'version = "{version}"')
	replace_first(
		CARGO_LOCK,
		rf'^name = "{CRATE}"\nversion = ".*"$',
		f'name = "{CRATE}"\nversion = "{version}"',
	)
	for path in (PACKAGE_JSON, PACKAGE_LOCK):
		replace_first(path, r'^(\s*)"version": ".*",$', rf'\g<1>"version": "{version}",')
	replace_first(
		PACKAGE_LOCK,
		r'^(\s+)"": \{\n(\s+)"name": "jphfmt",\n(\s+)"version": ".*",$',
		rf'\g<1>"": {{\n\g<2>"name": "jphfmt",\n\g<3>"version": "{version}",',
	)
	replace_first(PACKAGE_JSON, r'^(\s*)"publisher": ".*",$', rf'\g<1>"publisher": "{PUBLISHER}",')
	return check()


def git(*args: str) -> str:
	return subprocess.run(
		("git", *args), cwd=ROOT, check=True, capture_output=True, text=True
	).stdout.strip()


def send() -> int:
	if check() != 0:
		return 1
	version = cargo_version()
	tag = f"v{version}"
	if git("status", "--porcelain"):
		raise SystemExit("working tree is dirty; commit the release metadata first")
	if git("tag", "--list", tag):
		raise SystemExit(f"{tag} already exists; bump with: uv run release.py sync X.Y.Z")
	if git("rev-parse", "--abbrev-ref", "HEAD") != "main":
		raise SystemExit("release from main, so the default branch carries what was published")
	print(git("tag", "-a", tag, "-m", tag))
	print(git("push", "origin", tag))
	print(f"pushed {tag}: CI publishes the crate and the extension from the tag")
	return 0


def main(argv: tuple[str, ...]) -> int:
	match argv:
		case ("check",):
			return check()
		case ("sync", version):
			return sync(version)
		case ("send",):
			return send()
		case _:
			print(__doc__)
			print("usage: release.py check | sync X.Y.Z | send")
			return 2


if __name__ == "__main__":
	raise SystemExit(main(tuple(sys.argv[1:])))
