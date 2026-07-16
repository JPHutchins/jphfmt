//! Pure token predicates and slice helpers shared across the reflow submodules. A leaf: depends
//! only on [`crate::lexer`], never on a sibling reflow module, so it can be unit-tested in isolation.

use crate::lexer::{Token, TokenKind};

/// An identifier that names a callee: an `Ident` that is neither a control/operator keyword
/// ([`is_excluded_callee`]) nor a type keyword ([`is_type_context`], after which `(` opens a
/// declarator group, not an argument list). The single predicate shared by [`is_call_head`] and
/// the reflow builders' trivia-tolerant `call_head_before`, so the two never diverge.
pub(super) fn is_callee_ident(t: &Token) -> bool {
    t.kind == TokenKind::Ident && !is_excluded_callee(t.text) && !is_type_context(t.text)
}

/// A callee identifier ([`is_callee_ident`]) immediately followed by `(` (no intervening
/// whitespace) — a call or the structurally identical declaration parameter list.
pub(super) fn is_call_head(toks: &[Token], i: usize) -> bool {
    toks.get(i).is_some_and(is_callee_ident)
        && toks
            .get(i + 1)
            .is_some_and(|n| n.kind == TokenKind::Punct && n.text == "(")
}

/// A C type keyword or qualifier — a token after which a `*` is confidently a pointer declarator,
/// not a multiply, and after which `(` opens a declarator group, not a call's argument list. User
/// typedefs (idents) are excluded, so ambiguous `a*b`/`foo*p`/`foo(x)` pass through (§6).
pub(super) fn is_type_context(text: &str) -> bool {
    matches!(
        text,
        "void"
            | "char"
            | "short"
            | "int"
            | "long"
            | "float"
            | "double"
            | "signed"
            | "unsigned"
            | "_Bool"
            | "bool"
            | "const"
            | "volatile"
            | "_Atomic"
            | "restrict"
    )
}

/// Keywords that take a `(` but are not calls whose arguments split on commas. `_Generic` is not
/// excluded: its associations are a comma list and explode exactly per §2.2.
pub(super) fn is_excluded_callee(name: &str) -> bool {
    matches!(
        name,
        "if" | "for"
            | "while"
            | "switch"
            | "return"
            | "do"
            | "else"
            | "sizeof"
            | "alignof"
            | "_Alignof"
            | "alignas"
            | "_Alignas"
            | "typeof"
            | "typeof_unqual"
            | "defined"
            | "static_assert"
            | "_Static_assert"
            | "__attribute__"
            | "_Pragma"
            | "asm"
            | "__asm__"
            | "__asm"
    )
}

/// The `{` opening an `enum [tag] [: type] { ... }` body that begins at the `enum` keyword `i`,
/// or `None` if this `enum` does not introduce a body (a forward declaration or a variable use).
pub(super) fn enum_body_brace(toks: &[Token], i: usize) -> Option<usize> {
    for (j, t) in toks.iter().enumerate().skip(i + 1) {
        match t.kind {
            TokenKind::Whitespace | TokenKind::Newline | TokenKind::Ident => {}
            TokenKind::Punct if t.text == ":" => {}
            TokenKind::Punct if t.text == "{" => return Some(j),
            _ => return None,
        }
    }
    None
}

/// The `(` that follows control keyword `i` after only trivia, or `None`.
pub(super) fn next_paren(toks: &[Token], i: usize) -> Option<usize> {
    for (j, t) in toks.iter().enumerate().skip(i + 1) {
        match t.kind {
            TokenKind::Whitespace | TokenKind::Newline => {}
            TokenKind::Punct if t.text == "(" => return Some(j),
            _ => return None,
        }
    }
    None
}

/// The next non-trivia token index at or after `from`.
pub(super) fn next_nontrivia(toks: &[Token], from: usize) -> Option<usize> {
    next_nontrivia_in(toks, from, toks.len())
}

/// The next non-trivia token index in `[from, end)`.
pub(super) fn next_nontrivia_in(toks: &[Token], from: usize, end: usize) -> Option<usize> {
    (from..end).find(|&j| !is_trivia(&toks[j]))
}

/// One past the last token of the preprocessor directive starting at `start` (following `\` line
/// continuations).
pub(super) fn directive_end(toks: &[Token], start: usize) -> usize {
    let mut i = start;
    while i < toks.len() {
        let is_newline = toks[i].kind == TokenKind::Newline;
        let continued = is_newline && i > 0 && toks[i - 1].text == "\\";
        i += 1;
        if is_newline && !continued {
            break;
        }
    }
    i
}

