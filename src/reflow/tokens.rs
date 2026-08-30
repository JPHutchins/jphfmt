//! Pure token predicates and slice helpers shared across the reflow submodules. A leaf: depends
//! only on [`crate::lexer`], never on a sibling reflow module, so it can be unit-tested in isolation.

use crate::lexer::{Token, TokenKind};

/// An identifier that names a callee: an `Ident` that is neither a control/operator keyword
/// ([`is_excluded_callee`]) nor a type keyword ([`is_type_context`], after which `(` opens a
/// declarator group, not an argument list). The single predicate shared by [`is_call_head`] and
/// the reflow builders' trivia-tolerant `call_head_before`, so the two never diverge.
pub(super) fn is_callee_ident(t: &Token) -> bool {
    t.kind == TokenKind::Ident && !is_excluded_callee(t.text) && !is_type_context(t.text)
}

/// Whether `t` can end the value a `[` subscripts: an identifier that names something rather than
/// introducing a construct, a literal, or the `)`/`]` that closes one. Nothing else can be indexed, so
/// a `[` after anything else opens something other than a subscript — a `{}` list's designator, or an
/// attribute.
pub(super) fn ends_value(t: &Token) -> bool {
    match t.kind {
        TokenKind::Ident => is_callee_ident(t),
        TokenKind::Number | TokenKind::String | TokenKind::Char => true,
        // `)` closes a call or a parenthesized expression, `]` an earlier subscript, and `}` a
        // compound literal, which is an lvalue a subscript may index: `(int[]){1, 2}[0]`. Nothing
        // else in valid C puts a `}` before a `[` — a struct definition or a block there is a
        // syntax error, and an attribute's `[[` is excluded by its second bracket.
        TokenKind::Punct => matches!(t.text, ")" | "]" | "}"),
        // Postfix `++`/`--` end their operand, so `p++[i]` is `(p++)[i]`. The prefix forms cannot
        // appear here: `++arr[i]` puts the operator before the identifier, not before the `[`.
        // [`is_value_start`] carves the same two out for the mirror-image question.
        TokenKind::Operator => matches!(t.text, "++" | "--"),
        _ => false,
    }
}

/// A control keyword whose `(` heads a clause, not an argument list.
pub(super) fn is_control_keyword(text: &str) -> bool {
    matches!(text, "if" | "for" | "while" | "switch")
}

/// A token before a `(` whose `)` may be followed by a body brace: a function's own name, or a
/// control keyword. `return` and the other statement keywords introduce a value instead, so a `{`
/// after them opens a compound literal (§8.4), not a block.
pub(super) fn heads_body(t: &Token) -> bool {
    is_callee_ident(t) || is_control_keyword(t.text)
}

/// Whether `inner` spells a type and nothing else — the parenthesized `(struct s)` of a cast or of a
/// compound literal. A type keyword or tag must appear, so a grouped expression `(x)`, an attribute's
/// `(noreturn)`, or a parameter list is never mistaken for one.
pub(super) fn is_type_group(inner: &[Token]) -> bool {
    inner
        .iter()
        .filter(|t| !is_trivia(t))
        .any(|t| is_type_context(t.text) || is_tag_keyword(t.text))
        && spells_only_type_tokens(inner)
}

/// Whether every token in `inner` can appear in a type, without requiring one to prove it — the
/// shared half of [`is_type_group`] and [`closes_type_paren`].
fn spells_only_type_tokens(inner: &[Token]) -> bool {
    inner.iter().filter(|t| !is_trivia(t)).all(|t| {
        // `(` and `)` for a declarator inside the type — `(int (*)[10])` — but no keyword that
        // takes its own argument list, so `sizeof(int)` and an attribute stay expressions.
        (t.kind == TokenKind::Ident && !is_excluded_callee(t.text))
            || t.kind == TokenKind::Number
            || matches!(t.text, "*" | "[" | "]" | "(" | ")")
    })
}

/// A callee identifier ([`is_callee_ident`]) immediately followed by `(` (no intervening
/// whitespace) — a call or the structurally identical declaration parameter list.
pub(super) fn is_call_head(toks: &[Token], i: usize) -> bool {
    toks.get(i).is_some_and(is_callee_ident)
        && toks
            .get(i + 1)
            .is_some_and(|n| n.kind == TokenKind::Punct && n.text == "(")
}

/// A C type keyword or qualifier — a token after which a `*` is confidently a pointer declarator,
/// not a multiply, and after which `(` opens a declarator group, not a call's argument list. User
/// typedefs (idents) are excluded, so ambiguous `a*b`/`foo*p`/`foo(x)` pass through (§6).
pub(super) fn is_type_context(text: &str) -> bool {
    is_qualifier(text)
        || matches!(
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
        )
}

/// A type qualifier — a keyword that may follow a declarator's `*` but never a multiply's.
pub(super) fn is_qualifier(text: &str) -> bool {
    matches!(text, "const" | "volatile" | "restrict" | "_Atomic")
}

/// A keyword that introduces a `struct`/`union`/`enum` tag, after which an identifier names a type.
pub(super) fn is_tag_keyword(text: &str) -> bool {
    matches!(text, "struct" | "union" | "enum")
}

/// A keyword that can only introduce a declaration, so an `Ident *` after one is a declarator
/// rather than a multiply — the disambiguation `is_type_context` cannot make for a typedef name.
pub(super) fn is_decl_specifier(text: &str) -> bool {
    is_type_context(text)
        || is_tag_keyword(text)
        || matches!(
            text,
            "static"
                | "extern"
                | "register"
                | "inline"
                | "typedef"
                | "thread_local"
                | "_Thread_local"
                | "constexpr"
                | "_Noreturn"
                | "_Alignas"
                | "alignas"
        )
}

/// Keywords that take a `(` but are not calls whose arguments split on commas. `_Generic` is not
/// excluded: its associations are a comma list and explode exactly per §2.2.
pub(super) fn is_excluded_callee(name: &str) -> bool {
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
            | "_Noreturn"
            | "asm"
            | "__asm__"
            | "__asm"
    )
}

