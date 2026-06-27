//! The structuring pass. It reformats the constructs cfmt understands with the §2.2 rule and
//! emits everything else byte-for-byte:
//!
//! * function-call and declaration argument lists (M2), detected by the house rule that a callee
//!   hugs its `(` with no space (`foo(`), which excludes control headers (`if (`) for free;
//! * `{}` initializer lists and `enum` bodies (M3), with the §2.3 magic trailing comma.
//!
//! Anything not confidently one of these is emitted verbatim, so partial understanding never
//! corrupts code. Comments inside a list are deferred to M7: such a list is passed through.

use crate::doc::{Doc, TAB_WIDTH, display_width, render};
use crate::lexer::{Token, TokenKind, tokenize};

const WIDTH: usize = 100;

pub fn format(src: &str) -> String {
    let toks = tokenize(src);
    let mut out = String::new();
    let mut col = 0usize;
    let mut i = 0usize;
    let mut paren_depth = 0i32;
    let mut in_init = false;
    while i < toks.len() {
        let t = toks[i];

        if t.kind == TokenKind::Punct && t.text == "#" && current_line_is_blank(&out) {
            i = emit_directive(&toks, i, &mut out, &mut col);
            continue;
        }

        if t.kind == TokenKind::Ident
            && matches!(t.text, "if" | "while" | "switch" | "for")
            && let Some(open) = next_paren(&toks, i)
            && let Some(close) = match_bracket(&toks, open)
        {
            for tok in &toks[i..open] {
                emit_str(&mut out, &mut col, tok.text);
            }
            let inner = &toks[open + 1..close];
            let doc = if t.text == "for" {
                build_for_doc(inner)
            } else {
                build_cond_doc(inner)
            };
            let base_level = current_line_indent_cols(&out) / TAB_WIDTH;
            let reserved = trailing_reserved(&toks, close + 1);
            let rendered = render(&doc, WIDTH.saturating_sub(reserved), col, base_level);
            emit_str(&mut out, &mut col, &rendered);
            i = close + 1;
            continue;
        }

        if t.kind == TokenKind::Ident
            && t.text == "enum"
            && let Some(brace) = enum_body_brace(&toks, i)
        {
            for tok in &toks[i..brace] {
                emit_str(&mut out, &mut col, tok.text);
            }
            i = emit_brace(&toks, brace, true, &mut out, &mut col);
            continue;
        }

        if is_call_head(&toks, i)
            && let Some(close) = match_bracket(&toks, i + 1)
        {
            emit_str(&mut out, &mut col, t.text);
            let doc = build_call_doc(&toks[i + 2..close]);
            let base_level = current_line_indent_cols(&out) / TAB_WIDTH;
            let reserved = trailing_reserved(&toks, close + 1);
            let rendered = render(&doc, WIDTH.saturating_sub(reserved), col, base_level);
            emit_str(&mut out, &mut col, &rendered);
            i = close + 1;
            continue;
        }

        // An initializer brace: in an `= ... ;` region, a `{` that is not a statement-expression
        // `({ ... })` (whose `{` is hugged by `(`).
        if in_init
            && t.kind == TokenKind::Punct
            && t.text == "{"
            && last_nonspace_char(&out) != Some('(')
            && match_brace(&toks, i).is_some()
        {
            i = emit_brace(&toks, i, false, &mut out, &mut col);
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

/// Format the `{...}` opening at `open` (an initializer when `padded` is false, an enum body when
/// true) and return the index just past its `}`. Falls back to verbatim if the braces are
/// unbalanced or the list contains a comment or directive (deferred to M7).
fn emit_brace(
    toks: &[Token],
    open: usize,
    padded: bool,
    out: &mut String,
    col: &mut usize,
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
    if has_comment_or_directive {
        for tok in &toks[open..=close] {
            emit_str(out, col, tok.text);
        }
        return close + 1;
    }
    let base_level = current_line_indent_cols(out) / TAB_WIDTH;
    let reserved = trailing_reserved(toks, close + 1);
    let doc = build_brace_doc(inner, padded);
    let rendered = render(&doc, WIDTH.saturating_sub(reserved), *col, base_level);
    emit_str(out, col, &rendered);
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

/// An identifier immediately followed by `(` (no intervening whitespace) — a call or the
/// structurally identical declaration parameter list, excluding control/operator keywords.
fn is_call_head(toks: &[Token], i: usize) -> bool {
    toks[i].kind == TokenKind::Ident
        && !is_excluded_callee(toks[i].text)
        && toks
            .get(i + 1)
            .is_some_and(|n| n.kind == TokenKind::Punct && n.text == "(")
}

/// Keywords that take a `(` but are not calls whose arguments split on commas.
fn is_excluded_callee(name: &str) -> bool {
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
            | "_Generic"
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
fn enum_body_brace(toks: &[Token], i: usize) -> Option<usize> {
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

/// Index of the `)`/`]` matching the bracket at `open`, or `None` if unbalanced.
fn match_bracket(toks: &[Token], open: usize) -> Option<usize> {
    matching(toks, open, "(", ")").or_else(|| matching(toks, open, "[", "]"))
}

/// Index of the `}` matching the `{` at `open`, or `None` if unbalanced.
fn match_brace(toks: &[Token], open: usize) -> Option<usize> {
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
fn split_top_level<'a, 'src>(
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
fn split_on_commas<'a, 'src>(inner: &'a [Token<'src>]) -> Vec<&'a [Token<'src>]> {
    split_top_level(inner, |t| t.kind == TokenKind::Punct && t.text == ",")
}

/// The outermost logical operator present at bracket depth zero (`||` outranks `&&`), if any.
fn top_level_logical_op(inner: &[Token]) -> Option<&'static str> {
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

/// The `(` that follows control keyword `i` after only trivia, or `None`.
fn next_paren(toks: &[Token], i: usize) -> Option<usize> {
    for (j, t) in toks.iter().enumerate().skip(i + 1) {
        match t.kind {
            TokenKind::Whitespace | TokenKind::Newline => {}
            TokenKind::Punct if t.text == "(" => return Some(j),
            _ => return None,
        }
    }
    None
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

/// `for (init; cond; step)` — one clause per line when broken (§2.4).
fn build_for_doc(inner: &[Token]) -> Doc {
    let clauses = split_top_level(inner, |t| t.kind == TokenKind::Punct && t.text == ";")
        .iter()
        .map(|c| render_segment(c))
        .collect();
    build_clause_group(clauses, ";")
}

/// An `if`/`while`/`switch` condition — split on the outermost `&&`/`||` with the operator
/// trailing (§2.7); a condition with no such operator explodes as a single indented element.
fn build_cond_doc(inner: &[Token]) -> Doc {
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

fn is_trivia(t: &Token) -> bool {
    matches!(t.kind, TokenKind::Whitespace | TokenKind::Newline)
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

/// Build the §2.2 document for a call's argument list, including the surrounding parens.
fn build_call_doc(inner: &[Token]) -> Doc {
    let segments: Vec<String> = split_on_commas(inner)
        .iter()
        .map(|a| render_segment(a))
        .filter(|s| !s.is_empty())
        .collect();
    if segments.is_empty() {
        return Doc::text("()");
    }
    let mut items = vec![Doc::SoftLine];
    for (idx, seg) in segments.into_iter().enumerate() {
        if idx > 0 {
            items.push(Doc::text(","));
            items.push(Doc::Line);
        }
        items.push(Doc::Text(seg));
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
fn build_brace_doc(inner: &[Token], padded: bool) -> Doc {
    let segments = split_on_commas(inner);
    let magic = segments.len() > 1 && segments.last().is_some_and(|s| s.iter().all(is_trivia));
    let elements: Vec<&[Token]> = segments
        .into_iter()
        .filter(|s| s.iter().any(|t| !is_trivia(t)))
        .collect();
    if elements.is_empty() {
        return Doc::text("{}");
    }
    let pad = || {
        if padded { Doc::Line } else { Doc::SoftLine }
    };
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

/// Columns consumed by structural tokens trailing the construct on its line (e.g. `;` or ` {`), so
/// the group leaves room for them. Comments are ignored so a trailing comment never forces a break.
fn trailing_reserved(toks: &[Token], from: usize) -> usize {
    let mut w = 0;
    for t in toks.iter().skip(from) {
        match t.kind {
            TokenKind::Newline => break,
            TokenKind::LineComment | TokenKind::BlockComment => {}
            _ => w += display_width(t.text),
        }
    }
    w
}

/// Emit a preprocessor directive verbatim, following `\` line continuations; returns the index
/// just past it.
fn emit_directive(toks: &[Token], start: usize, out: &mut String, col: &mut usize) -> usize {
    let mut i = start;
    while i < toks.len() {
        let t = toks[i];
        emit_str(out, col, t.text);
        if t.kind == TokenKind::Newline {
            let continued = i > 0 && toks[i - 1].text == "\\";
            i += 1;
            if !continued {
                return i;
            }
        } else {
            i += 1;
        }
    }
    i
}
