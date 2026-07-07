//! The §2.5 token-spacing pass: middle-align pointer `*`, space C-style casts, K&R brace attach,
//! and bit-field colons. Whitespace is semantically inert, so this never changes meaning; only the
//! listed pairs are touched and everything else keeps its exact spacing. Runs before structuring so
//! the layout measures final widths (otherwise a later space could widen a line and flip a
//! fits/explode decision on the next pass, breaking idempotency).

use super::tokens::{is_excluded_callee, is_trivia, is_type_context};
use crate::lexer::{Token, TokenKind, tokenize};

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

    space_pointers(&mut pieces);
    space_casts(&mut pieces);
    space_braces(&mut pieces);
    space_bit_fields(&mut pieces);
    space_equals(&mut pieces);
    space_semicolons(&mut pieces);
    space_call_heads(&mut pieces);

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

/// A C-style cast `(type) x` gets a space after the `)` (§2.5) and tight `(` (no space inside).
/// Conservative: the parenthesized group must be type-only and contain a type keyword (so a grouped
/// expression is never mistaken for one), be in a non-value position, and be followed by an operand.
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

/// Normalize spacing around a single `=` (assignment, not `==`/`!=`/`<=`/`>=`/`+=` etc. which have
/// different text): exactly one space before and after, same-line only. No-op on canonical input.
fn space_equals(pieces: &mut [Piece]) {
    for j in 0..pieces.len() {
        if pieces[j].1.kind == TokenKind::Punct && pieces[j].1.text == "=" {
            if same_line(&pieces[j].0) {
                pieces[j].0 = " ".to_owned();
            }
            if let Some(after) = pieces.get_mut(j + 1)
                && same_line(&after.0)
            {
                after.0 = " ".to_owned();
            }
        }
    }
}

/// Strip trailing same-line whitespace before `;` at bracket depth zero. Leaves `;` inside
/// `()`/`[]`/`{}` alone — the structure pass may collapse newlines to spaces inside such
/// constructs (e.g. parenthesized ternaries), and stripping those collapsed spaces would break
/// idempotency because the original newline-gap form survives (not same-line) but the collapsed
/// form does not. Also leaves newline gaps alone (structural breaks), and leaves gaps before
/// `;`/`{`/`}` alone (defensive guard for `for(;;)`-style patterns, though those gaps are empty
/// in canonical form). No-op on canonical input.
fn space_semicolons(pieces: &mut [Piece]) {
    let mut depth = 0i32;
    for j in 0..pieces.len() {
        match pieces[j].1.text {
            "(" | "[" | "{" => {
                depth += 1;
                continue;
            }
            ")" | "]" | "}" => {
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
            && !matches!(pieces[j - 1].1.text, ";" | "{" | "}")
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
        let cur_is_ident = pieces[j].1.kind == TokenKind::Ident;
        let next_is_paren = pieces[j + 1].1.kind == TokenKind::Punct && pieces[j + 1].1.text == "(";
        if !(cur_is_ident && next_is_paren && same_line(&pieces[j + 1].0)) {
            continue;
        }
        match pieces[j].1.text {
            "if" | "for" | "while" | "switch" => pieces[j + 1].0 = " ".to_owned(),
            _ if is_type_context(pieces[j].1.text) => pieces[j + 1].0 = " ".to_owned(),
            _ if is_excluded_callee(pieces[j].1.text) => {}
            _ => pieces[j + 1].0.clear(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_line_newline() {
        assert!(!same_line("\n"));
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
    fn ternary_open_before_stops_at_semicolon() {
        // A `?` before a `;` is in a different statement, so no ternary is open at `j`.
        let pieces: [Piece; 4] = [
            (
                String::new(),
                Token {
                    kind: TokenKind::Punct,
                    text: "?",
                },
            ),
            (
                String::new(),
                Token {
                    kind: TokenKind::Punct,
                    text: ";",
                },
            ),
            (
                String::new(),
                Token {
                    kind: TokenKind::Ident,
                    text: "x",
                },
            ),
            (
                String::new(),
                Token {
                    kind: TokenKind::Punct,
                    text: ":",
                },
            ),
        ];
        assert!(!ternary_open_before(&pieces, 3));
    }

    #[test]
    fn ternary_open_before_unmatched_question() {
        let pieces: [Piece; 3] = [
            (
                String::new(),
                Token {
                    kind: TokenKind::Punct,
                    text: "?",
                },
            ),
            (
                String::new(),
                Token {
                    kind: TokenKind::Ident,
                    text: "x",
                },
            ),
            (
                String::new(),
                Token {
                    kind: TokenKind::Punct,
                    text: ":",
                },
            ),
        ];
        assert!(ternary_open_before(&pieces, 2));
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
    fn space_semicolons_preserves_inside_parens() {
        // A `;` inside `()` is not stripped — the structure pass may collapse a
        // newline to a space inside such constructs, and stripping that space would
        // break idempotency (the original newline form survives, the collapsed
        // form would not).
        assert_eq!(space_tokens("(foo ;)"), "(foo ;)");
        assert_eq!(space_tokens("(foo ;)"), "(foo ;)");
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
