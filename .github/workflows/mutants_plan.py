# /// script
# requires-python = ">=3.14"
# ///
"""The mutants workflow's shard plan: one cell per src file, and builders.rs split into four of
cargo-mutants' own ``--shard`` slices — the file whose whole-file sweep kept losing its runner to
host shutdowns (#65). The partition is cargo-mutants', not this file's.

>>> plan(["src/a.rs", "src/reflow/builders.rs"], 4)
[{'file': 'src/a.rs', 'shard': '', 'index': '0'}, {'file': 'src/reflow/builders.rs', 'shard': '0/4', 'index': '1'}, {'file': 'src/reflow/builders.rs', 'shard': '1/4', 'index': '2'}, {'file': 'src/reflow/builders.rs', 'shard': '2/4', 'index': '3'}, {'file': 'src/reflow/builders.rs', 'shard': '3/4', 'index': '4'}]
"""

import json
import sys

type Json = dict[str, object]


def plan(files: list[str], shards: int) -> list[Json]:
    cells: list[Json] = []
    for file in files:
        if file == "src/reflow/builders.rs":
            for part in range(shards):
                cells.append(
                    {"file": file, "shard": f"{part}/{shards}", "index": str(len(cells))}
                )
        else:
            cells.append({"file": file, "shard": "", "index": str(len(cells))})
    return cells


def main() -> int:
    files = sys.stdin.read().splitlines()
    print(json.dumps(plan(files, 4)))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
