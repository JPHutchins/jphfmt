//! Wadler/Leijen `Doc` builders. Every construct jphfmt breaks is one container — an argument list, a
//! `{}` or `enum` body, a `for` header, a condition, an operator chain, a ternary's arms, a macro's
//! parameters — laid out by [`build_container`] under §2.2's fits-flat-or-fully-broken rule. Because
//! the comma trails in an argument list, the operator trails in a chain and the ternary `:` trails its
//! arm: one rule, and per-construct values for how it is bracketed, what separates it, whether a
//! separator follows the last element on the break, and whether the width decides at all (#71).
//!
//! Each builder turns a token slice into a [`Doc`] that [`crate::doc::render`] later flattens or fully
//! breaks. Depends on [`super::tokens`] for depth-aware splitting and balance checks.

use super::tokens::{
    has_non_trivia, has_top_level, has_top_level_question, is_balanced, is_callee_ident,
    is_ternary_chain, is_trivia, match_brace, match_bracket, spans_lines, split_chain,
    split_designators, split_on_commas, split_top_level,
};
use crate::doc::Doc;
use crate::lexer::{Token, TokenKind};

/// A call's argument list, brackets included: the elements are `,`-separated and the flat form is
/// tight (§2.5). `fit` is the caller's, because a `#define` whose body overflows has already decided
/// to break its parameters before this is built.
pub(super) fn build_call_body(inner: &[Token], fit: Fit) -> Doc {
    if !is_balanced(inner) {
        return Doc::Text(format!("({})", render_segment(inner)));
    }
    let args: Vec<&[Token]> = split_on_commas(inner)
        .into_iter()
        .filter(|a| has_non_trivia(a))
        .collect();
    if args.is_empty() {
        return Doc::text("()");
    }
    // A sole argument's span is exactly the span of these parens, so a chain of arms inside it needs
    // no pair of its own; with siblings, unbounded arms read as further arguments. A `{}` element is
    // bounded either way, because its list writes a trailing comma on the break (#59).
    let bound = if args.len() == 1 {
        Bound::Enclosing
    } else {
        Bound::Parens
    };
    let seps = vec![",".to_owned(); args.len() - 1];
    let elements = args
        .into_iter()
        .map(|a| build_element_doc(a, bound))
        .collect();
    build_container(&PARENS, elements, seps, None, fit)
}

/// A `{}` or `enum` body: `,`-separated elements, a trailing comma when broken, and §2.3's magic
/// comma — a trailing comma in the source — which is the same forced fit a ternary chain takes.
/// `padded` is the flat form's inner space, `enum { A, B }` against `{1, 2}`.
pub(super) fn build_brace_doc(inner: &[Token], padded: bool) -> Doc {
    if !is_balanced(inner) {
        return Doc::Text(format!("{{{}}}", render_segment(inner)));
    }
    let segments = split_on_commas(inner);
    let magic = segments.len() > 1 && segments.last().is_some_and(|s| s.iter().all(is_trivia));
    let elements: Vec<&[Token]> = segments.into_iter().filter(|s| has_non_trivia(s)).collect();
    if elements.is_empty() {
        return Doc::text("{}");
    }
    let bracketing = Bracketing::Written {
        open: "{",
        close: "}",
        pad: if padded { Pad::Spaced } else { Pad::Tight },
    };
    let seps = vec![",".to_owned(); elements.len() - 1];
    let docs = elements.iter().map(|e| build_juxtaposed_doc(e)).collect();
    let fit = if magic { Fit::Forced } else { Fit::Measured };
    build_container(&bracketing, docs, seps, Some(","), fit)
}

/// One `{}` element: its juxtaposed items each on their own line when the list breaks, so a
/// brace-less initializer macro is not joined onto the designator that follows it.
fn build_juxtaposed_doc(element: &[Token]) -> Doc {
    let items = split_designators(element);
    if items.len() < 2 {
        return build_element_doc(element, Bound::Parens);
    }
    Doc::concat(
        items
            .iter()
            .map(|item| build_element_doc(item, Bound::Parens))
            .flat_map(|doc| [Doc::Line, doc])
            .skip(1)
            .collect::<Vec<_>>(),
    )
}

