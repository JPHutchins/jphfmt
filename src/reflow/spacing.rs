//! The §2.5 token-spacing pass: collapse inter-token whitespace runs, middle-align pointer `*`,
//! space C-style casts, K&R brace attach, and bit-field colons. Whitespace is semantically inert, so
//! this never changes meaning. Runs before structuring so the layout measures final widths
//! (otherwise a later space could widen a line and flip a fits/explode decision on the next pass,
//! breaking idempotency).
//!
//! Two gaps belong to nobody here, and a rule that writes one is a bug in every rule's shape:
//! the gap before a `\` line continuation, which the layout writes (`emit_define` writes ` \`), and
//! the gap before a comment, which [`collapse_runs`] preserves as the author positioned it (§2.1).
//! `reflow::tests::the_output_is_a_fixpoint_of_the_spacing_pass` catches the first, since the layout
//! disagrees about it; the second is a fixpoint either way, so only a fixture can hold it.

use super::tokens::{
    cast_tightens, closes_literal_type, heads_body, is_backslash, is_bit_field_colon,
    is_call_head_pair, is_callee_ident, is_control_keyword, is_decl_specifier, is_excluded_callee,
    is_qualifier, is_subscript, is_tag_keyword, is_trivia, is_type_context,
    padded_after_paren_open, ternary_open_before,
};
use crate::lexer::{Token, TokenKind, tokenize};

/// A significant token paired with the whitespace that preceded it.
type Piece<'src> = (String, Token<'src>);

fn same_line(gap: &str) -> bool {
    !gap.contains(['\n', '\r'])
}

/// The line breaks in a gap: a `\r\n`, a `\n` and a lone `\r` each end one line.
fn line_breaks(gap: &str) -> usize {
    gap.replace("\r\n", "\n").matches(['\n', '\r']).count()
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

/// The significant tokens of `s`, each paired with the trivia run that preceded it, and the trivia
/// run after the last one (the `trailing` half — line-ending whitespace, which no rule owns).
fn pieces_of(s: &str) -> (Vec<Piece<'_>>, String) {
    let mut pieces: Vec<Piece> = Vec::new();
    let mut gap = String::new();
    for t in tokenize(s) {
        if is_trivia(&t) {
            gap.push_str(t.text);
        } else {
            pieces.push((std::mem::take(&mut gap), t));
        }
    }
    (pieces, gap)
}

fn reassemble(pieces: &[Piece], trailing: &str) -> String {
    let mut out = String::with_capacity(
        pieces
            .iter()
            .map(|(g, t)| g.len() + t.text.len())
            .sum::<usize>()
            + trailing.len(),
    );
    for (g, t) in pieces {
        out.push_str(g);
        out.push_str(t.text);
    }
    out.push_str(trailing);
    out
}

/// A §2.5 pass, rewriting the gaps of a piece list in place.
type Pass = for<'src> fn(&mut [Piece<'src>]);

/// The §2.5 passes, in the order [`space_tokens`] runs them. The verify helper runs each one alone
/// over the layout's output, so the order matters only to [`space_pieces`] itself.
const PASSES: [(&str, Pass); 9] = [
    ("collapse_runs", collapse_runs),
    ("space_pointers", space_pointers),
    ("space_casts", space_casts),
    ("space_braces", space_braces),
    ("space_bit_fields", space_bit_fields),
    ("space_equals", space_equals),
    ("space_semicolons", space_semicolons),
    ("space_call_heads", space_call_heads),
    ("space_subscripts", space_subscripts),
];

/// Apply every pass in order to `pieces` — [`space_tokens`]'s body, shared with the verify helper
/// so the combined check and the feed are one spelling.
fn space_pieces(pieces: &mut [Piece]) {
    for (_, pass) in PASSES {
        pass(pieces);
    }
}

/// Apply the §2.5 token-spacing rules. Whitespace is semantically inert, so this never changes
/// meaning. [`collapse_runs`] goes first so every later rule sees a canonical one-space gap.
pub(super) fn space_tokens(s: &str) -> String {
    let (mut pieces, trailing) = pieces_of(s);
    space_pieces(&mut pieces);
    reassemble(&pieces, &trailing)
}

/// Which §2.5 pass would rewrite `s`'s gaps, if any, as `(name, its rewrite)`. The combined
/// [`space_tokens`] is checked first — the property the layout's output must hold, that the spacing
/// passes stop rewriting what it wrote — then each pass alone, since two passes canceling hides a
/// drift the combined form cannot see. `None` when `s` is a fixpoint of every pass.
///
/// Test support: the reflow module's fixture sweep, the property suites, and the emit-side guards'
/// spacing round all read the same spelling of the contract.
#[doc(hidden)]
pub fn first_respacing_pass(s: &str) -> Option<(&'static str, String)> {
    let (pieces, trailing) = pieces_of(s);
    let mut combined = pieces.clone();
    space_pieces(&mut combined);
    let after = reassemble(&combined, &trailing);
    if after != s {
        return Some(("space_tokens", after));
    }
    for (name, pass) in PASSES {
        let mut run = pieces.clone();
        pass(&mut run);
        let after = reassemble(&run, &trailing);
        if after != s {
            return Some((name, after));
        }
    }
    None
}

fn is_comment(t: &Token) -> bool {
    matches!(t.kind, TokenKind::LineComment | TokenKind::BlockComment)
}

/// Canonicalize one inter-token gap to a single space, keeping the indentation that follows its last
/// line break for `retab` and the line breaks themselves for `normalize_endings`. `keep_inline_run`
/// spares a same-line run that positions a comment (§2.1). `None` when the gap is already canonical,
/// which on formatted input is nearly every gap in the file.
fn collapse_gap(gap: &str, keep_inline_run: bool) -> Option<String> {
    match gap.rfind(['\n', '\r']) {
        None if keep_inline_run || gap.is_empty() => None,
        None => (gap != " ").then(|| " ".to_owned()),
        Some(last) => {
            let (breaks, indent) = gap.split_at(last + 1);
            (!breaks.chars().all(|c| matches!(c, '\n' | '\r'))).then(|| {
                breaks
                    .chars()
                    .filter(|c| matches!(c, '\n' | '\r'))
                    .chain(indent.chars())
                    .collect()
            })
        }
    }
}

