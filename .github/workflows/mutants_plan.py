# /// script
# requires-python = ">=3.14"
# ///
"""The mutants workflow's shard plan: one cell per src file, and builders.rs split into four
ordinal chunks of cargo-mutants' own ``--list`` — the file whose whole-file sweep kept losing its
runner to host shutdowns (#65). Each chunk's cell names exactly its mutants with a ``-F`` regex of
exact-name alternations, so the cells partition the file by construction.

>>> plan(["src/a.rs", "src/reflow/builders.rs"], lambda file: [f"builders:{n}" for n in range(7)])
[{'file': 'src/a.rs', 're': '', 'index': '0'}, {'file': 'src/reflow/builders.rs', 're': '^(builders:0|builders:1)$', 'index': '1'}, {'file': 'src/reflow/builders.rs', 're': '^(builders:2|builders:3)$', 'index': '2'}, {'file': 'src/reflow/builders.rs', 're': '^(builders:4|builders:5)$', 'index': '3'}, {'file': 'src/reflow/builders.rs', 're': '^(builders:6)$', 'index': '4'}]
"""

import json
import re
import subprocess
import sys
from collections.abc import Callable

type Json = dict[str, object]


def plan(files: list[str], listed: Callable[[str], list[str]]) -> list[Json]:
    """One cell per file; builders.rs becomes four ordinal chunks of its mutant names.

    The chunk regexes are exact-name alternations, so the cells partition the file by
    construction, and a failed ``--list`` falls back to one whole-file cell rather than dropping
    the file.
    """
    cells: list[Json] = []
    for file in files:
        if file != "src/reflow/builders.rs":
            cells.append({"file": file, "re": "", "index": str(len(cells))})
            continue
        names = listed(file)
        if not names:
            cells.append({"file": file, "re": "", "index": str(len(cells))})
            continue
        chunk = max(1, (len(names) + 3) // 4)
        for part in (names[i : i + chunk] for i in range(0, len(names), chunk)):
            alt = "|".join(re.escape(name) for name in part)
            cells.append({"file": file, "re": f"^({alt})$", "index": str(len(cells))})
    return cells


def main() -> int:
    files = sys.stdin.read().splitlines()
    cells = plan(
        files,
        lambda file: subprocess.run(
            ["cargo", "mutants", "--file", file, "--list"],
            capture_output=True,
            text=True,
        ).stdout.splitlines(),
    )
    print(json.dumps(cells))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