/// The `{` opening an `enum [tag] [: type] { ... }` body that begins at the `enum` keyword `i`,
/// or `None` if this `enum` does not introduce a body (a forward declaration or a variable use).
pub(super) fn enum_body_brace(toks: &[Token], i: usize) -> Option<usize> {
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
pub(super) fn next_paren(toks: &[Token], i: usize) -> Option<usize> {
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
pub(super) fn next_nontrivia(toks: &[Token], from: usize) -> Option<usize> {
    next_nontrivia_in(toks, from, toks.len())
}

/// The last non-trivia token index before `before`.
pub(super) fn prev_nontrivia(toks: &[Token], before: usize) -> Option<usize> {
    (0..before).rev().find(|&j| !is_trivia(&toks[j]))
}

/// The last token before `before` that carries meaning — trivia and comments skipped, so a commented
/// `f /* c */ (void)` still reads as the function `f`.
pub(super) fn prev_significant(toks: &[Token], before: usize) -> Option<usize> {
    (0..before)
        .rev()
        .find(|&j| !is_trivia(&toks[j]) && !contains_comment(&toks[j..=j]))
}

/// Whether the `)` at `close` closes a compound literal's type — the `(T)` of `(T){…}` — rather than
/// a parameter list, a `__attribute__` argument, or a declarator suffix, each of which can also put a
/// `)` before a body's `{`.
///
/// [`can_precede_cast`] is the whole test for what comes before the `(`, and it is the same question
/// a cast asks: an identifier there makes the `(` an argument list, so `defined(X)`, `sizeof(int)` and
/// `f(x)` all close no type. Spelling it out again here is what #109 cost — the hand-written guard
/// rejected a preceding *callee* but not a preceding excluded callee, so `#if !defined(X)` followed by
/// a block had its `{` taken for a literal's, and the block was laid out as an initializer list.
///
/// The one exception is the case that predicate exists to exclude: a `)` before the type. That one is
/// a declarator's — `int (*f)(void) {` — or a control header's, which introduces a statement, and a
/// statement may open with a literal: `if (x) (struct s){1, 2}.a;`. What heads *that* pair says which.
pub(super) fn closes_literal_type(toks: &[Token], close: usize) -> bool {
    match_open_paren(toks, close).is_some_and(|open| {
        names_literal_type(&toks[open + 1..close])
            && prev_significant(toks, open).is_none_or(|before| {
                can_precede_cast(&toks[before]) || closes_control_header(toks, before)
            })
    })
}

/// Whether `inner` names the type of a compound literal: [`is_type_group`], a lone identifier, or a tag
/// keyword opening the group.
///
/// Both relaxations are provable from the *position*, which is what makes them safe. `(x){…}` is an
/// expression in no C, so a single parenthesized name before a `{` is a typedef name. And nothing but a
/// type puts `struct`, `union` or `enum` first in a parenthesized group, so an anonymous
/// `(struct { int x; }){…}` needs no reading of the body it spells out — which [`is_type_group`] refuses,
/// since a `{` is not a token a type is made of (#95).
///
/// A cast cannot assume as much, which is why [`closes_type_paren`] keeps the stricter test:
/// `(count) & mask` stays binary.
fn names_literal_type(inner: &[Token]) -> bool {
    let mut named = inner.iter().filter(|t| !is_trivia(t));
    match named.next() {
        Some(first) if is_tag_keyword(first.text) => true,
        Some(first) => (is_callee_ident(first) && named.next().is_none()) || is_type_group(inner),
        None => false,
    }
}

/// Whether the `)` at `close` closes an `if`/`for`/`while`/`switch` header, so what follows it is the
/// statement that header governs rather than more of a declarator.
pub(super) fn closes_control_header(toks: &[Token], close: usize) -> bool {
    match_open_paren(toks, close)
        .and_then(|open| prev_nontrivia(toks, open))
        .is_some_and(|head| is_control_keyword(toks[head].text))
}

/// Whether the `}` at `close` closes a block — a function or statement body — rather than a value.
/// An initializer list and a compound literal's body each *end* a value, so what follows their `}`
/// goes on with the statement they sit in; only a block's `}` ends one.
///
/// An unmatched `}` reads as a block: nothing about the value it might close is known, and §6 prefers
/// the reading that leaves the tokens where the author put them.
pub(super) fn closes_block(toks: &[Token], close: usize) -> bool {
    match_open_brace(toks, close).is_none_or(|open| !opens_value(toks, open))
}

/// Whether the `{` at `open` opens a value rather than a block. What precedes it says which: the `=`
/// or `,` of the declaration an initializer belongs to, or the `(T)` of a compound literal. A nested
/// list's `{` follows the `{` of the list holding it and is a value exactly when that one is, walked
/// rather than recursed so that brace nesting cannot reach the stack.
///
/// Read past comments, not merely trivia: a `(T) /* c */ {…}` is the same literal, and stopping at the
/// comment would read its `}` as a block's.
fn opens_value(toks: &[Token], open: usize) -> bool {
    let mut brace = open;
    while let Some(k) = prev_significant(toks, brace) {
        match toks[k].text {
            "=" | "," => return true,
            ")" => return closes_literal_type(toks, k),
            "{" => brace = k,
            _ => return false,
        }
    }
    false
}

/// Whether the `)` at `close` closes a parenthesized *type* — a cast, or a compound literal's type —
/// so it ends no value and an operator after it takes one operand, not two.
///
/// Provable cases only. `(A) & b` cannot be told apart from a cast without knowing whether `A` names
/// a type, so the group must either spell a type keyword or tag ([`is_type_group`]) or end in a `*`,
/// which no expression can end with. `(count) & mask`, where the parentheses are merely redundant,
/// keeps its binary reading.
pub(super) fn closes_type_paren(toks: &[Token], close: usize) -> bool {
    match_open_paren(toks, close).is_some_and(|open| {
        let inner = &toks[open + 1..close];
        let ends_in_star = || {
            inner
                .iter()
                .rfind(|t| !is_trivia(t))
                .is_some_and(|t| t.text == "*")
        };
        spells_only_type_tokens(inner)
            && (is_type_group(inner) || ends_in_star())
            && prev_significant(toks, open).is_none_or(|before| {
                let t = &toks[before];
                can_precede_cast(t)
            })
    })
}

/// Whether `t` can precede a cast's `(`. Only an operator, a bracket or `return` can: an identifier
/// before the `(` makes it an argument list — a call, or `sizeof`/`alignof`/`typeof`, which take a
/// parenthesized type and yield a *value*, so `sizeof(int) & mask` is binary.
///
/// Read here to recognize a cast and negated in `space_casts` to space one. They were two conditions
/// in opposite polarity, and #64 is what their disagreement cost: the lexer has no keyword kind, so
/// `return` is an `Ident`, and only one of the two carved it out.
pub(super) fn can_precede_cast(t: &Token) -> bool {
    !matches!(t.text, ")" | "]") && (t.kind != TokenKind::Ident || t.text == "return")
}

/// Index of the `(` matching the `)` at `close`, or `None` if unbalanced.
pub(super) fn match_open_paren(toks: &[Token], close: usize) -> Option<usize> {
    matching_back(toks, close, "(", ")")
}

/// Index of the `{` matching the `}` at `close`, or `None` if unbalanced.
pub(super) fn match_open_brace(toks: &[Token], close: usize) -> Option<usize> {
    matching_back(toks, close, "{", "}")
}

/// The next non-trivia token index in `[from, end)`.
pub(super) fn next_nontrivia_in(toks: &[Token], from: usize, end: usize) -> Option<usize> {
    (from..end).find(|&j| !is_trivia(&toks[j]))
}

/// A line-continuation `\`.
pub(super) fn is_backslash(t: &Token) -> bool {
    t.kind == TokenKind::Punct && t.text == "\\"
}

/// One past the last token of the preprocessor directive starting at `start` (following `\` line
/// continuations).
pub(super) fn directive_end(toks: &[Token], start: usize) -> usize {
    let mut i = start;
    while i < toks.len() {
        let is_newline = toks[i].kind == TokenKind::Newline;
        let continued = is_newline && i > 0 && is_backslash(&toks[i - 1]);
        i += 1;
        if is_newline && !continued {
            break;
        }
    }
    i
}

/// Index of the `)`/`]` matching the bracket at `open`, or `None` if unbalanced.
pub(super) fn match_bracket(toks: &[Token], open: usize) -> Option<usize> {
    matching(toks, open, "(", ")").or_else(|| matching(toks, open, "[", "]"))
}

/// Index of the `}` matching the `{` at `open`, or `None` if unbalanced.
pub(super) fn match_brace(toks: &[Token], open: usize) -> Option<usize> {
    matching(toks, open, "{", "}")
}

fn matching(toks: &[Token], open: usize, lhs: &str, rhs: &str) -> Option<usize> {
    (toks.get(open).map(|t| t.text) == Some(lhs)).then_some(())?;
    paired(toks, lhs, rhs, open..toks.len())
}

fn matching_back(toks: &[Token], close: usize, lhs: &str, rhs: &str) -> Option<usize> {
    (toks.get(close).map(|t| t.text) == Some(rhs)).then_some(())?;
    paired(toks, rhs, lhs, (0..=close).rev())
}

/// The index at which `closes` balances the `opens` the walk starts on. One count in either direction:
/// forward from an opening bracket, or backward from a closing one, which is the same walk with the
/// pair read the other way round.
fn paired(
    toks: &[Token],
    opens: &str,
    closes: &str,
    mut order: impl Iterator<Item = usize>,
) -> Option<usize> {
    let mut depth = 0usize;
    order.find(|&j| {
        depth += usize::from(toks[j].text == opens);
        depth = depth.saturating_sub(usize::from(toks[j].text == closes));
        depth == 0 && toks[j].text == closes
    })
}

/// The tokens outside every bracket group, paired with their index — the level a construct's own
/// separators live at. The brackets themselves are not yielded; a construct is never separated by one.
pub(super) fn at_depth_zero<'a, 'src>(
    toks: &'a [Token<'src>],
) -> impl Iterator<Item = (usize, &'a Token<'src>)> {
    let mut depth = 0i32;
    toks.iter().enumerate().filter(move |(_, t)| {
        match t.text {
            "(" | "[" | "{" => depth += 1,
            ")" | "]" | "}" => depth -= 1,
            _ => return depth == 0,
        }
        false
    })
}

/// The spans `cuts` separates: before the first, between each pair, after the last. A cut token
/// belongs to no span — it is the separator, which the layout re-spells.
pub(super) fn segments_at<'a, 'src>(
    inner: &'a [Token<'src>],
    cuts: &[usize],
) -> Vec<&'a [Token<'src>]> {
    std::iter::once(0)
        .chain(cuts.iter().map(|&j| j + 1))
        .zip(cuts.iter().copied().chain(std::iter::once(inner.len())))
        .map(|(from, to)| &inner[from..to])
        .collect()
}

/// Split `inner` into segments at the depth-zero tokens for which `is_sep` holds.
pub(super) fn split_top_level<'a, 'src>(
    inner: &'a [Token<'src>],
    is_sep: impl Fn(&Token) -> bool,
) -> Vec<&'a [Token<'src>]> {
    split_top_level_with_cuts(inner, is_sep).0
}

/// [`split_top_level`], with the cut indices too — a caller that must judge the cut tokens
/// themselves (the ternary layout refuses a bit-field colon) reads one spelling instead of
/// re-collecting the cuts a second way.
pub(super) fn split_top_level_with_cuts<'a, 'src>(
    inner: &'a [Token<'src>],
    is_sep: impl Fn(&Token) -> bool,
) -> (Vec<&'a [Token<'src>]>, Vec<usize>) {
    let cuts: Vec<usize> = at_depth_zero(inner)
        .filter(|(_, t)| is_sep(t))
        .map(|(j, _)| j)
        .collect();
    (segments_at(inner, &cuts), cuts)
}

/// The next `;` at bracket depth zero at or after `from`.
pub(super) fn statement_end(toks: &[Token], from: usize) -> Option<usize> {
    at_depth_zero(&toks[from..])
        .find(|(_, t)| t.kind == TokenKind::Punct && t.text == ";")
        .map(|(j, _)| from + j)
}

/// Split one `{}` element at each designator that follows a complete value, so a brace-less
/// initializer macro keeps its own line: `PyVarObject_HEAD_INIT(a, b) .tp_name = "x"` is two items
/// juxtaposed without a comma, which is legal only here.
///
/// The gap is what says so. `f().field = v` with no gap is a member assignment, token-for-token the
/// same shape, so a designator written tight against the `)` is left as the author wrote it (§6).
pub(super) fn split_designators<'a, 'src>(element: &'a [Token<'src>]) -> Vec<&'a [Token<'src>]> {
    let cuts: Vec<usize> = at_depth_zero(element)
        .filter(|&(j, t)| {
            t.kind == TokenKind::Punct
                && t.text == "."
                && j > 0
                && is_trivia(&element[j - 1])
                && prev_nontrivia(element, j).is_some_and(|k| element[k].text == ")")
                && next_nontrivia(element, j + 1).is_some_and(|k| {
                    element[k].kind == TokenKind::Ident
                        && next_nontrivia(element, k + 1).is_some_and(|eq| element[eq].text == "=")
                })
        })
        .map(|(j, _)| j)
        .collect();
    std::iter::once(0)
        .chain(cuts.iter().copied())
        .zip(cuts.iter().copied().chain(std::iter::once(element.len())))
        .map(|(from, to)| &element[from..to])
        .collect()
}