/// Index of the `)`/`]` matching the bracket at `open`, or `None` if unbalanced.
pub(super) fn match_bracket(toks: &[Token], open: usize) -> Option<usize> {
    matching(toks, open, "(", ")").or_else(|| matching(toks, open, "[", "]"))
}

/// Index of the `}` matching the `{` at `open`, or `None` if unbalanced.
pub(super) fn match_brace(toks: &[Token], open: usize) -> Option<usize> {
    matching(toks, open, "{", "}")
}

fn matching(toks: &[Token], open: usize, lhs: &str, rhs: &str) -> Option<usize> {
    if toks.get(open).map(|t| t.text) != Some(lhs) {
        return None;
    }
    let mut depth = 0usize;
    for (j, t) in toks.iter().enumerate().skip(open) {
        if t.text == lhs {
            depth += 1;
        } else if t.text == rhs {
            depth -= 1;
            if depth == 0 {
                return Some(j);
            }
        }
    }
    None
}

/// Split `inner` into segments at the depth-zero tokens for which `is_sep` holds.
pub(super) fn split_top_level<'a, 'src>(
    inner: &'a [Token<'src>],
    is_sep: impl Fn(&Token) -> bool,
) -> Vec<&'a [Token<'src>]> {
    let mut segments = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (j, t) in inner.iter().enumerate() {
        match t.text {
            "(" | "[" | "{" => depth += 1,
            ")" | "]" | "}" => depth -= 1,
            _ if depth == 0 && is_sep(t) => {
                segments.push(&inner[start..j]);
                start = j + 1;
            }
            _ => {}
        }
    }
    segments.push(&inner[start..]);
    segments
}

/// Split `inner` on commas at bracket depth zero.
pub(super) fn split_on_commas<'a, 'src>(inner: &'a [Token<'src>]) -> Vec<&'a [Token<'src>]> {
    split_top_level(inner, |t| t.kind == TokenKind::Punct && t.text == ",")
}

/// The outermost logical operator present at bracket depth zero (`||` outranks `&&`), if any.
pub(super) fn top_level_logical_op(inner: &[Token]) -> Option<&'static str> {
    let mut depth = 0i32;
    let mut has_and = false;
    for t in inner {
        match t.text {
            "(" | "[" | "{" => depth += 1,
            ")" | "]" | "}" => depth -= 1,
            "||" if depth == 0 && t.kind == TokenKind::Operator => return Some("||"),
            "&&" if depth == 0 && t.kind == TokenKind::Operator => has_and = true,
            _ => {}
        }
    }
    has_and.then_some("&&")
}

pub(super) fn is_trivia(t: &Token) -> bool {
    matches!(t.kind, TokenKind::Whitespace | TokenKind::Newline)
}

pub(super) fn contains_comment(toks: &[Token]) -> bool {
    toks.iter()
        .any(|t| matches!(t.kind, TokenKind::LineComment | TokenKind::BlockComment))
}

/// Whether `()`, `[]`, and `{}` are all balanced (never negative, net zero) in `toks`. Unbalanced
/// inner brackets defeat depth-aware splitting, so such a construct is unstructurable and is passed
/// through verbatim rather than risk mis-splitting (which could accumulate commas across passes).
pub(super) fn is_balanced(toks: &[Token]) -> bool {
    let (mut paren, mut brack, mut brace) = (0i32, 0i32, 0i32);
    for t in toks {
        if t.kind != TokenKind::Punct {
            continue;
        }
        match t.text {
            "(" => paren += 1,
            ")" => paren -= 1,
            "[" => brack += 1,
            "]" => brack -= 1,
            "{" => brace += 1,
            "}" => brace -= 1,
            _ => {}
        }
        if paren < 0 || brack < 0 || brace < 0 {
            return false;
        }
    }
    paren == 0 && brack == 0 && brace == 0
}

/// Whether a `?` ternary operator appears at bracket depth zero in `inner`.
pub(super) fn has_top_level_question(inner: &[Token]) -> bool {
    let mut depth = 0i32;
    for t in inner {
        match t.text {
            "(" | "[" | "{" => depth += 1,
            ")" | "]" | "}" => depth -= 1,
            "?" if depth == 0 && t.kind == TokenKind::Punct => return true,
            _ => {}
        }
    }
    false
}

