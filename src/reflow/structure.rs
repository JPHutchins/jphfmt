//! The structuring pass: a single left-to-right walk over the token stream that reformats the
//! constructs jphfmt understands (call/declaration lists, `{}`/`enum` bodies, control headers,
//! parenthesized ternaries, `#define` bodies, GNU statement-expressions, function bodies) and
//! emits everything else byte-for-byte. Output is built into a [`String`] with a tracked display
//! column; [`emit_str`] is the single mutator. Pure helpers for column accounting and trailing-token
//! reservation live alongside.

use super::builders::{
    build_brace_doc, build_call_doc, build_cond_doc, build_for_doc, build_ternary_doc,
    render_segment,
};
use super::tokens::{
    contains_comment, directive_end, enum_body_brace, has_middle_newline, has_top_level_question,
    is_balanced, is_call_head, is_excluded_callee, is_trivia, match_brace, match_bracket,
    next_nontrivia, next_nontrivia_in, next_paren, split_top_level,
};
use crate::doc::{TAB_WIDTH, display_width, render};
use crate::lexer::{Token, TokenKind};

/// Run the structuring pass over `toks`, with the cursor starting at `start_col` (non-zero when
/// formatting a fragment such as a macro body that follows a prefix).
pub(super) fn structure(toks: &[Token], start_col: usize, width: usize) -> String {
    let mut out = String::new();
    let mut col = start_col;
    let mut i = 0usize;
    let mut paren_depth = 0i32;
    let mut in_init = false;
    let mut pending_func_def = false;
    while i < toks.len() {
        let t = toks[i];

        if t.kind == TokenKind::Punct && t.text == "#" && current_line_is_blank(&out) {
            let is_define = next_nontrivia(toks, i + 1)
                .is_some_and(|j| toks[j].kind == TokenKind::Ident && toks[j].text == "define");
            i = if is_define {
                emit_define(toks, i, &mut out, &mut col, width)
            } else {
                emit_directive(toks, i, &mut out, &mut col)
            };
            continue;
        }

        if t.kind == TokenKind::Ident
            && matches!(t.text, "if" | "while" | "switch" | "for")
            && let Some(open) = next_paren(toks, i)
            && let Some(close) = match_bracket(toks, open)
            && !contains_comment(&toks[open + 1..close])
            && is_balanced(&toks[open + 1..close])
        {
            // §2.5: control keywords take exactly one space before `(` (`if (`, not `if(`).
            emit_str(&mut out, &mut col, t.text);
            emit_str(&mut out, &mut col, " ");
            let inner = &toks[open + 1..close];
            let doc = if t.text == "for" {
                build_for_doc(inner)
            } else {
                build_cond_doc(inner)
            };
            let base_level = current_line_indent_cols(&out) / TAB_WIDTH;
            let reserved = trailing_reserved(toks, close + 1);
            let rendered = render(&doc, width.saturating_sub(reserved), col, base_level);
            emit_str(&mut out, &mut col, &rendered);
            i = close + 1;
            continue;
        }

        if t.kind == TokenKind::Ident
            && t.text == "enum"
            && let Some(brace) = enum_body_brace(toks, i)
        {
            for tok in &toks[i..brace] {
                emit_str(&mut out, &mut col, tok.text);
            }
            i = emit_brace(toks, brace, true, &mut out, &mut col, width);
            continue;
        }

        if is_call_head(toks, i)
            && let Some(close) = match_bracket(toks, i + 1)
            && !contains_comment(&toks[i + 2..close])
            && is_balanced(&toks[i + 2..close])
            && !has_middle_newline(&toks[i + 2..close])
        {
            emit_str(&mut out, &mut col, t.text);
            let doc = build_call_doc(&toks[i + 2..close]);
            let base_level = current_line_indent_cols(&out) / TAB_WIDTH;
            let reserved = trailing_reserved(toks, close + 1);
            let rendered = render(&doc, width.saturating_sub(reserved), col, base_level);
            emit_str(&mut out, &mut col, &rendered);
            pending_func_def = next_nontrivia(toks, close + 1).is_some_and(|j| toks[j].text == "{");
            i = close + 1;
            continue;
        }

        // GNU statement-expression `({ ... })` — block-indent its statements.
        if t.kind == TokenKind::Punct
            && t.text == "("
            && toks
                .get(i + 1)
                .is_some_and(|n| n.kind == TokenKind::Punct && n.text == "{")
        {
            let base_level = current_line_indent_cols(&out) / TAB_WIDTH;
            if let Some((block, next)) = format_stmt_expr(toks, i, base_level) {
                emit_str(&mut out, &mut col, &block);
                i = next;
                continue;
            }
        }

        // A parenthesized ternary `( ... ? ... : ... )` — flat chain, each `cond ? val :` on its
        // own line with the colon trailing (§2.4). Parens are author-written (§8.2), not inserted.
        // Skip `(` that are part of a function call (`ident(`): the call handler (§2.2) already
        // tried and fell through (typically due to `has_middle_newline`), so the ternary handler
        // would accidentally reformat the call's argument list, collapsing whitespace and losing
        // empty leading arguments. Let it passthrough instead.
        if t.kind == TokenKind::Punct
            && t.text == "("
            && let Some(close) = match_bracket(toks, i)
            && has_top_level_question(&toks[i + 1..close])
            && !contains_comment(&toks[i + 1..close])
            && is_balanced(&toks[i + 1..close])
            && !(i > 0
                && toks[i - 1].kind == TokenKind::Ident
                && !is_excluded_callee(toks[i - 1].text))
        {
            let doc = build_ternary_doc(&toks[i + 1..close]);
            let base_level = current_line_indent_cols(&out) / TAB_WIDTH;
            let reserved = trailing_reserved(toks, close + 1);
            let rendered = render(&doc, width.saturating_sub(reserved), col, base_level);
            emit_str(&mut out, &mut col, &rendered);
            i = close + 1;
            continue;
        }

        // Function definition body: `{` after `)` from a function/macro definition. Always break
        // with one statement per line, body indented, `}` at the definition's own indent level.
        if t.kind == TokenKind::Punct && t.text == "{" && pending_func_def {
            pending_func_def = false;
            i = emit_func_body(toks, i, &mut out, &mut col, width);
            continue;
        }

        // An initializer brace: in an `= ... ;` region, a `{` that is not a statement-expression.
        if in_init
            && t.kind == TokenKind::Punct
            && t.text == "{"
            && last_nonspace_char(&out) != Some('(')
            && match_brace(toks, i).is_some()
        {
            i = emit_brace(toks, i, false, &mut out, &mut col, width);
            continue;
        }

        if t.kind == TokenKind::Punct {
            match t.text {
                "(" | "[" => paren_depth += 1,
                ")" | "]" => paren_depth = (paren_depth - 1).max(0),
                "=" if paren_depth == 0 => in_init = true,
                ";" if paren_depth == 0 => in_init = false,
                _ => {}
            }
        }
        emit_str(&mut out, &mut col, t.text);
        i += 1;
    }
    out
}

