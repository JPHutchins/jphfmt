//! Property tests: the lexer is total and the pipeline is safe, so these must hold for *any*
//! input, not just valid C. proptest also catches panics, so this doubles as a fuzz harness.

use jphfmt::{format, format_with_width};
use proptest::prelude::*;
use std::collections::HashMap;

/// Strings of C-relevant characters (brackets, operators, comments, strings, whitespace), which
/// exercise the structurer far more than uniform random bytes would.
fn c_ish() -> impl Strategy<Value = String> {
    proptest::string::string_regex("[a-zA-Z0-9_(){}\\[\\];,*=<>?:&|+/.# \"'\\n\\t]{0,200}").unwrap()
}

/// Multi-character pieces of C — the tokens a handler dispatches on, and the bracket pairs that open
/// and close a construct. Character-level generation reaches a shape like `({x}y)` only by spelling
/// six specific characters in order, which it effectively never does; assembling from pieces reaches
/// it constantly, so the structurer's handler boundaries actually get probed.
const PIECES: &[&str] = &[
    "({", "})", "{", "}", "(", ")", "[", "]", "[[", "]]", ";", ",", "x", "0", "\"\"", "''", "=",
    "+", "?", ":", " ", "\n", "\t", "\\\n", "f", "if", "for", "while", "switch", "case", "return",
    "sizeof", "#define", "/*c*/", "//c\n", "*", "&", "|", "->", ".", "<<", "&&", "||",
];

fn pieced() -> impl Strategy<Value = String> {
    proptest::collection::vec(proptest::sample::select(PIECES), 1..24)
        .prop_map(|pieces| pieces.concat())
}

/// How many times each character that formatting must not discard occurs.
///
/// Whitespace and a `\` continuation are the layout's to place. So is a `,`: §2.3's magic trailing
/// comma means the layout writes them, and an all-empty `{,}` collapses to `{}`. Everything else is
/// the author's, and no amount of relayout may drop one.
fn kept(s: &str) -> HashMap<char, usize> {
    s.chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, ',' | '\\'))
        .fold(HashMap::new(), |mut counts, c| {
            *counts.entry(c).or_default() += 1;
            counts
        })
}

/// Whichever of `before`'s characters the output holds fewer of, if any.
fn dropped(before: &str, after: &str) -> Option<(char, usize, usize)> {
    let out = kept(after);
    kept(before).into_iter().find_map(|(c, n)| {
        let m = out.get(&c).copied().unwrap_or(0);
        (m < n).then_some((c, n, m))
    })
}

/// The author's characters in order, dropping the ones a relayout may *write* as well as discard:
/// a `;` terminating a statement expression's last statement, and the `()` that bound a broken chain.
/// Counting alone would accept `a + b` becoming `b + a`; this would not.
fn ordered(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, ',' | ';' | '\\' | '(' | ')'))
        .collect()
}

proptest! {
    #[test]
    fn format_is_idempotent(s in c_ish()) {
        let once = format(&s);
        prop_assert_eq!(format(&once), once);
    }

    #[test]
    fn format_never_panics_on_arbitrary_bytes(s in ".{0,200}") {
        let _ = format(&s);
    }

    #[test]
    fn idempotent_across_widths(s in c_ish(), width in 1usize..=120) {
        let once = format_with_width(&s, width);
        prop_assert_eq!(format_with_width(&once, width), once);
    }

    /// Formatting is a relayout, so it may add a separator the layout owns but must never discard or
    /// reorder what the author wrote. A handler that reports more tokens consumed than it renders
    /// deletes source silently, which no idempotency check catches: the truncated output is a fixpoint.
    ///
    /// Both halves are needed. [`dropped`] counts `;` so a lost one fails, but counting cannot see a
    /// reordering; [`ordered`] sees order but must excuse the `;` a statement expression writes.
    #[test]
    fn formatting_never_drops_what_the_author_wrote(s in prop_oneof![c_ish(), pieced()]) {
        let once = format(&s);
        if let Some((c, had, has)) = dropped(&s, &once) {
            prop_assert!(false, "{c:?} appears {had}x in input, {has}x in output: {s:?} -> {once:?}");
        }
        prop_assert_eq!(ordered(&s), ordered(&once), "reordered: {:?} -> {:?}", s, once);
    }

    #[test]
    fn pieced_input_is_idempotent(s in pieced()) {
        let once = format(&s);
        prop_assert_eq!(format(&once), once);
    }
}
