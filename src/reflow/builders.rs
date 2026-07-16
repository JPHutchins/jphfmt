//! Wadler/Leijen `Doc` builders for the constructs jphfmt lays out: call argument lists, `{}` and
//! `enum` bodies, `for`/condition clause groups, and parenthesized ternary chains. Each builder
//! turns a token slice into a [`Doc`] that [`crate::doc::render`] later flattens or fully breaks per
//! §2.2. Depends on [`super::tokens`] for depth-aware splitting and balance checks.

use super::tokens::{
    has_non_trivia, is_balanced, is_callee_ident, is_trivia, match_brace, match_bracket,
    split_on_commas, split_top_level, top_level_logical_op,
};
use crate::doc::Doc;
use crate::lexer::{Token, TokenKind};

/// Build the §2.2 document for a call's argument list, including the surrounding parens. Each
/// argument is built recursively (via [`build_expr_doc`]), so a nested `{...}` or a nested call
/// is its own group that can collapse or explode independently.
pub(super) fn build_call_doc(inner: &[Token]) -> Doc {
    if !is_balanced(inner) {
        return Doc::Text(format!("({})", render_segment(inner)));
    }
    let args: Vec<&[Token]> = split_on_commas(inner)
        .into_iter()
        .filter(|a| has_non_trivia(a))
        .collect();
    if args.is_empty() {
        return Doc::text("()");
    }
    let last = args.len() - 1;
    let mut items = vec![Doc::SoftLine];
    for (idx, arg) in args.into_iter().enumerate() {
        items.push(build_expr_doc(arg));
        if idx < last {
            items.push(Doc::text(","));
            items.push(Doc::Line);
        }
    }
    Doc::group(Doc::concat([
        Doc::text("("),
        Doc::nest(Doc::concat(items)),
        Doc::SoftLine,
        Doc::text(")"),
    ]))
}

/// Build the document for a `{}` list: comma-separated elements, a trailing comma when broken, and
/// the §2.3 magic comma (a trailing comma in the source forces explosion). `padded` adds an inner
/// space in the flat form (`enum { A, B }`) versus the tight initializer form (`{1, 2}`).
pub(super) fn build_brace_doc(inner: &[Token], padded: bool) -> Doc {
    if !is_balanced(inner) {
        return Doc::Text(format!("{{{}}}", render_segment(inner)));
    }
    let segments = split_on_commas(inner);
    let magic = segments.len() > 1 && segments.last().is_some_and(|s| s.iter().all(is_trivia));
    let elements: Vec<&[Token]> = segments.into_iter().filter(|s| has_non_trivia(s)).collect();
    if elements.is_empty() {
        return Doc::text("{}");
    }
    let pad = || if padded { Doc::Line } else { Doc::SoftLine };
    let last = elements.len() - 1;
    let mut items = vec![pad()];
    for (idx, element) in elements.into_iter().enumerate() {
        items.push(build_expr_doc(element));
        if idx < last {
            items.push(Doc::text(","));
            items.push(Doc::Line);
        } else {
            items.push(Doc::IfBreak {
                broken: ",".to_owned(),
                flat: String::new(),
            });
        }
    }
    let body = Doc::concat([
        Doc::text("{"),
        Doc::nest(Doc::concat(items)),
        pad(),
        Doc::text("}"),
    ]);
    if magic {
        Doc::ForceBreak(Box::new(body))
    } else {
        Doc::group(body)
    }
}

/// Whether the nearest non-trivia token before `open` names a callee ([`is_callee_ident`]). Unlike
/// [`super::tokens::is_call_head`], trivia (including a newline) between the ident and `(` is
/// tolerated: [`build_expr_doc`] must flatten such a gap to nothing (§2.5's tight `foo(`) rather
/// than a collapsed space, since a collapsed space is itself same-line and would be tightened by
/// `space_call_heads` on the next pass — collapsing to a space here instead would render this
/// pass's output as a fixpoint of a *different* pass, breaking idempotency.
///
/// Only an *identifier* callee is recognized: calls through a function pointer (`(*p)(args)`) or a
/// parenthesized expression (`(expr)(args)`) are left as flat text, because a `)` before `(` is
/// token-level indistinguishable from a C-style cast `(type)(expr)` — exploding the latter as a
/// call would be wrong, so §6 "prefer passthrough when ambiguous" applies.
///
/// Only whitespace/newline trivia is skipped, never comments: a commented `foo /* c */ (a)` stops
/// the walk, but the structure pass rejects comment-bearing constructs before they reach here.
fn call_head_before(toks: &[Token], open: usize) -> bool {
    let mut k = open;
    while k > 0 && is_trivia(&toks[k - 1]) {
        k -= 1;
    }
    k > 0 && is_callee_ident(&toks[k - 1])
}