/// Format a `#define`: a function-like macro whose body is a single call/`_Generic` or a
/// statement-expression is laid out with the body opening on the `#define` line and `\`
/// continuations one space after each line; any other body is emitted verbatim.
fn emit_define(
    toks: &[Token],
    start: usize,
    out: &mut String,
    col: &mut usize,
    width: usize,
) -> usize {
    let end = directive_end(toks, start);
    if let Some((prefix, body)) = split_define(toks, start, end)
        && let Some(body_str) = format_define_body(&body, display_width(&prefix), width)
    {
        let full = format!("{prefix}{body_str}");
        let continued = full.replace('\n', " \\\n");
        emit_str(out, col, &continued);
        emit_str(out, col, "\n");
        return end;
    }
    for tok in &toks[start..end] {
        emit_str(out, col, tok.text);
    }
    end
}

/// Split a `#define` into its `#define NAME(params) ` prefix text and its body tokens (with
/// continuation backslashes removed and surrounding trivia trimmed). `None` if it has no body.
fn split_define<'src>(
    toks: &[Token<'src>],
    start: usize,
    end: usize,
) -> Option<(String, Vec<Token<'src>>)> {
    let define = next_nontrivia_in(toks, start + 1, end)?;
    let name = next_nontrivia_in(toks, define + 1, end)?;
    let prefix_end = match toks.get(name + 1) {
        Some(n) if n.kind == TokenKind::Punct && n.text == "(" => {
            match_bracket(toks, name + 1)? + 1
        }
        _ => name + 1,
    };
    let prefix: String = toks[start..prefix_end]
        .iter()
        .map(|t| t.text)
        .collect::<String>()
        + " ";
    let mut body: Vec<Token> = toks[prefix_end..end]
        .iter()
        .filter(|t| !(t.kind == TokenKind::Punct && t.text == "\\"))
        .copied()
        .collect();
    while body.first().is_some_and(is_trivia) {
        body.remove(0);
    }
    while body.last().is_some_and(is_trivia) {
        body.pop();
    }
    if body.is_empty() {
        return None;
    }
    Some((prefix, body))
}

