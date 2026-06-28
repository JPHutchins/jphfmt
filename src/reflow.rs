//! The structuring pass. It reformats the constructs cfmt understands with the §2.2 rule and
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

use crate::doc::{Doc, TAB_WIDTH, display_width, render};
use crate::lexer::{Token, TokenKind, tokenize};

/// Default column limit (§8.5).
pub const DEFAULT_WIDTH: usize = 100;

pub fn format(src: &str) -> String {
    format_with_width(src, DEFAULT_WIDTH)
}

/// Format with an explicit column limit. Tab width for the overflow measurement is fixed at
/// [`TAB_WIDTH`] (§8.5 default).
pub fn format_with_width(src: &str, width: usize) -> String {
    normalize_endings(&space_tokens(&retab(&structure(&tokenize(src), 0, width))))
}

/// A C type keyword or qualifier — a token after which a `*` is confidently a pointer declarator,
/// not a multiply. User typedefs (idents) are excluded, so ambiguous `a*b`/`foo*p` pass through
/// (§2.5 pointers, §6 "prefer passthrough when ambiguous").
fn is_type_context(text: &str) -> bool {
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

/// A significant token paired with the whitespace that preceded it.
type Piece<'src> = (String, Token<'src>);

fn same_line(gap: &str) -> bool {
    !gap.contains('\n')
}

/// Index of the `(` matching the `)` at `close`, scanning the piece list backward.
fn piece_open_paren(pieces: &[Piece], close: usize) -> Option<usize> {
    let mut depth = 0i32;
    for j in (0..=close).rev() {
        match pieces[j].1.text {
            ")" => depth += 1,
            "(" => {
                depth -= 1;
                if depth == 0 {
                    return Some(j);
                }
            }
            _ => {}
        }
    }
    None
}

/// Index of the `)` matching the `(` at `open`, scanning forward.
fn piece_close_paren(pieces: &[Piece], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (j, p) in pieces.iter().enumerate().skip(open) {
        match p.1.text {
            "(" => depth += 1,
            ")" => {
                depth -= 1;
                if depth == 0 {
                    return Some(j);
                }
            }
            _ => {}
        }
    }
    None
}

/// Apply the §2.5 token-spacing rules. Whitespace is semantically inert, so this never changes
/// meaning; only the listed pairs are touched and everything else keeps its exact spacing.
fn space_tokens(s: &str) -> String {
    let mut pieces: Vec<Piece> = Vec::new();
    let mut gap = String::new();
    for t in tokenize(s) {
        if is_trivia(&t) {
            gap.push_str(t.text);
        } else {
            pieces.push((std::mem::take(&mut gap), t));
        }
    }
    let trailing = gap;

    space_pointers(&mut pieces);
    space_casts(&mut pieces);
    space_braces(&mut pieces);
    space_bit_fields(&mut pieces);

    let mut out = String::with_capacity(s.len());
    for (g, t) in &pieces {
        out.push_str(g);
        out.push_str(t.text);
    }
    out.push_str(&trailing);
    out
}

/// Middle-align pointer `*` (§2.5: `T * p`, `T ** p`). A `*` cluster is a pointer only when
/// preceded by a type keyword/qualifier or a `struct`/`union`/`enum` tag; multiply, deref,
/// function pointers `(*f)`, and bare-typedef pointers are left as is (§6).
fn space_pointers(pieces: &mut [Piece]) {
    let is_star = |t: &Token| t.kind == TokenKind::Punct && t.text == "*";
    let mut j = 0;
    while j < pieces.len() {
        let prev_is_type = j > 0
            && (is_type_context(pieces[j - 1].1.text)
                || (pieces[j - 1].1.kind == TokenKind::Ident
                    && j >= 2
                    && matches!(pieces[j - 2].1.text, "struct" | "union" | "enum")));
        if is_star(&pieces[j].1) && prev_is_type && same_line(&pieces[j].0) {
            let mut k = j;
            while k + 1 < pieces.len() && is_star(&pieces[k + 1].1) && same_line(&pieces[k + 1].0) {
                k += 1;
            }
            pieces[j].0 = " ".to_owned();
            for piece in &mut pieces[j + 1..=k] {
                piece.0.clear();
            }
            if let Some(after) = pieces.get_mut(k + 1)
                && same_line(&after.0)
            {
                after.0 = if after.1.kind == TokenKind::Ident {
                    " ".to_owned()
                } else {
                    String::new()
                };
            }
            j = k + 1;
            continue;
        }
        j += 1;
    }
}

