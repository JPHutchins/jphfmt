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
- Hard-tab indentation and LF line-ending normalization with a single trailing
  newline (§2.1).
- CLI: stdin→stdout, `-i`/`--in-place`, `--check`, `--width N`, `--version`.
- An editor-agnostic LSP server and a VS Code client (`editors/vscode`).

### Known limitations

- Comment attachment/reflow is deferred: a list containing a comment passes
  through (re-tabbed) rather than being re-laid-out.
- Input must be valid UTF-8.
- Tab width for the overflow measurement is fixed at 4.