/// Whether joining `inner` onto fewer lines would hand a later pass a shape it respaces: a `;` that
/// `space_semicolons` tightens, or the `Ident : Number` that `space_bit_fields` reads as a bit-field.
/// The gap the layout writes is a space, and those rules take it away again, so laying this out
/// would make the pass's output a fixpoint of a different pass rather than of itself.
///
/// Both are invalid C in a `{}` list, which is the only place this is asked — an initializer, an
/// `enum` body, a compound literal — so refusing them costs no real code its layout (§6).
/// Whether a ternary's `?` is still open at `j`: the nearest `?`, `;` or brace before it is a `?`.
/// What `space_bit_fields` asks before reading an `Ident : Number` as a bit-field, and what
/// [`respaced_when_joined`] must ask the same way — at any depth, since that rule does not track one.
pub(super) fn ternary_open_before(toks: &[Token], j: usize) -> bool {
    toks[..j]
        .iter()
        .rev()
        .find_map(|t| match t.text {
            "?" => Some(true),
            ";" | "{" | "}" => Some(false),
            _ => None,
        })
        .unwrap_or(false)
}

/// Whether an element's first token is a separator — a `;`, `,` or `:` cannot open an operand, and
/// laying such an element out would put a gap before it that the spacing pass tightens on the next
/// pass. The one spelling for the call, brace and ternary layouts' refusal (#121's search).
pub(super) fn opens_with_separator(toks: &[Token]) -> bool {
    next_nontrivia(toks, 0).is_some_and(|k| matches!(toks[k].text, ";" | "," | ":"))
}

/// Whether the `(` at `j` follows a callee identifier — the pair `space_call_heads` tightens. The
/// one spelling for that pass and for [`respaced_when_joined`], whose joined pair the tightening
/// would respace (#121's search).
pub(super) fn is_call_head_pair(toks: &[Token], j: usize) -> bool {
    toks[j].kind == TokenKind::Punct
        && toks[j].text == "("
        && prev_nontrivia(toks, j).is_some_and(|k| is_callee_ident(&toks[k]))
}

/// Whether the `[` at `j` indexes a value — the shape `space_subscripts` tightens. An attribute's
/// `[[` opens a construct of its own and keeps its gap. The one spelling for that pass and for
/// [`respaced_when_joined`], whose joined pair the tightening would respace (#121's search).
pub(super) fn is_subscript(toks: &[Token], j: usize) -> bool {
    toks[j].kind == TokenKind::Punct
        && toks[j].text == "["
        && next_nontrivia(toks, j + 1).is_none_or(|k| toks[k].text != "[")
        && prev_nontrivia(toks, j).is_some_and(|k| ends_value(&toks[k]))
}

/// Whether the gap between the `*` before `j` and the token at `j` is one `space_pointers` respaces:
/// a declarator star's gap to a non-identifier is tightened — `* :` respaces to `*:` — and a layout
/// that writes the space back hands the spacing pass a shape it rewrites. The followers that pass
/// keeps spaced are excluded: an identifier (its other branch — `* p` is the canonical declarator
/// spelling), an `=`-led token (`*=` re-lexes as a compound assignment and the next pass respaces
/// what this one wrote), and the two it refuses to hug at all — a `\` and a comment. Every other
/// follower is refused only where a declarator verdict could fire: the star run's true predecessor
/// decides — a type keyword, a tag's identifier, a comma whose head reads as a declaration, or a
/// span start (a comma-list declarator's head is outside the span) makes a declarator possible,
/// while a value predecessor is provably a multiply and joins freely (#121's search).
pub(super) fn star_gap_respaced(toks: &[Token], j: usize) -> bool {
    let star = prev_nontrivia(toks, j).filter(|&k| toks[k].text == "*");
    // `int **` walks to `int`, the same walk `space_pointers`'s run handling makes.
    let mut before = star;
    while let Some(k) = before.filter(|&k| toks[k].text == "*") {
        before = prev_nontrivia(toks, k);
    }
    let declarator_possible = before.is_none_or(|k| {
        is_type_context(toks[k].text)
            || (toks[k].kind == TokenKind::Ident
                && prev_nontrivia(toks, k).is_some_and(|t| is_tag_keyword(toks[t].text)))
            || (toks[k].text == "," && comma_declares(toks, k))
    });
    !is_trivia(&toks[j])
        && toks[j].kind != TokenKind::Ident
        && !toks[j].text.starts_with('=')
        && toks[j].text != "\\"
        && !is_comment(&toks[j])
        && declarator_possible
        && star.is_some()
}

/// [`star_gap_respaced`]'s `,`-predecessor question, the span-local half of `space_pointers`'
/// `declares_head`: the head before the comma reads as a declaration — a specifier, or two or more
/// identifiers separated only by declarator tokens. A head that runs past the span's start is
/// truncated and refused (§6), so the verdict cannot silently narrow.
fn comma_declares(toks: &[Token], comma: usize) -> bool {
    let declarator_shaped = |t: &Token| {
        (t.kind == TokenKind::Ident && !is_excluded_callee(t.text))
            || matches!(t.text, "*" | "[" | "]" | ",")
    };
    let mut k = comma;
    loop {
        while k > 0 && is_trivia(&toks[k - 1]) {
            k -= 1;
        }
        if k == 0 || !declarator_shaped(&toks[k - 1]) {
            break;
        }
        k -= 1;
    }
    // The walk stopped at a non-declarator boundary token, or ran into the span's start — the
    // latter is an unknown head, refused.
    if k == 0 {
        return true;
    }
    let head: Vec<&Token> = toks[k..comma].iter().filter(|t| !is_trivia(t)).collect();
    head.iter().any(|t| is_decl_specifier(t.text))
        || head.iter().filter(|t| t.kind == TokenKind::Ident).count() >= 2
}

/// Whether the `:` at `j` is the bit-field colon `space_bit_fields` reads — an identifier before,
/// a number after, no ternary `?` still open. The one spelling for a question asked by that pass, by
/// [`respaced_when_joined`], and by the ternary layout, which must not separate the same pair with
/// ` : ` and leave a later pass writing `: ` (#64's class). The cheap shape checks come before the
/// backward scan. A label whose statement opens with a number reads the same way at a span start;
/// the ternary layout refuses it like any other (§6), and the spacing pass canonicalizes its colon
/// like any other's.
pub(super) fn is_bit_field_colon(toks: &[Token], j: usize) -> bool {
    prev_nontrivia(toks, j).is_some_and(|k| toks[k].kind == TokenKind::Ident)
        && next_nontrivia(toks, j + 1).is_some_and(|k| toks[k].kind == TokenKind::Number)
        && !ternary_open_before(toks, j)
}

/// Whether a line break sits in the trivia run directly before `j` — a trivia run is more than one
/// token, a `Newline` then the next line's indentation, so a break is looked for across the whole
/// run, not just the token adjacent to the punctuator.
fn broken_before(toks: &[Token], j: usize) -> bool {
    toks[..j]
        .iter()
        .rev()
        .take_while(|t| is_trivia(t))
        .any(|t| t.text.contains(['\n', '\r']))
}

fn broken_after(toks: &[Token], j: usize) -> bool {
    toks[j + 1..]
        .iter()
        .take_while(|t| is_trivia(t))
        .any(|t| t.text.contains(['\n', '\r']))
}

pub(super) fn respaced_when_joined(inner: &[Token]) -> bool {
    joined_pair_respaced(inner, false, false)
}

/// The depth-zero reading of [`respaced_when_joined`]: a nested break is the nested group's own to
/// refuse — its handler writes the canonical tight form — so a hit below depth zero would freeze the
/// enclosing container for nothing. Callers whose collapse joins only the span's own breaks ask this
/// one; a caller that joins every break, nested included, asks [`respaced_when_joined`].
pub(super) fn respaced_when_joined_top(inner: &[Token]) -> bool {
    joined_pair_respaced(inner, true, false)
}

/// The joins the element fallback's collapse writes wrong — a space a later pass respaces — where
/// the group and call arms already write the canonical tight form and take no refusal of their own:
/// the depth-zero bit-field colon, a `*` whose gap to a non-identifier a declarator verdict would
/// tighten (ambiguous locally, so §6 refuses it), and a `;` the collapse puts a space before, which
/// `space_semicolons` strips. The top-level reading, braces included, since a nested construct
/// refuses its own breaks (#121's search).
pub(super) fn element_join_respaced(toks: &[Token]) -> bool {
    joined_pair_respaced(toks, true, true)
}

