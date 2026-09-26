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

/// The #146 and #172 class shapes: a braced element, its label tail, and a call head the walk
/// attaches across a newline; and a declarator head holding a nested group with a break, whose
/// join refusal flips between the canonical and verbatim readings. The character-level generator
/// cannot spell either shape in any realistic draw, and the pieced one reaches them only when a
/// dozen pieces line up; this assembles them constantly, so a two-pass flip of either mechanism
/// is found, not hoped for.
const BIASED_PIECES: &[&str] = &[
    "A", "a", "{", "}", "*=", "?", ":", ",", "=", ")", "(", "()", "(aa() /)", "\n", "\t", " ",
    "\\", "\"", "x", ";", "&", "|", "/",
    "int", "(*", "*\n(", "\n(", ") = ",
];

/// The #172 class shape: a declarator head holding a nested group with a break — the head's
/// join refusal flipped between the canonical and verbatim readings. Assembled constantly, like
/// [`biased_bracket`], so a regression of that mechanism fails here rather than depending on one
/// conformance pin.
fn declarator_head() -> impl Strategy<Value = String> {
    (
        proptest::sample::select(&["int (*f", "int (*f\n", "int (*"][..]),
        proptest::sample::select(&["(int)", "\n(int)", "(int\n)", "(aa() /)"][..]),
    )
        .prop_map(|(head, inner)| format!("{head}{inner}) = a | b;"))
}

fn biased_bracket() -> impl Strategy<Value = String> {
    prop_oneof![
        proptest::collection::vec(proptest::sample::select(BIASED_PIECES), 1..12)
            .prop_map(|pieces| pieces.concat()),
        declarator_head(),
    ]
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

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200_000))]
    /// The #146 class: a brace whose reserve measured a callee-`(` newline gap one way on the pass
    /// that laid it and another on the next, flipping the brace's fits verdict. The character and
    /// pieced generators almost never spell it; [`biased_bracket`] assembles it constantly, so a
    /// regression of that mechanism fails here rather than depending on one conformance pin.
    #[test]
    fn biased_bracket_input_is_idempotent_across_widths(s in biased_bracket(), width in 1usize..=32) {
        let once = format_with_width(&s, width);
        prop_assert_eq!(format_with_width(&once, width), once);
    }
}
