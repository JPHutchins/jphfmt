//! The structuring pass: a single left-to-right walk over the token stream that reformats the
//! constructs jphfmt understands (call/declaration lists, `{}`/`enum` bodies, control headers,
//! parenthesized ternaries, `#define` bodies, GNU statement-expressions, function bodies) and
//! emits everything else byte-for-byte. Output is built into a [`String`] with a tracked display
//! column; [`emit_str`] is the single mutator. Pure helpers for column accounting and trailing-token
//! reservation live alongside.

use super::builders::{
    Bound, Fit, build_brace_doc, build_bracketed_group, build_call_body, build_chain_doc,
    build_cond_doc, build_for_doc, build_statement_element, group_bracketing, statement_segments,
};
use super::scope::scoped;
use super::tokens::{
    closes_block, closes_control_header, closes_literal_type, contains_comment, directive_end,
    enum_body_brace, has_middle_newline, has_non_trivia, is_backslash, is_balanced, is_call_head,
    is_chain_break, is_comment, is_control_keyword, is_excluded_callee, is_trivia, match_brace,
    match_bracket, next_nontrivia, next_nontrivia_in, next_paren, prev_nontrivia, prev_significant,
    respaced_when_joined, split_brace_line_comment, statement_end,
};
use crate::doc::{Doc, TAB_WIDTH, display_width, render};
use crate::lexer::{Token, TokenKind};

/// Run the structuring pass over `toks`, with the cursor starting at `start_col` (non-zero when
/// formatting a fragment such as a macro body that follows a prefix).
pub(super) fn structure(toks: &[Token], start_col: usize, width: usize) -> String {
    let mut out = String::new();
    let mut col = start_col;
    let mut depth = 0usize;
    emit_tokens(toks, &mut out, &mut col, &mut depth, width);
    out
}

/// Whether the bracket at `open` is a call's `(`. Its argument list belongs to the call handler: a call
/// whose arguments hold a comment or are unbalanced falls through to per-token verbatim, and laying it
/// out here instead would collapse that whitespace and lose empty leading arguments. A `[` is never a
/// call's, so it is never excluded.
fn heads_call(toks: &[Token], open: usize) -> bool {
    toks[open].text == "("
        && open > 0
        && toks[open - 1].kind == TokenKind::Ident
        && !is_excluded_callee(toks[open - 1].text)
}

/// Render `doc` for the line it is landing on and emit it. `reserved` is the width of what must still
/// fit after it: the tokens the construct does not own but shares its last line with. Every handler
/// that lays a construct out goes through here, so none of them can drift apart on how they measure.
fn emit_doc(doc: &Doc, reserved: usize, out: &mut String, col: &mut usize, width: usize) {
    let base_level = current_line_indent_cols(out) / TAB_WIDTH;
    let rendered = render(doc, width.saturating_sub(reserved), *col, base_level);
    emit_str(out, col, &rendered);
}