/// Format a macro body if it is a single call/`_Generic` or a statement-expression; else `None`.
fn format_define_body(body: &[Token], prefix_col: usize, width: usize) -> Option<String> {
    if contains_comment(body) {
        return None;
    }
    if is_call_head(body, 0) && match_bracket(body, 1) == Some(body.len() - 1) {
        return Some(structure(body, prefix_col, width));
    }
    if body.len() >= 2
        && body[0].kind == TokenKind::Punct
        && body[0].text == "("
        && body[1].kind == TokenKind::Punct
        && body[1].text == "{"
    {
        return format_stmt_expr(body, 0, 0).map(|(s, _)| s);
    }
    None
}

/// Format a `({ ... })` statement-expression: `({` opens the line, each statement on its own line
/// at `base_level + 1`, `})` at `base_level`. Returns the block and the index past the `)`, or
/// `None` if the braces are unbalanced or a statement nests a block or carries a comment.
fn format_stmt_expr(toks: &[Token], open: usize, base_level: usize) -> Option<(String, usize)> {
    let paren_close = match_bracket(toks, open)?;
    let brace_close = match_brace(toks, open + 1)?;
    let inner = &toks[open + 2..brace_close];
    let unformattable = inner.iter().any(|t| {
        (t.kind == TokenKind::Punct && t.text == "{")
            || matches!(t.kind, TokenKind::LineComment | TokenKind::BlockComment)
    });
    if unformattable || !is_balanced(inner) {
        return None;
    }
    let statements: Vec<String> =
        split_top_level(inner, |t| t.kind == TokenKind::Punct && t.text == ";")
            .iter()
            .map(|s| render_segment(s))
            .filter(|s| !s.is_empty())
            .collect();
    if statements.is_empty() {
        return None;
    }
    let inner_indent = "\t".repeat(base_level + 1);
    let close_indent = "\t".repeat(base_level);
    let mut s = String::from("({");
    for statement in &statements {
        s.push('\n');
        s.push_str(&inner_indent);
        s.push_str(statement);
        s.push(';');
    }
    s.push('\n');
    s.push_str(&close_indent);
    s.push_str("})");
    Some((s, paren_close + 1))
}

/// Format the `{...}` opening at `open` (an initializer when `padded` is false, an enum body when
/// true) and return the index just past its `}`. Falls back to verbatim if the braces are
/// unbalanced or the list contains a comment or directive (deferred to M7).
fn emit_brace(
    toks: &[Token],
    open: usize,
    padded: bool,
    out: &mut String,
    col: &mut usize,
    width: usize,
) -> usize {
    let Some(close) = match_brace(toks, open) else {
        emit_str(out, col, toks[open].text);
        return open + 1;
    };
    let inner = &toks[open + 1..close];
    let has_comment_or_directive = inner.iter().any(|t| {
        matches!(t.kind, TokenKind::LineComment | TokenKind::BlockComment)
            || (t.kind == TokenKind::Punct && t.text == "#")
    });
    if has_comment_or_directive || !is_balanced(inner) {
        for tok in &toks[open..=close] {
            emit_str(out, col, tok.text);
        }
        return close + 1;
    }
    let base_level = current_line_indent_cols(out) / TAB_WIDTH;
    let reserved = trailing_reserved(toks, close + 1);
    let doc = build_brace_doc(inner, padded);
    let rendered = render(&doc, width.saturating_sub(reserved), *col, base_level);
    emit_str(out, col, &rendered);
    close + 1
}