fn joined_pair_respaced(inner: &[Token], top_only: bool, canonical_joins: bool) -> bool {
    // The question is only ever about a break, so a span with none is answered without the scan —
    // the element fallback asks it of every element, the formatter's hot path.
    if !inner.iter().any(|t| t.kind == TokenKind::Newline) {
        return false;
    }
    let broken_before = |j: usize| broken_before(inner, j);
    let broken_after = |j: usize| broken_after(inner, j);
    // Two depths: the join arms read every bracket pair, braces included — a break inside a nested
    // `{}` list is the nested list's own to refuse — while the `;`-branch mirrors
    // `space_semicolons`'s own depth, which counts parens and brackets but not braces.
    let mut brackets = 0i32;
    let mut parens = 0i32;
    for (j, t) in inner.iter().enumerate() {
        // Read before the depth update: `[` and `(` open a level themselves, so their joins read at
        // the level they join from. A `[` whose join the subscript rule tightens — `0\n[]` joined to
        // `0 []` respaces to `0[]` — and a `(` whose join the call-head rule tightens — `A\n(` joined
        // to `A (` respaces to `A(` — are both the same class (#121's search). A `*` whose break to a
        // following operator is joined — `*\n<` joined to `* <` respaces to `*<` when the star reads
        // as a declarator's — the same class, one star over. The element callers' group and call
        // arms join the subscript and call-head shapes to the canonical tight form, so those two
        // arms are not theirs.
        if (!top_only || brackets == 0) && broken_before(j) && star_gap_respaced(inner, j) {
            return true;
        }
        if !canonical_joins
            && (!top_only || brackets == 0)
            && broken_before(j)
            && (is_subscript(inner, j) || is_call_head_pair(inner, j))
        {
            return true;
        }
        // `space_bit_fields` reads at any depth, so a colon whose join it would tighten is refused at
        // any depth in the all-depth reading.
        if (!top_only || brackets == 0)
            && t.kind == TokenKind::Punct
            && t.text == ":"
            && (broken_before(j) || broken_after(j))
            && is_bit_field_colon(inner, j)
        {
            return true;
        }
        match t.text {
            "(" | "[" => {
                parens += 1;
                brackets += 1;
            }
            ")" | "]" => {
                parens -= 1;
                brackets -= 1;
            }
            "{" => brackets += 1,
            "}" => brackets -= 1,
            _ => {}
        }
        if parens != 0 || t.kind != TokenKind::Punct {
            continue;
        }
        // `space_semicolons` leaves a `;` that opens its line alone, and never tightens one that
        // follows a `;` or a `{`.
        if t.text == ";"
            && broken_before(j)
            && prev_nontrivia(inner, j).is_some_and(|k| !matches!(inner[k].text, ";" | "{"))
        {
            return true;
        }
    }
    false
}

/// Split `inner` on commas at bracket depth zero.
pub(super) fn split_on_commas<'a, 'src>(inner: &'a [Token<'src>]) -> Vec<&'a [Token<'src>]> {
    split_top_level(inner, |t| t.kind == TokenKind::Punct && t.text == ",")
}

/// Whether `text` is one of the comparison operators — the class whose chain a #52 conjunct is.
/// Spelled by the class table itself, not a second list of the same operators.
pub(super) fn is_comparison(text: &str) -> bool {
    CHAIN_CLASSES[5].contains(&text) || CHAIN_CLASSES[6].contains(&text)
}

/// Binary operator classes in C precedence order, lowest first — the order a chain breaks in, since
/// the loosest binding is the one whose operands read as the elements of the container.
///
/// `*` is absent, and so is unary `&`: `T * p` and `T ** p` are declarators, and a declaration is
/// never a chain to break (§6). A `*` chain therefore splits on whatever looser operator it contains,
/// or stays flat.
const CHAIN_CLASSES: [&[&str]; 10] = [
    &["||"],
    &["&&"],
    &["|"],
    &["^"],
    &["&"],
    &["==", "!="],
    &["<", "<=", ">", ">="],
    &["<<", ">>"],
    &["+", "-"],
    &["/", "%"],
];

/// Whether the operator at `j` binds two operands rather than one: `a - b` splits, `a + -b` and
/// `&x` do not. A keyword that takes an expression (`return`, `sizeof`) does not end a value.
///
/// Both sides are checked. An operator with nothing a value can start with after it — `A / = x`, the
/// shape a bounded chain leaves behind when its own operators move inside the parentheses — is not
/// binary however well its left side reads, and splitting there put the `=` in its own segment.
fn is_binary_position(inner: &[Token], j: usize) -> bool {
    if !next_nontrivia(inner, j + 1).is_some_and(|k| is_value_start(&inner[k])) {
        return false;
    }
    prev_nontrivia(inner, j).is_some_and(|k| {
        // The same question [`ends_value`] answers, with one refinement: a `)` that closes a *type*
        // ends no value, so `(PyObject *) &x` is an address-of rather than a bitwise-and, and the
        // same holds for the `-`/`+` a cast can precede.
        ends_value(&inner[k]) && !(inner[k].text == ")" && closes_type_paren(inner, k))
    })
}

/// Whether `t` can begin a value: an operand, an opening bracket, or a unary prefix. An assignment,
/// a closer and a separator cannot.
pub(super) fn is_value_start(t: &Token) -> bool {
    match t.kind {
        TokenKind::Ident | TokenKind::Number | TokenKind::String | TokenKind::Char => true,
        // C11 §6.5.2-6.5.3: a primary expression or a unary prefix. `[` and `{` open a subscript
        // and a brace list, neither of which begins a value. A `#` stands for a directive
        // interleaved into the expression, and taking it as a start is what keeps the operator
        // before it binary.
        TokenKind::Punct => matches!(t.text, "(" | "-" | "+" | "!" | "~" | "*" | "&" | "#"),
        // `&&label` is GNU C's label address, an operand like any other.
        TokenKind::Operator => matches!(t.text, "++" | "--" | "&&"),
        TokenKind::Newline
        | TokenKind::Whitespace
        | TokenKind::Unknown
        | TokenKind::LineComment
        | TokenKind::BlockComment => false,
    }
}

/// Whether the token at `j` is an operator a chain breaks after — so what follows it can land on a
/// later line, exactly as the contents of a bracket group can.
pub(super) fn is_chain_break(toks: &[Token], j: usize) -> bool {
    matches!(toks[j].kind, TokenKind::Operator | TokenKind::Punct)
        && CHAIN_CLASSES.iter().any(|c| c.contains(&toks[j].text))
        && is_binary_position(toks, j)
}

/// The depth-zero indices where the loosest-binding class present binds two operands, in one pass: a
/// cut in a looser class discards the cuts collected for a tighter one, since only the loosest is the
/// container this chain breaks as.
fn loosest_cuts(inner: &[Token]) -> Vec<usize> {
    use std::cmp::Ordering;
    // Never inside an assignment's left side. `=` binds looser than every class here, which is why
    // it is absent from them and why [`operand_span`] reads its left side as a head — so an operator
    // there is not one of these operands' separators, and cutting at one spells `0/a = A & A` as a
    // `/` chain. That is #43: the parentheses the layout writes around `A & A` send the whole
    // assignment back through this split on the *next* pass, where a cut the first pass never made
    // moved the break.
    //
    // Only the whole-span callers reach it — `super::builders`' `build_expr_doc` and
    // `build_clause_contents`. `build_chain_doc` strips the head with [`operand_span`] before
    // calling, so the slice it passes holds no depth-zero assignment and this is a no-op there.
    //
    // [`operand_span`]'s other head, a leading `return`, is deliberately not mirrored: a `return`
    // heads a span only by leading it, so there is never an operator before one to discard — while
    // discarding on any depth-zero `return` would let a stray one mid-span kill a valid chain.
    //
    // An assignment resets the accumulator, which is the restriction stated as the one pass it is:
    // those operators are in its left side, and no later token can put them back. One fold, so no
    // intermediate list of candidates is built for a rule that only ever keeps the loosest.
    let nothing = (CHAIN_CLASSES.len(), Vec::new());

    at_depth_zero(inner)
        .fold(nothing, |(loosest, mut cuts), (j, t)| {
            if assigns(t) {
                return (CHAIN_CLASSES.len(), Vec::new());
            }
            let Some(class) = matches!(t.kind, TokenKind::Operator | TokenKind::Punct)
                .then(|| CHAIN_CLASSES.iter().position(|c| c.contains(&t.text)))
                .flatten()
                .filter(|_| is_binary_position(inner, j))
            else {
                return (loosest, cuts);
            };
            match class.cmp(&loosest) {
                Ordering::Less => (class, vec![j]),
                Ordering::Equal => {
                    cuts.push(j);
                    (loosest, cuts)
                }
                Ordering::Greater => (loosest, cuts),
            }
        })
        .1
}

/// Split `inner` into the operands of its loosest-binding binary operator run at depth zero, paired
/// with the operator that trails each. `None` when there is no such run, or when a depth-zero `?`/`:`
/// says the layout is a ternary's (§2.4), a label's or a bit-field's — collapsing a line break past a
/// `:` would leave `space_bit_fields` a same-line `Ident : Number` to reinterpret on the next pass.
pub(super) fn split_chain<'a, 'src>(
    inner: &'a [Token<'src>],
) -> Option<(Vec<&'a [Token<'src>]>, Vec<&'src str>)> {
    // A depth-zero `,` is a list, not a chain: its parts are not this operator's operands.
    if has_top_level(inner, "?") || has_top_level(inner, ":") || has_top_level(inner, ",") {
        return None;
    }
    let cuts = loosest_cuts(inner);
    if cuts.is_empty() {
        return None;
    }
    let segments = segments_at(inner, &cuts);
    // An operator missing an operand is not a chain: rendering the empty segment would leave the
    // separator stranded, and the space it lands beside is not this pass's to keep. A segment that
    // opens with a separator strands it the same way — `a + ;:` — and the spacing pass would tighten
    // the gap this layout wrote before it (#121's search).
    //
    // A separator whose preceding token is a `*` needs no refusal of its own here: a `*` ends no
    // value ([`ends_value`]), so an operator after one is never a binary cut ([`is_binary_position`])
    // and the shape never becomes a segment boundary — `x * < y` refuses on the cuts being empty,
    // before this looks. The star-gap refusal lives where such a separator can be written: the
    // ternary's arms and the break-join's ([`star_gap_respaced`]).
    segments
        .iter()
        .all(|s| has_non_trivia(s) && !opens_with_separator(s))
        .then(|| (segments, cuts.iter().map(|&j| inner[j].text).collect()))
}

pub(super) fn is_trivia(t: &Token) -> bool {
    matches!(t.kind, TokenKind::Whitespace | TokenKind::Newline)
}