/// Collapse the inter-token whitespace runs (§2.5): the alignment padding a no-column-alignment
/// formatter must not preserve. Line-one indentation is not an inter-token run, and the `#`-to-keyword
/// gap belongs to `scope_directives` — collapsing that one hands `emit_define` a prefix a column wider
/// than the one that reaches the output, flipping a body's fits/explode decision on the next pass.
fn collapse_runs(pieces: &mut [Piece]) {
    for j in 1..pieces.len() {
        let directive_hash =
            pieces[j - 1].1.text == "#" && (j == 1 || !same_line(&pieces[j - 1].0));
        if directive_hash {
            continue;
        }
        let keep_inline_run = is_comment(&pieces[j].1);
        if let Some(collapsed) = collapse_gap(&pieces[j].0, keep_inline_run) {
            pieces[j].0 = collapsed;
        }
    }
}

/// The innermost bracket still open at `j`.
fn enclosing_open(pieces: &[Piece], j: usize) -> Option<usize> {
    let mut depth = 0i32;
    (0..j).rev().find(|&k| match pieces[k].1.text {
        ")" | "]" | "}" => {
            depth += 1;
            false
        }
        "(" | "[" | "{" => {
            depth -= 1;
            depth < 0
        }
        _ => false,
    })
}

/// A token a declarator can be made of. `,` is one: a declaration may list several declarators, and
/// the type of the second belongs to the first.
fn declarator_shaped(t: &Token) -> bool {
    (t.kind == TokenKind::Ident && !is_excluded_callee(t.text))
        || matches!(t.text, "*" | "[" | "]" | ",")
}

/// The declarator run ending at `before`.
fn declaration_head<'a, 'src>(pieces: &'a [Piece<'src>], before: usize) -> &'a [Piece<'src>] {
    let start = (0..before)
        .rev()
        .find(|&k| !declarator_shaped(&pieces[k].1))
        .map_or(0, |k| k + 1);
    &pieces[start..before]
}

/// Whether the declarator run ending at `before` reads as a declaration: a declaration specifier, or
/// two or more identifiers separated only by `*` and `[]`.
fn declares_head(pieces: &[Piece], before: usize) -> bool {
    let head = declaration_head(pieces, before);
    head.iter().any(|p| is_decl_specifier(p.1.text))
        || head.iter().filter(|p| p.1.kind == TokenKind::Ident).count() >= 2
}

/// Whether the `(` at `open` heads a declaration's parameter list rather than a call's argument
/// list: `Ident * Ident` splits on that distinction and nothing else in the token stream does. The
/// two are structurally identical, so the verdict comes from what precedes the `(` — a declaration
/// specifier, or the bare `T name(` shape that a call's single callee cannot produce.
fn declares_parameters(pieces: &[Piece], open: usize) -> bool {
    declares_head(pieces, open)
}

/// Whether the `{` at `open` opens a block rather than an initializer list, whose elements are
/// expressions: the structure pass collapses an element's newline to a space, which would otherwise
/// let a multiply reach [`declares_pointer`] as a same-line run on the next pass. A top-level `=`
/// since the last top-level `;` marks an initializer, and so does a preceding `(T)` — a compound
/// literal reaches neither `=` nor a statement boundary in `return (T){…}` or `f((T){…})`.
///
/// The scan skips `(` and `[` interiors: an `=` or `;` the layout wrote inside a group — a
/// statement expression's `=;` — must not decide the verdict, or the pass that wrote it reads a
/// different answer than the pass that read the author's (#130). A brace is transparent: an
/// initializer's `=` must stay visible through it (`int m[] = {{a*b}, …}`), and its own elements
/// read the same statement level as it does.
///
/// A `{` directly closing a control header's or function definition's `)` is that construct's
/// block before any of the scan below. An opener met at depth zero closes no group to the left, so
/// its interior reaches past the brace: a statement body's enclosing group is a statement
/// expression whose opener must mask — the `=` assigning the whole expression is not the body's
/// verdict — and an expression body's is the layout's bounding group, transparent, the `=` left
/// of it deciding (#143).
fn opens_block(pieces: &[Piece], toks: &[Token], open: usize) -> bool {
    if opens_literal(toks, open) {
        return false;
    }
    if open
        .checked_sub(1)
        .is_some_and(|close| pieces[close].1.text == ")" && body_after_close(pieces, close))
    {
        return true;
    }
    let mut statement_body = None;
    let mut depth = 0i32;
    for k in (0..open).rev() {
        match pieces[k].1.text {
            ")" | "]" => depth = if depth < 0 { depth - 1 } else { depth + 1 },
            "(" | "[" => {
                depth = match depth {
                    0 if *statement_body
                        .get_or_insert_with(|| holds_statement_boundary(pieces, open)) =>
                    {
                        -1
                    }
                    0 => 0,
                    _ => depth - 1,
                };
            }
            ";" if depth == 0 => return true,
            "=" if depth == 0 => return false,
            _ => {}
        }
    }
    true
}

/// Whether the `)` at `close` ends a control header or function definition, making the `{` after
/// it that construct's body — the one spelling for [`opens_block`]'s early return and
/// [`space_braces`]'s K&R attach.
fn body_after_close(pieces: &[Piece], close: usize) -> bool {
    enclosing_open(pieces, close)
        .and_then(|o| o.checked_sub(1))
        .is_some_and(|before| heads_body(&pieces[before].1))
}

/// A `;` at the brace's own depth with content after it — a statement boundary, which an
/// initializer's elements cannot hold. A trailing `;`, the layout's magic trailing comma after
/// one, or a comment does not qualify, or the next pass reads its own additions as statements.
/// An `=` cannot tell them apart — an element is an assignment-expression, `int a[] = { x = 1 };`.
fn holds_statement_boundary(pieces: &[Piece], open: usize) -> bool {
    let mut depth = 0i32;
    let mut boundary = false;
    for piece in pieces.iter().skip(open + 1) {
        if depth == 0 && boundary && !is_comment(&piece.1) && !matches!(piece.1.text, "}" | ",") {
            return true;
        }
        match piece.1.text {
            "(" | "[" | "{" => depth += 1,
            ")" | "]" | "}" if depth == 0 => return false,
            ")" | "]" | "}" => depth -= 1,
            ";" if depth == 0 => boundary = true,
            _ => {}
        }
    }
    false
}