/// Format a function definition body: always break with `{\n\tstatements\n}`. Preserves blank
/// lines within the body (they survive `retab` normalization). Falls back to verbatim for bodies
/// with comments, directives, or nested braces (same M7/M8 policy as [`emit_brace`]).
fn emit_func_body(
    toks: &[Token],
    open: usize,
    out: &mut String,
    col: &mut usize,
    _width: usize,
) -> usize {
    let Some(close) = match_brace(toks, open) else {
        emit_str(out, col, toks[open].text);
        return open + 1;
    };
    let inner = &toks[open + 1..close];
    let unformattable = inner.iter().any(|t| {
        matches!(t.kind, TokenKind::LineComment | TokenKind::BlockComment)
            || (t.kind == TokenKind::Punct && matches!(t.text, "#" | "{"))
    });
    if unformattable || !is_balanced(inner) {
        emit_str(out, col, toks[open].text);
        for tok in &toks[open + 1..=close] {
            emit_str(out, col, tok.text);
        }
        return close + 1;
    }

    let base_level = current_line_indent_cols(out) / TAB_WIDTH;
    let inner_indent = "\t".repeat(base_level + 1);
    let close_indent = "\t".repeat(base_level);

    // The space from `space_braces` is already in the token stream before `{`.
    emit_str(out, col, "{");

    // Strip leading and trailing trivia from body tokens, then emit verbatim.
    // `retab` at the end normalizes indentation; blank lines are naturally preserved.
    let start = inner
        .iter()
        .position(|t| !is_trivia(t))
        .unwrap_or(inner.len());
    let end = inner
        .iter()
        .rposition(|t| !is_trivia(t))
        .map_or(0, |p| p + 1);

    if start < end {
        let body_core = &inner[start..end];
        emit_str(out, col, "\n");
        emit_str(out, col, &inner_indent);
        for tok in body_core {
            emit_str(out, col, tok.text);
        }
        emit_str(out, col, "\n");
        emit_str(out, col, &close_indent);
        emit_str(out, col, "}");
    } else {
        // Empty body — keep `{}` inline
        emit_str(out, col, "}");
    }

    close + 1
}

/// Append `s` to `out`, tracking the display column (tabs count as [`TAB_WIDTH`]).
fn emit_str(out: &mut String, col: &mut usize, s: &str) {
    for ch in s.chars() {
        match ch {
            '\n' => {
                out.push('\n');
                *col = 0;
            }
            '\t' => {
                out.push('\t');
                *col += TAB_WIDTH;
            }
            c => {
                out.push(c);
                *col += 1;
            }
        }
    }
}

/// True when nothing but whitespace has been emitted on the current output line — so a `#` here
/// begins a preprocessor directive.
fn current_line_is_blank(out: &str) -> bool {
    out.rsplit('\n')
        .next()
        .is_none_or(|line| line.chars().all(|c| c == ' ' || c == '\t'))
}

/// Indentation, in columns, of the current output line.
fn current_line_indent_cols(out: &str) -> usize {
    let line = out.rsplit('\n').next().unwrap_or(out);
    let mut cols = 0;
    for ch in line.chars() {
        match ch {
            '\t' => cols += TAB_WIDTH,
            ' ' => cols += 1,
            _ => break,
        }
    }
    cols
}

/// The last non-whitespace character emitted, used to tell a compound literal `){` from a
/// statement-expression `({`.
fn last_nonspace_char(out: &str) -> Option<char> {
    out.chars().rev().find(|c| !c.is_whitespace())
}