/// Whether the nearest non-trivia token before `open` names a callee ([`is_callee_ident`]). Unlike
/// [`super::tokens::is_call_head`], trivia (including a newline) between the ident and `(` is
/// tolerated: [`build_expr_doc`] must flatten such a gap to nothing (§2.5's tight `foo(`) rather
/// than a collapsed space, since a collapsed space is itself same-line and would be tightened by
/// `space_call_heads` on the next pass — collapsing to a space here instead would render this
/// pass's output as a fixpoint of a *different* pass, breaking idempotency.
///
/// Only an *identifier* callee is recognized: calls through a function pointer (`(*p)(args)`) or a
/// parenthesized expression (`(expr)(args)`) are left as flat text, because a `)` before `(` is
/// token-level indistinguishable from a C-style cast `(type)(expr)` — exploding the latter as a
/// call would be wrong, so §6 "prefer passthrough when ambiguous" applies.
///
/// Only whitespace/newline trivia is skipped, never comments: a commented `foo /* c */ (a)` stops
/// the walk, but the structure pass rejects comment-bearing constructs before they reach here.
fn call_head_before(toks: &[Token], open: usize) -> bool {
    let mut k = open;
    while k > 0 && is_trivia(&toks[k - 1]) {
        k -= 1;
    }
    k > 0 && is_callee_ident(&toks[k - 1])
}

/// Build one element/argument: collapsed text, with any nested `{...}` or nested call `f(...)`
/// rendered as its own group so it collapses or explodes independently of its parent.
/// A statement or element at the top of its container: a chain or ternary here is bounded by
/// parentheses when it breaks, because nothing else bounds it. Its operands go through
/// [`build_expr_doc`], which never adds a token — a bounded operand would gain another pair as its
/// indent deepened, one per pass.
pub(super) fn build_element_doc(toks: &[Token], headless: Bound) -> Doc {
    if is_balanced(toks)
        && let Some(bounded) = build_chain_doc(toks, headless)
    {
        return bounded;
    }
    build_expr_doc(toks)
}

pub(super) fn build_expr_doc(toks: &[Token]) -> Doc {
    if is_balanced(toks)
        && let Some((segments, ops)) = split_chain(toks)
    {
        return build_container(
            &Bracketing::Hanging,
            segment_docs(&segments),
            chain_seps(&ops),
            None,
            Fit::Measured,
        );
    }
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
        } else if t.kind == TokenKind::Punct
            && t.text == "("
            && call_head_before(toks, j)
            && let Some(close) = match_bracket(toks, j)
        {
            // The callee is already in `text`; any trivia between it and `(` is dropped rather
            // than collapsed to a space, so this matches `space_call_heads`'s tight-call spacing
            // and stays a fixpoint across passes (§2.5).
            pending_space = false;
            if !text.is_empty() {
                parts.push(Doc::Text(std::mem::take(&mut text)));
            }
            parts.push(build_call_body(&toks[j + 1..close], Fit::Measured));
            j = close + 1;
        } else if t.kind == TokenKind::Punct
            && t.text == "("
            && let Some(close) = match_bracket(toks, j)
            && let Some(group) = build_paren_group(&toks[j + 1..close])
        {
            if pending_space && !text.is_empty() {
                text.push(' ');
            }
            pending_space = false;
            if !text.is_empty() {
                parts.push(Doc::Text(std::mem::take(&mut text)));
            }
            parts.push(group);
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

/// `segments` with `seps[i]` trailing segment `i`: flat `a sep b`, or one element per line with the
/// separator ending each (§2.4, §2.7).
fn trailing_items(segments: Vec<Doc>, seps: Vec<String>) -> Vec<Doc> {
    let mut seps = seps.into_iter();
    let mut items = Vec::new();
    for seg in segments {
        items.push(seg);
        if let Some(sep) = seps.next() {
            items.push(Doc::text(sep));
            items.push(Doc::Line);
        }
    }
    items
}

/// A bracket's inner space in the flat form: `{1, 2}` and `f(a, b)` against `enum { A, B }`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pad {
    Tight,
    Spaced,
}

impl Pad {
    fn doc(self) -> Doc {
        match self {
            Self::Tight => Doc::SoftLine,
            Self::Spaced => Doc::Line,
        }
    }
}