/// Whether the `{` at `open` follows a compound literal's `(T)`. The piece list is the token stream
/// minus trivia, so the shared predicate reads it directly.
fn opens_literal(toks: &[Token], open: usize) -> bool {
    open > 0 && toks[open - 1].text == ")" && closes_literal_type(toks, open - 1)
}

/// Whether the piece at `k` stands where a statement may: outside every bracket, or directly in a
/// block.
fn at_statement_level(pieces: &[Piece], toks: &[Token], k: usize) -> bool {
    enclosing_open(pieces, k)
        .is_none_or(|open| pieces[open].1.text == "{" && opens_block(pieces, toks, open))
}

/// Whether the type name at `name` opens a declaration, which makes a following `*` run a
/// declarator rather than a multiply. A statement boundary or declaration specifier settles it
/// outright; inside brackets, only a parameter list does.
fn declares_pointer(pieces: &[Piece], toks: &[Token], name: usize) -> bool {
    match name.checked_sub(1).map(|k| pieces[k].1.text) {
        None | Some(";" | "{" | "}") => at_statement_level(pieces, toks, name),
        Some("(" | ",") => enclosing_open(pieces, name).is_some_and(|open| {
            pieces[open].1.text == "("
                && open > 0
                && is_callee_ident(&pieces[open - 1].1)
                && declares_parameters(pieces, open)
        }),
        Some(text) => is_decl_specifier(text),
    }
}

/// Middle-align pointer `*` (§2.5: `T * p`, `T * * p`) — only the dereference operator clusters with
/// its operand. A `*` run is a declarator when a type keyword or `struct`/`union`/`enum` tag precedes
/// it, when a qualifier follows it (`*const` is no expression), or when a typedef name in declaration
/// position precedes it and a name follows; multiply, deref, and function pointers `(*f)` are left as
/// is (§6).
fn space_pointers(pieces: &mut [Piece]) {
    // One view of the piece list as tokens, for the predicates `tokens` owns; a `Token` is `Copy`, so
    // this neither borrows `pieces` nor is rebuilt per candidate.
    let toks: Vec<Token> = pieces.iter().map(|p| p.1).collect();
    let is_star = |t: &Token| t.kind == TokenKind::Punct && t.text == "*";
    let mut j = 0;
    while j < pieces.len() {
        if !(is_star(&pieces[j].1) && j > 0) {
            j = j.saturating_add(1);
            continue;
        }
        let mut k = j;
        while k + 1 < pieces.len() && is_star(&pieces[k + 1].1) {
            k = k.saturating_add(1);
        }
        let prev_is_type = is_type_context(pieces[j - 1].1.text)
            || (pieces[j - 1].1.kind == TokenKind::Ident
                && j >= 2
                && is_tag_keyword(pieces[j - 2].1.text));
        // `int *p, *q` — the second declarator's type is back past the comma.
        let continues_declarator = pieces[j - 1].1.text == "," && declares_head(pieces, j - 1);
        // What follows the run settles the verdict wherever it sits. Reading only a same-line
        // neighbour would make the verdict depend on where the breaks are, and the layout closes
        // breaks: a run that joined `a *` onto its name would hand the next pass a declarator this
        // one never saw, and that pass would respace it. Only the rewrite below is same-line — a
        // newline gap is not this pass's to close.
        let (next_is_qualifier, next_names_declarator) =
            pieces.get(k + 1).map_or((false, false), |after| {
                (is_qualifier(after.1.text), after.1.kind == TokenKind::Ident)
            });
        let typedef_declarator = pieces[j - 1].1.kind == TokenKind::Ident
            && !is_excluded_callee(pieces[j - 1].1.text)
            && next_names_declarator
            && declares_pointer(pieces, &toks, j - 1);
        // A run right after a `(` consults the shared after-paren pad verdict — the qualifier-run
        // pad with the cast override — instead of the bare qualifier term. The two must be one
        // spelling: [`space_casts`] tightens what a bare pad writes, and a lone pass that pads a
        // cast is the disagreement the layout's output must not hold (#178).
        let qualifier_pad = if j >= 1 && pieces[j - 1].1.text == "(" {
            padded_after_paren_open(&toks, j - 1, None, None)
        } else {
            next_is_qualifier
        };
        if prev_is_type || qualifier_pad || typedef_declarator || continues_declarator {
            for piece in pieces[j..=k].iter_mut().filter(|p| same_line(&p.0)) {
                piece.0 = " ".to_owned();
            }
            // A `\` is not a token to hug: it ends the line, and the space before one is the layout's
            // (`emit_define` writes ` \`). Tightening it against the run gave `struct s *\` where the
            // layout writes `struct s * \`, so the two passes spelled the same tokens differently —
            // a disagreement `format_with_width` is a fixpoint of only because the layout runs
            // second, and `reflow::tests` now asserts it does not happen.
            //
            // Nor is a comment. [`collapse_runs`] passes `keep_inline_run` for one precisely so its
            // gap survives as the author positioned it (§2.1), and clearing it here made the two
            // rules of a single pass disagree: `struct s * /* c */ x` came out `struct s */* c */ x`.
            if let Some(after) = pieces.get_mut(k + 1)
                && same_line(&after.0)
                && after.1.text != "\\"
                && !is_comment(&after.1)
            {
                // An `=`-led operator stays spaced: `*=` and `*==` re-lex as `*=` and the next
                // pass respaces what this one wrote (#121's search).
                after.0 = if after.1.kind == TokenKind::Ident || after.1.text.starts_with('=') {
                    " ".to_owned()
                } else {
                    String::new()
                };
            }
        }
        j = k.saturating_add(1);
    }
}

