//! Wadler/Leijen `Doc` builders for the constructs jphfmt lays out: call argument lists, `{}` and
//! `enum` bodies, `for`/condition clause groups, and parenthesized ternary chains. Each builder
//! turns a token slice into a [`Doc`] that [`crate::doc::render`] later flattens or fully breaks per
//! §2.2. Depends on [`super::tokens`] for depth-aware splitting and balance checks.

use super::tokens::{
    is_balanced, is_trivia, match_brace, split_on_commas, split_top_level, top_level_logical_op,
};
use crate::doc::Doc;
use crate::lexer::{Token, TokenKind};

/// Build the §2.2 document for a call's argument list, including the surrounding parens. Each
/// argument is built recursively (via [`build_element_doc`]), so a compound-literal argument's
/// `{...}` is a nested group that can collapse or explode on its own.
pub(super) fn build_call_doc(inner: &[Token]) -> Doc {
    if !is_balanced(inner) {
        return Doc::Text(format!("({})", render_segment(inner)));
    }
    let args: Vec<&[Token]> = split_on_commas(inner)
        .into_iter()
        .filter(|a| a.iter().any(|t| !is_trivia(t)))
        .collect();
    if args.is_empty() {
        return Doc::text("()");
    }
    let last = args.len() - 1;
    let mut items = vec![Doc::SoftLine];
    for (idx, arg) in args.into_iter().enumerate() {
        items.push(build_element_doc(arg));
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
    let elements: Vec<&[Token]> = segments
        .into_iter()
        .filter(|s| s.iter().any(|t| !is_trivia(t)))
        .collect();
    if elements.is_empty() {
        return Doc::text("{}");
    }
    let pad = || if padded { Doc::Line } else { Doc::SoftLine };
    let last = elements.len() - 1;
    let mut items = vec![pad()];
    for (idx, element) in elements.into_iter().enumerate() {
        items.push(build_element_doc(element));
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

/// Build one `{}` element: collapsed text, with any nested `{}` rendered as its own (tight) list so
/// it collapses or explodes on its own.
fn build_element_doc(toks: &[Token]) -> Doc {
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
fn build_clause_group(segments: Vec<String>, sep: &str) -> Doc {
    if segments.is_empty() {
        return Doc::text("()");
    }
    let last = segments.len() - 1;
    let mut items = vec![Doc::SoftLine];
    for (idx, seg) in segments.into_iter().enumerate() {
        items.push(Doc::Text(seg));
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

/// Build the document for a parenthesized ternary chain: split on the depth-zero `:`, each
/// `cond ? val` segment carrying a trailing ` :` when broken, flat otherwise (§2.4).
pub(super) fn build_ternary_doc(inner: &[Token]) -> Doc {
    let segments = split_top_level(inner, |t| t.kind == TokenKind::Punct && t.text == ":")
        .iter()
        .map(|s| render_segment(s))
        .collect();
    build_clause_group(segments, " :")
}

/// A segment's text: its non-trivia tokens with runs of whitespace collapsed to one space.
pub(super) fn render_segment(toks: &[Token]) -> String {
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
    let clauses = split_top_level(inner, |t| t.kind == TokenKind::Punct && t.text == ";")
        .iter()
        .map(|c| render_segment(c))
        .collect();
    build_clause_group(clauses, ";")
}

/// An `if`/`while`/`switch` condition — split on the outermost `&&`/`||` with the operator
/// trailing (§2.7); a condition with no such operator explodes as a single indented element.
pub(super) fn build_cond_doc(inner: &[Token]) -> Doc {
    match top_level_logical_op(inner) {
        Some(op) => {
            let operands =
                split_top_level(inner, |t| t.kind == TokenKind::Operator && t.text == op)
                    .iter()
                    .map(|o| render_segment(o))
                    .collect();
            build_clause_group(operands, &format!(" {op}"))
        }
        None => build_clause_group(vec![render_segment(inner)], ""),
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
}
