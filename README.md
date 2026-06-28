# jphfmt — an opinionated C formatter

`jphfmt` is a zero-config, opinionated C code formatter: gofmt/black philosophy
applied to C. It enforces **one uniform layout rule** and performs **no column
alignment, ever** — the thing clang-format hardwires and cannot turn off for
multi-line ternaries, bare parenthesized expressions, `for` clauses, and
function-like macros.

## The rule (the entire core)

For every bracket group — `(...)`, `{...}`, `[...]` — and for `for` headers:

1. If the whole construct **fits** within the column limit on its current line,
   keep it on one line.
2. Otherwise **explode**: a newline after the open bracket, every top-level
   element on its own line indented one tab deeper, a newline before the close
   bracket, and the close bracket at the parent's indentation.
3. **Never align** to an opening delimiter, an operator, or a previous token.
   Indentation is always whole tabs.

This is the Wadler/Prettier `group` combinator (fits-flat **or** fully-broken),
without Prettier's `fill` mode. A trailing comma before `}` is "magic": it forces
the list to explode (`{}` lists only). Binary operators and the ternary `:`
**trail** the line. Comments are sacred — never reflowed, moved, or re-aligned.

## Usage

```sh
jphfmt < in.c > out.c         # stdin → stdout
jphfmt -i file.c …            # rewrite files in place
jphfmt --check file.c …       # exit non-zero if any file is not formatted
jphfmt --width 80 < in.c      # column limit (default 100); tab width is 4
```

Format an entire tree (jphfmt accepts multiple files; use shell globs to discover them):

```sh
jphfmt -i **/*.c **/*.h       # bash (shopt -s globstar) / zsh
jphfmt -i **/*.{c,h}          # same, brace expansion
find . -name '*.[ch]' -exec jphfmt -i {} +  # POSIX sh
git ls-files '*.c' '*.h' | xargs jphfmt --check  # CI: only tracked files
```

Input that `jphfmt` cannot confidently structure is emitted verbatim, so it never
corrupts code: formatting only ever changes whitespace, magic-comma explosion,
and `\` line continuations — never any other token.

## Architecture

A token-stream pipeline (no full C parse needed — the rules are local):

```
source → lexer (logos, lossless: comments/whitespace are trivia tokens)
       → structurer (find calls, initializers, enums, control headers,
                     macros, statement-expressions, ternaries)
       → Doc builder (Wadler document: text / line / group / nest)
       → renderer (fits-or-fully-break at the width)
       → retab (normalize leading indentation to hard tabs)
```

- `src/lexer.rs` — the lossless C lexer.
- `src/doc.rs` — the `Doc` IR and width-aware renderer.
- `src/reflow.rs` — the structuring pass.

## Tests

`cargo test` (or `camas test`) runs the conformance suite:

- **Golden acceptance** — `tests/golden.c` is `jphfmt`'s fixpoint on
  `showcase.c` with its `// clang-format off|on` guards removed; `jphfmt` must
  reproduce the hand-laid forms those guards protected.
- Per-construct unit tests for every milestone.
- Idempotency, semantic preservation (whitespace/comma/continuation-only), and
  zero-column-alignment invariants, on `showcase.c` and a messy real-world
  fixture.

`camas all` runs format, clippy (`-D warnings`), tests, and docs — the same
checks CI runs.

## Status

Feature-complete for the [specification](FORMATTER_HANDOFF.md) §2: calls,
parameter lists, initializers, enums, `for`/`if`/`while`/`switch` headers,
function-like and statement-expression macros, and parenthesized ternaries.
Verified idempotent and content-preserving across 250+ real C files.