/// How a container is bracketed — the only thing that differs between one construct and another,
/// beyond its separators.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Bracketing {
    /// The enclosing container's brackets: elements that are its whole span add nothing of their own,
    /// and take no indent of their own either, because that container already indented them.
    Enclosing,
    /// Nothing but a hanging indent. An operand inside a larger expression adds no parentheses — it
    /// has no claim on the span — but its continuation lines still sit one level in.
    Hanging,
    /// Brackets the author wrote, which this pass lays the elements out inside.
    Written {
        open: &'static str,
        close: &'static str,
        pad: Pad,
    },
    /// Brackets that appear only on the break, after `head` when there is one — the only tokens
    /// jphfmt writes, legal because the elements are already an implicit container.
    OnBreak { head: String },
}

/// The one layout every container in the language gets (§2.2): the elements in order, each `seps[i]`
/// *trailing* its element, `trailing` after the last one only when broken (§2.3's magic comma), all of
/// it bounded per `bounds` and flat-or-broken per `fit`.
///
/// An argument list, a `{}` or `enum` body, a `for` header, a condition, an operator chain, a
/// ternary's arms and a macro's parameters are the same construct: because the comma trails, the
/// operator trails and the ternary `:` trails. What differs between them is the four values passed
/// here, not the shape they are laid out in (#71).
fn build_container(
    bracketing: &Bracketing,
    elements: Vec<Doc>,
    seps: Vec<String>,
    trailing: Option<&str>,
    fit: Fit,
) -> Doc {
    let mut items = trailing_items(elements, seps);
    items.extend(trailing.map(|text| Doc::IfBreak {
        broken: text.to_owned(),
        flat: String::new(),
    }));
    let nested = |lead: Doc, items: Vec<Doc>| {
        Doc::nest(Doc::concat(
            std::iter::once(lead).chain(items).collect::<Vec<_>>(),
        ))
    };
    match bracketing {
        Bracketing::Enclosing => fit.wrap(Doc::concat(items)),
        Bracketing::Hanging => fit.wrap(Doc::nest(Doc::concat(items))),
        Bracketing::Written { open, close, pad } => fit.wrap(Doc::concat([
            Doc::text(*open),
            nested(pad.doc(), items),
            pad.doc(),
            Doc::text(*close),
        ])),
        Bracketing::OnBreak { head } => fit.wrap(Doc::concat(
            (!head.is_empty())
                .then(|| Doc::Text(format!("{head} ")))
                .into_iter()
                .chain([
                    Doc::IfBreak {
                        broken: "(".to_owned(),
                        flat: String::new(),
                    },
                    nested(Doc::SoftLine, items),
                    Doc::SoftLine,
                    Doc::IfBreak {
                        broken: ")".to_owned(),
                        flat: String::new(),
                    },
                ])
                .collect::<Vec<_>>(),
        )),
    }
}

/// The author's `(…)` around a clause run: a `for` header, a condition, a parenthesized chain.
const PARENS: Bracketing = Bracketing::Written {
    open: "(",
    close: ")",
    pad: Pad::Tight,
};

/// A parenthesized clause group: flat `(a sep b sep c)` or one element per line, with each `seps[i]`
/// trailing its element (`;` for a `for` header, ` &&` for a condition, ` |` for a bit chain).
fn build_clause_group(segments: Vec<Doc>, seps: Vec<String>, fit: Fit) -> Doc {
    if segments.is_empty() {
        return Doc::text("()");
    }
    build_container(&PARENS, segments, seps, None, fit)
}

/// An assignment operator: `=` and the compound forms, but not a comparison.
fn assigns(t: &Token) -> bool {
    (t.kind == TokenKind::Punct && t.text == "=")
        || (t.kind == TokenKind::Operator
            && t.text.ends_with('=')
            && !matches!(t.text, "==" | "!=" | "<=" | ">="))
}

/// Where an expression's operands begin: after the last depth-zero assignment, or after a leading
/// `return`. That head is not part of the expression, so the parentheses this module adds bound the
/// operands alone.
fn operand_span(toks: &[Token]) -> usize {
    let mut depth = 0i32;
    let mut head = None;
    for (j, t) in toks.iter().enumerate() {
        match t.text {
            "(" | "[" | "{" => depth += 1,
            ")" | "]" | "}" => depth -= 1,
            _ if depth == 0 && assigns(t) => head = Some(j),
            "return" if depth == 0 && head.is_none() => head = Some(j),
            _ => {}
        }
    }
    head.map_or(0, |j| j + 1)
}

