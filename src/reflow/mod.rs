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
mod scope;
mod spacing;
mod structure;
mod tokens;

use std::borrow::Cow;

use self::tokens::is_comment;
use crate::doc::{TAB_WIDTH, display_width};
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
    let structured = structure::structure(&tokenize(&spaced), 0, width);
    let scoped = scope::scope_directives(&structured);
    normalize_endings(&collapse_blank_lines(&trim_comment_lines(&retab(&scoped))))
}

/// Strip trailing whitespace from every line of a comment (§2.1). Everywhere else it is already gone:
/// the gap before a newline is dropped by [`spacing::space_tokens`], and a whitespace-only line is
/// emitted as empty by [`collapse_blank_lines`]. A comment is one token, so its own line ends are the
/// only ones those two never reach.
///
/// Only comments. A string or character literal continued with `\` keeps the spaces before it, which
/// are part of its value, and an unterminated literal is not this pass's to edit.
fn trim_comment_lines(s: &str) -> String {
    tokenize(s)
        .into_iter()
        .map(|t| {
            // Borrowed unless there is something to trim, so the pass that reads its own output —
            // every run after the first — allocates nothing.
            if is_comment(&t) && t.text.split('\n').any(|l| l != l.trim_end()) {
                return Cow::Owned(
                    t.text
                        .split('\n')
                        .map(str::trim_end)
                        .collect::<Vec<_>>()
                        .join("\n"),
                );
            }
            Cow::Borrowed(t.text)
        })
        .collect()
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
            let cols = display_width(t.text);
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

#[cfg(test)]
mod tests {
    use super::{DEFAULT_WIDTH, format_with_width, spacing::space_tokens};
    use proptest::prelude::*;

    /// The formatter's output must be a fixpoint of the spacing pass *alone*.
    ///
    /// [`format_with_width`] runs the layout second, so a construct it spells one way and
    /// [`space_tokens`] spells another still comes out stable — the layout rewrites the disagreement
    /// away every run. What it cannot do is make the two agree, and the width the layout measured is
    /// then a width no pass will keep: whether the disagreement is visible depends on whether the
    /// construct was claimed, which is how #98, #99 and #43 each reached the output.
    ///
    /// Narrower than idempotency, not weaker: it localises the failure at the pass boundary rather
    /// than waiting for the disagreement to flip a fits/explode verdict, which needs an odd input and
    /// an odd width to happen. It is also not a superset — #100 and #102 are pass-boundary bugs this
    /// cannot see, because their outputs *are* fixpoints of the spacing pass.
    ///
    /// Fixtures in [`the_output_is_a_fixpoint_of_the_spacing_pass`];
    /// [`the_output_is_a_spacing_fixpoint_over_random_input`] searches the same property over random
    /// input, which is what stood between this class and a standing search before #43 was fixed.
    fn is_a_spacing_fixpoint(src: &str, width: usize) -> Result<(), String> {
        let out = format_with_width(src, width);
        let respaced = space_tokens(&out);
        if respaced == out {
            return Ok(());
        }
        // Whole strings when no line disagrees: `str::lines` drops a trailing newline and stops at the
        // shorter side, so a difference in either is a difference this zip cannot show.
        match out.lines().zip(respaced.lines()).find(|(a, b)| a != b) {
            Some((a, b)) => Err(format!("layout wrote {a:?}, the spacing pass writes {b:?}")),
            None => Err(format!(
                "layout wrote {out:?}, the spacing pass writes {respaced:?}"
            )),
        }
    }

    /// Every fixture the repo keeps, at the default width.
    ///
    /// Every read is unwrapped rather than skipped. A fixture this silently passed over would be a
    /// green run reporting coverage it did not have, which is the failure mode the check exists to
    /// close.
    #[test]
    fn the_output_is_a_fixpoint_of_the_spacing_pass() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let read_dir = |dir: std::path::PathBuf| {
            std::fs::read_dir(&dir)
                .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
                .map(|entry| entry.expect("dir entry").path())
        };
        let files = [root.join("tests/golden.c"), root.join("tests/messy.c")]
            .into_iter()
            .chain(
                read_dir(root.join("tests/cases"))
                    .filter(|shape| shape.is_dir())
                    .flat_map(read_dir)
                    .filter(|p| p.extension().is_some_and(|x| x == "c")),
            );
        for path in files {
            let src = std::fs::read_to_string(&path).expect("read fixture");
            if let Err(why) = is_a_spacing_fixpoint(&src, DEFAULT_WIDTH) {
                panic!("{}: {why}", path.display());
            }
        }
    }

    /// Strings of C-relevant characters — the generator `tests/properties.rs` uses, mirrored here
    /// because that binary cannot see [`space_tokens`].
    fn c_ish() -> impl Strategy<Value = String> {
        proptest::string::string_regex("[a-zA-Z0-9_(){}\\[\\];,*=<>?:&|+/.# \"'\\n\\t]{0,200}")
            .unwrap()
    }

    proptest! {
        #[test]
        fn the_output_is_a_spacing_fixpoint_over_random_input(s in c_ish(), width in 1usize..=120) {
            prop_assert!(
                is_a_spacing_fixpoint(&s, width).is_ok(),
                "not a spacing fixpoint at width {width}: {s:?}"
            );
        }
    }
}