/// A C-style cast `(type) x` gets a space after the `)` (§2.5) and tight `(` (no space inside).
/// Conservative: the parenthesized group must be type-only and contain a type keyword (so a grouped
/// expression is never mistaken for one), be in a non-value position, and be followed by an operand.
fn space_casts(pieces: &mut [Piece]) {
    for open in 0..pieces.len() {
        if pieces[open].1.text != "(" {
            continue;
        }
        let Some(close) = piece_close_paren(pieces, open) else {
            continue;
        };
        let inner: Vec<Token> = pieces[open + 1..close].iter().map(|p| p.1).collect();
        // The verdict itself is [`cast_tightens`], shared with the collapse's pad mirror — #64 was
        // the two drifting apart, and without the `return` carve-out a cast is spaced only once the
        // layout's own bounding parenthesis has replaced `return` as the token before it — a verdict
        // that changes between runs. The follower's line stays this pass's own term.
        let prev = open.checked_sub(1).map(|before| &pieces[before].1);
        let after = pieces.get(close + 1).map(|a| &a.1);
        let followed_by_operand = pieces
            .get(close + 1)
            .is_some_and(|after| same_line(&after.0));
        if cast_tightens(&inner, prev, after) && followed_by_operand {
            // Tighten the `(`: strip a same-line gap after `(` so `( int)` -> `(int)`. No-op on
            // canonical `(int)`. (Stripping the gap before `)` was tried but broke idempotency on
            // barely-cast proptest input — the cast detector's verdict shifts across passes once
            // the close-side gap changes, so `space_semicolons` then disagrees with itself. Leave
            // the close-side gap alone; `(int )` is a rarer mutation and not worth the risk here.)
            if let Some(first_inner) = pieces.get_mut(open + 1)
                && same_line(&first_inner.0)
            {
                first_inner.0.clear();
            }
            pieces[close + 1].0 = " ".to_owned();
        }
    }
}

/// How [`space_braces`] spells a gap it attaches.
enum Attach {
    /// One space: a body's `{` against its head, and `else` or a do-while's `while` against the `}`
    /// before it.
    Spaced,
    /// None: a compound literal's `{` against its `(T)` (§8.4).
    Tight,
}

/// The attach verdict for the piece at `j` against the piece before it — the construct's head —
/// or `None` where the pair is not one a brace attaches. What precedes a `)`'s matching `(` decides
/// that `)`: a callee name or a control keyword opens a body, while `&`, `=`, `return` and every
/// other operator or statement keyword introduce a value. A `:` attaches only as a label's, which
/// only a statement may carry: a ternary's and a list element's are breaks the layout writes.
///
/// `else` and `do` open a body wherever they stand: the statement-level test below reads the brace
/// of an `=`-assigned statement expression as an initializer's, and would miss the ones inside it.
///
/// Where a statement may stand, a `{` after any other `)` or a name begins no statement of its own,
/// so it is the body of what precedes it — a declarator the heads above cannot read, like a macro
/// that spells a function's name or a function returning a function pointer, or an attribute before
/// the body.
fn attach_verdict(pieces: &[Piece], toks: &[Token], j: usize) -> Option<Attach> {
    let head = j - 1;
    match (pieces[head].1.text, pieces[j].1.text) {
        (")", "{") if body_after_close(pieces, head) => Some(Attach::Spaced),
        (")", "{") if closes_literal_type(toks, head) => Some(Attach::Tight),
        ("else" | "do" | "=", "{") | ("}", "else") => Some(Attach::Spaced),
        (":", "{")
            if !ternary_open_before(toks, head) && at_statement_level(pieces, toks, head) =>
        {
            Some(Attach::Spaced)
        }
        (_, "{") if opens_tag_body(pieces, j) => Some(Attach::Spaced),
        (text, "{")
            if (text == ")" || pieces[head].1.kind == TokenKind::Ident)
                && at_statement_level(pieces, toks, j) =>
        {
            Some(Attach::Spaced)
        }
        ("}", "while") if closes_do_body(pieces, head) => Some(Attach::Spaced),
        _ => None,
    }
}

/// Whether the `{` at `open` opens a `struct`, `union` or `enum` body: the nearest tag keyword before
/// it is reached past names alone — and, for an `enum`, the `:` of a fixed underlying type, the run
/// [`enum_body_brace`](super::tokens::enum_body_brace) reads forward from the keyword. A `struct` or
/// `union` takes at most a name.
fn opens_tag_body(pieces: &[Piece], open: usize) -> bool {
    (0..open)
        .rev()
        .find(|&k| {
            is_tag_keyword(pieces[k].1.text)
                || !(pieces[k].1.kind == TokenKind::Ident || pieces[k].1.text == ":")
        })
        .is_some_and(|tag| match pieces[tag].1.text {
            "enum" => true,
            "struct" | "union" => {
                open - tag <= 2 && pieces[tag + 1..open].iter().all(|p| p.1.text != ":")
            }
            _ => false,
        })
}

/// Whether the `}` at `close` closes a brace directly after `do` — a do-while's body, which makes the
/// `while` after it that statement's tail rather than a loop of its own. A body that is a braceless
/// statement (`do if (x) {…} while (0);`) is not read, and its `while` keeps the author's break.
fn closes_do_body(pieces: &[Piece], close: usize) -> bool {
    enclosing_open(pieces, close)
        .and_then(|open| open.checked_sub(1))
        .is_some_and(|before| pieces[before].1.text == "do")
}

/// Whether the piece at `k` sits on a preprocessor directive's logical line: its first significant
/// piece is a `#`, and a `\` splices the physical line after it in — one line break, so a blank line
/// after a `\` ends the logical line. A directive ends at its line end, so nothing may be attached
/// onto one — `#else⏎{` joined is `#else {`, and `#define X(y)⏎{` joined defines a different macro.
fn on_directive_line(pieces: &[Piece], k: usize) -> bool {
    let line_start = (0..=k)
        .rev()
        .find(|&m| starts_logical_line(pieces, m))
        .unwrap_or(0);
    directive_lines(&pieces[line_start..=k])
        .last()
        .is_some_and(|&directive| directive)
}