/// Whether `toks` is one expression jphfmt may bound with parentheses of its own.
///
/// A depth-zero `,` means it is a list — a second declarator, or a comma expression — and
/// `(a | b, c)` is not `a | b, c`. A token carrying a line break is an unterminated literal, which the
/// width model does not describe ([`crate::doc::display_width`] measures one line), and a `#` means the
/// span is a directive fragment whose column a later pass rewrites. Bounding either would decide a
/// layout from a width that the next pass measures differently.
fn is_boundable(toks: &[Token], operands: &[Token]) -> bool {
    // A line break inside a *token* is an unterminated literal spanning lines, which a one-line
    // width cannot describe, and a `#` means a directive fragment whose column a later pass
    // rewrites. Either anywhere in the construct — head included — and the width this decides from
    // is not the width the next pass measures. A tab needs no refusal: `display_width` counts the
    // columns it occupies, the same as every other measure in the pipeline.
    if spans_lines(toks) || toks.iter().any(|t| t.text == "#") {
        return false;
    }
    // A depth-zero `,` makes the operands a list, not one expression: `x = (a ? b : c, d)` assigns
    // `d` where `x = a ? b : c, d` assigns the ternary. `split_chain` refuses one too, but the
    // ternary arm below it does not, and this is the gate both pass through.
    !has_top_level(operands, ",")
}

/// Whether a construct's layout is still the width's to decide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Fit {
    /// Flat if it fits, broken if it does not — §2.2's group.
    Measured,
    /// Broken whatever the width says, and reported as not fitting so its parents break too.
    Forced,
}

impl Fit {
    /// A ternary *chain* reads as the `cond -> value` map it is only when every arm has its own line,
    /// so the width does not get to decide (#59). A single conditional is one thing, and a line is
    /// where it reads best — including when a depth-zero `:` gives it a third arm it does not own
    /// ([`is_ternary_chain`]).
    fn of_ternary(inner: &[Token]) -> Self {
        if is_ternary_chain(inner) {
            Self::Forced
        } else {
            Self::Measured
        }
    }

    fn wrap(self, body: Doc) -> Doc {
        match self {
            Self::Measured => Doc::group(body),
            Self::Forced => Doc::ForceBreak(Box::new(body)),
        }
    }
}

/// What bounds a run of operands once it breaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Bound {
    /// The parentheses this pass writes on the break — the only tokens jphfmt adds, legal because the
    /// operands are already an implicit container: bounding one changes the layout and nothing else.
    Parens,
    /// The enclosing container's own brackets. A binary chain that is a call argument or a `{}`
    /// element adds nothing of its own, since its operands are that bracket's whole span.
    Enclosing,
}

/// Lay `segments` out bounded per `bound`, after `head` when there is one.
fn build_bounded_doc(
    head: &str,
    segments: Vec<Doc>,
    seps: Vec<String>,
    fit: Fit,
    bound: Bound,
) -> Doc {
    let bracketing = match bound {
        Bound::Enclosing => {
            // The enclosing bracket bounds the operands, which is only true when they are its whole
            // span — a head would mean they are not, and would be dropped silently here.
            debug_assert!(
                head.is_empty(),
                "a head is bounded, never enclosed: {head:?}"
            );
            Bracketing::Enclosing
        }
        Bound::Parens => Bracketing::OnBreak {
            head: head.to_owned(),
        },
    };
    build_container(&bracketing, segments, seps, None, fit)
}

/// An operator chain or ternary with no parentheses of its own: flat, or one operand per line with the
/// operator trailing, bounded by parentheses [`build_bounded_doc`] adds on the break.
pub(super) fn build_chain_doc(toks: &[Token], headless: Bound) -> Option<Doc> {
    let start = operand_span(toks);
    let operands = &toks[start..];
    if !is_boundable(toks, operands) {
        return None;
    }
    let head = render_segment(&toks[..start]);
    // A head means these operands are only part of their container's span, so they are bounded
    // whatever they are; with no head it is the position that decides, and it decides the same for a
    // ternary and for a binary chain — unbounded operands read as elements of whatever list encloses
    // them either way (#59, #63).
    let bound = if head.is_empty() {
        headless
    } else {
        Bound::Parens
    };
    if let Some((segments, ops)) = split_chain(operands) {
        return Some(build_bounded_doc(
            &head,
            segment_docs(&segments),
            chain_seps(&ops),
            Fit::Measured,
            bound,
        ));
    }
    // §2.4's chain, with the `:` trailing, for a ternary the author left unparenthesized.
    let (arms, seps, fit) = ternary_layout(operands)?;
    Some(build_bounded_doc(&head, arms, seps, fit, bound))
}