/// Whether `toks[i]` opens a GNU statement expression — the `({` of `({ int t = x; t; })`.
pub(super) fn opens_stmt_expr(toks: &[Token], i: usize) -> bool {
    toks.get(i)
        .is_some_and(|t| t.kind == TokenKind::Punct && t.text == "(")
        && toks
            .get(i + 1)
            .is_some_and(|n| n.kind == TokenKind::Punct && n.text == "{")
}

/// Whether `toks` holds any non-trivia token — a segment worth emitting as its own element rather
/// than dropping as empty.
pub(super) fn has_non_trivia(toks: &[Token]) -> bool {
    toks.iter().any(|t| !is_trivia(t))
}

pub(super) fn is_comment(t: &Token) -> bool {
    matches!(t.kind, TokenKind::LineComment | TokenKind::BlockComment)
}

pub(super) fn contains_comment(toks: &[Token]) -> bool {
    toks.iter().any(is_comment)
}

/// Whether `()`, `[]`, and `{}` are all balanced (never negative, net zero) in `toks`. Unbalanced
/// inner brackets defeat depth-aware splitting, so such a construct is unstructurable and is passed
/// through verbatim rather than risk mis-splitting (which could accumulate commas across passes).
pub(super) fn is_balanced(toks: &[Token]) -> bool {
    let (mut paren, mut brack, mut brace) = (0i32, 0i32, 0i32);
    for t in toks {
        if t.kind != TokenKind::Punct {
            continue;
        }
        match t.text {
            "(" => paren += 1,
            ")" => paren -= 1,
            "[" => brack += 1,
            "]" => brack -= 1,
            "{" => brace += 1,
            "}" => brace -= 1,
            _ => {}
        }
        if paren < 0 || brack < 0 || brace < 0 {
            return false;
        }
    }
    paren == 0 && brack == 0 && brace == 0
}

/// Whether the punctuator `text` appears at bracket depth zero in `inner`.
pub(super) fn has_top_level(inner: &[Token], text: &str) -> bool {
    at_depth_zero(inner).any(|(_, t)| t.kind == TokenKind::Punct && t.text == text)
}

/// Whether a `?` ternary operator appears at bracket depth zero in `inner`.
pub(super) fn has_top_level_question(inner: &[Token]) -> bool {
    has_top_level(inner, "?")
}

/// An assignment operator: `=` and the compound forms, but not a comparison.
pub(super) fn assigns(t: &Token) -> bool {
    (t.kind == TokenKind::Punct && t.text == "=")
        || (t.kind == TokenKind::Operator
            && t.text.ends_with('=')
            && !matches!(t.text, "==" | "!=" | "<=" | ">="))
}

/// Where an expression's operands begin: after the last depth-zero assignment, or — with no
/// assignment anywhere — after the first depth-zero `return`. That head is not part of the
/// expression, so the parentheses [`super::builders`] adds bound the operands alone.
///
/// One pass, because a later assignment always wins and a `return` only counts while nothing has:
/// the fold's own state is what says so, where asking twice would walk the span twice to answer it.
pub(super) fn operand_span(toks: &[Token]) -> usize {
    at_depth_zero(toks)
        .fold(None, |head, (j, t)| match head {
            _ if assigns(t) => Some(j),
            None if t.text == "return" => Some(j),
            _ => head,
        })
        .map_or(0, |j| j + 1)
}

/// Whether more than one `?` appears at bracket depth zero — a ternary *chain*, `a ? b : c ? d : e`.
/// Counting the `?` rather than the arms is what distinguishes one from a single ternary sharing its
/// span with a depth-zero `:` that opens no arm: a bit-field's width (`int f : c ? 1 : 2`) or a
/// labeled statement (`done: p ? x() : y()`) splits into three arms and is still one conditional.
pub(super) fn is_ternary_chain(inner: &[Token]) -> bool {
    at_depth_zero(inner)
        .filter(|(_, t)| t.kind == TokenKind::Punct && t.text == "?")
        .nth(1)
        .is_some()
}

/// Whether any significant token's own text spans lines — an unterminated literal, which the lexer
/// runs to the end of the file. A one-line width cannot describe it, so no layout may be decided from
/// a span holding one.
pub(super) fn spans_lines(toks: &[Token]) -> bool {
    toks.iter()
        .any(|t| !is_trivia(t) && t.text.contains(['\n', '\r']))
}

/// Whether `toks` holds a preprocessor directive's `#`. The lines a directive spans are not the
/// construct's to lay out — its own column belongs to `scope_directives`, and the tokens on either
/// side of it are the preprocessor's alternatives rather than one expression. A handler that measures
/// across one writes the `#` mid-line, and the output does not compile (#112):
///
/// ```c
/// if (a == 1 #if defined(X) || b == 2 #endif) {
/// ```
///
/// §6's passthrough is the whole answer, exactly as it is for a comment ([`contains_comment`]).
///
/// What separates it from the stringize `#` of `foo(#x, y)` — an ordinary call whose arguments still
/// lay out — is what *follows* it: a directive names one, a line marker puts a number there
/// (`# 42 "gen.c"` — the preprocessor-output form GNU emits, not C11 §6.10.4's `#line`), or nothing
/// follows it on its line (the null directive). `##` is an
/// [`TokenKind::Operator`] and never reaches the test.
///
/// Position cannot answer this. Which line a `#` is on is what the layout decides, so a predicate that
/// reads it is not a fixpoint of the pass that owns it — and reading it cost one: the first form of this
/// asked whether the `#` began its line, so a group holding `A[A&#0` was laid out on pass 1, which put
/// the `#` at a line start, and refused on pass 2. That is #43's defect in the guard whose whole reason
/// is that a directive's column belongs to a later pass.
///
/// Neither a block comment nor a `\` continuation separates a `#` from its name: phase 2 splices the
/// continuation — the *name* too, so a `#include` split across one is still `#include` — and phase 3
/// makes the comment
/// whitespace, both before phase 4 reads the directive.
///
/// This is deliberately more inclusive than `scope_directives::parse_directive`, which scans for the
/// keyword textually and so does not see `# /* c */ if` as one. The two disagreeing is safe only in this
/// direction: this one refuses to lay a construct out and the scope pass declines to indent it, where the
/// reverse would write a `#` mid-line. Not a divergence to copy into the scope pass without deciding what
/// `# /* c */ if` should do about depth.
///
/// **The list is audited, not complete**, and that is now tolerable: #118 gave the call sites the
/// context this lacks, so the list is consulted only where a stringize can occur — inside a
/// `#define` replacement list ([`holds_unsafe_hash`]). Everywhere else any `#` refuses, which is
/// where the twelfth missing name bit and the reason it no longer can.
///
/// **Both errors remain possible inside a define body, and they are not symmetric.** A name this
/// does not know is a false negative and writes a `#` mid-line — the defect itself — so
/// [`names_directive`] must stay complete. A false positive costs only layout: `#define STR(define)
/// f(#define, …)` names a parameter that is not a keyword, so its stringize reads as a directive
/// and the argument list passes through instead of breaking. §6 prefers that direction, which is
/// why the test is a name list rather than "any `#`".
pub(super) fn holds_directive(toks: &[Token]) -> bool {
    toks.iter().enumerate().any(|(i, t)| {
        t.kind == TokenKind::Punct && t.text == "#" && opens_directive(&toks[i + 1..])
    })
}

/// Whether laying `toks` out could write a directive's `#` mid-line, the answer #118's context
/// provides: outside a `#define` replacement list a stringize cannot occur, so *any* `#` is unsafe
/// and [`holds_directive`]'s name list — open-ended, three rounds of missing names — is not
/// consulted. Inside one a `#` may be a stringize, so only a directive the list names refuses;
/// a `#param` stays laid out (§2.5).
pub(super) fn holds_unsafe_hash(toks: &[Token], in_define_body: bool) -> bool {
    if in_define_body {
        holds_directive(toks)
    } else {
        holds_hash_fragment(toks)
    }
}

/// Whether any token is a `#` or `##` fragment — the shape every laid path guards, since its lines
/// are not the layout's to own. One spelling for the call arm and for the reserve's attach
/// prediction.
pub(super) fn holds_hash_fragment(toks: &[Token]) -> bool {
    toks.iter().any(|t| matches!(t.text, "#" | "##"))
}