/// [`on_directive_line`] for every piece at once, in one pass — a pass that asks it per piece stays
/// linear on a long line (#193's review). A piece is on a directive's line from the `#` that begins
/// one ([`begins_directive`]) to that logical line's end.
fn directive_lines(pieces: &[Piece]) -> Vec<bool> {
    (0..pieces.len())
        .scan(false, |directive, m| {
            *directive =
                begins_directive(pieces, m) || *directive && !starts_logical_line(pieces, m);
            Some(*directive)
        })
        .collect()
}

/// [`super::tokens::begins_directive`] over pieces: the `#` at `m` begins a directive when the white
/// space before it holds a line end — a break no `\` splices, or a comment spanning lines — or when
/// nothing precedes it. The one reading the walk and the scoping take over tokens (#194).
fn begins_directive(pieces: &[Piece], m: usize) -> bool {
    pieces[m].1.text == "#"
        && (0..=m)
            .rev()
            .find_map(|k| {
                if k < m {
                    let piece = &pieces[k].1;
                    let spans_lines = piece.text.contains(['\n', '\r']);
                    if is_comment(piece) && spans_lines {
                        return Some(true);
                    }
                    let splices = is_backslash(piece)
                        && (k + 1..pieces.len())
                            .find(|&j| {
                                !same_line(&pieces[j].0) || !pieces[j].1.text.trim_end().is_empty()
                            })
                            .is_some_and(|j| !same_line(&pieces[j].0));
                    if !(piece.text.trim_end().is_empty() || is_comment(piece) || splices) {
                        return Some(false);
                    }
                }
                starts_logical_line(pieces, k).then_some(true)
            })
            .unwrap_or(true)
}

/// Whether the piece at `m` opens a logical line: the first piece, or one after a line break that
/// no `\` splices away — the last piece on the break's line that is not blank, read as
/// [`crate::lexer::splices`] trims blanks, with one line break, so a blank line after a `\` ends
/// the logical line. The scan stops at the line's start, so a run of blanks costs its own length.
fn starts_logical_line(pieces: &[Piece], m: usize) -> bool {
    m == 0
        || !(same_line(&pieces[m].0)
            || line_breaks(&pieces[m].0) == 1
                && (0..m)
                    .rev()
                    .find(|&k| !pieces[k].1.text.trim_end().is_empty() || !same_line(&pieces[k].0))
                    .is_some_and(|k| is_backslash(&pieces[k].1)))
}

/// K&R brace attach (§2.5): a brace goes on the line of the construct it belongs to — a body's `{`
/// on its head's line, and an `else` or a do-while's `while` on the line of the `}` before it —
/// whether the author broke the pair or not, the statement-expression `({` left alone. The one
/// §2.5 rule that closes a line break; a comment or a directive's line end between the pair keeps
/// it, since neither can be joined past.
fn space_braces(pieces: &mut [Piece]) {
    let toks: Vec<Token> = pieces.iter().map(|p| p.1).collect();
    for j in 1..pieces.len() {
        if !same_line(&pieces[j].0) && on_directive_line(pieces, j - 1) {
            continue;
        }
        match attach_verdict(pieces, &toks, j) {
            Some(Attach::Spaced) => pieces[j].0 = " ".to_owned(),
            Some(Attach::Tight) => pieces[j].0.clear(),
            None => {}
        }
    }
}

/// Bit-field colon spacing (§2.5: `x: 1` — no space before, one after). A `:` qualifies only when
/// it follows an identifier, precedes an integer literal, and no `?` opened a ternary earlier in
/// the statement (which would make it a ternary colon, not a bit-field).
fn space_bit_fields(pieces: &mut [Piece]) {
    // Projected once, not per `:`: the backward scan is over the whole prefix, so building it inside
    // the loop made a struct of many bit-fields quadratic.
    let toks: Vec<Token> = pieces.iter().map(|p| p.1).collect();
    for j in 1..pieces.len().saturating_sub(1) {
        let is_bit_field = pieces[j].1.text == ":"
            && is_bit_field_colon(&toks, j)
            && same_line(&pieces[j].0)
            && same_line(&pieces[j + 1].0);
        if is_bit_field {
            pieces[j].0.clear();
            pieces[j + 1].0 = " ".to_owned();
        }
    }
}

/// Normalize spacing around a single `=` (assignment, not `==`/`!=`/`<=`/`>=`/`+=` etc. which have
/// different text): exactly one space before and after, same-line only. Never before a `;` or a `,`,
/// which are separators every layout writes tight — the space `space_semicolons` exists to remove, and
/// the one a `{}` list would drop on the next pass. No-op on canonical input.
fn space_equals(pieces: &mut [Piece]) {
    for j in 0..pieces.len() {
        if pieces[j].1.kind == TokenKind::Punct && pieces[j].1.text == "=" {
            if same_line(&pieces[j].0) {
                pieces[j].0 = " ".to_owned();
            }
            if let Some(after) = pieces.get_mut(j + 1)
                && same_line(&after.0)
                && !matches!(after.1.text, ";" | ",")
            {
                after.0 = " ".to_owned();
            }
        }
    }
}

/// Strip trailing same-line whitespace before `;` at paren depth zero — a statement terminator,
/// wherever the statement lives, so a `;` inside a function body or a `struct` body qualifies.
/// Leaves `;` inside `()`/`[]` alone — the structure pass may collapse newlines to spaces inside such
/// constructs (e.g. parenthesized ternaries), and stripping those collapsed spaces would break
/// idempotency because the original newline-gap form survives (not same-line) but the collapsed
/// form does not. Also leaves newline gaps alone (structural breaks), and leaves gaps before
/// `;`/`{` alone (defensive guard for `for(;;)`-style patterns, though those gaps are empty
/// in canonical form). No-op on canonical input.
fn space_semicolons(pieces: &mut [Piece]) {
    let mut depth = 0i32;
    for j in 0..pieces.len() {
        match pieces[j].1.text {
            "(" | "[" => {
                depth += 1;
                continue;
            }
            ")" | "]" => {
                depth = (depth - 1).max(0);
                continue;
            }
            _ => {}
        }
        if depth != 0 {
            continue;
        }
        if j > 0
            && pieces[j].1.kind == TokenKind::Punct
            && pieces[j].1.text == ";"
            && same_line(&pieces[j].0)
            && !pieces[j].0.is_empty()
            && !matches!(pieces[j - 1].1.text, ";" | "{")
        {
            pieces[j].0.clear();
        }
    }
}