/// Build one element/argument: collapsed text, with any nested `{...}` or nested call `f(...)`
/// rendered as its own group so it collapses or explodes independently of its parent.
pub(super) fn build_expr_doc(toks: &[Token]) -> Doc {
    let mut parts: Vec<Doc> = Vec::new();
    let mut text = String::new();
    let mut pending_space = false;
    let mut j = 0usize;
    while j < toks.len() {
        let t = toks[j];
        if is_trivia(&t) {
            if !text.is_empty() || !parts.is_empty() {
                pending_space = true;
            }
            j += 1;
        } else if t.kind == TokenKind::Punct
            && t.text == "{"
            && let Some(close) = match_brace(toks, j)
        {
            if pending_space && !text.is_empty() {
                text.push(' ');
            }
            pending_space = false;
            if !text.is_empty() {
                parts.push(Doc::Text(std::mem::take(&mut text)));
            }
            parts.push(build_brace_doc(&toks[j + 1..close], false));
            j = close + 1;
        } else if t.kind == TokenKind::Punct
            && t.text == "("
            && call_head_before(toks, j)
            && let Some(close) = match_bracket(toks, j)
        {
            // The callee is already in `text`; any trivia between it and `(` is dropped rather
            // than collapsed to a space, so this matches `space_call_heads`'s tight-call spacing
            // and stays a fixpoint across passes (§2.5).
            pending_space = false;
            if !text.is_empty() {
                parts.push(Doc::Text(std::mem::take(&mut text)));
            }
            parts.push(build_call_doc(&toks[j + 1..close]));
            j = close + 1;
        } else {
            if pending_space {
                text.push(' ');
                pending_space = false;
            }
            text.push_str(t.text);
            j += 1;
        }
    }
    if !text.is_empty() {
        parts.push(Doc::Text(text));
    }
    match parts.len() {
        0 => Doc::text(""),
        1 => parts.pop().unwrap(),
        _ => Doc::concat(parts),
    }
}

/// A parenthesized clause group: flat `(a sep b sep c)` or one element per line, with `sep`
/// trailing each element but the last (`;` for a `for` header, ` &&`/` ||` for a condition).
fn build_clause_group(segments: Vec<Doc>, sep: &str) -> Doc {
    if segments.is_empty() {
        return Doc::text("()");
    }
    let last = segments.len() - 1;
    let mut items = vec![Doc::SoftLine];
    for (idx, seg) in segments.into_iter().enumerate() {
        items.push(seg);
        if idx < last {
            items.push(Doc::text(sep.to_owned()));
            items.push(Doc::Line);
        }
    }
    Doc::group(Doc::concat([
        Doc::text("("),
        Doc::nest(Doc::concat(items)),
        Doc::SoftLine,
        Doc::text(")"),
    ]))
}

/// Split `inner` on the depth-zero separators `is_sep` selects, build each segment as its own
/// expression [`Doc`], and lay them out as a [`build_clause_group`] with `sep` trailing all but the
/// last — the shared shape of a ternary chain, a `for` header, and a logical-operator condition.
fn build_clause_doc(inner: &[Token], is_sep: impl Fn(&Token) -> bool, sep: &str) -> Doc {
    if !is_balanced(inner) {
        return Doc::Text(format!("({})", render_segment(inner)));
    }
    let segments = split_top_level(inner, is_sep)
        .iter()
        .map(|s| build_expr_doc(s))
        .collect();
    build_clause_group(segments, sep)
}

/// Build the document for a parenthesized ternary chain: split on the depth-zero `:`, each
/// `cond ? val` segment carrying a trailing ` :` when broken, flat otherwise (§2.4).
pub(super) fn build_ternary_doc(inner: &[Token]) -> Doc {
    build_clause_doc(inner, |t| t.kind == TokenKind::Punct && t.text == ":", " :")
}

/// A segment's text: its non-trivia tokens with runs of whitespace collapsed to one space.
fn render_segment(toks: &[Token]) -> String {
    let mut s = String::new();
    let mut pending_space = false;
    for t in toks {
        if is_trivia(t) {
            if !s.is_empty() {
                pending_space = true;
            }
        } else {
            if pending_space {
                s.push(' ');
                pending_space = false;
            }
            s.push_str(t.text);
        }
    }
    s
}

/// `for (init; cond; step)` — one clause per line when broken (§2.4).
pub(super) fn build_for_doc(inner: &[Token]) -> Doc {
    build_clause_doc(inner, |t| t.kind == TokenKind::Punct && t.text == ";", ";")
}