/// Walk `toks`, appending to `out` so an enclosing construct's indentation is already in view when a
/// nested one measures its own base level. `depth` is the `#if` nesting the walk has reached, carried
/// through nested bodies because a scope opened in one can close outside it.
fn emit_tokens(toks: &[Token], out: &mut String, col: &mut usize, depth: &mut usize, width: usize) {
    let mut i = 0usize;
    let mut paren_depth = 0i32;
    let mut in_init = false;
    let mut pending_func_def = false;
    while i < toks.len() {
        let t = toks[i];

        if t.kind == TokenKind::Punct && t.text == "#" && current_line_is_blank(out) {
            let is_define = next_nontrivia(toks, i + 1)
                .is_some_and(|j| toks[j].kind == TokenKind::Ident && toks[j].text == "define");
            i = if is_define {
                emit_define(toks, i, out, col, *depth, width)
            } else {
                emit_directive(toks, i, out, col, depth)
            };
            continue;
        }

        if t.kind == TokenKind::Ident
            && is_control_keyword(t.text)
            && let Some(open) = next_paren(toks, i)
            && let Some(close) = match_bracket(toks, open)
            && !contains_comment(&toks[open + 1..close])
            && is_balanced(&toks[open + 1..close])
        {
            // §2.5: control keywords take exactly one space before `(` (`if (`, not `if(`).
            emit_str(out, col, t.text);
            emit_str(out, col, " ");
            let inner = &toks[open + 1..close];
            let doc = if t.text == "for" {
                build_for_doc(inner)
            } else {
                build_cond_doc(inner)
            };
            emit_doc(&doc, trailing_reserved(toks, close + 1), out, col, width);
            i = close + 1;
            continue;
        }

        if t.kind == TokenKind::Ident
            && t.text == "enum"
            && let Some(brace) = enum_body_brace(toks, i)
        {
            for tok in &toks[i..brace] {
                emit_str(out, col, tok.text);
            }
            i = emit_brace(toks, brace, true, out, col, width);
            continue;
        }

        if is_call_head(toks, i)
            && let Some(close) = match_bracket(toks, i + 1)
        {
            let inner = &toks[i + 2..close];
            if !contains_comment(inner) && is_balanced(inner) && !has_middle_newline(inner) {
                emit_str(out, col, t.text);
                let doc = build_call_body(inner, Fit::Measured);
                emit_doc(&doc, trailing_reserved(toks, close + 1), out, col, width);
                pending_func_def =
                    next_nontrivia(toks, close + 1).is_some_and(|j| toks[j].text == "{");
                i = close + 1;
                continue;
            }
            if !contains_comment(inner) && is_balanced(inner) && has_middle_newline(inner) {
                // The whole call is passed through verbatim: skip past `close` so nested calls
                // inside the args are not re-entered and reflowed. Reflowing them would strip
                // their intra-arg newlines, flipping this call's fits/explode decision on the
                // next pass and breaking idempotency.
                for tok in &toks[i..=close] {
                    emit_str(out, col, tok.text);
                }
                pending_func_def =
                    next_nontrivia(toks, close + 1).is_some_and(|j| toks[j].text == "{");
                i = close + 1;
                continue;
            }
        }

        // GNU statement-expression `({ ... })` — block-indent its statements.
        if t.kind == TokenKind::Punct
            && t.text == "("
            && toks
                .get(i + 1)
                .is_some_and(|n| n.kind == TokenKind::Punct && n.text == "{")
        {
            let base_level = current_line_indent_cols(out) / TAB_WIDTH;
            if let Some((block, next)) = format_stmt_expr(toks, i, base_level, width) {
                emit_str(out, col, &block);
                i = next;
                continue;
            }
        }

        // Function definition body: `{` after `)` from a function/macro definition. Always break
        // with one statement per line, body indented, `}` at the definition's own indent level.
        if t.kind == TokenKind::Punct && t.text == "{" && pending_func_def {
            pending_func_def = false;
            i = emit_func_body(toks, i, out, col, depth, width);
            continue;
        }

        // An initializer brace: in an `= ... ;` region, a `{` that is not a statement-expression and
        // not a `struct`/`union` definition's own body.
        if in_init
            && t.kind == TokenKind::Punct
            && t.text == "{"
            && last_nonspace_char(out) != Some('(')
            && !opens_definition_body(toks, i)
            && match_brace(toks, i).is_some()
        {
            i = emit_brace(toks, i, false, out, col, width);
            continue;
        }

        // A compound literal `(T){...}` outside an `= ... ;` region — `return (T){...}`, a call
        // argument, a bare statement. The parenthesized group must spell a type, and must not be a
        // suffix of something larger: a function-pointer return type (`…))(int) {`) and an attribute
        // (`__attribute__((noreturn)) {`) both put a `)` before the `{` of a body, and mistaking
        // either for a literal would lay that body out as an initializer list.
        if t.kind == TokenKind::Punct
            && t.text == "{"
            && let Some(paren) = prev_nontrivia(toks, i)
            && toks[paren].text == ")"
            && closes_literal_type(toks, paren)
            && !contains_comment(&toks[paren..i])
            && match_brace(toks, i).is_some()
        {
            i = emit_brace(toks, i, false, out, col, width);
            continue;
        }

        // A bracketed group the author wrote — a parenthesized chain or ternary, or an index (#77).
        // One handler, because they are one construct: the operator trails each line either way
        // (§2.7), and only the pair differs. These brackets are the author's; a bare chain is bounded
        // by `build_chain_doc` instead, which adds its own.
        //
        // An index reaches nothing else. The chain handler below needs a chain at the statement's own
        // top level, and `int j = arr[…];` has none, so without this it would overrun at any length.
        if t.kind == TokenKind::Punct
            && let Some(bracketing) = group_bracketing(&t)
            && !heads_call(toks, i)
            && let Some(close) = match_bracket(toks, i)
            && !contains_comment(&toks[i + 1..close])
            && is_balanced(&toks[i + 1..close])
            && let Some(doc) = build_bracketed_group(&toks[i + 1..close], bracketing)
        {
            emit_doc(&doc, trailing_reserved(toks, close + 1), out, col, width);
            i = close + 1;
            continue;
        }

        // A statement whose own top level is an operator chain. Nothing else lays a bare statement
        // out, so without this the one construct that cannot be a container is also the one that
        // can overrun the width.
        // `else` starts no statement of its own: it introduces the one after it, which reaches this
        // handler on its own token. Beginning the span here instead would put `else` in the chain's
        // head, and a head renders flat — joining a braceless body onto the `else` line whenever it
        // happened to hold an operator.
        if !is_trivia(&t)
            && t.text != "else"
            && starts_statement(toks, i)
            && let Some(semi) = statement_end(toks, i)
            && !contains_comment(&toks[i..semi])
            && is_balanced(&toks[i..semi])
            && !toks[i..semi].iter().any(|s| s.text == "{")
            && let Some(doc) = build_chain_doc(&toks[i..semi], Bound::Parens)
        {
            // Only the `;` is reserved. `trailing_reserved` would also count whatever shares the
            // line after it, which this pass's own whitespace changes shift — an unstable measure.
            emit_doc(&doc, 1, out, col, width);
            i = semi;
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
        emit_str(out, col, t.text);
        i += 1;
    }
}

/// Whether the `{` at `open` opens a `struct`/`union` definition's member list — the body of the type an
/// anonymous compound literal names, `(struct { int x; }){1}`. Its members are `;`-terminated rather than
/// `,`-separated, so it is not the container [`build_brace_doc`] lays out: exploding it wrote §2.3's magic
/// comma into a member list, `{ int x;, }`, which does not compile (#95). Emitted verbatim instead, which
/// keeps the author's spacing too.
///
/// Only reachable inside an `= … ;` region — a definition elsewhere never enters the initializer handler.
/// An `enum` body *is* a comma list, and has its own handler ([`enum_body_brace`]).
fn opens_definition_body(toks: &[Token], open: usize) -> bool {
    let tags = |k: usize| matches!(toks[k].text, "struct" | "union");
    prev_significant(toks, open).is_some_and(|k| {
        tags(k) || (toks[k].kind == TokenKind::Ident && prev_significant(toks, k).is_some_and(tags))
    })
}

/// Whether the token at `i` opens a statement: nothing precedes it, or what does ended the previous
/// one — a `;`, a `{`, a block's `}`, an `else`, or the `)` of a braceless `if`/`for`/`while`/`switch`
/// header. A property of the token stream, not of what has been emitted so far: any other `)` is
/// inside the statement, which some handler has already claimed from its first token.
///
/// A `}` that closes a value ends no statement (#88), and reading one as if it did is what let a chain
/// after a compound literal be claimed as a statement of its own: the parentheses bounding it landed
/// against the literal, making it a *call* on it, and the output did not compile.
fn starts_statement(toks: &[Token], i: usize) -> bool {
    let Some(k) = prev_nontrivia(toks, i) else {
        return true;
    };
    match toks[k].text {
        ";" | "{" | "else" => true,
        "}" => closes_block(toks, k),
        ")" => closes_control_header(toks, k),
        _ => false,
    }
}

/// Format a `#define`: a function-like macro whose body is a single call/`_Generic` or a
/// statement-expression is laid out with the body opening on the `#define` line and `\`
/// continuations one space after each line; any other body is emitted verbatim.
fn emit_define(
    toks: &[Token],
    start: usize,
    out: &mut String,
    col: &mut usize,
    depth: usize,
    width: usize,
) -> usize {
    let end = directive_end(toks, start);
    // The columns [`super::scope::scope_directives`] will put between `#` and `define` for the `#if`
    // nesting this line sits at. Measuring the gap as written instead is what made a `#define` at
    // exactly the limit inside an `#if` alternate forever: the first run has no gap to count, and
    // every run after it counts the tab the previous one grew.
    let scoped_col = depth * TAB_WIDTH;
    if let Some(def) = split_define(toks, start, end)
        && let Some(body_str) =
            format_define_body(&def.body, scoped_col + display_width(&def.prefix), width)
    {
        let flat = format!("{prefix}{body_str}", prefix = def.prefix);
        // Each line is trimmed before its ` \` is added: a body that passed through verbatim carries
        // the whitespace the *previous* run put before its `\`, and adding another would widen every
        // continued line by one column per run.
        let continued = explode_params(&def, &flat, scoped_col, width)
            .unwrap_or(flat)
            .lines()
            .map(str::trim_end)
            .collect::<Vec<_>>()
            .join(" \\\n");
        emit_str(out, col, &continued);
        emit_str(out, col, "\n");
        return end;
    }
    for tok in &toks[start..end] {
        emit_str(out, col, tok.text);
    }
    end
}

/// A `#define`'s parameter list is a container like any other (§2.2), so it explodes one parameter
/// per line when the flat form's first line — the parameters *and* however much of the body opens on
/// it — overruns the width. The body then starts the line after the `)`, indented one level; its own
/// layout is measured against the width that indent leaves. `None` when the flat form fits, or for an
/// object-like macro, which has no list to break.
fn explode_params(def: &Define, flat: &str, scoped_col: usize, width: usize) -> Option<String> {
    let continuation = usize::from(flat.contains('\n')) * CONTINUATION_WIDTH;
    if scoped_col + display_width(flat.lines().next().unwrap_or(flat)) + continuation <= width {
        return None;
    }
    let params = def.params.as_deref()?;
    let continued = width.saturating_sub(CONTINUATION_WIDTH);
    let body = format_define_body(&def.body, 0, continued.saturating_sub(TAB_WIDTH))?;
    // A body of more than one line cannot be indented under the `)`: on the next pass its own line
    // breaks make it a passthrough, so the tab added here would be part of the text and another
    // would be added on top of it, once per run.
    if body.contains('\n') {
        return None;
    }
    let params = render(
        &build_call_body(params, Fit::Forced),
        continued,
        scoped_col + display_width(&def.head),
        0,
    );
    Some(format!("{head}{params}\n\t{body}", head = def.head))
}

/// Columns the ` \` a continued line ends with occupies.
const CONTINUATION_WIDTH: usize = 2;

/// Join a continued run of tokens onto one line, dropping its `\`s. [`emit_define`] re-adds a
/// continuation at every newline it is handed, so a newline surviving here would take the `\` beside
/// it and grow another on each pass. Same-line whitespace is left as written: the `#`-to-keyword gap
/// that [`super::scope::scope_directives`] rewrites must reach the width measurement unchanged.
fn flatten_continuations(toks: &[Token]) -> String {
    let mut out = String::new();
    let mut broken = false;
    for t in toks {
        if is_backslash(t) || t.kind == TokenKind::Newline {
            while out.ends_with([' ', '\t']) {
                out.pop();
            }
            broken = true;
        } else if !(broken && t.kind == TokenKind::Whitespace) {
            if broken {
                out.push(' ');
                broken = false;
            }
            out.push_str(t.text);
        }
    }
    out
}

/// Split a `#define` into its `#define NAME(params) ` prefix text and its body tokens (with
/// continuation backslashes removed and surrounding trivia trimmed). `None` if it has no body.
struct Define<'src> {
    prefix: String,
    head: String,
    params: Option<Vec<Token<'src>>>,
    body: Vec<Token<'src>>,
}