/// The trailing separators for an operator chain: ` |`, ` &&`, and so on.
fn chain_seps(ops: &[&str]) -> Vec<String> {
    ops.iter().map(|op| format!(" {op}")).collect()
}

/// A ternary's arms as documents with the ` :` that trails each, and whether the width decides —
/// the whole of what the three places a ternary can appear need from one.
fn ternary_layout(inner: &[Token]) -> Option<(Vec<Doc>, Vec<String>, Fit)> {
    let arms = ternary_arms(inner)?;
    let seps = vec![" :".to_owned(); arms.len() - 1];
    Some((segment_docs(&arms), seps, Fit::of_ternary(inner)))
}

/// The `:`-separated arms of a ternary, or `None` if any arm is missing its operand — a stranded
/// separator would put this layout's spacing where the author had none.
fn ternary_arms<'a, 'src>(inner: &'a [Token<'src>]) -> Option<Vec<&'a [Token<'src>]>> {
    if !has_top_level_question(inner) {
        return None;
    }
    let arms = split_top_level(inner, |t| t.kind == TokenKind::Punct && t.text == ":");
    (arms.len() >= 2 && arms.iter().all(|s| has_non_trivia(s))).then_some(arms)
}

/// Each segment as its own expression, paired with the separators that trail them.
fn segment_docs(segments: &[&[Token]]) -> Vec<Doc> {
    segments.iter().map(|s| build_expr_doc(s)).collect()
}