/// An `if`/`while`/`switch` condition — split on the outermost `&&`/`||` with the operator
/// trailing (§2.7); a condition with no such operator explodes as a single indented element.
pub(super) fn build_cond_doc(inner: &[Token]) -> Doc {
    match top_level_logical_op(inner) {
        Some(op) => build_clause_doc(
            inner,
            |t| t.kind == TokenKind::Operator && t.text == op,
            &format!(" {op}"),
        ),
        None => build_clause_doc(inner, |_| false, ""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tok(kind: TokenKind, text: &'static str) -> Token<'static> {
        Token { kind, text }
    }

    #[test]
    fn render_segment_collapses_whitespace() {
        // Leading/trailing trivia trimmed, inner trivia collapsed to one space.
        let toks = [
            tok(TokenKind::Whitespace, "  "),
            tok(TokenKind::Ident, "a"),
            tok(TokenKind::Whitespace, " "),
            tok(TokenKind::Newline, "\n"),
            tok(TokenKind::Ident, "b"),
            tok(TokenKind::Whitespace, "\t"),
        ];
        assert_eq!(render_segment(&toks), "a b");
    }

    #[test]
    fn render_segment_empty_for_all_trivia() {
        let toks = [
            tok(TokenKind::Whitespace, " "),
            tok(TokenKind::Newline, "\n"),
        ];
        assert_eq!(render_segment(&toks), "");
    }

    #[test]
    fn render_segment_single_token() {
        let toks = [tok(TokenKind::Number, "42")];
        assert_eq!(render_segment(&toks), "42");
    }

    #[test]
    fn build_expr_doc_nested_call_is_a_breakable_group() {
        // A call nested in an expression must render as its own group, not flat text: at a width
        // too narrow for it flat, its args explode one per line.
        use crate::lexer::tokenize;
        let toks = tokenize("bllll(aaaaaaaaaaaaaaaaaaaaaa, bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb)");
        let doc = build_expr_doc(&toks);
        assert_eq!(
            crate::doc::render(&doc, 10, 0, 0),
            "bllll(\n\taaaaaaaaaaaaaaaaaaaaaa,\n\tbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\n)"
        );
    }

    #[test]
    fn build_cond_doc_recursively_explodes_nested_call() {
        // Regression guard for issue #10: an operand that is itself an over-width call must
        // explode its own argument list, not stay flat.
        use crate::lexer::tokenize;
        let toks = tokenize(
            "io_detect_pin() && bllllaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa(aaaaaaaaaaaaaaaaaaaaaa, bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb)",
        );
        let doc = build_cond_doc(&toks);
        let rendered = crate::doc::render(&doc, 40, 0, 0);
        assert_eq!(
            rendered,
            "(\n\tio_detect_pin() &&\n\tbllllaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa(\n\t\taaaaaaaaaaaaaaaaaaaaaa,\n\t\tbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\n\t)\n)"
        );
    }

    #[test]
    fn build_expr_doc_tightens_call_across_a_newline_gap() {
        // Regression guard: a whitespace mutant can put a newline between a callee and its `(`.
        // `is_call_head` (strict adjacency) would miss this, collapsing the gap to a space instead
        // of dropping it — same-line, that space is then tightened by `space_call_heads` on the
        // *next* format pass, changing the output and breaking idempotency (issue found while
        // adding the cond-nested-call-explode fixture).
        use crate::lexer::tokenize;
        let toks = tokenize("io_detect_pin\n( )");
        let doc = build_expr_doc(&toks);
        assert_eq!(crate::doc::render(&doc, 80, 0, 0), "io_detect_pin()");
    }

    #[test]
    fn build_expr_doc_type_keyword_is_not_a_call_head() {
        // `int (*cb)` is a function-pointer declarator, not a call: `int` is a type keyword, which
        // `space_call_heads` always spaces (never tightens), so `call_head_before` must not treat
        // it as one either.
        use crate::lexer::tokenize;
        let toks = tokenize("int (*cb)(void)");
        let doc = build_expr_doc(&toks);
        assert_eq!(crate::doc::render(&doc, 80, 0, 0), "int (*cb)(void)");
    }

    #[test]
    fn build_call_doc_recursively_explodes_nested_call() {
        // A call whose argument is itself an over-width call: both levels must explode.
        use crate::lexer::tokenize;
        let toks = tokenize(
            "first_argument, inner_function_with_a_very_long_name(nested_argument_one, nested_argument_two, nested_argument_three)",
        );
        let doc = build_call_doc(&toks);
        let rendered = crate::doc::render(&doc, 40, 0, 0);
        assert_eq!(
            rendered,
            "(\n\tfirst_argument,\n\tinner_function_with_a_very_long_name(\n\t\tnested_argument_one,\n\t\tnested_argument_two,\n\t\tnested_argument_three\n\t)\n)"
        );
    }
}