/// Drop the continuation `\`s from `toks` — they belong to the input's line breaks, not to the
/// construct, and are re-added per line by [`emit_define`].
fn without_continuations<'src>(toks: &[Token<'src>]) -> Vec<Token<'src>> {
    toks.iter().filter(|t| !is_backslash(t)).copied().collect()
}

fn split_define<'src>(toks: &[Token<'src>], start: usize, end: usize) -> Option<Define<'src>> {
    let define = next_nontrivia_in(toks, start + 1, end)?;
    let name = next_nontrivia_in(toks, define + 1, end)?;
    let function_like = toks
        .get(name + 1)
        .is_some_and(|n| n.kind == TokenKind::Punct && n.text == "(");
    // `match_bracket` scans past `end`; a `)` beyond this directive means the param list is not
    // closed within it (e.g. a newline ended the directive mid-params), so it is not a
    // function-like macro we can split — pass through verbatim.
    let close = if function_like {
        let close = match_bracket(toks, name + 1)?;
        if close >= end {
            return None;
        }
        Some(close)
    } else {
        None
    };
    let prefix_end = close.map_or(name + 1, |close| close + 1);
    let body = {
        let mut body = without_continuations(&toks[prefix_end..end]);
        while body.first().is_some_and(is_trivia) {
            body.remove(0);
        }
        while body.last().is_some_and(is_trivia) {
            body.pop();
        }
        body
    };
    if body.is_empty() {
        return None;
    }
    // `#` then the keyword, with nothing between: the gap belongs to
    // [`super::scope::scope_directives`], which rewrites it to the nesting depth's tabs.
    let from_hash = |to: usize| {
        format!(
            "{hash}{rest}",
            hash = toks[start].text,
            rest = flatten_continuations(&toks[define..to])
        )
    };
    Some(Define {
        prefix: from_hash(prefix_end) + " ",
        head: from_hash(name + 1),
        params: close.map(|close| without_continuations(&toks[name + 2..close])),
        body,
    })
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
        return format_stmt_expr(body, 0, 0, width).map(|(s, _)| s);
    }
    None
}