/// Split `inner` on the depth-zero separators `is_sep` selects, build each segment as its own
/// expression [`Doc`], and lay them out as a [`build_clause_group`] with `sep` trailing all but the
/// last — the shared shape of a ternary chain, a `for` header, and a logical-operator condition.
fn build_clause_doc(inner: &[Token], is_sep: impl Fn(&Token) -> bool, sep: &str) -> Doc {
    if !is_balanced(inner) {
        return Doc::Text(format!("({})", render_segment(inner)));
    }
    let segments: Vec<&[Token]> = split_top_level(inner, is_sep);
    let seps = vec![sep.to_owned(); segments.len().saturating_sub(1)];
    build_clause_group(segment_docs(&segments), seps, Fit::Measured)
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

/// A parenthesized chain or ternary as its own container. A ternary belongs here as much as a chain
/// does: [`build_chain_doc`] bounds a bare one with parentheses, and this is the same content on the
/// next pass, so both must reach the same layout or neither is a fixpoint.
pub(super) fn build_paren_group(inner: &[Token]) -> Option<Doc> {
    // The author's parentheses do not exempt the span from the width model: a literal running to the
    // end of the file has no one-line width, so every group holding one passes through, exactly as
    // `is_boundable` refuses one a chain would have bounded.
    if spans_lines(inner) {
        return None;
    }
    if let Some((segments, ops)) = split_chain(inner) {
        return Some(build_clause_group(
            segment_docs(&segments),
            chain_seps(&ops),
            Fit::Measured,
        ));
    }
    let (arms, seps, fit) = ternary_layout(inner)?;
    Some(build_clause_group(arms, seps, fit))
}

/// `for (init; cond; step)` — one clause per line when broken (§2.4).
pub(super) fn build_for_doc(inner: &[Token]) -> Doc {
    build_clause_doc(inner, |t| t.kind == TokenKind::Punct && t.text == ";", ";")
}

/// An `if`/`while`/`switch` condition — split on its loosest-binding operator with that operator
/// trailing (§2.7), so `a | b | c` breaks on the same rule `&&` does; a condition with no operator at
/// depth zero explodes as a single indented element.
///
/// A ternary condition is the same span in the same parentheses [`build_paren_group`] would lay out,
/// so it splits at its arms here too — otherwise `while (a ? b : c ? d : e)` and `x = (a ? b : c ? d
/// : e)` would disagree about a construct that is bracket-for-bracket identical.
pub(super) fn build_cond_doc(inner: &[Token]) -> Doc {
    if !is_balanced(inner) {
        return Doc::Text(format!("({})", render_segment(inner)));
    }
    if let Some((segments, ops)) = split_chain(inner) {
        return build_clause_group(segment_docs(&segments), chain_seps(&ops), Fit::Measured);
    }
    if let Some((arms, seps, fit)) = ternary_layout(inner) {
        return build_clause_group(arms, seps, fit);
    }
    build_clause_doc(inner, |_| false, "")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tok(kind: TokenKind, text: &'static str) -> Token<'static> {
        Token { kind, text }
    }

    #[test]
    fn render_segment_collapses_whitespace() {
        // Leading/trailing trivia trimmed, inner trivia collapsed to one space.
        let toks = [
            tok(TokenKind::Whitespace, "  "),
            tok(TokenKind::Ident, "a"),
            tok(TokenKind::Whitespace, " "),
            tok(TokenKind::Newline, "\n"),
            tok(TokenKind::Ident, "b"),
            tok(TokenKind::Whitespace, "\t"),
        ];
        assert_eq!(render_segment(&toks), "a b");
    }

    #[test]
    fn render_segment_empty_for_all_trivia() {
        let toks = [
            tok(TokenKind::Whitespace, " "),
            tok(TokenKind::Newline, "\n"),
        ];
        assert_eq!(render_segment(&toks), "");
    }

    #[test]
    fn render_segment_single_token() {
        let toks = [tok(TokenKind::Number, "42")];
        assert_eq!(render_segment(&toks), "42");
    }

    #[test]
    fn build_expr_doc_nested_call_is_a_breakable_group() {
        // A call nested in an expression must render as its own group, not flat text: at a width
        // too narrow for it flat, its args explode one per line.
        use crate::lexer::tokenize;
        let toks = tokenize("bllll(aaaaaaaaaaaaaaaaaaaaaa, bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb)");
        let doc = build_expr_doc(&toks);
        assert_eq!(
            crate::doc::render(&doc, 10, 0, 0),
            "bllll(\n\taaaaaaaaaaaaaaaaaaaaaa,\n\tbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\n)"
        );
    }

    #[test]
    fn build_cond_doc_recursively_explodes_nested_call() {
        // Regression guard for issue #10: an operand that is itself an over-width call must
        // explode its own argument list, not stay flat.
        use crate::lexer::tokenize;
        let toks = tokenize(
            "io_detect_pin() && bllllaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa(aaaaaaaaaaaaaaaaaaaaaa, bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb)",
        );
        let doc = build_cond_doc(&toks);
        let rendered = crate::doc::render(&doc, 40, 0, 0);
        assert_eq!(
            rendered,
            "(\n\tio_detect_pin() &&\n\tbllllaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa(\n\t\taaaaaaaaaaaaaaaaaaaaaa,\n\t\tbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\n\t)\n)"
        );
    }

    #[test]
    fn build_expr_doc_tightens_call_across_a_newline_gap() {
        // Regression guard: a whitespace mutant can put a newline between a callee and its `(`.
        // `is_call_head` (strict adjacency) would miss this, collapsing the gap to a space instead
        // of dropping it — same-line, that space is then tightened by `space_call_heads` on the
        // *next* format pass, changing the output and breaking idempotency (issue found while
        // adding the cond-nested-call-explode fixture).
        use crate::lexer::tokenize;
        let toks = tokenize("io_detect_pin\n( )");
        let doc = build_expr_doc(&toks);
        assert_eq!(crate::doc::render(&doc, 80, 0, 0), "io_detect_pin()");
    }

    #[test]
    fn build_expr_doc_type_keyword_is_not_a_call_head() {
        // `int (*cb)` is a function-pointer declarator, not a call: `int` is a type keyword, which
        // `space_call_heads` always spaces (never tightens), so `call_head_before` must not treat
        // it as one either.
        use crate::lexer::tokenize;
        let toks = tokenize("int (*cb)(void)");
        let doc = build_expr_doc(&toks);
        assert_eq!(crate::doc::render(&doc, 80, 0, 0), "int (*cb)(void)");
    }

    #[test]
    fn build_call_doc_recursively_explodes_nested_call() {
        // A call whose argument is itself an over-width call: both levels must explode.
        use crate::lexer::tokenize;
        let toks = tokenize(
            "first_argument, inner_function_with_a_very_long_name(nested_argument_one, nested_argument_two, nested_argument_three)",
        );
        let doc = build_call_body(&toks, Fit::Measured);
        let rendered = crate::doc::render(&doc, 40, 0, 0);
        assert_eq!(
            rendered,
            "(\n\tfirst_argument,\n\tinner_function_with_a_very_long_name(\n\t\tnested_argument_one,\n\t\tnested_argument_two,\n\t\tnested_argument_three\n\t)\n)"
        );
    }
}
