# Changelog

All notable changes to cfmt are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/), and the project adheres to
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- Lossless C lexer and a Wadler-style document engine (fits-flat-or-fully-break,
  no `fill`).
- The §2.2 rule applied to function-call / declaration argument lists, `{}`
  initializers, `enum` bodies (with the §2.3 magic trailing comma),
  `for`/`if`/`while`/`switch` headers, function-like and statement-expression
  macros, and parenthesized ternaries.
- §2.5 spacing: control-keyword space before `(`; pointer middle-spacing
  (`int*p` → `int * p`, including `struct`/`union`/`enum` tags); C-style cast
  spacing (`(int)x` → `(int) x`); bit-field colons (`x:1` → `x: 1`); and K&R
  brace-attach (`){` → `) {` for functions/control, while compound literals
  `(T){…}` and statement-expressions `({` stay tight).
- Compound-literal arguments break their `{…}` initializer when a call explodes.
- Hard-tab indentation and LF line-ending normalization with a single trailing
  newline (§2.1).
- CLI: stdin→stdout, `-i`/`--in-place`, `--check`, `--width N`, `--version`.
- An editor-agnostic LSP server and a VS Code client (`editors/vscode`).

### Known limitations

- Comments are never reflowed, moved, or re-aligned (§2.1, sacred). As a
  consequence, a `{}`/call list that contains a comment is passed through as-is
  (re-tabbed) rather than re-exploded or collapsed.
- A `*` after a bare user typedef (`mytype*p`) is not middle-spaced — it is
  token-level ambiguous with multiply, so it passes through (§6); spacing after
  type keywords and `struct`/`union`/`enum` tags is handled.
- Input must be valid UTF-8.
- Tab width for the overflow measurement is fixed at 4.
