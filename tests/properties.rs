//! Property tests: the lexer is total and the pipeline is safe, so these must hold for *any*
//! input, not just valid C. proptest also catches panics, so this doubles as a fuzz harness.

use cfmt::{format, format_with_width};
use proptest::prelude::*;

/// Strings of C-relevant characters (brackets, operators, comments, strings, whitespace), which
/// exercise the structurer far more than uniform random bytes would.
fn c_ish() -> impl Strategy<Value = String> {
    proptest::string::string_regex("[a-zA-Z0-9_(){}\\[\\];,*=<>?:&|+/.# \"'\\n\\t]{0,200}").unwrap()
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
}