/// Columns consumed by structural tokens trailing the construct on its line (e.g. `;` or ` {`), so
/// the group leaves room for them. Counting stops after the first bracket-opener, because anything
/// past it can itself break onto later lines — making the measure stable across passes (a chained
/// `f(x)->g(...)` reserves only `->g(`, not `g`'s arguments), which keeps formatting idempotent.
/// Comments are ignored so a trailing comment never forces a break.
fn trailing_reserved(toks: &[Token], from: usize) -> usize {
    let mut w = 0;
    for t in toks.iter().skip(from) {
        match t.kind {
            TokenKind::Newline => break,
            TokenKind::LineComment | TokenKind::BlockComment => {}
            TokenKind::Punct if matches!(t.text, "(" | "[" | "{") => {
                w += col_width(t.text);
                break;
            }
            _ => {
                // Stop at the first newline embedded in any token (not just Newline tokens),
                // so Unknown tokens containing multiple lines don't inflate the reserve.
                if let Some(nl) = t.text.find('\n') {
                    w += col_width(&t.text[..nl]);
                    break;
                }
                w += col_width(t.text);
            }
        }
    }
    w
}

/// Column width of raw token text, counting a tab as [`TAB_WIDTH`] (unlike [`display_width`], which
/// assumes tab-free text). Used where the measured slice may contain a mid-line whitespace tab, so
/// the reserve matches the cursor's own tab accounting and formatting stays idempotent.
fn col_width(s: &str) -> usize {
    s.chars()
        .map(|c| if c == '\t' { TAB_WIDTH } else { 1 })
        .sum()
}

/// Emit a preprocessor directive verbatim, following `\` line continuations; returns the index
/// just past it.
fn emit_directive(toks: &[Token], start: usize, out: &mut String, col: &mut usize) -> usize {
    let end = directive_end(toks, start);
    for tok in &toks[start..end] {
        emit_str(out, col, tok.text);
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::TokenKind;

    fn tok(kind: TokenKind, text: &'static str) -> Token<'static> {
        Token { kind, text }
    }

    #[test]
    fn current_line_is_blank_empty_string() {
        assert!(current_line_is_blank(""));
    }

    #[test]
    fn current_line_is_blank_whitespace_only() {
        assert!(current_line_is_blank("  	"));
    }

    #[test]
    fn current_line_is_blank_content() {
        assert!(!current_line_is_blank("x"));
    }

    #[test]
    fn current_line_is_blank_after_newline_content() {
        assert!(!current_line_is_blank("a\nb"));
    }

    #[test]
    fn current_line_is_blank_after_newline_whitespace() {
        assert!(current_line_is_blank("a\n  "));
    }

    #[test]
    fn last_nonspace_char_empty() {
        assert_eq!(last_nonspace_char(""), None);
    }

    #[test]
    fn last_nonspace_char_single() {
        assert_eq!(last_nonspace_char("x"), Some('x'));
    }

    #[test]
    fn last_nonspace_char_trailing_space() {
        assert_eq!(last_nonspace_char("x "), Some('x'));
    }

    #[test]
    fn last_nonspace_char_multi_word() {
        assert_eq!(last_nonspace_char("a b "), Some('b'));
    }

    #[test]
    fn last_nonspace_char_with_newline() {
        assert_eq!(last_nonspace_char("x\ny"), Some('y'));
    }

    #[test]
    fn col_width_plain() {
        assert_eq!(col_width("abc"), 3);
    }

    #[test]
    fn col_width_tab_counts_as_tab_width() {
        // `a` (1) + tab (TAB_WIDTH=4) + `b` (1) = 6
        assert_eq!(col_width("a\tb"), 1 + TAB_WIDTH + 1);
    }

    #[test]
    fn trailing_reserved_stops_at_newline() {
        let toks = [tok(TokenKind::Newline, "\n"), tok(TokenKind::Punct, ";")];
        assert_eq!(trailing_reserved(&toks, 0), 0);
    }

    #[test]
    fn trailing_reserved_counts_punct_then_stops_at_bracket() {
        // `;` counts (1), then `(` opens a bracket and stops the reserve.
        let toks = [tok(TokenKind::Punct, ";"), tok(TokenKind::Punct, "(")];
        assert_eq!(trailing_reserved(&toks, 0), 2);
    }

    #[test]
    fn trailing_reserved_ignores_comments() {
        let toks = [
            tok(TokenKind::LineComment, "// hi"),
            tok(TokenKind::Punct, ";"),
        ];
        assert_eq!(trailing_reserved(&toks, 0), 1);
    }
}