/// Normalize `ident (` spacing for call heads: non-excluded idents become tight (`foo(`), as does a
/// list after the group it applies to (`(*fp)(`, #191),
/// control-flow keywords and type keywords get exactly one space (`if (`, `int (*cb)`),
/// and other excluded callees (`sizeof`, `typeof`, `return`, etc.) are left as-is so we
/// don't fight the house style (e.g. golden.c has `sizeof(int)` tight).
fn space_call_heads(pieces: &mut [Piece]) {
    // Projected once, not per `(`: the backward scan is over the whole prefix.
    let toks: Vec<Token> = pieces.iter().map(|p| p.1).collect();
    let directive = directive_lines(pieces);
    for j in 0..pieces.len().saturating_sub(1) {
        let next_is_paren = pieces[j + 1].1.kind == TokenKind::Punct && pieces[j + 1].1.text == "(";
        if !same_line(&pieces[j + 1].0) || (next_is_paren && names_a_macro(pieces, j)) {
            continue;
        }
        // A directive's line keeps the author's `)(` gaps: in a `#define` a group may be the
        // parameters, the body, or text an expansion pastes into a call, none of it a list the line
        // spells (§6). The layout never walks a directive's own line, so only this pass reads one.
        if is_call_head_pair(&toks, j + 1) && !(pieces[j].1.text == ")" && directive[j]) {
            pieces[j + 1].0.clear();
        } else if pieces[j + 1].1.text == "("
            && (is_control_keyword(pieces[j].1.text) || is_type_context(pieces[j].1.text))
        {
            pieces[j + 1].0 = " ".to_owned();
        }
    }
}

/// Whether the token at `j` is the name in a `#define`, where the gap before a `(` is not spacing but
/// meaning: `#define X (y)` defines `X` as `(y)`, and `#define X(y)` a function-like macro taking `y`.
/// Neither spelling may become the other, so the author's gap stands exactly as written (§6).
///
/// Tightening it turned every object-like macro whose body is parenthesized into a function-like one, and
/// the output did not compile — the most common shape in the corpus that jphfmt got wrong, and invisible
/// to every check because the character it dropped was whitespace.
fn names_a_macro(pieces: &[Piece], j: usize) -> bool {
    // Past comments, which are pieces of their own: a comment is whitespace by the time the
    // preprocessor reads the line, so `#define /* c */ X (y)` defines exactly what `#define X (y)` does.
    let before = |k: usize| (0..k).rev().find(|&i| !is_comment(&pieces[i].1));
    before(j)
        .filter(|&k| pieces[k].1.text == "define")
        .and_then(before)
        .is_some_and(|k| pieces[k].1.text == "#")
}

