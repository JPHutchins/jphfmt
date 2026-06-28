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
- §2.5 spacing: a space before `(` for control keywords (`if (`), and pointer
  middle-spacing after a type keyword/qualifier (`int*p` → `int * p`).
- Hard-tab indentation and LF line-ending normalization with a single trailing
  newline (§2.1).
- CLI: stdin→stdout, `-i`/`--in-place`, `--check`, `--width N`, `--version`.
- An editor-agnostic LSP server and a VS Code client (`editors/vscode`).

### Known limitations

- Comment attachment/reflow is deferred: a list containing a comment passes
  through (re-tabbed) rather than being re-laid-out.
- Some §2.5 spacing is token-level ambiguous and therefore preserved rather than
  normalized (§6 "prefer passthrough when ambiguous"): cast spacing, bit-field
  colons, brace-attach (`) {` vs the tight `({`/compound-literal `){`), and
  pointers after a user typedef (`mytype*p`). Already-correct input is preserved;
  these messy forms are simply left as-is.
- Input must be valid UTF-8.
- Tab width for the overflow measurement is fixed at 4.
