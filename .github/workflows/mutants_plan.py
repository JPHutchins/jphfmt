# /// script
# requires-python = ">=3.14"
# dependencies = ["msgspec"]
# ///
"""The mutants workflow's shard plan: one cell per src file, and builders.rs split into four of
cargo-mutants' own ``--shard`` slices — the file whose whole-file sweep kept losing its runner to
host shutdowns (#65). The partition is cargo-mutants', not this file's.

>>> plan(["src/a.rs", "src/reflow/builders.rs"], 4)
[Cell(file='src/a.rs', index='0', shard=''), Cell(file='src/reflow/builders.rs', index='1', shard='0/4'), Cell(file='src/reflow/builders.rs', index='2', shard='1/4'), Cell(file='src/reflow/builders.rs', index='3', shard='2/4'), Cell(file='src/reflow/builders.rs', index='4', shard='3/4')]
"""

import sys

import msgspec


class Cell(msgspec.Struct):
    """One shard cell: a file, its cargo-mutants --shard slice when it has one, and the index the
    artifact name and the merge key on."""

    file: str
    index: str
    shard: str = ""


def plan(files: list[str], shards: int) -> list[Cell]:
    cells: list[Cell] = []
    for file in files:
        if file == "src/reflow/builders.rs":
            for part in range(shards):
                cells.append(Cell(file=file, index=str(len(cells)), shard=f"{part}/{shards}"))
        else:
            cells.append(Cell(file=file, index=str(len(cells))))
    return cells


def main() -> int:
    files = sys.stdin.read().splitlines()
    print(msgspec.json.encode(plan(files, 4)).decode())
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
