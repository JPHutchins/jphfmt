//! The structuring pass: a single left-to-right walk over the token stream that reformats the
//! constructs jphfmt understands (call/declaration lists, `{}`/`enum` bodies, control headers,
//! parenthesized ternaries, `#define` bodies, GNU statement-expressions, function bodies) and
//! emits everything else byte-for-byte. Output is built into a [`String`] with a tracked display
//! column; [`emit_str`] is the single mutator. Pure helpers for column accounting and trailing-token
//! reservation live alongside.

use super::builders::{
    Bound, Fit, build_brace_doc, build_bracketed_group, build_call_body, build_chain_doc,
    build_cond_doc, build_for_doc, build_statement_element, group_bracketing, holds_forced_break,
    statement_segments,
};
use super::scope::scoped;
use super::tokens::{
    assigns, closes_block, closes_control_header, closes_literal_type, contains_comment,
    directive_end, element_join_respaced, enum_body_brace, has_middle_newline, has_non_trivia,
    holds_unsafe_hash, is_backslash, is_balanced, is_call_head, is_call_head_pair, is_chain_break,
    is_comment, is_control_keyword, is_trivia, match_brace, match_bracket, next_nontrivia,
    next_nontrivia_in, next_paren, opens_stmt_expr, prev_nontrivia, prev_significant, spans_lines,
    split_brace_line_comment, statement_end,
};
use crate::doc::{Doc, TAB_WIDTH, display_width, render};
use crate::lexer::{Token, TokenKind};