/// Format a `({ ... })` statement-expression: `({` opens the line, each statement on its own line
/// at `base_level + 1` laid out with the §2.2 rule (so a nested call explodes when it overflows),
/// `})` at `base_level`. Returns the block and the index past the `)`, or `None` if the braces are
/// unbalanced or a statement nests a block or carries a comment.
///
/// That last guard is load-bearing here, unlike in [`emit_func_body`], which hands its body back to
/// [`emit_tokens`]: this splits on `;`, and a nested block is not `;`-terminated, so a `{` inside
/// would land mid-statement. Such a body passes through instead.
fn format_stmt_expr(
    toks: &[Token],
    open: usize,
    base_level: usize,
    width: usize,
) -> Option<(String, usize)> {
    let paren_close = match_bracket(toks, open)?;
    let brace_close = match_brace(toks, open + 1)?;
    // This reports `paren_close + 1` as consumed but renders only as far as `}`, so anything between
    // the two would be deleted from the output. In a statement-expression the `)` follows the `}`
    // directly; where it does not — or where the `}` is outside the parentheses entirely, which is
    // what `({)}` lexes as — the construct is something else and §6 prefers passthrough.
    if brace_close > paren_close || has_non_trivia(&toks[brace_close + 1..paren_close]) {
        return None;
    }
    let inner = &toks[open + 2..brace_close];
    let unformattable = inner
        .iter()
        .any(|t| is_comment(t) || (t.kind == TokenKind::Punct && t.text == "{"));
    if unformattable || !is_balanced(inner) {
        return None;
    }
    let inner_indent = "\t".repeat(base_level + 1);
    let close_indent = "\t".repeat(base_level);
    let stmt_col = (base_level + 1) * TAB_WIDTH;
    let segments = statement_segments(inner);
    let (trailing, leading) = segments.split_last()?;
    // Every leading segment becomes a statement, empty or not, because each gets exactly one `;`
    // written back: dropping an empty one would lose the `;` that produced it. Only the last may be
    // dropped when empty — that is what a body ending in `;` splits to, which is the canonical form,
    // and keeping it would write a `;` the author did not.
    let statements: Vec<String> = leading
        .iter()
        .chain(has_non_trivia(trailing).then_some(trailing))
        .map(|s| {
            render(
                &build_statement_element(s),
                width.saturating_sub(1),
                stmt_col,
                base_level + 1,
            )
        })
        .collect();
    if statements.is_empty() {
        return None;
    }
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
    let has_comment_or_directive = inner
        .iter()
        .any(|t| is_comment(t) || (t.kind == TokenKind::Punct && t.text == "#"));
    if has_comment_or_directive || !is_balanced(inner) || respaced_when_joined(inner) {
        for tok in &toks[open..=close] {
            emit_str(out, col, tok.text);
        }
        return close + 1;
    }
    let doc = build_brace_doc(inner, padded);
    emit_doc(&doc, trailing_reserved(toks, close + 1), out, col, width);
    close + 1
}