/// A C-style cast `(type) x` gets a space after the `)` (§2.5). Conservative: the parenthesized
/// group must be type-only and contain a type keyword (so a grouped expression is never mistaken
/// for one), be in a non-value position, and be followed by an operand.
fn space_casts(pieces: &mut [Piece]) {
    let value_start = |t: &Token| {
        matches!(
            t.kind,
            TokenKind::Ident | TokenKind::Number | TokenKind::String | TokenKind::Char
        ) || (t.kind == TokenKind::Punct
            && matches!(t.text, "(" | "*" | "&" | "!" | "~" | "-" | "+"))
    };
    for open in 0..pieces.len() {
        if pieces[open].1.text != "(" {
            continue;
        }
        let Some(close) = piece_close_paren(pieces, open) else {
            continue;
        };
        let inner = &pieces[open + 1..close];
        let type_only = inner.iter().all(|p| {
            p.1.kind == TokenKind::Ident
                || matches!(p.1.text, "*" | "[" | "]")
                || p.1.kind == TokenKind::Number
        });
        let has_type = inner
            .iter()
            .any(|p| is_type_context(p.1.text) || matches!(p.1.text, "struct" | "union" | "enum"));
        let prev_is_value = open > 0
            && (pieces[open - 1].1.kind == TokenKind::Ident
                || matches!(pieces[open - 1].1.text, ")" | "]"));
        let followed_by_operand = pieces
            .get(close + 1)
            .is_some_and(|after| same_line(&after.0) && value_start(&after.1));
        if type_only && has_type && !prev_is_value && followed_by_operand {
            pieces[close + 1].0 = " ".to_owned();
        }
    }
}

/// K&R brace attach: `) {` keeps one space (§2.5) for function and control bodies, but the tight
/// `({` statement-expression and `(type){...}` compound literal are left alone. The matching `(`
/// follows an identifier for the former and an operator/`&`/`=` for the latter, which decides it.
fn space_braces(pieces: &mut [Piece]) {
    for j in 1..pieces.len() {
        if pieces[j].1.text == "{" && pieces[j - 1].1.text == ")" && same_line(&pieces[j].0) {
            let function_or_control = piece_open_paren(pieces, j - 1)
                .and_then(|open| open.checked_sub(1))
                .is_some_and(|before| pieces[before].1.kind == TokenKind::Ident);
            if function_or_control {
                pieces[j].0 = " ".to_owned();
            }
        }
    }
}

/// Bit-field colon spacing (§2.5: `x: 1` — no space before, one after). A `:` qualifies only when
/// it follows an identifier, precedes an integer literal, and no `?` opened a ternary earlier in
/// the statement (which would make it a ternary colon, not a bit-field).
fn space_bit_fields(pieces: &mut [Piece]) {
    for j in 1..pieces.len().saturating_sub(1) {
        let is_bit_field = pieces[j].1.text == ":"
            && pieces[j].1.kind == TokenKind::Punct
            && pieces[j - 1].1.kind == TokenKind::Ident
            && pieces[j + 1].1.kind == TokenKind::Number
            && same_line(&pieces[j].0)
            && same_line(&pieces[j + 1].0)
            && !ternary_open_before(pieces, j);
        if is_bit_field {
            pieces[j].0.clear();
            pieces[j + 1].0 = " ".to_owned();
        }
    }
}