/// Whether a chain's head holds a shape the two passes measure differently. Pass 1's single
/// lookahead flattens through every construct in the head; pass 2 lays each out one handler at a
/// time, each reserve stopping at the next bracket. They agree unless the head holds a
/// *breakable* construct a bracket deep — a chain operator or `?`, a `,` list a builder actually
/// breaks (a call's arguments, a brace list — not a comma operator in a group), or a call —
/// which refuses, unless the call sits directly in the head's outermost group or subscript
/// and no chain operator or `?` marks that bracket — before or after the call — whose author's
/// brackets read back verbatim; or a second construct after a breakable first —
/// `f(x)(y)`, a double assignment's `…] = f(x) =` — which pass 1's parens would turn into pass
/// 2's second construct and refuse. Unbreakable nested content — a cast `(size_t)i`, parens
/// around an atom `(a)`, a subscript chain `a[0][1]`, a call through a group `(*fp)(x)` —
/// measures the same on both passes and is allowed (§6, #108's review). One pass; returns at
/// the first offending bracket.
pub(super) fn holds_head_split(toks: &[Token]) -> bool {
    #[derive(Clone, Copy, PartialEq)]
    enum Kind {
        /// A call's `(` — breakable whatever its content is.
        Call,
        /// A group's `(` — a cast's or a parenthesized expression's, whose comma is an operator.
        Group,
        /// A `[` subscript — its comma is an operator too, and no builder breaks it as a list.
        Subscript,
        /// A `{` brace list, whose comma a builder does break.
        Brace,
    }
    let mut frames: Vec<(Kind, bool, bool)> = Vec::new();
    let mut close_seen = false;
    let mut closed_breakable = false;
    for (i, t) in toks.iter().enumerate().filter(|(_, t)| !is_trivia(t)) {
        match t.text {
            "(" | "[" | "{" => {
                // A second construct after a breakable first: pass 1's lookahead crosses it
                // where pass 2's reserve stops at the bracket.
                if close_seen && closed_breakable {
                    return true;
                }
                let kind = match t.text {
                    "(" if is_call_head_pair(toks, i) => Kind::Call,
                    "[" if is_subscript(toks, i) => Kind::Subscript,
                    "(" | "[" => Kind::Group,
                    _ => Kind::Brace,
                };
                frames.push((kind, false, false));
            }
            ")" | "]" | "}" => {
                let (kind, chain_marked, has_call) =
                    frames.pop().unwrap_or((Kind::Group, false, false));
                let breakable = chain_marked || has_call || kind == Kind::Call;
                // A breakable construct a bracket deep refuses — the head's own outermost bracket
                // is the one construct the boundary covers. The exemption: a call directly inside
                // the head's outermost group or subscript — `(f(x)) + b =` and `x[f(y)] + z =`
                // re-parse as themselves: the author's brackets read back verbatim, so pass 1's
                // operand parens never become pass 2's second construct (the force-allow build is
                // stable and compliant). A chain operator or `?` in the call's own arguments, or
                // in the enclosing bracket before the call — `arr[f(a | b)]`, `arr[a | f(y)]` —
                // is the deep class: pass 1's lookahead crosses the call's bracket where pass 2's
                // reserve stops, so it stays refused. Neither mark is set by a `,` list — the
                // call's own builder breaks it the same way on both passes.
                let candidate = kind == Kind::Call
                    && !chain_marked
                    && frames.len() == 1
                    && !frames[0].1
                    && (frames[0].0 == Kind::Group || frames[0].0 == Kind::Subscript);
                if breakable && !frames.is_empty() && !candidate {
                    return true;
                }
                // The candidate's breakability moves to the enclosing frame — its close leaves
                // `closed_breakable` armed, so an open or `?` after the bracket is still the
                // second-construct class (`(f(x))(y)`, `(f(x)) ? a : b`). A chain mark that
                // arrives *after* the call — `arr[f(y) | a]` — marks the frame instead, and its
                // close refuses: the same two-pass disagreement as the mark-before order.
                if candidate {
                    frames.last_mut().unwrap().2 = true;
                }
                if chain_marked && has_call {
                    return true;
                }
                close_seen = true;
                closed_breakable = breakable;
            }
            // A ternary after a construct at the head's own level: pass 1's operand parens
            // become pass 2's second construct and the gate refuses its own output. A `,` marks
            // only a brace list — a call's arguments are the call's own builder, broken the same
            // way on both passes, so a call stays exemptible with them; a group's and a
            // subscript's commas are operators no layout splits. A `?` marks any enclosing
            // frame: a ternary in the head's bracket, before or after the call, alternates once
            // the operands grow (`x[a ? f(y) : b] =` at w=19-21) — the walk's ternary arms and
            // the head's own groups measure the call against different budgets.
            "?" | "," => {
                if frames.is_empty() {
                    // A ternary after a *breakable* construct at the head's own level: pass 1's
                    // operand parens become pass 2's second construct. After an unbreakable one
                    // the head re-parses as itself — `(a) ? b : c =` and `x[0] ? a : b =` are the
                    // base's stable class.
                    if close_seen && closed_breakable && t.text == "?" {
                        return true;
                    }
                } else if let Some((kind, breakable, _)) = frames.last_mut()
                    && (t.text == "?" || *kind == Kind::Brace)
                {
                    *breakable = true;
                }
            }
            _text => {
                if is_chain_break(toks, i) {
                    // A depth-zero binary chain operator after any close passes: the head
                    // re-parses as itself — the closed construct's brackets are the author's and
                    // read back verbatim, so pass 1's operand parens cannot become pass 2's
                    // second construct. Only an open or a `?` after a breakable close refuses.
                    if !frames.is_empty()
                        && let Some((_, breakable, _)) = frames.last_mut()
                    {
                        *breakable = true;
                    }
                }
            }
        }
    }
    false
}

/// Whether what follows a `#` on its logical line makes it a directive's.
fn opens_directive(after: &[Token]) -> bool {
    let line: Vec<&Token> = after
        .iter()
        .enumerate()
        .take_while(|&(k, _)| !ends_logical_line(after, k))
        .map(|(_, t)| t)
        // A spliced newline and the `\` that splices it are gone by phase 2, so neither is part of the
        // name — only a real line end stops the scan ([`ends_logical_line`]). A comment is *not* dropped:
        // phase 3 makes it whitespace, so it may precede the name and must also end one.
        .filter(|t| t.kind != TokenKind::Newline && !is_backslash(t))
        .skip_while(|t| t.kind == TokenKind::Whitespace || is_comment(t))
        .collect();
    match line.first() {
        // Nothing on the line after the `#`: the null directive.
        None => true,
        // A line marker's name is a number — `# 42 "gen.c"`, GNU's preprocessor-output form. C11
        // §6.10.4 spells it `#line 42`, whose name is in the list already.
        Some(first) if first.kind == TokenKind::Number => true,
        // Phase 2 splices the *name* too, so a name broken across a continuation is one name again.
        // Whitespace and a comment both end it, which
        // is why it is filtered out above but not skipped over: `# region x` names `region`, not
        // `regionx`.
        Some(_) => names_directive(
            &line
                .iter()
                .take_while(|t| t.kind == TokenKind::Ident)
                .map(|t| t.text)
                .collect::<String>(),
        ),
    }
}

/// Whether `toks[k]` ends the *logical* line — a newline the preprocessor does not splice away.
///
/// Only a `\` **immediately** before it splices (C11 5.1.1.2), which is the same test
/// [`directive_end`] makes and the reason this does not skip whitespace to find one. GCC splices
/// `\`+space+newline too, with a warning, and neither predicate honours that extension — a limitation
/// both share rather than two answers to one question.
fn ends_logical_line(toks: &[Token], k: usize) -> bool {
    toks[k].kind == TokenKind::Newline && !(k > 0 && is_backslash(&toks[k - 1]))
}

/// A preprocessing directive's name: C23 §6.10.1, plus the extensions a real corpus writes. Not every
/// one is a keyword — only `if` and `else` are — so an identifier here may legally be a macro parameter
/// instead; see [`holds_directive`] for why that direction is the safe one to be wrong in.
fn names_directive(text: &str) -> bool {
    matches!(
        text,
        "if" | "ifdef"
            | "ifndef"
            | "elif"
            | "elifdef"
            | "elifndef"
            | "else"
            | "endif"
            | "define"
            | "undef"
            | "include"
            | "embed"
            | "line"
            | "error"
            | "warning"
            | "pragma"
            // Not in the standard, and all of them appear in headers a compiler is handed:
            // `include_next` and `import` guard re-inclusion, `ident` and `sccs` carry version strings,
            // `assert`/`unassert` are GCC's retired predicates, `system_header` suppresses warnings for
            // the rest of the file, `using` is C++/CLI, and `region`/`endregion` are editor folds.
            | "include_next"
            | "import"
            | "ident"
            | "sccs"
            | "assert"
            | "unassert"
            | "system_header"
            | "using"
            | "region"
            | "endregion"
            | "push_macro"
            | "pop_macro"
    )
}

/// Whether a comma-separated call argument has a newline in its body (after stripping leading
/// and trailing trivia). Such arguments would render differently on subsequent passes because
/// `build_expr_doc` collapses the newline into a space, which can then be reinterpreted by
/// `space_bit_fields`, breaking idempotency. When this is true the whole call is passed through
/// verbatim instead of being laid out via [`super::builders::build_call_body`].
///
/// A newline inside a *token* counts as much as one between two, which is what [`spans_lines`] asks:
/// a `\`-continued literal holds its own line break, so a width measured across it describes no line
/// that will be written. Reading only `Newline` tokens was enough while such a literal lexed as
/// fragments; #110 made it one token, and this is the guard that noticed.
pub(super) fn has_middle_newline(inner: &[Token]) -> bool {
    let args = split_top_level(inner, |t| t.kind == TokenKind::Punct && t.text == ",");
    for arg in args {
        let first = arg.iter().position(|t| !is_trivia(t));
        let last = arg.iter().rposition(|t| !is_trivia(t));
        if let (Some(f), Some(l)) = (first, last)
            && (arg[f..=l].iter().any(|t| t.kind == TokenKind::Newline) || spans_lines(&arg[f..=l]))
        {
            return true;
        }
    }
    false
}