/// Run the structuring pass over `toks`, with the cursor starting at `start_col` (non-zero when
/// formatting a fragment such as a macro body that follows a prefix).
pub(super) fn structure(
    toks: &[Token],
    start_col: usize,
    width: usize,
    in_define_body: bool,
) -> String {
    let mut out = String::new();
    let mut col = start_col;
    let mut depth = 0usize;
    emit_tokens(toks, &mut out, &mut col, &mut depth, width, in_define_body);
    out
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
fn emit_tokens(
    toks: &[Token],
    out: &mut String,
    col: &mut usize,
    depth: &mut usize,
    width: usize,
    in_define_body: bool,
) {
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
            && !holds_unsafe_hash(&toks[open + 1..close], in_define_body)
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
            emit_doc(
                &doc,
                trailing_reserved(toks, close + 1, in_define_body),
                out,
                col,
                width,
            );
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
            i = emit_brace(toks, brace, true, in_define_body, out, col, width);
            continue;
        }

        if let Some(open) = next_nontrivia(toks, i + 1).filter(|&k| toks[k].text == "(")
            && is_call_head_pair(toks, open)
            && let Some(close) = match_bracket(toks, open)
        {
            let inner = &toks[open + 1..close];
            if !contains_comment(inner)
                && !holds_unsafe_hash(inner, in_define_body)
                && is_balanced(inner)
                && !has_middle_newline(inner)
            {
                // The pair-tolerant reading: trivia between the callee and `(` is dropped, and the
                // tight `f(` this writes is the form `space_call_heads` canonicalizes — the same
                // join `build_expr_doc`'s call arm makes for nested calls.
                emit_str(out, col, t.text);
                let doc = build_call_body(inner, Fit::Measured);
                emit_doc(
                    &doc,
                    trailing_reserved(toks, close + 1, in_define_body),
                    out,
                    col,
                    width,
                );
                pending_func_def =
                    next_nontrivia(toks, close + 1).is_some_and(|j| toks[j].text == "{");
                i = close + 1;
                continue;
            }
            if !contains_comment(inner) && is_balanced(inner) && has_middle_newline(inner) {
                // The whole call is passed through verbatim: skip past `close` so nested calls
                // inside the args are not re-entered and reflowed. Reflowing them would strip
                // their intra-arg newlines, flipping this call's fits/explode decision on the
                // next pass and breaking idempotency. No edge pad: `space_equals` runs first and
                // pre-spaces every same-line `=` edge, so this verbatim cannot write the tight one.
                //
                // Unless the re-laid call would be forced broken anyway — a magic trailing comma
                // — where the passthrough's text form loses the force: the enclosing group the
                // call sits in then measures a doc without the ForceBreak and joins what the
                // previous pass broke, two passes for one line (#108's draw). A forced break has
                // no fits decision to flip, so the re-laid form is the one every pass reaches. A
                // `#` or `##` fragment in the arguments keeps the verbatim — its lines are not
                // the layout's to own, the same guard the laid arm carries.
                let holds_hash = inner.iter().any(|t| matches!(t.text, "#" | "##"));
                if !holds_hash {
                    let doc = build_call_body(inner, Fit::Measured);
                    if holds_forced_break(&doc) {
                        emit_str(out, col, t.text);
                        emit_doc(
                            &doc,
                            trailing_reserved(toks, close + 1, in_define_body),
                            out,
                            col,
                            width,
                        );
                        pending_func_def =
                            next_nontrivia(toks, close + 1).is_some_and(|j| toks[j].text == "{");
                        i = close + 1;
                        continue;
                    }
                }
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
        if opens_stmt_expr(toks, i) {
            let base_level = current_line_indent_cols(out) / TAB_WIDTH;
            if let Some((block, next)) =
                format_stmt_expr(toks, i, base_level, width, in_define_body)
            {
                emit_str(out, col, &block);
                i = next;
                continue;
            }
        }

        // Function definition body: `{` after `)` from a function/macro definition. Always break
        // with one statement per line, body indented, `}` at the definition's own indent level.
        if t.kind == TokenKind::Punct && t.text == "{" && pending_func_def {
            pending_func_def = false;
            i = emit_func_body(toks, i, out, in_define_body, col, depth, width);
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
            i = emit_brace(toks, i, false, in_define_body, out, col, width);
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
            i = emit_brace(toks, i, false, in_define_body, out, col, width);
            continue;
        }

        // A bracketed group the author wrote — a parenthesized chain or ternary, or an index (#77).
        // One handler, because they are one construct: the operator trails each line either way
        // (§2.7), and only the pair differs. These brackets are the author's; a bare chain is bounded
        // by `build_chain_doc` instead, which adds its own.
        //
        // An index reaches nothing else. The chain handler below needs a chain at the statement's own
        // top level, and `int j = arr[…];` has none, so without this it would overrun at any length.
        // A call head is excluded with the shared trivia-tolerant predicate — a type keyword is not
        // a callee, so `int (` stays a declarator group here as everywhere else.
        if t.kind == TokenKind::Punct
            && let Some(bracketing) = group_bracketing(&t)
            && !is_call_head_pair(toks, i)
            && let Some(close) = match_bracket(toks, i)
            && !contains_comment(&toks[i + 1..close])
            && is_balanced(&toks[i + 1..close])
            && !holds_unsafe_hash(&toks[i + 1..close], in_define_body)
            && let Some(doc) = build_bracketed_group(&toks[i + 1..close], bracketing)
        {
            emit_doc(
                &doc,
                trailing_reserved(toks, close + 1, in_define_body),
                out,
                col,
                width,
            );
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
        // A `{` after a `[` is one construct — the juxtaposed-bracket join the group doc writes
        // tight. A refused bracket group falls back to this walk, and a gap kept there would
        // re-read as the author's own on the next pass and join then: two passes for one line,
        // keyed on whitespace where the doc keys on tokens (#108's fresh draw).
        if is_trivia(&t)
            && prev_nontrivia(toks, i).is_some_and(|j| toks[j].text == "[")
            && next_nontrivia(toks, i + 1).is_some_and(|j| toks[j].text == "{")
        {
            i += 1;
            continue;
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
    // The body is the last line, and [`emit_define`] writes ` \` *between* lines, so this one takes
    // none: only the tab is reserved, and [`format_define_body`] owns whatever the continuation costs.
    let body = format_define_body(&def.body, 0, width.saturating_sub(TAB_WIDTH))?;
    // A body of more than one line cannot be indented under the `)`: the tab added here would be
    // part of the text on the next pass, where a broken body is re-claimed and re-laid out — or
    // emitted verbatim for the call shape — and another tab would be added on top of it, once per
    // run.
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
    // A `\` is not a name. `from_hash` flattens the continuations inside the head, so splitting here
    // would delete this one with nothing to write it back — and what follows it is then read as the
    // body rather than as the name, which is how `#define \` + `(})` came out as `#define (})`. §6
    // prefers passthrough, and a continued name is not a shape this needs to lay out.
    //
    // A `\` touching the name is the same hazard on the other side: the splice joins the two lines
    // with nothing between them, so `#define NAME\` + `(x)` defines the function-like `NAME(x)`,
    // while the `(x)` reads here as an object-like body. Only the adjacent one — `#define NAME \` +
    // `(x)` splices to a space, and `NAME` really is object-like there.
    let after_name = toks.get(name + 1);
    if is_backslash(&toks[name]) || after_name.is_some_and(is_backslash) {
        return None;
    }
    let function_like = after_name.is_some_and(|n| n.kind == TokenKind::Punct && n.text == "(");
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

/// Format a macro body if it is a single call/`_Generic`, a statement-expression, or one whole
/// bracket group ([`define_body_layout`]'s shapes); else `None`.
///
/// Laid out twice when it breaks. [`emit_define`] ends every line of a continued body with ` \`, and
/// those columns are the continuation's, not the layout's — measured against the whole width, a nested
/// group stays flat on the strength of two columns it does not own and the line as written overruns
/// §8.5 by exactly them (#93). A body that fits on one line takes no continuation and keeps the whole
/// width, which is why one measurement cannot serve both.
///
/// The second layout cannot return to the first's: a body that breaks at `width` breaks at anything
/// narrower.
///
/// This owns the reservation, so a caller passes the width the body's *first* line has and no less:
/// pre-subtracting would reserve the continuation twice.
fn format_define_body(body: &[Token], prefix_col: usize, width: usize) -> Option<String> {
    let measured = define_body_layout(body, prefix_col, width)?;
    if !measured.contains('\n') {
        return Some(measured);
    }
    define_body_layout(body, prefix_col, width.saturating_sub(CONTINUATION_WIDTH))
}

/// Three shapes: a body that is one whole call, one whole statement expression, and — #77's fourth
/// item — one whole bracket, which the structurer's group arm lays out with `\` continuations.
/// Anything else passes through: a body that is one bracket is not one construct, it is whatever the
/// macro's use makes of it, and §6 prefers passthrough — `a_define_body_that_is_one_whole_bracket_is_claimed`
/// records the shapes a claim regressed, and the nested-ternary two-cycle that still passes through.
///
/// **Whole-body** in all three shapes. A group with anything beside it would put the rest on the
/// line the group's own break ends, which is a layout this has no measure for — and claiming such a
/// body from its first two tokens while rendering only as far as the group is how `({ ... }) + 1` lost
/// its `+ 1` (#104).
fn define_body_layout(body: &[Token], prefix_col: usize, width: usize) -> Option<String> {
    let last = body.len().checked_sub(1)?;
    if contains_comment(body) {
        return None;
    }
    // A `\`-continued literal is one token whose *text* holds the newline (#110/#111), so the body's
    // rendered form already carries it — and `emit_define` re-splits at every `\n` to place the
    // continuations, putting its ` \` inside the literal. Re-lexing that back into the same token makes
    // it compound: `f("a\` + ` b")` became `f("a\ \` + ` b")`, then `"a\ \ \`, once per pass, and the
    // macro expands to different text each time (#117). `spans_lines` is the refusal
    // `is_boundable` and `build_bracketed_group` already make for the same reason.
    if spans_lines(body) {
        return None;
    }
    if is_call_head(body, 0) && match_bracket(body, 1) == Some(last) {
        return Some(structure(body, prefix_col, width, true));
    }
    // `format_stmt_expr` renders as far as the `})` and reports the `)` consumed, so a body with
    // anything after it had that tail dropped — `({ int t = (x); t; }) + 1` lost its `+ 1` and the
    // expansion changed on valid GNU C (#104). The `)` must close the body for the render to be it.
    if opens_stmt_expr(body, 0) && match_bracket(body, 0) == Some(last) {
        return format_stmt_expr(body, 0, 0, width, true).map(|(s, _)| s);
    }
    // #77's fourth item: a body that is one whole bracket — `((x) ? a : b ? c : d)` — is a
    // container, and the structurer's group arm lays it out with the continuation columns the
    // two-pass measurement above accounts for. Only the *whole* body: a group with anything
    // beside it passes through, the same whole-span rule the two retained shapes above keep
    // (#104). A bare chain is deliberately not claimed — parentheses are tokens the author did
    // not write, in a body whose expansion is the author's to control. A ternary nested inside an
    // arm's own bracket is refused too: its measured layout and the parameter list's explosion
    // decide against each other, and the two passes alternate (`((123 ? 0xff : (a)) ? (t))` at
    // width 40) — §6.
    let bracketing = group_bracketing(&body[0])?;
    if match_bracket(body, 0) == Some(last)
        && is_balanced(body)
        && !body.iter().any(|t| t.text == "{")
        && !has_nested_question(body)
        && build_bracketed_group(&body[1..last], bracketing).is_some()
    {
        return Some(structure(body, prefix_col, width, true));
    }
    None
}

/// Whether a `?` sits at paren depth 2 or deeper — inside an arm's, a condition's, a call's, or an
/// operand's paren, whose measured layout and the parameter list's explosion can decide against
/// each other in [`define_body_layout`]. Deliberately conservative: the two-cycle the guard exists
/// for is one of these shapes, and the rest are refused with it rather than probed one by one.
/// Reads parens only — a subscript's ternary does not nest the way that cycles. A `[`-outer body
/// seeds the depth at one, so the same shape the paren form refuses is refused in bracket form too.
/// The closer floors at zero — a malformed body must fall through to [`is_balanced`]'s refusal,
/// not panic.
fn has_nested_question(toks: &[Token]) -> bool {
    let mut depth = i32::from(toks[0].text == "[");
    toks.iter().any(|t| {
        match t.text {
            "(" => depth += 1,
            ")" => depth = depth.saturating_sub(1),
            "?" => return depth >= 2,
            _ => {}
        }
        false
    })
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
    in_define_body: bool,
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
    // A directive here too: the statements are joined one per line, and a directive joined onto the
    // statement after it swallows that statement into the directive — `#pragma pack(1)` + `int t = 1;`
    // came out as one line and `t` was then undeclared (#112).
    let unformattable = holds_unsafe_hash(inner, in_define_body)
        || inner
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
    in_define_body: bool,
    out: &mut String,
    col: &mut usize,
    width: usize,
) -> usize {
    let Some(close) = match_brace(toks, open) else {
        emit_str(out, col, toks[open].text);
        return open + 1;
    };
    let inner = &toks[open + 1..close];
    // The blanket `#` is load-bearing, not a stale copy of [`holds_directive`]: a `{}` list holding any
    // `#` is one whose spacing a later pass may rewrite, and narrowing this to a directive broke
    // idempotency on `{""/0AaA=#a*0_:…}` — laid out on pass 1, respaced on pass 2 (#112's review).
    let has_comment_or_directive = contains_comment(inner)
        || inner
            .iter()
            .any(|t| t.kind == TokenKind::Punct && t.text == "#");
    if has_comment_or_directive || !is_balanced(inner) || element_join_respaced(inner) {
        for tok in &toks[open..=close] {
            emit_str(out, col, tok.text);
        }
        return close + 1;
    }
    let doc = build_brace_doc(inner, padded);
    emit_doc(
        &doc,
        trailing_reserved(toks, close + 1, in_define_body),
        out,
        col,
        width,
    );
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
    in_define_body: bool,
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
    emit_tokens(body, out, col, depth, width, in_define_body);
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
/// Whether the reserve's span ends at `j` — the same three stops [`trailing_reserved`]'s loop makes:
/// a line break, a bracket or a `;`. Comments are not stops, and neither is a same-line comment's
/// own text.
fn ends_reserve(toks: &[Token], j: usize) -> bool {
    let t = &toks[j];
    match t.kind {
        TokenKind::Newline => true,
        TokenKind::LineComment | TokenKind::BlockComment => false,
        TokenKind::Punct if matches!(t.text, "(" | "[" | "{" | ";") => true,
        _ => t.text.contains('\n'),
    }
}

/// Whether the walk's call arm will attach the `(` across the newline at `nl`, dropping the gap —
/// the reserve then measures the attached form the next pass will see, or the two passes' reserves
/// differ by the call head's width and a fits/explode verdict flips on the pass that attaches
/// (#146). The same conditions the arm itself carries, so the prediction and the attach cannot
/// disagree.
fn closes_call_gap(toks: &[Token], nl: usize, in_define_body: bool) -> bool {
    let Some(open) = next_nontrivia(toks, nl + 1).filter(|&k| toks[k].text == "(") else {
        return false;
    };
    if !is_call_head_pair(toks, open) {
        return false;
    }
    let Some(close) = match_bracket(toks, open) else {
        return false;
    };
    let inner = &toks[open + 1..close];
    if contains_comment(inner) || !is_balanced(inner) {
        return false;
    }
    if !has_middle_newline(inner) {
        return !holds_unsafe_hash(inner, in_define_body);
    }
    let holds_hash = inner.iter().any(|t| matches!(t.text, "#" | "##"));
    !holds_hash && holds_forced_break(&build_call_body(inner, Fit::Measured))
}

fn trailing_reserved(toks: &[Token], from: usize, in_define_body: bool) -> usize {
    // `pending` holds the width of a whitespace run: it counts only once something follows it, since
    // whitespace ending the line never reaches the output — reserving for it would measure a line
    // this pass is about to shorten, and reach a different verdict than the next pass does.
    let (mut width, mut pending) = (0usize, 0usize);
    // The head [`super::tokens::operand_span`] would strip from this fragment: everything through
    // the last assignment, or a leading `return`. An operator in the head is not a chain break — a
    // chain is not cut inside an assignment's left side (#119) — and reading one here is what made
    // this reserve disagree with `loosest_cuts`, which sees the span with the head already gone
    // (#126). The search takes the same span the loop below measures: the three stops in
    // [`ends_reserve`], add a fourth to both.
    let head = (from..toks.len())
        .take_while(|&j| !ends_reserve(toks, j))
        .fold(None, |head, j| match head {
            _ if assigns(&toks[j]) => Some(j),
            None if toks[j].text == "return" => Some(j),
            _ => head,
        })
        .map_or(0, |j| j + 1);
    for (j, t) in toks.iter().enumerate().skip(from) {
        // A chain breaks after its operator as a bracket group breaks after its bracket: what
        // follows can land on a later line, so its flat width is not this construct's to reserve —
        // and once it has broken, the next pass measures a shorter run and decides differently.
        if is_chain_break(toks, j) && j >= head {
            return width + pending + display_width(t.text);
        }
        let counted = match t.kind {
            TokenKind::Newline => {
                if closes_call_gap(toks, j, in_define_body) {
                    continue;
                }
                break;
            }
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
                // The last token's trailing whitespace does not reach the output either — an
                // unterminated string or char literal carries it *inside* the token, so `pending`
                // never sees it, and `normalize_endings` trims it from the file's end (#102).
                None if j + 1 == toks.len() => t.text.trim_end(),
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
        assert_eq!(trailing_reserved(&toks, 0, false), 1 + TAB_WIDTH + 1 + 1);
    }

    #[test]
    fn trailing_reserved_stops_at_newline() {
        let toks = [tok(TokenKind::Newline, "\n"), tok(TokenKind::Punct, ";")];
        assert_eq!(trailing_reserved(&toks, 0, false), 0);
    }

    #[test]
    fn trailing_reserved_measures_the_call_head_across_a_newline() {
        // #146: the walk's call arm attaches `a(` across the newline, so the reserve measures the
        // attached form — `a` and the `(` — the same width the next pass's token stream reserves.
        let toks = [
            tok(TokenKind::Ident, "a"),
            tok(TokenKind::Newline, "\n"),
            tok(TokenKind::Punct, "("),
            tok(TokenKind::Punct, ")"),
        ];
        assert_eq!(trailing_reserved(&toks, 0, false), 2);
    }

    #[test]
    fn trailing_reserved_stops_at_a_newline_the_walk_keeps() {
        // A comment in the arguments refuses the attach (the walk passes the call through
        // verbatim), so the newline is the stop it always was.
        let toks = [
            tok(TokenKind::Ident, "a"),
            tok(TokenKind::Newline, "\n"),
            tok(TokenKind::Punct, "("),
            tok(TokenKind::BlockComment, "/*c*/"),
            tok(TokenKind::Punct, ")"),
        ];
        assert_eq!(trailing_reserved(&toks, 0, false), 1);
    }

    #[test]
    fn trailing_reserved_stops_at_the_statement_end() {
        // The `;` counts (1) and ends the reserve: what follows it is another statement's.
        let toks = [tok(TokenKind::Punct, ";"), tok(TokenKind::Punct, "(")];
        assert_eq!(trailing_reserved(&toks, 0, false), 1);
    }

    #[test]
    fn trailing_reserved_counts_punct_then_stops_at_bracket() {
        // ` {` of a function body: the space and brace count, and the brace stops the reserve.
        let toks = [tok(TokenKind::Whitespace, " "), tok(TokenKind::Punct, "{")];
        assert_eq!(trailing_reserved(&toks, 0, false), 2);
    }

    #[test]
    fn trailing_reserved_does_not_count_the_last_tokens_trailing_space() {
        let last = [tok(TokenKind::Unknown, "'x ")];
        assert_eq!(trailing_reserved(&last, 0, false), display_width("'x"));
        // The same whitespace in a token that is not the last reaches the output, and counts.
        let inner = [tok(TokenKind::Unknown, "'x "), tok(TokenKind::Ident, "y")];
        assert_eq!(trailing_reserved(&inner, 0, false), display_width("'x y"));
    }

    #[test]
    fn trailing_reserved_ignores_comments() {
        let toks = [
            tok(TokenKind::LineComment, "// hi"),
            tok(TokenKind::Punct, ";"),
        ];
        assert_eq!(trailing_reserved(&toks, 0, false), 1);
    }

    #[test]
    fn trailing_reserved_keeps_counting_past_an_assignments_left_side() {
        // An operator before the fragment's last `=` is in the assignment's left side, where a chain
        // does not break (#119), so the reserve must not stop there either — `loosest_cuts` reads the
        // span with the head already gone, and the reserve reading a break made the two disagree
        // (#126).
        let toks = [
            tok(TokenKind::Ident, "a"),
            tok(TokenKind::Whitespace, " "),
            tok(TokenKind::Operator, "|"),
            tok(TokenKind::Whitespace, " "),
            tok(TokenKind::Ident, "b"),
            tok(TokenKind::Whitespace, " "),
            tok(TokenKind::Punct, "="),
            tok(TokenKind::Whitespace, " "),
            tok(TokenKind::Ident, "c"),
            tok(TokenKind::Punct, ";"),
        ];
        assert_eq!(
            trailing_reserved(&toks, 0, false),
            display_width("a | b = c;")
        );
        // A `return` heads a fragment the same way, while no assignment has.
        let toks = [
            tok(TokenKind::Ident, "return"),
            tok(TokenKind::Whitespace, " "),
            tok(TokenKind::Ident, "a"),
            tok(TokenKind::Whitespace, " "),
            tok(TokenKind::Operator, "|"),
            tok(TokenKind::Whitespace, " "),
            tok(TokenKind::Ident, "b"),
            tok(TokenKind::Punct, ";"),
        ];
        assert_eq!(
            trailing_reserved(&toks, 0, false),
            display_width("return a |")
        );
        // After the last `=` the chain is the reserve's again.
        let toks = [
            tok(TokenKind::Ident, "a"),
            tok(TokenKind::Whitespace, " "),
            tok(TokenKind::Punct, "="),
            tok(TokenKind::Whitespace, " "),
            tok(TokenKind::Ident, "b"),
            tok(TokenKind::Whitespace, " "),
            tok(TokenKind::Operator, "|"),
            tok(TokenKind::Whitespace, " "),
            tok(TokenKind::Ident, "c"),
            tok(TokenKind::Punct, ";"),
        ];
        assert_eq!(trailing_reserved(&toks, 0, false), display_width("a = b |"));
    }
}
