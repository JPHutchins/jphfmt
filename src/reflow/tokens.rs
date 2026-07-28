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
    let significant = || inner.iter().filter(|t| !is_trivia(t));
    significant().any(|t| is_type_context(t.text) || is_tag_keyword(t.text))
        && significant().all(|t| {
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
pub(super) fn closes_literal_type(toks: &[Token], close: usize) -> bool {
    match_open_paren(toks, close).is_some_and(|open| {
        is_type_group(&toks[open + 1..close])
            && prev_significant(toks, open).is_none_or(|before| {
                !heads_body(&toks[before]) && !matches!(toks[before].text, ")" | "]")
            })
    })
}

/// Index of the `(` matching the `)` at `close`, or `None` if unbalanced.
pub(super) fn match_open_paren(toks: &[Token], close: usize) -> Option<usize> {
    if toks.get(close).map(|t| t.text) != Some(")") {
        return None;
    }
    let mut depth = 0usize;
    (0..=close).rev().find(|&j| {
        match toks[j].text {
            ")" => depth += 1,
            "(" => depth -= 1,
            _ => {}
        }
        depth == 0 && toks[j].text == "("
    })
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

/// The tokens outside every bracket group, paired with their index — the level a construct's own
/// separators live at. The brackets themselves are not yielded; a construct is never separated by one.
fn at_depth_zero<'a, 'src>(
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
fn segments_at<'a, 'src>(inner: &'a [Token<'src>], cuts: &[usize]) -> Vec<&'a [Token<'src>]> {
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
    let cuts: Vec<usize> = at_depth_zero(inner)
        .filter(|(_, t)| is_sep(t))
        .map(|(j, _)| j)
        .collect();
    segments_at(inner, &cuts)
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

pub(super) fn respaced_when_joined(inner: &[Token]) -> bool {
    // A trivia run is more than one token — a `Newline`, then the next line's indentation — so a
    // break is looked for across the whole run, not just the token adjacent to the punctuator.
    let broken_before = |j: usize| {
        inner[..j]
            .iter()
            .rev()
            .take_while(|t| is_trivia(t))
            .any(|t| t.text.contains(['\n', '\r']))
    };
    let broken_after = |j: usize| {
        inner[j + 1..]
            .iter()
            .take_while(|t| is_trivia(t))
            .any(|t| t.text.contains(['\n', '\r']))
    };
    let mut depth = 0i32;
    for (j, t) in inner.iter().enumerate() {
        match t.text {
            "(" | "[" => depth += 1,
            ")" | "]" => depth -= 1,
            _ => {}
        }
        if depth != 0 || t.kind != TokenKind::Punct {
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
        if t.text == ":"
            && !ternary_open_before(inner, j)
            && (broken_before(j) || broken_after(j))
            && prev_nontrivia(inner, j).is_some_and(|k| inner[k].kind == TokenKind::Ident)
            && next_nontrivia(inner, j + 1).is_some_and(|k| inner[k].kind == TokenKind::Number)
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
    prev_nontrivia(inner, j).is_some_and(|k| match inner[k].kind {
        TokenKind::Ident => is_callee_ident(&inner[k]),
        TokenKind::Number | TokenKind::String | TokenKind::Char => true,
        TokenKind::Punct => matches!(inner[k].text, ")" | "]"),
        // A postfix `++`/`--` ends a value as much as its operand does.
        TokenKind::Operator => matches!(inner[k].text, "++" | "--"),
        TokenKind::Newline
        | TokenKind::Whitespace
        | TokenKind::Unknown
        | TokenKind::LineComment
        | TokenKind::BlockComment => false,
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
    at_depth_zero(inner)
        .filter_map(|(j, t)| {
            matches!(t.kind, TokenKind::Operator | TokenKind::Punct)
                .then(|| CHAIN_CLASSES.iter().position(|c| c.contains(&t.text)))
                .flatten()
                .filter(|_| is_binary_position(inner, j))
                .map(|class| (class, j))
        })
        .fold(
            (CHAIN_CLASSES.len(), Vec::new()),
            |(loosest, mut cuts), (class, j)| match class.cmp(&loosest) {
                Ordering::Less => (class, vec![j]),
                Ordering::Equal => {
                    cuts.push(j);
                    (loosest, cuts)
                }
                Ordering::Greater => (loosest, cuts),
            },
        )
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
    // separator stranded, and the space it lands beside is not this pass's to keep.
    segments
        .iter()
        .all(|s| has_non_trivia(s))
        .then(|| (segments, cuts.iter().map(|&j| inner[j].text).collect()))
}

pub(super) fn is_trivia(t: &Token) -> bool {
    matches!(t.kind, TokenKind::Whitespace | TokenKind::Newline)
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

/// Whether any significant token's own text spans lines — an unterminated literal, which the lexer
/// runs to the end of the file. A one-line width cannot describe it, so no layout may be decided from
/// a span holding one.
pub(super) fn spans_lines(toks: &[Token]) -> bool {
    toks.iter()
        .any(|t| !is_trivia(t) && t.text.contains(['\n', '\r']))
}

/// Whether a comma-separated call argument has a newline in its body (after stripping leading
/// and trailing trivia). Such arguments would render differently on subsequent passes because
/// `build_expr_doc` collapses the newline into a space, which can then be reinterpreted by
/// `space_bit_fields`, breaking idempotency. When this is true the whole call is passed through
/// verbatim instead of being laid out via [`build_call_doc`].
pub(super) fn has_middle_newline(inner: &[Token]) -> bool {
    let args = split_top_level(inner, |t| t.kind == TokenKind::Punct && t.text == ",");
    for arg in args {
        let first = arg.iter().position(|t| !is_trivia(t));
        let last = arg.iter().rposition(|t| !is_trivia(t));
        if let (Some(f), Some(l)) = (first, last)
            && arg[f..=l].iter().any(|t| t.kind == TokenKind::Newline)
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