/// A subscript is tight against what it indexes, exactly as a call is tight against its callee
/// (§2.5): `arr [i]` is `arr[i]`, which was the one pair of brackets §2.5 did not reach. Whether a
/// `[` indexes is [`is_subscript`], the one spelling shared with the layout's join refusal.
fn space_subscripts(pieces: &mut [Piece]) {
    // Projected once, not per `[`: the backward scan is over the whole prefix.
    let toks: Vec<Token> = pieces.iter().map(|p| p.1).collect();
    for (j, (gap, _)) in pieces.iter_mut().enumerate().skip(1) {
        if is_subscript(&toks, j) && same_line(gap) {
            gap.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn tmp_probe_130() {
        use crate::lexer::tokenize;
        for input in ["({=}){fx*f", "({\n\t=;\n}){fx*f"] {
            let mut pieces: Vec<Piece> = Vec::new();
            let mut gap = String::new();
            for t in tokenize(input) {
                if is_trivia(&t) {
                    gap.push_str(t.text);
                } else {
                    pieces.push((std::mem::take(&mut gap), t));
                }
            }
            let star = pieces.iter().position(|(_, t)| t.text == "*").unwrap();
            let name = star - 1;
            let enclosing = enclosing_open(&pieces, name);
            let statement_level = enclosing.is_none_or(|open| {
                pieces[open].1.text == "{" && opens_block(&pieces, &toks_view(&pieces), open)
            });
            let prev = pieces[name.saturating_sub(1)].1.text.to_string();
            eprintln!(
                "PROBE {input:?} star={star} prev={prev:?} enclosing={enclosing:?} statement_level={statement_level}"
            );
        }
    }

    fn toks_view<'src>(pieces: &[Piece<'src>]) -> Vec<crate::lexer::Token<'src>> {
        pieces.iter().map(|p| p.1).collect()
    }

    use super::*;

    #[test]
    fn the_three_splice_readings_agree() {
        // "Does the next line continue this one?" read over text, tokens and pieces (#190): each
        // snippet's second line is spliced onto its first by all three readings, or by none.
        for (src, spliced) in [
            ("a \\\nb", true),
            ("a \\ \t\nb", true),
            ("a \\\u{a0}\nb", true),
            ("a \\\\\nb", true),
            ("a \\\r\nb", true),
            ("a \\\rb", true),
            ("a \\ x\nb", false),
            ("a\nb", false),
            ("a \\ // c\nb", false),
            ("a \\\n\nb", false),
        ] {
            let lines: Vec<&str> = src.lines().flat_map(|line| line.split('\r')).collect();
            let toks = tokenize(src);
            let last_newline = toks
                .iter()
                .rposition(|t| t.kind == TokenKind::Newline)
                .unwrap_or(0);
            let (pieces, _) = pieces_of(src);
            let b = pieces.iter().position(|p| p.1.text == "b").unwrap_or(0);
            assert_eq!(
                lines[..lines.len() - 1]
                    .iter()
                    .all(|line| crate::lexer::splices(line)),
                spliced,
                "text: {src:?}"
            );
            assert_eq!(
                !super::super::tokens::ends_logical_line(&toks, last_newline),
                spliced,
                "tokens: {src:?}"
            );
            assert_eq!(!starts_logical_line(&pieces, b), spliced, "pieces: {src:?}");
        }
    }

    #[test]
    fn the_token_and_piece_readings_of_a_directive_start_agree() {
        // Whether a `#` begins a directive (#194), over tokens and over pieces: each snippet's last
        // `#` begins one by both readings, or by neither.
        for (src, directive) in [
            ("#if x", true),
            ("a;\n#if x", true),
            ("a;\n\t#if x", true),
            ("/* c */#if x", true),
            ("a;\n/* c */ #if x", true),
            ("a; /* b\n c */ #if x", true),
            ("a;\n\\\n#if x", true),
            ("a; #if x", false),
            ("a; /* c */ #if x", false),
            ("a; \\\n#if x", false),
            ("a; \\ \n#if x", false),
            ("#define S(x) \\\n#x", false),
        ] {
            let toks = tokenize(src);
            let hash = toks.iter().rposition(|t| t.text == "#").unwrap_or(0);
            let (pieces, _) = pieces_of(src);
            let piece = pieces.iter().rposition(|p| p.1.text == "#").unwrap_or(0);
            assert_eq!(
                super::super::tokens::begins_directive(&toks, hash),
                directive,
                "tokens: {src:?}"
            );
            assert_eq!(
                begins_directive(&pieces, piece),
                directive,
                "pieces: {src:?}"
            );
        }
    }

    #[test]
    fn same_line_newline() {
        assert!(!same_line("\n"));
        // A `\r`-only ending is a line break too, and `collapse_gap` copies it through, so the
        // spacing rules must not replace that gap with a space and merge the two lines.
        assert!(!same_line("\r"));
        assert!(!same_line("\r\n"));
    }

    #[test]
    fn same_line_space() {
        assert!(same_line(" "));
    }

    #[test]
    fn same_line_empty() {
        assert!(same_line(""));
    }

    #[test]
    fn same_line_multiple_chars() {
        assert!(same_line("a b"));
    }

    #[test]
    fn is_type_context_keyword() {
        assert!(is_type_context("int"));
        assert!(is_type_context("const"));
        assert!(is_type_context("unsigned"));
    }

    #[test]
    fn is_type_context_not_keyword() {
        assert!(!is_type_context("foo"));
        assert!(!is_type_context("size_t"));
    }

    #[test]
    fn space_semicolons_strips_trailing_ws() {
        // Depth-zero `;` has trailing whitespace stripped to canonical.
        assert_eq!(space_tokens("foo ;"), "foo;");
        assert_eq!(space_tokens("foo  ;"), "foo;");
        assert_eq!(space_tokens("foo\t;"), "foo;");
        assert_eq!(space_tokens("foo \t ;"), "foo;");
    }

    #[test]
    fn space_semicolons_strips_inside_braces() {
        // A `;` is a statement terminator wherever the statement lives: only `()`/`[]` are
        // excluded, so a function body and a `struct` body both canonicalize.
        assert_eq!(space_tokens("{ return x ; }"), "{ return x; }");
        assert_eq!(space_tokens("struct s { int x ; }"), "struct s { int x; }");
    }

    /// The gap the collapse leaves behind: its rewrite, or the original when it declines one.
    fn collapsed(gap: &str, keep_inline_run: bool) -> String {
        collapse_gap(gap, keep_inline_run).unwrap_or_else(|| gap.to_owned())
    }

    #[test]
    fn collapse_gap_same_line_run_becomes_one_space() {
        assert_eq!(collapsed("   ", false), " ");
        assert_eq!(collapsed("\t", false), " ");
        assert_eq!(collapsed(" \t ", false), " ");
    }

    #[test]
    fn collapse_gap_declines_an_already_canonical_gap() {
        // Formatted input is nearly all canonical gaps; none of them is rewritten.
        assert_eq!(collapse_gap(" ", false), None);
        assert_eq!(collapse_gap("", false), None);
        assert_eq!(collapse_gap("\n\t", false), None);
        assert_eq!(collapse_gap("\r\n", false), None);
        assert_eq!(collapse_gap("   ", true), None);
    }

    #[test]
    fn collapse_gap_empty_stays_empty() {
        assert_eq!(collapsed("", false), "");
    }

    #[test]
    fn collapse_gap_keeps_indentation_after_the_last_break() {
        // The run before a break is trailing whitespace and goes; the run after it is
        // indentation, left for `retab`.
        assert_eq!(collapsed("   \n\t\t", false), "\n\t\t");
        assert_eq!(collapsed("\n", false), "\n");
    }

    #[test]
    fn collapse_gap_drops_blank_line_padding() {
        assert_eq!(collapsed("  \n  \n\t", false), "\n\n\t");
    }

    #[test]
    fn collapse_gap_preserves_crlf() {
        // Line breaks are copied verbatim so `normalize_endings` still sees `\r\n`.
        assert_eq!(collapsed("  \r\n\t", false), "\r\n\t");
    }

    #[test]
    fn collapse_gap_keeps_an_inline_run_before_a_comment() {
        assert_eq!(collapsed("   ", true), "   ");
        // Only the same-line run is sacred; a run that ends a line still goes.
        assert_eq!(collapsed("   \n\t", true), "\n\t");
    }

    #[test]
    fn collapse_runs_leaves_line_one_indentation() {
        // The first piece's gap is indentation, not an inter-token run — collapsing it to one
        // space would leave a space-indented line after `retab`.
        assert_eq!(space_tokens("\t\tfoo"), "\t\tfoo");
    }

    #[test]
    fn collapse_runs_leaves_the_directive_hash_gap() {
        // `scope_directives` owns the `#`-to-keyword gap and rewrites it to the nesting depth.
        assert_eq!(space_tokens("#\t\tdefine A 1"), "#\t\tdefine A 1");
    }

    #[test]
    fn collapse_runs_collapses_a_declaration_run() {
        assert_eq!(space_tokens("static int   f"), "static int f");
    }

    #[test]
    fn space_semicolons_preserves_inside_parens() {
        // A `;` inside `()` is not stripped — the structure pass may collapse a
        // newline to a space inside such constructs, and stripping that space would
        // break idempotency (the original newline form survives, the collapsed
        // form would not).
        assert_eq!(space_tokens("(foo ;)"), "(foo ;)");
        assert_eq!(space_tokens("[foo ;]"), "[foo ;]");
    }

    #[test]
    fn space_semicolons_noop_on_canonical() {
        assert_eq!(space_tokens("foo;"), "foo;");
    }

    #[test]
    fn space_semicolons_preserves_newline_gap() {
        assert_eq!(space_tokens("foo\n;"), "foo\n;");
    }

    #[test]
    fn space_equals_normalizes_assignment() {
        assert_eq!(space_tokens("x=1"), "x = 1");
        assert_eq!(space_tokens("x\t=  1"), "x = 1");
    }

    #[test]
    fn space_equals_noop_on_comparison() {
        assert_eq!(space_tokens("a==b"), "a==b");
    }

    #[test]
    fn space_equals_noop_on_canonical() {
        assert_eq!(space_tokens("x = 1"), "x = 1");
    }

    #[test]
    fn space_call_heads_tightens_call() {
        assert_eq!(space_tokens("foo ("), "foo(");
        assert_eq!(space_tokens("foo\t("), "foo(");
    }

    #[test]
    fn space_subscripts_tightens_an_index() {
        assert_eq!(space_tokens("arr ["), "arr[");
        assert_eq!(space_tokens("arr\t["), "arr[");
        assert_eq!(space_tokens("m[i] ["), "m[i][");
        assert_eq!(space_tokens("f() ["), "f()[");
        assert_eq!(space_tokens("\"abc\" ["), "\"abc\"[");
        assert_eq!(space_tokens("p++ ["), "p++[");
        assert_eq!(space_tokens("q-- ["), "q--[");
        assert_eq!(space_tokens("{1, 2} ["), "{1, 2}[");
    }

    #[test]
    fn space_subscripts_leaves_an_attribute_alone() {
        // `int x [[deprecated]];` is valid C23, so the gap before `[[` is not a subscript's.
        assert_eq!(space_tokens("x [["), "x [[");
        // A designator follows a `{` or `,`, which end no value.
        assert_eq!(space_tokens("{ ["), "{ [");
        assert_eq!(space_tokens(", ["), ", [");
        // A keyword introduces a construct rather than naming a value.
        assert_eq!(space_tokens("return ["), "return [");
    }

    #[test]
    fn space_subscripts_leaves_a_newline_gap_alone() {
        assert_eq!(space_tokens("arr\n["), "arr\n[");
    }

    #[test]
    fn space_call_heads_spaces_control() {
        assert_eq!(space_tokens("if ("), "if (");
        assert_eq!(space_tokens("if\t("), "if (");
    }

    #[test]
    fn space_call_heads_leaves_a_macro_name_alone() {
        // The gap is the definition: object-like keeps its space, function-like keeps its tightness.
        assert_eq!(space_tokens("#define X (y)"), "#define X (y)");
        assert_eq!(space_tokens("#define F(x) x"), "#define F(x) x");
        // A call elsewhere on a `#define` line is still tightened.
        assert_eq!(space_tokens("#define F(x) g (x)"), "#define F(x) g(x)");
        // A comment is whitespace to the preprocessor, so it does not change what is being defined.
        assert_eq!(
            space_tokens("#define /* c */ X (y)"),
            "#define /* c */ X (y)"
        );
        assert_eq!(
            space_tokens("#/* c */ define Y (z)"),
            "#/* c */ define Y (z)"
        );
    }

    #[test]
    fn space_call_heads_leaves_sizeof() {
        // `sizeof(` tight — no-op (already canonical).
        assert_eq!(space_tokens("sizeof("), "sizeof(");
        // `sizeof (` with space — left as-is (not control-4, excluded callee).
        assert_eq!(space_tokens("sizeof ("), "sizeof (");
    }

    #[test]
    fn space_call_heads_spaces_type_keyword() {
        // `int (*cb)` house style: type keyword gets one space before `(`.
        assert_eq!(space_tokens("int(*cb)(void);"), "int (*cb)(void);");
        assert_eq!(space_tokens("int  (*cb)"), "int (*cb)");
    }

    #[test]
    fn space_casts_tightens_open_paren() {
        // `( int) x` -> `(int) x`: strip the same-line gap after `(` in a cast.
        assert_eq!(space_tokens("( int)x"), "(int) x");
        assert_eq!(space_tokens("(int) x"), "(int) x");
    }

    #[test]
    fn space_braces_attaches_a_labels_colon_only() {
        assert_eq!(space_tokens("case 1:\n{"), "case 1: {");
        assert_eq!(space_tokens("done:\n{"), "done: {");
        // The layout breaks after a ternary's `:` and a list element's, so neither is closed.
        assert_eq!(space_tokens("x = c ? a :\n{"), "x = c ? a :\n{");
        assert_eq!(space_tokens("x = {0:\n{}}"), "x = {0:\n{}}");
    }

    #[test]
    fn space_braces_attaches_while_to_a_do_body_only() {
        assert_eq!(space_tokens("do {\n}\nwhile (0);"), "do {\n} while (0);");
        assert_eq!(space_tokens("{\n}\nwhile (x) {\n}"), "{\n}\nwhile (x) {\n}");
    }

    #[test]
    fn space_braces_reads_a_tag_head_inside_a_group() {
        // A group holds no statement, so only the tag arm can read these heads.
        assert_eq!(
            space_tokens("sizeof(enum e : unsigned long\n{A})"),
            "sizeof(enum e : unsigned long {A})"
        );
        assert_eq!(
            space_tokens("sizeof(enum : int\n{A})"),
            "sizeof(enum : int {A})"
        );
        assert_eq!(
            space_tokens("sizeof(struct s x\n{A})"),
            "sizeof(struct s x\n{A})"
        );
    }
}