/// Whether an unmatched `?` precedes index `j` within the current statement (back to `;`/`{`/`}`).
fn ternary_open_before(pieces: &[Piece], j: usize) -> bool {
    for p in pieces[..j].iter().rev() {
        match p.1.text {
            "?" => return true,
            ";" | "{" | "}" => return false,
            _ => {}
        }
    }
    false
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
            let cols: usize = t
                .text
                .chars()
                .map(|c| if c == '\t' { TAB_WIDTH } else { 1 })
                .sum();
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

/// Run the structuring pass over `toks`, with the cursor starting at `start_col` (non-zero when
/// formatting a fragment such as a macro body that follows a prefix).
fn structure(toks: &[Token], start_col: usize, width: usize) -> String {
    let mut out = String::new();
    let mut col = start_col;
    let mut i = 0usize;
    let mut paren_depth = 0i32;
    let mut in_init = false;
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
        {
            emit_str(&mut out, &mut col, t.text);
            let doc = build_call_doc(&toks[i + 2..close]);
            let base_level = current_line_indent_cols(&out) / TAB_WIDTH;
            let reserved = trailing_reserved(toks, close + 1);
            let rendered = render(&doc, width.saturating_sub(reserved), col, base_level);
            emit_str(&mut out, &mut col, &rendered);
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
        if t.kind == TokenKind::Punct
            && t.text == "("
            && let Some(close) = match_bracket(toks, i)
            && has_top_level_question(&toks[i + 1..close])
            && !contains_comment(&toks[i + 1..close])
        {
            let doc = build_ternary_doc(&toks[i + 1..close]);
            let base_level = current_line_indent_cols(&out) / TAB_WIDTH;
            let reserved = trailing_reserved(toks, close + 1);
            let rendered = render(&doc, width.saturating_sub(reserved), col, base_level);
            emit_str(&mut out, &mut col, &rendered);
            i = close + 1;
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
        let continued = full.split('\n').collect::<Vec<_>>().join(" \\\n");
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
    if unformattable {
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
    if has_comment_or_directive {
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
    toks.get(i).is_some_and(|t| t.kind == TokenKind::Ident)
        && !is_excluded_callee(toks[i].text)
        && toks
            .get(i + 1)
            .is_some_and(|n| n.kind == TokenKind::Punct && n.text == "(")
}

/// Keywords that take a `(` but are not calls whose arguments split on commas. `_Generic` is not
/// excluded: its associations are a comma list and explode exactly per §2.2.
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

/// The next non-trivia token index at or after `from`.
fn next_nontrivia(toks: &[Token], from: usize) -> Option<usize> {
    (from..toks.len()).find(|&j| !is_trivia(&toks[j]))
}

/// The next non-trivia token index in `[from, end)`.
fn next_nontrivia_in(toks: &[Token], from: usize, end: usize) -> Option<usize> {
    (from..end).find(|&j| !is_trivia(&toks[j]))
}

/// One past the last token of the preprocessor directive starting at `start` (following `\` line
/// continuations).
fn directive_end(toks: &[Token], start: usize) -> usize {
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

fn is_trivia(t: &Token) -> bool {
    matches!(t.kind, TokenKind::Whitespace | TokenKind::Newline)
}

fn contains_comment(toks: &[Token]) -> bool {
    toks.iter()
        .any(|t| matches!(t.kind, TokenKind::LineComment | TokenKind::BlockComment))
}

/// Whether a `?` ternary operator appears at bracket depth zero in `inner`.
fn has_top_level_question(inner: &[Token]) -> bool {
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

/// Build the document for a parenthesized ternary chain: split on the depth-zero `:`, each
/// `cond ? val` segment carrying a trailing ` :` when broken, flat otherwise (§2.4).
fn build_ternary_doc(inner: &[Token]) -> Doc {
    let segments = split_top_level(inner, |t| t.kind == TokenKind::Punct && t.text == ":")
        .iter()
        .map(|s| render_segment(s))
        .collect();
    build_clause_group(segments, " :")
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

/// Build the §2.2 document for a call's argument list, including the surrounding parens. Each
/// argument is built recursively (via [`build_element_doc`]), so a compound-literal argument's
/// `{...}` is a nested group that can collapse or explode on its own.
fn build_call_doc(inner: &[Token]) -> Doc {
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
                w += display_width(t.text);
                break;
            }
            _ => w += display_width(t.text),
        }
    }
    w
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