/// Split a body into the comment run sharing the `{`'s line — sacred, so it stays there (§2.1) — and
/// the statements after it, trimmed of trivia.
pub(super) fn split_brace_line_comment<'a, 'src>(
    inner: &'a [Token<'src>],
) -> (&'a [Token<'src>], &'a [Token<'src>]) {
    let line_end = inner
        .iter()
        .position(|t| t.kind == TokenKind::Newline)
        .unwrap_or(inner.len());
    let head_len = if contains_comment(&inner[..line_end])
        && inner[..line_end]
            .iter()
            .all(|t| is_trivia(t) || is_comment(t))
    {
        line_end
    } else {
        0
    };
    let rest = &inner[head_len..];
    let start = rest
        .iter()
        .position(|t| !is_trivia(t))
        .unwrap_or(rest.len());
    let end = rest
        .iter()
        .rposition(|t| !is_trivia(t))
        .map_or(0, |p| p + 1);
    (
        &inner[..head_len],
        if start < end { &rest[start..end] } else { &[] },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_punct(text: &'static str) -> Token<'static> {
        Token {
            kind: TokenKind::Punct,
            text,
        }
    }

    fn tok(kind: TokenKind, text: &'static str) -> Token<'static> {
        Token { kind, text }
    }

    #[test]
    fn is_balanced_parens() {
        assert!(is_balanced(&[mk_punct("("), mk_punct(")")]));
    }

    #[test]
    fn is_balanced_brackets() {
        assert!(is_balanced(&[mk_punct("["), mk_punct("]")]));
    }

    #[test]
    fn is_balanced_braces() {
        assert!(is_balanced(&[mk_punct("{"), mk_punct("}")]));
    }

    #[test]
    fn is_balanced_combined() {
        assert!(is_balanced(&[
            mk_punct("("),
            mk_punct("["),
            mk_punct("]"),
            mk_punct("{"),
            mk_punct("}"),
            mk_punct(")"),
        ]));
    }

    #[test]
    fn is_balanced_unmatched_open() {
        assert!(!is_balanced(&[mk_punct("(")]));
    }

    #[test]
    fn is_balanced_mismatched() {
        assert!(!is_balanced(&[mk_punct("("), mk_punct("]")]));
    }

    #[test]
    fn is_balanced_negative_depth() {
        assert!(!is_balanced(&[mk_punct(")"), mk_punct("(")]));
    }

    #[test]
    fn is_balanced_empty() {
        assert!(is_balanced(&[]));
    }

    #[test]
    fn has_top_level_question_at_depth_zero() {
        assert!(has_top_level_question(&[mk_punct("?")]));
    }

    #[test]
    fn has_top_level_question_inside_parens() {
        assert!(!has_top_level_question(&[
            mk_punct("("),
            mk_punct("?"),
            mk_punct(")"),
        ]));
    }

    #[test]
    fn has_top_level_question_none() {
        assert!(!has_top_level_question(&[mk_punct("+"), mk_punct("-")]));
    }

    #[test]
    fn has_top_level_question_multiple_at_depth_zero() {
        assert!(has_top_level_question(&[
            mk_punct("?"),
            mk_punct("("),
            mk_punct("?"),
            mk_punct(")"),
            mk_punct("?"),
        ]));
    }

    #[test]
    fn is_ternary_chain_needs_two_questions() {
        assert!(!is_ternary_chain(&[mk_punct("?"), mk_punct(":")]));
        assert!(is_ternary_chain(&[
            mk_punct("?"),
            mk_punct(":"),
            mk_punct("?"),
            mk_punct(":"),
        ]));
    }

    #[test]
    fn is_ternary_chain_ignores_a_bracketed_question() {
        assert!(!is_ternary_chain(&[
            mk_punct("?"),
            mk_punct("("),
            mk_punct("?"),
            mk_punct(")"),
        ]));
    }

    #[test]
    fn is_excluded_callee_if() {
        assert!(is_excluded_callee("if"));
    }

    #[test]
    fn is_excluded_callee_for() {
        assert!(is_excluded_callee("for"));
    }

    #[test]
    fn is_excluded_callee_sizeof() {
        assert!(is_excluded_callee("sizeof"));
    }

    #[test]
    fn is_excluded_callee_printf() {
        assert!(!is_excluded_callee("printf"));
    }

    #[test]
    fn is_excluded_callee_myfunc() {
        assert!(!is_excluded_callee("myfunc"));
    }

    #[test]
    fn is_excluded_callee_empty() {
        assert!(!is_excluded_callee(""));
    }

    #[test]
    fn match_bracket_balanced() {
        assert_eq!(match_bracket(&[mk_punct("("), mk_punct(")")], 0), Some(1));
    }

    #[test]
    fn match_bracket_nested() {
        assert_eq!(
            match_bracket(
                &[mk_punct("("), mk_punct("("), mk_punct(")"), mk_punct(")")],
                0
            ),
            Some(3)
        );
    }

    #[test]
    fn match_bracket_unmatched_open() {
        assert_eq!(match_bracket(&[mk_punct("(")], 0), None);
    }

    #[test]
    fn match_bracket_wrong_kind() {
        // `match_bracket` only pairs `()`/`[]`, never `{}`.
        assert_eq!(match_bracket(&[mk_punct("{"), mk_punct("}")], 0), None);
    }

    #[test]
    fn match_brace_balanced() {
        assert_eq!(match_brace(&[mk_punct("{"), mk_punct("}")], 0), Some(1));
    }

    #[test]
    fn match_brace_unmatched_open() {
        assert_eq!(match_brace(&[mk_punct("{")], 0), None);
    }

    #[test]
    fn closes_block_reads_an_anonymous_literal_type() {
        use crate::lexer::tokenize;
        // The *last* `}`, which is the literal's: an anonymous type puts its own pair first, and those
        // spell a `{` that no type-token test accepts. The tag keyword opening the group says it is a
        // type without the body being read at all (#95).
        for src in [
            "int v = (struct { int x; }){1}",
            "int w = (union { int a; float b; }){.a = 2}",
            "int u = (enum { A }){A}",
        ] {
            let toks = tokenize(src);
            let close = toks.iter().rposition(|t| t.text == "}").unwrap();
            assert!(!closes_block(&toks, close), "{src}");
        }
    }

    #[test]
    fn match_open_brace_balanced() {
        assert_eq!(
            match_open_brace(&[mk_punct("{"), mk_punct("}")], 1),
            Some(0)
        );
    }

    #[test]
    fn match_open_brace_nested() {
        assert_eq!(
            match_open_brace(
                &[mk_punct("{"), mk_punct("{"), mk_punct("}"), mk_punct("}")],
                3
            ),
            Some(0)
        );
    }

    #[test]
    fn match_open_brace_unmatched_close() {
        assert_eq!(match_open_brace(&[mk_punct("}")], 0), None);
    }

    #[test]
    fn match_open_brace_wrong_kind() {
        assert_eq!(match_open_brace(&[mk_punct("("), mk_punct(")")], 1), None);
    }

    #[test]
    fn closes_block_tells_a_body_from_a_value() {
        use crate::lexer::tokenize;
        // The *first* `}` in each: a nested brace is the interesting one, and it is what says whether
        // the rule reaches the list or the block that holds it.
        for (src, is_block) in [
            ("void f(void) { g(); }", true),
            ("int main(void) { { int t; } }", true),
            ("do { g(); } while (x)", true),
            ("struct s { int a; }", true),
            ("switch (x) { case 1: break; }", true),
            ("x = ({ int t = 1; t; })", true),
            ("}", true),
            ("int a[] = {1, 2}", false),
            ("int m[2][2] = {{1, 2}, {3, 4}}", false),
            ("int a[] = {1, {2}}", false),
            ("struct s v = {.a = 1}", false),
            ("int * p = (int[]){1, 2}", false),
            ("return (struct s){1, 2}", false),
            // A typedef name spells no type keyword, and a parenthesized single name before a `{` can
            // be nothing but a type.
            ("int p = (vec2_t){1, 2}", false),
            // The `)` before the type is a control header's, so a statement follows it, and a
            // statement may open with a literal. A declarator's `)` may not.
            ("if (c) (struct s){1, 2}.a;", false),
            ("int (*fp(void))(int) { return 0; }", true),
            ("int (paren)(void) { return 1; }", true),
            // Read past comments: the same literal, written with one in the middle.
            ("int p = (int[]) /* c */ {1, 2}", false),
        ] {
            let toks = tokenize(src);
            let close = toks.iter().position(|t| t.text == "}").unwrap();
            assert_eq!(closes_block(&toks, close), is_block, "{src}");
        }
    }

    #[test]
    fn split_on_commas_depth_aware() {
        // A comma inside parens is at depth 1, so it does not split.
        let toks = [
            mk_punct("("),
            tok(TokenKind::Ident, "a"),
            mk_punct(","),
            tok(TokenKind::Ident, "b"),
            mk_punct(")"),
        ];
        assert_eq!(split_on_commas(&toks).len(), 1);
    }

    #[test]
    fn split_on_commas_top_level() {
        let toks = [
            tok(TokenKind::Ident, "a"),
            mk_punct(","),
            tok(TokenKind::Ident, "b"),
            mk_punct(","),
            tok(TokenKind::Ident, "c"),
        ];
        assert_eq!(split_on_commas(&toks).len(), 3);
    }

    fn chain_ops(src: &str) -> Option<Vec<&str>> {
        split_chain(&crate::lexer::tokenize(src)).map(|(_, ops)| ops)
    }

    #[test]
    fn split_chain_takes_the_loosest_operator() {
        assert_eq!(chain_ops("a || b && c"), Some(vec!["||"]));
        assert_eq!(chain_ops("a && b"), Some(vec!["&&"]));
        assert_eq!(chain_ops("a | b & c"), Some(vec!["|"]));
        assert_eq!(chain_ops("a + b * c"), Some(vec!["+"]));
    }

    #[test]
    fn split_chain_mixes_one_class() {
        assert_eq!(chain_ops("a + b - c"), Some(vec!["+", "-"]));
    }

    /// The head [`operand_span`] strips, pinned at the function the cut restriction mirrors — the two
    /// encode "what is a head", and drifting apart is what #64 cost. The `return` arm especially: it is
    /// the half `loosest_cuts` deliberately does *not* mirror, so nothing else would notice it change.
    #[test]
    fn operand_span_takes_the_last_assignment_or_a_leading_return() {
        // The operands the span leaves, rather than the index — an index moves with the trivia
        // tokenization puts between them, and what is being asserted is where the head ends.
        let operands = |src: &str| {
            let toks = crate::lexer::tokenize(src);
            toks[operand_span(&toks)..]
                .iter()
                .map(|t| t.text)
                .collect::<String>()
                .trim_start()
                .to_owned()
        };
        assert_eq!(operands("x = a | b"), "a | b");
        // The *last* assignment, however many there are.
        assert_eq!(operands("x = y = a"), "a");
        // A `return` heads a span only while no assignment has.
        assert_eq!(operands("return a | b"), "a | b");
        assert_eq!(operands("return x = a"), "a");
        // Neither: the whole span is operands.
        assert_eq!(operands("a | b"), "a | b");
        // A bracketed assignment is no head — the rule is depth zero, which is #125's whole subject.
        assert_eq!(operands("f(x = 1) | b"), "f(x = 1) | b");
    }

    /// A chain is not cut at *depth zero* before an assignment. Only the three inputs whose loosest
    /// operator lives there can say so — everywhere else the looseness rule was already choosing the
    /// right side's operator, which is the [`split_chain_prefers_the_right_sides_operator_anyway`]
    /// below, kept apart because it passes with the restriction removed and guards nothing.
    ///
    /// Depth zero is the whole claim, which is why the conformance guard is named for it. A chain
    /// inside parentheses that a later `=` puts in *its* left side — `s = (a | b) = c | d` — is still
    /// cut on the second pass; that is #125, it predates this, and it is not C since `(a | b)` is no
    /// lvalue.
    #[test]
    fn split_chain_cuts_only_past_the_last_assignment() {
        assert_eq!(chain_ops("a | b = c"), None);
        // The *last* assignment, so an operator between two of them is in the second's left side.
        assert_eq!(chain_ops("x = a | b = c + d"), Some(vec!["+"]));
        // A compound assignment is an assignment.
        assert_eq!(chain_ops("a | b += c"), None);
    }

    /// What the rule looks like where it is not what decides — asserted because the shapes read as
    /// if they were the guard and are not, which cost the review a round to establish. `0/a = A & A`
    /// is #43's own input and cuts at the `&` either way: `&` binds looser than `/`, so a single
    /// pass never wanted the `/`. #43 is a *two-pass* effect — pass 2 finds the `&` already inside
    /// the parentheses pass 1 wrote, leaving only the `/` at depth zero — which nothing at this
    /// level can see. `a_depth_zero_chain_is_not_cut_before_an_assignment` is its guard.
    #[test]
    fn split_chain_prefers_the_right_sides_operator_anyway() {
        assert_eq!(chain_ops("0/a = A & A"), Some(vec!["&"]));
        assert_eq!(chain_ops("x = a | b"), Some(vec!["|"]));
        assert_eq!(chain_ops("x = a | b | c"), Some(vec!["|", "|"]));
        // A depth-zero `,` refuses the whole span before the cuts are looked for.
        assert_eq!(chain_ops("x = a | b, y = c + d"), None);
        // A comparison that ends in `=` is no assignment.
        assert_eq!(chain_ops("a | b == c"), Some(vec!["|"]));
    }

    #[test]
    fn split_chain_ignores_a_nested_operator() {
        assert_eq!(chain_ops("f(a | b)"), None);
        assert_eq!(chain_ops("(a | b)"), None);
    }

    #[test]
    fn split_chain_needs_two_operands() {
        // Unary `-`, `&` and `*` bind one operand, and a declarator is never a chain (§6).
        assert_eq!(chain_ops("-a"), None);
        assert_eq!(chain_ops("&x"), None);
        assert_eq!(chain_ops("a + -b"), Some(vec!["+"]));
        assert_eq!(chain_ops("int * p"), None);
        assert_eq!(chain_ops("a * b"), None);
        assert_eq!(chain_ops("return -1"), None);
    }

    #[test]
    fn split_chain_defers_to_a_ternary() {
        assert_eq!(chain_ops("a | b ? c : d"), None);
    }

    #[test]
    fn split_chain_segments_span_the_input() {
        let toks = crate::lexer::tokenize("a | b | c");
        let (segments, ops) = split_chain(&toks).unwrap();
        assert_eq!(segments.len(), ops.len() + 1);
        let rejoined: String = segments
            .iter()
            .map(|s| s.iter().map(|t| t.text).collect::<String>())
            .collect::<Vec<_>>()
            .join("|");
        assert_eq!(rejoined, "a | b | c");
    }

    #[test]
    fn has_middle_newline_strips_trailing_trivia() {
        // A newline only in trailing trivia does not count as a middle newline.
        let toks = [
            tok(TokenKind::Ident, "a"),
            mk_punct(","),
            tok(TokenKind::Ident, "b"),
            tok(TokenKind::Whitespace, " "),
            tok(TokenKind::Newline, "\n"),
        ];
        assert!(!has_middle_newline(&toks));
    }

    #[test]
    fn has_middle_newline_inside_argument() {
        let toks = [
            tok(TokenKind::Ident, "a"),
            mk_punct(","),
            tok(TokenKind::Ident, "b"),
            tok(TokenKind::Newline, "\n"),
            tok(TokenKind::Ident, "c"),
        ];
        assert!(has_middle_newline(&toks));
    }

    #[test]
    fn has_middle_newline_inside_a_continued_literal() {
        // A `\`-continued literal holds its own line break, so a width measured across it describes
        // no line that will be written — and #110 made it one token, where reading only `Newline`
        // tokens stopped seeing it.
        use crate::lexer::tokenize;
        let toks = tokenize("\"a\\\nb\", c");
        assert!(has_middle_newline(&toks));
        assert!(!has_middle_newline(&tokenize("\"ab\", c")));
    }

    #[test]
    fn has_middle_newline_nested_call_with_internal_newline() {
        // Regression guard: an arg that is itself a call with a newline inside its parens must
        // count as a middle newline so the whole call is passed through verbatim.
        use crate::lexer::tokenize;
        let src =
            "(handler), (event), read_monotonic_timestamp_ms(\n\t), current_execution_context_id()";
        let toks = tokenize(src);
        assert!(has_middle_newline(&toks));
    }

    #[test]
    fn is_call_head_ident_then_paren() {
        let toks = [tok(TokenKind::Ident, "foo"), mk_punct("(")];
        assert!(is_call_head(&toks, 0));
    }

    #[test]
    fn is_call_head_excluded_keyword() {
        let toks = [tok(TokenKind::Ident, "if"), mk_punct("(")];
        assert!(!is_call_head(&toks, 0));
    }

    #[test]
    fn is_call_head_no_paren() {
        let toks = [tok(TokenKind::Ident, "foo"), tok(TokenKind::Ident, "bar")];
        assert!(!is_call_head(&toks, 0));
    }

    #[test]
    fn is_call_head_type_keyword() {
        // `int (` is a declarator group, not a call — `is_call_head` and `call_head_before`
        // agree via the shared `is_callee_ident` guard.
        let toks = [tok(TokenKind::Ident, "int"), mk_punct("(")];
        assert!(!is_call_head(&toks, 0));
    }

    #[test]
    fn is_callee_ident_plain_ident() {
        assert!(is_callee_ident(&tok(TokenKind::Ident, "foo")));
    }

    #[test]
    fn is_callee_ident_excludes_keyword_and_type() {
        assert!(!is_callee_ident(&tok(TokenKind::Ident, "sizeof")));
        assert!(!is_callee_ident(&tok(TokenKind::Ident, "int")));
    }

    #[test]
    fn is_callee_ident_non_ident() {
        assert!(!is_callee_ident(&mk_punct("(")));
    }

    #[test]
    fn is_value_start_accepts_what_can_begin_a_value() {
        for (kind, text) in [
            (TokenKind::Ident, "x"),
            (TokenKind::Number, "1"),
            (TokenKind::String, "\"s\""),
            (TokenKind::Char, "'c'"),
            (TokenKind::Punct, "("),
            (TokenKind::Punct, "-"),
            (TokenKind::Punct, "!"),
            (TokenKind::Punct, "*"),
            (TokenKind::Punct, "&"),
            (TokenKind::Punct, "#"),
            (TokenKind::Operator, "++"),
            (TokenKind::Operator, "&&"),
        ] {
            assert!(is_value_start(&Token { kind, text }), "{text}");
        }
    }

    #[test]
    fn is_value_start_refuses_what_cannot() {
        // `[` and `{` open a subscript and a brace list; an assignment, a closer and a separator
        // end a value rather than begin one.
        for (kind, text) in [
            (TokenKind::Punct, "["),
            (TokenKind::Punct, "{"),
            (TokenKind::Punct, "="),
            (TokenKind::Punct, ")"),
            (TokenKind::Punct, ","),
            (TokenKind::Punct, ";"),
            (TokenKind::Operator, "+="),
            (TokenKind::Whitespace, " "),
        ] {
            assert!(!is_value_start(&Token { kind, text }), "{text}");
        }
    }

    #[test]
    fn ternary_open_before_stops_at_a_statement_boundary() {
        // A `?` before a `;` is in a different statement, so no ternary is open at `j`.
        let toks = [
            tok(TokenKind::Punct, "?"),
            tok(TokenKind::Punct, ";"),
            tok(TokenKind::Ident, "x"),
            tok(TokenKind::Punct, ":"),
        ];
        assert!(!ternary_open_before(&toks, 3));
    }

    #[test]
    fn ternary_open_before_finds_an_unmatched_question() {
        let toks = [
            tok(TokenKind::Punct, "?"),
            tok(TokenKind::Ident, "x"),
            tok(TokenKind::Punct, ":"),
        ];
        assert!(ternary_open_before(&toks, 2));
    }
}
