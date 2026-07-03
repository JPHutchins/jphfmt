//! The structuring pass. It reformats the constructs jphfmt understands with the §2.2 rule and
//! emits everything else byte-for-byte:
//!
//! * function-call / declaration argument lists (M2), detected by the house rule that a callee
//!   hugs its `(` with no space (`foo(`), which excludes control headers (`if (`) for free;
//! * `{}` initializer lists and `enum` bodies (M3), with the §2.3 magic trailing comma;
//! * `for`/`if`/`while`/`switch` headers (M4), one clause per line, operators trailing;
//! * `#define` bodies and GNU statement-expressions `({ ... })` (M5), the constructs clang-format
//!   cannot lay out — function-like macro bodies open on the `#define` line with `\` continuations
//!   one space after the content, and statement-expressions block-indent their statements.
//!
//! Anything not confidently one of these is emitted verbatim, so partial understanding never
//! corrupts code; lists containing comments are deferred to M7 and pass through.

mod builders;
mod spacing;
mod structure;
mod tokens;

use crate::doc::TAB_WIDTH;
use crate::lexer::{TokenKind, tokenize};

/// Default column limit (§8.5).
pub const DEFAULT_WIDTH: usize = 100;

/// Format C source with the default column limit ([`DEFAULT_WIDTH`]). Idempotent.
///
/// ```
/// assert_eq!(jphfmt::format("int*p = f(a,b);\n"), "int * p = f(a, b);\n");
/// ```
pub fn format(src: &str) -> String {
    format_with_width(src, DEFAULT_WIDTH)
}

/// Format with an explicit column limit. Tab width for the overflow measurement is fixed at
/// [`TAB_WIDTH`] (§8.5 default).
///
/// ```
/// let narrow = jphfmt::format_with_width("call(aaa, bbb, ccc);\n", 10);
/// assert_eq!(narrow, "call(\n\taaa,\n\tbbb,\n\tccc\n);\n");
/// ```
pub fn format_with_width(src: &str, width: usize) -> String {
    // Token spacing runs first so the layout measures final widths — otherwise a space added
    // afterward (`(int)x` -> `(int) x`) could widen a line and flip a fits/explode decision on
    // the next pass, breaking idempotency.
    let spaced = spacing::space_tokens(src);
    normalize_endings(&collapse_blank_lines(&retab(&structure::structure(
        &tokenize(&spaced),
        0,
        width,
    ))))
}

/// Collapse runs of two or more blank lines to a single blank line everywhere (file scope and
/// function bodies). Never inserts a blank line, so grouped declarations and adjacent closers are
/// preserved. Comment interiors are untouched — their newlines live inside one comment token.
fn collapse_blank_lines(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut line = String::new();
    let mut has_content = false;
    let mut blank_run = 0usize;
    for t in tokenize(s) {
        match t.kind {
            TokenKind::Newline => {
                if has_content {
                    out.push_str(&line);
                    out.push('\n');
                    blank_run = 0;
                } else {
                    blank_run += 1;
                    if blank_run <= 1 {
                        out.push('\n');
                    }
                }
                line.clear();
                has_content = false;
            }
            TokenKind::Whitespace => line.push_str(t.text),
            _ => {
                line.push_str(t.text);
                has_content = true;
            }
        }
    }
    out.push_str(&line);
    out
}

/// Normalize every line ending to LF and guarantee exactly one trailing newline (§2.1). An
/// all-whitespace input yields the empty string.
fn normalize_endings(s: &str) -> String {
    let lf = s.replace("\r\n", "\n").replace('\r', "\n");
    let trimmed = lf.trim_end();
    if trimmed.is_empty() {
        return String::new();
    }
    let mut out = String::with_capacity(trimmed.len() + 1);
    out.push_str(trimmed);
    out.push('\n');
    out
}

/// Normalize every line's leading indentation to hard tabs (§2.1): re-lex the output and rewrite
/// each line-leading whitespace run as `cols / TAB_WIDTH` tabs plus the remainder in spaces.
/// Comment- and string-safe — their bodies are single tokens, never line-leading whitespace.
fn retab(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut at_line_start = true;
    for t in tokenize(s) {
        let pure_indent =
            t.kind == TokenKind::Whitespace && t.text.bytes().all(|b| b == b' ' || b == b'\t');
        if at_line_start && pure_indent {
            let cols: usize = t
                .text
                .chars()
                .map(|c| if c == '\t' { TAB_WIDTH } else { 1 })
                .sum();
            for _ in 0..cols / TAB_WIDTH {
                out.push('\t');
            }
            for _ in 0..cols % TAB_WIDTH {
                out.push(' ');
            }
        } else {
            out.push_str(t.text);
        }
        at_line_start = t.kind == TokenKind::Newline;
    }
    out
}
