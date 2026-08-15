//! Property tests: the lexer is total and the pipeline is safe, so these must hold for *any*
//! input, not just valid C. proptest also catches panics, so this doubles as a fuzz harness.

mod support;

use jphfmt::{format, format_with_width};
use proptest::prelude::*;
use support::{kept, ordered};

/// Strings of C-relevant characters (brackets, operators, comments, strings, whitespace), which
/// exercise the structurer far more than uniform random bytes would. The charset is [`jphfmt::PROPTEST_C_ISH`],
/// shared with the reflow module's spacing-fixpoint search so a widened generator is a widened search
/// everywhere.
fn c_ish() -> impl Strategy<Value = String> {
    proptest::string::string_regex(jphfmt::PROPTEST_C_ISH).unwrap()
}

/// Multi-character pieces of C — the tokens a handler dispatches on, and the bracket pairs that open
/// and close a construct. Character-level generation reaches a shape like `({x}y)` only by spelling
/// six specific characters in order, which it effectively never does; assembling from pieces reaches
/// it constantly, so the structurer's handler boundaries actually get probed.
const PIECES: &[&str] = &[
    "({", "})", "{", "}", "(", ")", "[", "]", "[[", "]]", ";", ",", "x", "0", "\"\"", "''", "=",
    "+", "?", ":", " ", "\n", "\t", "\\\n", "f", "if", "for", "while", "switch", "case", "return",
    "sizeof", "#define", "/*c*/", "//c\n", "*", "&", "|", "->", ".", "<<", "&&", "||", "int",
    "struct", "union", "enum",
];

fn pieced() -> impl Strategy<Value = String> {
    proptest::collection::vec(proptest::sample::select(PIECES), 1..24)
        .prop_map(|pieces| pieces.concat())
}

/// Whichever of `before`'s characters the output holds fewer of, if any.
fn dropped(before: &str, after: &str) -> Option<(char, usize, usize)> {
    let out = kept(after);
    kept(before).into_iter().find_map(|(c, n)| {
        let m = out.get(&c).copied().unwrap_or(0);
        (m < n).then_some((c, n, m))
    })
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

    /// The pieced generator can spell `#define`, which the width-sweeping test's generator cannot —
    /// so a width-specific two-cycle in a claimed shape (the define-body group's at width 40) fails
    /// here rather than depending on one conformance pin.
    #[test]
    fn pieced_input_is_idempotent_across_widths(s in pieced(), width in 1usize..=120) {
        let once = format_with_width(&s, width);
        prop_assert_eq!(format_with_width(&once, width), once);
    }
}