/// Whether a comma-separated call argument has a newline in its body (after stripping leading
/// and trailing trivia). Such arguments would render differently on subsequent passes because
/// `build_expr_doc` collapses the newline into a space, which can then be reinterpreted by
/// `space_bit_fields`, breaking idempotency. When this is true the whole call is passed through
/// verbatim instead of being laid out via [`build_call_doc`].
pub(super) fn has_middle_newline(inner: &[Token]) -> bool {
    let args = split_top_level(inner, |t| t.kind == TokenKind::Punct && t.text == ",");
    for arg in args {
        let first = arg.iter().position(|t| !is_trivia(t));
        let last = arg.iter().rposition(|t| !is_trivia(t));
        if let (Some(f), Some(l)) = (first, last)
            && arg[f..=l].iter().any(|t| t.kind == TokenKind::Newline)
        {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_punct(text: &'static str) -> Token<'static> {
        Token {
            kind: TokenKind::Punct,
            text,
        }
    }

    fn tok(kind: TokenKind, text: &'static str) -> Token<'static> {
        Token { kind, text }
    }

    #[test]
    fn is_balanced_parens() {
        assert!(is_balanced(&[mk_punct("("), mk_punct(")")]));
    }

    #[test]
    fn is_balanced_brackets() {
        assert!(is_balanced(&[mk_punct("["), mk_punct("]")]));
    }

    #[test]
    fn is_balanced_braces() {
        assert!(is_balanced(&[mk_punct("{"), mk_punct("}")]));
    }

    #[test]
    fn is_balanced_combined() {
        assert!(is_balanced(&[
            mk_punct("("),
            mk_punct("["),
            mk_punct("]"),
            mk_punct("{"),
            mk_punct("}"),
            mk_punct(")"),
        ]));
    }

    #[test]
    fn is_balanced_unmatched_open() {
        assert!(!is_balanced(&[mk_punct("(")]));
    }

    #[test]
    fn is_balanced_mismatched() {
        assert!(!is_balanced(&[mk_punct("("), mk_punct("]")]));
    }

    #[test]
    fn is_balanced_negative_depth() {
        assert!(!is_balanced(&[mk_punct(")"), mk_punct("(")]));
    }

    #[test]
    fn is_balanced_empty() {
        assert!(is_balanced(&[]));
    }

    #[test]
    fn has_top_level_question_at_depth_zero() {
        assert!(has_top_level_question(&[mk_punct("?")]));
    }

    #[test]
    fn has_top_level_question_inside_parens() {
        assert!(!has_top_level_question(&[
            mk_punct("("),
            mk_punct("?"),
            mk_punct(")"),
        ]));
    }

    #[test]
    fn has_top_level_question_none() {
        assert!(!has_top_level_question(&[mk_punct("+"), mk_punct("-")]));
    }

    #[test]
    fn has_top_level_question_multiple_at_depth_zero() {
        assert!(has_top_level_question(&[
            mk_punct("?"),
            mk_punct("("),
            mk_punct("?"),
            mk_punct(")"),
            mk_punct("?"),
        ]));
    }

    #[test]
    fn is_excluded_callee_if() {
        assert!(is_excluded_callee("if"));
    }

    #[test]
    fn is_excluded_callee_for() {
        assert!(is_excluded_callee("for"));
    }

    #[test]
    fn is_excluded_callee_sizeof() {
        assert!(is_excluded_callee("sizeof"));
    }

    #[test]
    fn is_excluded_callee_printf() {
        assert!(!is_excluded_callee("printf"));
    }

    #[test]
    fn is_excluded_callee_myfunc() {
        assert!(!is_excluded_callee("myfunc"));
    }

    #[test]
    fn is_excluded_callee_empty() {
        assert!(!is_excluded_callee(""));
    }

    #[test]
    fn match_bracket_balanced() {
        assert_eq!(match_bracket(&[mk_punct("("), mk_punct(")")], 0), Some(1));
    }

    #[test]
    fn match_bracket_nested() {
        assert_eq!(
            match_bracket(
                &[mk_punct("("), mk_punct("("), mk_punct(")"), mk_punct(")")],
                0
            ),
            Some(3)
        );
    }

    #[test]
    fn match_bracket_unmatched_open() {
        assert_eq!(match_bracket(&[mk_punct("(")], 0), None);
    }

    #[test]
    fn match_bracket_wrong_kind() {
        // `match_bracket` only pairs `()`/`[]`, never `{}`.
        assert_eq!(match_bracket(&[mk_punct("{"), mk_punct("}")], 0), None);
    }

    #[test]
    fn match_brace_balanced() {
        assert_eq!(match_brace(&[mk_punct("{"), mk_punct("}")], 0), Some(1));
    }

    #[test]
    fn match_brace_unmatched_open() {
        assert_eq!(match_brace(&[mk_punct("{")], 0), None);
    }

    #[test]
    fn split_on_commas_depth_aware() {
        // A comma inside parens is at depth 1, so it does not split.
        let toks = [
            mk_punct("("),
            tok(TokenKind::Ident, "a"),
            mk_punct(","),
            tok(TokenKind::Ident, "b"),
            mk_punct(")"),
        ];
        assert_eq!(split_on_commas(&toks).len(), 1);
    }

    #[test]
    fn split_on_commas_top_level() {
        let toks = [
            tok(TokenKind::Ident, "a"),
            mk_punct(","),
            tok(TokenKind::Ident, "b"),
            mk_punct(","),
            tok(TokenKind::Ident, "c"),
        ];
        assert_eq!(split_on_commas(&toks).len(), 3);
    }

    #[test]
    fn top_level_logical_op_or_outranks_and() {
        // `||` at depth zero wins even when `&&` is also present.
        let toks = [
            tok(TokenKind::Ident, "a"),
            tok(TokenKind::Operator, "||"),
            tok(TokenKind::Ident, "b"),
            tok(TokenKind::Operator, "&&"),
            tok(TokenKind::Ident, "c"),
        ];
        assert_eq!(top_level_logical_op(&toks), Some("||"));
    }

    #[test]
    fn top_level_logical_op_and_only() {
        let toks = [
            tok(TokenKind::Ident, "a"),
            tok(TokenKind::Operator, "&&"),
            tok(TokenKind::Ident, "b"),
        ];
        assert_eq!(top_level_logical_op(&toks), Some("&&"));
    }

    #[test]
    fn top_level_logical_op_none() {
        let toks = [
            tok(TokenKind::Ident, "a"),
            tok(TokenKind::Punct, "+"),
            tok(TokenKind::Ident, "b"),
        ];
        assert_eq!(top_level_logical_op(&toks), None);
    }

    #[test]
    fn has_middle_newline_strips_trailing_trivia() {
        // A newline only in trailing trivia does not count as a middle newline.
        let toks = [
            tok(TokenKind::Ident, "a"),
            mk_punct(","),
            tok(TokenKind::Ident, "b"),
            tok(TokenKind::Whitespace, " "),
            tok(TokenKind::Newline, "\n"),
        ];
        assert!(!has_middle_newline(&toks));
    }

    #[test]
    fn has_middle_newline_inside_argument() {
        let toks = [
            tok(TokenKind::Ident, "a"),
            mk_punct(","),
            tok(TokenKind::Ident, "b"),
            tok(TokenKind::Newline, "\n"),
            tok(TokenKind::Ident, "c"),
        ];
        assert!(has_middle_newline(&toks));
    }

    #[test]
    fn has_middle_newline_nested_call_with_internal_newline() {
        // Regression guard: an arg that is itself a call with a newline inside its parens must
        // count as a middle newline so the whole call is passed through verbatim.
        use crate::lexer::tokenize;
        let src =
            "(handler), (event), read_monotonic_timestamp_ms(\n\t), current_execution_context_id()";
        let toks = tokenize(src);
        assert!(has_middle_newline(&toks));
    }

    #[test]
    fn is_call_head_ident_then_paren() {
        let toks = [tok(TokenKind::Ident, "foo"), mk_punct("(")];
        assert!(is_call_head(&toks, 0));
    }

    #[test]
    fn is_call_head_excluded_keyword() {
        let toks = [tok(TokenKind::Ident, "if"), mk_punct("(")];
        assert!(!is_call_head(&toks, 0));
    }

    #[test]
    fn is_call_head_no_paren() {
        let toks = [tok(TokenKind::Ident, "foo"), tok(TokenKind::Ident, "bar")];
        assert!(!is_call_head(&toks, 0));
    }

    #[test]
    fn is_call_head_type_keyword() {
        // `int (` is a declarator group, not a call — `is_call_head` and `call_head_before`
        // agree via the shared `is_callee_ident` guard.
        let toks = [tok(TokenKind::Ident, "int"), mk_punct("(")];
        assert!(!is_call_head(&toks, 0));
    }

    #[test]
    fn is_callee_ident_plain_ident() {
        assert!(is_callee_ident(&tok(TokenKind::Ident, "foo")));
    }

    #[test]
    fn is_callee_ident_excludes_keyword_and_type() {
        assert!(!is_callee_ident(&tok(TokenKind::Ident, "sizeof")));
        assert!(!is_callee_ident(&tok(TokenKind::Ident, "int")));
    }

    #[test]
    fn is_callee_ident_non_ident() {
        assert!(!is_callee_ident(&mk_punct("(")));
    }
}