/// Format a function definition body: always break with `{\n\tstatements\n}`, the statements walked
/// by [`emit_tokens`] so every construct inside is laid out. Blank lines within the body survive.
/// Falls back to verbatim only for unbalanced braces — a comment, a directive or a nested block is
/// handled rather than refused, which is where this differs from [`emit_brace`].
fn emit_func_body(
    toks: &[Token],
    open: usize,
    out: &mut String,
    col: &mut usize,
    depth: &mut usize,
    width: usize,
) -> usize {
    let Some(close) = match_brace(toks, open) else {
        emit_str(out, col, toks[open].text);
        return open + 1;
    };
    let inner = &toks[open + 1..close];
    if !is_balanced(inner) {
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

    let (head, body) = split_brace_line_comment(inner);
    for tok in head {
        emit_str(out, col, tok.text);
    }

    if body.is_empty() {
        // Nothing to lay out. A comment-only body keeps its own spacing rather than have its `}`
        // moved for no gain; an empty one collapses to `{}`.
        if !head.is_empty() {
            for tok in &inner[head.len()..] {
                emit_str(out, col, tok.text);
            }
        }
        emit_str(out, col, "}");
        return close + 1;
    }

    emit_str(out, col, "\n");
    if body[0].text != "#" {
        emit_str(out, col, &inner_indent);
    }
    emit_tokens(body, out, col, depth, width);
    // A directive carries its own line break, so the one before `}` is already there.
    if !out.ends_with('\n') {
        emit_str(out, col, "\n");
    }
    emit_str(out, col, &close_indent);
    emit_str(out, col, "}");

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
    // `pending` holds the width of a whitespace run: it counts only once something follows it, since
    // whitespace ending the line never reaches the output — reserving for it would measure a line
    // this pass is about to shorten, and reach a different verdict than the next pass does.
    let (mut width, mut pending) = (0usize, 0usize);
    for (j, t) in toks.iter().enumerate().skip(from) {
        // A chain breaks after its operator as a bracket group breaks after its bracket: what
        // follows can land on a later line, so its flat width is not this construct's to reserve —
        // and once it has broken, the next pass measures a shorter run and decides differently.
        if is_chain_break(toks, j) {
            return width + pending + display_width(t.text);
        }
        let counted = match t.kind {
            TokenKind::Newline => break,
            TokenKind::LineComment | TokenKind::BlockComment => continue,
            // Nothing past a bracket or a `;` shares this construct's fate: anything past the
            // bracket can break onto a later line, and the `;` ends the statement.
            TokenKind::Punct if matches!(t.text, "(" | "[" | "{" | ";") => {
                return width + pending + display_width(t.text);
            }
            // Stop at the first newline embedded in any token (not just Newline tokens),
            // so Unknown tokens containing multiple lines don't inflate the reserve.
            _ => match t.text.find('\n') {
                Some(nl) => return width + pending + display_width(t.text[..nl].trim_end()),
                None => t.text,
            },
        };
        if counted.trim().is_empty() {
            pending += display_width(counted);
        } else {
            width += pending + display_width(counted);
            pending = 0;
        }
    }
    width
}

/// Emit a preprocessor directive verbatim, following `\` line continuations; returns the index
/// just past it. Advances `depth` by the same rule [`super::scope::scope_directives`] applies, so a
/// `#define` further on is measured at the nesting it will be indented to.
fn emit_directive(
    toks: &[Token],
    start: usize,
    out: &mut String,
    col: &mut usize,
    depth: &mut usize,
) -> usize {
    let end = directive_end(toks, start);
    if let Some(keyword) = next_nontrivia_in(toks, start + 1, end) {
        *depth = scoped(toks[keyword].text, *depth).after;
    }
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
    fn trailing_reserved_counts_a_tab_as_tab_width() {
        let toks = [tok(TokenKind::Unknown, "a\tb"), tok(TokenKind::Punct, ";")];
        assert_eq!(trailing_reserved(&toks, 0), 1 + TAB_WIDTH + 1 + 1);
    }

    #[test]
    fn trailing_reserved_stops_at_newline() {
        let toks = [tok(TokenKind::Newline, "\n"), tok(TokenKind::Punct, ";")];
        assert_eq!(trailing_reserved(&toks, 0), 0);
    }

    #[test]
    fn trailing_reserved_stops_at_the_statement_end() {
        // The `;` counts (1) and ends the reserve: what follows it is another statement's.
        let toks = [tok(TokenKind::Punct, ";"), tok(TokenKind::Punct, "(")];
        assert_eq!(trailing_reserved(&toks, 0), 1);
    }

    #[test]
    fn trailing_reserved_counts_punct_then_stops_at_bracket() {
        // ` {` of a function body: the space and brace count, and the brace stops the reserve.
        let toks = [tok(TokenKind::Whitespace, " "), tok(TokenKind::Punct, "{")];
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
