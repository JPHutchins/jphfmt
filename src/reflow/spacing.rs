//! The §2.5 token-spacing pass: collapse inter-token whitespace runs, middle-align pointer `*`,
//! space C-style casts, K&R brace attach, and bit-field colons. Whitespace is semantically inert, so
//! this never changes meaning. Runs before structuring so the layout measures final widths
//! (otherwise a later space could widen a line and flip a fits/explode decision on the next pass,
//! breaking idempotency).

use super::tokens::{
    can_precede_cast, closes_literal_type, ends_value, heads_body, is_callee_ident,
    is_control_keyword, is_decl_specifier, is_excluded_callee, is_qualifier, is_tag_keyword,
    is_trivia, is_type_context, is_type_group, is_value_start, ternary_open_before,
};
use crate::lexer::{Token, TokenKind, tokenize};

/// A significant token paired with the whitespace that preceded it.
type Piece<'src> = (String, Token<'src>);

fn same_line(gap: &str) -> bool {
    !gap.contains(['\n', '\r'])
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
/// meaning. [`collapse_runs`] goes first so every later rule sees a canonical one-space gap.
pub(super) fn space_tokens(s: &str) -> String {
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

    collapse_runs(&mut pieces);
    space_pointers(&mut pieces);
    space_casts(&mut pieces);
    space_braces(&mut pieces);
    space_bit_fields(&mut pieces);
    space_equals(&mut pieces);
    space_semicolons(&mut pieces);
    space_call_heads(&mut pieces);
    space_subscripts(&mut pieces);

    let mut out = String::with_capacity(s.len());
    for (g, t) in &pieces {
        out.push_str(g);
        out.push_str(t.text);
    }
    out.push_str(&trailing);
    out
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
/// let a multiply reach [`declares_pointer`] as a same-line run on the next pass. An `=` since the
/// last `;` marks an initializer, and so does a preceding `(T)` — a compound literal reaches neither
/// `=` nor a statement boundary in `return (T){…}` or `f((T){…})`.
fn opens_block(pieces: &[Piece], toks: &[Token], open: usize) -> bool {
    !opens_literal(toks, open)
        && (0..open)
            .rev()
            .take_while(|&k| pieces[k].1.text != ";")
            .all(|k| pieces[k].1.text != "=")
}

/// Whether the `{` at `open` follows a compound literal's `(T)`. The piece list is the token stream
/// minus trivia, so the shared predicate reads it directly.
fn opens_literal(toks: &[Token], open: usize) -> bool {
    open > 0 && toks[open - 1].text == ")" && closes_literal_type(toks, open - 1)
}

/// Whether the type name at `name` opens a declaration, which makes a following `*` run a
/// declarator rather than a multiply. A statement boundary or declaration specifier settles it
/// outright; inside brackets, only a parameter list does.
fn declares_pointer(pieces: &[Piece], toks: &[Token], name: usize) -> bool {
    let enclosing = enclosing_open(pieces, name);
    let statement_level =
        enclosing.is_none_or(|open| pieces[open].1.text == "{" && opens_block(pieces, toks, open));
    match name.checked_sub(1).map(|k| pieces[k].1.text) {
        None | Some(";" | "{" | "}") => statement_level,
        Some("(" | ",") => enclosing.is_some_and(|open| {
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
            j += 1;
            continue;
        }
        let mut k = j;
        while k + 1 < pieces.len() && is_star(&pieces[k + 1].1) {
            k += 1;
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
        if prev_is_type || next_is_qualifier || typedef_declarator || continues_declarator {
            for piece in pieces[j..=k].iter_mut().filter(|p| same_line(&p.0)) {
                piece.0 = " ".to_owned();
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
        }
        j = k + 1;
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
        // `can_precede_cast` is the same rule `closes_type_paren` reads, negated. Sharing it is the
        // point: #64 was the two drifting apart, and without the `return` carve-out a cast is spaced
        // only once the layout's own bounding parenthesis has replaced `return` as the token before
        // it — a verdict that changes between runs.
        let prev_is_value = open
            .checked_sub(1)
            .is_some_and(|before| !can_precede_cast(&pieces[before].1));
        let followed_by_operand = pieces
            .get(close + 1)
            .is_some_and(|after| same_line(&after.0) && is_value_start(&after.1));
        if is_type_group(&inner) && !prev_is_value && followed_by_operand {
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

/// K&R brace attach: `) {` keeps one space (§2.5) for function and control bodies, but the tight
/// `({` statement-expression and `(type){...}` compound literal are left alone (§8.4). What precedes
/// the matching `(` decides it: a callee name or a control keyword opens a body, while `&`, `=`,
/// `return` and every other operator or statement keyword introduce a value.
fn space_braces(pieces: &mut [Piece]) {
    let toks: Vec<Token> = pieces.iter().map(|p| p.1).collect();
    for j in 1..pieces.len() {
        if pieces[j].1.text == "{" && pieces[j - 1].1.text == ")" && same_line(&pieces[j].0) {
            let function_or_control = enclosing_open(pieces, j - 1)
                .and_then(|open| open.checked_sub(1))
                .is_some_and(|before| heads_body(&pieces[before].1));
            if function_or_control {
                pieces[j].0 = " ".to_owned();
            } else if closes_literal_type(&toks, j - 1) {
                pieces[j].0.clear();
            }
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
            && pieces[j].1.kind == TokenKind::Punct
            && pieces[j - 1].1.kind == TokenKind::Ident
            && pieces[j + 1].1.kind == TokenKind::Number
            && same_line(&pieces[j].0)
            && same_line(&pieces[j + 1].0)
            && !ternary_open_before(&toks, j);
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

/// Normalize `ident (` spacing for call heads: non-excluded idents become tight (`foo(`),
/// control-flow keywords and type keywords get exactly one space (`if (`, `int (*cb)`),
/// and other excluded callees (`sizeof`, `typeof`, `return`, etc.) are left as-is so we
/// don't fight the house style (e.g. golden.c has `sizeof(int)` tight).
fn space_call_heads(pieces: &mut [Piece]) {
    for j in 0..pieces.len().saturating_sub(1) {
        let next_is_paren = pieces[j + 1].1.kind == TokenKind::Punct && pieces[j + 1].1.text == "(";
        if !(next_is_paren && same_line(&pieces[j + 1].0)) {
            continue;
        }
        if is_callee_ident(&pieces[j].1) {
            pieces[j + 1].0.clear();
        } else if is_control_keyword(pieces[j].1.text) || is_type_context(pieces[j].1.text) {
            pieces[j + 1].0 = " ".to_owned();
        }
    }
}

/// A subscript is tight against what it indexes, exactly as a call is tight against its callee
/// (§2.5): `arr [i]` is `arr[i]`, which was the one pair of brackets §2.5 did not reach.
///
/// Only a `[` that *indexes* qualifies. An attribute's `[[` opens a construct of its own and keeps its
/// gap — `int x [[deprecated]];` and `int arr[10] [[deprecated]];` are both valid C23 — and a `{}`
/// list's designator follows a `{` or `,`, which end no value ([`ends_value`]).
fn space_subscripts(pieces: &mut [Piece]) {
    for j in 1..pieces.len() {
        let indexes = pieces[j].1.kind == TokenKind::Punct
            && pieces[j].1.text == "["
            && pieces.get(j + 1).is_none_or(|next| next.1.text != "[")
            && ends_value(&pieces[j - 1].1);
        if indexes && same_line(&pieces[j].0) {
            pieces[j].0.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
