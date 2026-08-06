//! Wadler/Leijen `Doc` builders. Every construct jphfmt breaks is one container — an argument list, a
//! `{}` or `enum` body, a `[…]` index, a `for` header, a condition, an operator chain, a ternary's
//! arms, a macro's parameters — laid out by [`build_container`] under §2.2's fits-flat-or-fully-broken
//! rule. Because the comma trails in an argument list, the operator trails in a chain and the ternary
//! `:` trails its arm: one rule, and per-construct values for how it is bracketed, what separates it,
//! whether a separator follows the last element on the break, and whether the width decides at all
//! (#71).
//!
//! Each builder turns a token slice into a [`Doc`] that [`crate::doc::render`] later flattens or fully
//! breaks. Depends on [`super::tokens`] for depth-aware splitting and balance checks.

use super::tokens::{
    closes_literal_type, element_join_respaced, has_middle_newline, has_non_trivia, has_top_level,
    has_top_level_question, holds_directive, is_balanced, is_bit_field_colon, is_call_head_pair,
    is_comparison, is_subscript, is_ternary_chain, is_trivia, match_brace, match_bracket,
    next_nontrivia, opens_with_separator, operand_span, prev_nontrivia, respaced_when_joined,
    respaced_when_joined_top, segments_at, spans_lines, split_chain, split_designators,
    split_on_commas, split_top_level, split_top_level_with_cuts, star_gap_respaced,
};
use crate::doc::Doc;
use crate::lexer::{Token, TokenKind};

/// A call's argument list, brackets included: the elements are `,`-separated and the flat form is
/// tight (§2.5). `fit` is the caller's, because a `#define` whose body overflows has already decided
/// to break its parameters before this is built.
pub(super) fn build_call_body(inner: &[Token], fit: Fit) -> Doc {
    if !is_balanced(inner) {
        return render_passthrough("(", inner, ")");
    }
    // The structure pass's call handler refuses a call whose arguments hold a break, and the calls
    // that reach here nested need the same contract: collapsing the break joins what the author
    // separated, and a later pass may respace the join — `f(a\n:0)` to `f(a :0)`, which
    // `space_bit_fields` tightens (#121's search).
    if has_middle_newline(inner) {
        return render_passthrough("(", inner, ")");
    }
    let args = split_on_commas(inner);
    // An empty element is a hole a macro invocation spells with a bare comma — `PICK(x, , y)`, valid C99
    // and later. There is no element to lay out for it, and dropping it drops the comma that spells it,
    // which changes the argument count and does not compile (#90). §6 prefers passthrough, and how a hole
    // is spaced is the one thing the layout has no rule for: an empty element takes no separator space
    // (#85), which would write `F(a,, b)` where every other C formatter writes `F(a, , b)`.
    //
    // A sole empty element is no hole — it is the empty list `f()`, with nothing between the parentheses
    // to hold apart.
    if args.iter().any(|arg| !has_non_trivia(arg)) {
        return if args.len() == 1 {
            Doc::text("()")
        } else {
            render_passthrough("(", inner, ")")
        };
    }
    // A separator cannot open an argument: laying `f(a, ;)` out would put the `,` gap before a `;`
    // that `space_semicolons` tightens on the next pass — the same refusal the brace and ternary
    // layouts make (#121's search).
    if args.iter().any(|arg| opens_with_separator(arg)) {
        return render_passthrough("(", inner, ")");
    }
    // A sole argument's span is exactly the span of these parens, so a chain of arms inside it needs
    // no pair of its own; with siblings, unbounded arms read as further arguments. A `{}` element is
    // bounded either way, because its list writes a trailing comma on the break (#59).
    let bound = if args.len() == 1 {
        Bound::Enclosing
    } else {
        Bound::Parens
    };
    let elements = args
        .into_iter()
        .map(|a| build_element_doc(a, bound))
        .collect();
    build_container(
        &pad_for(inner, &PARENS),
        elements,
        Seps::Every(","),
        None,
        fit,
    )
}

/// A `{}` or `enum` body: `,`-separated elements, a trailing comma when broken, and §2.3's magic
/// comma — a trailing comma in the source — which is the same forced fit a ternary chain takes.
/// `padded` is the flat form's inner space, `enum { A, B }` against `{1, 2}`.
pub(super) fn build_brace_doc(inner: &[Token], padded: bool) -> Doc {
    if !is_balanced(inner) {
        return render_passthrough("{", inner, "}");
    }
    let segments = split_on_commas(inner);
    let magic = segments.len() > 1 && segments.last().is_some_and(|s| s.iter().all(is_trivia));
    // A hole here too (#90): an empty element that is not the *trailing* one, which §2.3 reads as the
    // magic comma. A braced list is not valid C with a hole in it, but it reaches here as a macro
    // argument — `MACRO({a, , b}, c)` — where the macro decides what the tokens mean, and the call-level
    // passthrough cannot see it because the braces put it a bracket deeper. An all-empty `{,}` is not a
    // hole: nothing is being held apart, and collapsing it to `{}` is what the suite already excuses.
    let holed = segments.iter().rev().skip(1).any(|s| !has_non_trivia(s))
        && segments.iter().any(|s| has_non_trivia(s));
    if holed || segments.iter().any(|s| opens_with_separator(s)) {
        return render_passthrough("{", inner, "}");
    }
    let elements: Vec<&[Token]> = segments.into_iter().filter(|s| has_non_trivia(s)).collect();
    if elements.is_empty() {
        return Doc::text("{}");
    }
    let bracketing = pad_for(
        inner,
        &Bracketing::Written {
            open: "{",
            close: "}",
            open_pad: if padded { Pad::Spaced } else { Pad::Tight },
            close_pad: if padded { Pad::Spaced } else { Pad::Tight },
        },
    );
    let docs = elements.iter().map(|e| build_juxtaposed_doc(e)).collect();
    let fit = if magic { Fit::Forced } else { Fit::Measured };
    build_container(&bracketing, docs, Seps::Every(","), Some(","), fit)
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
            .skip(1),
    )
}

/// Whether the nearest non-trivia token before `open` names a callee ([`super::tokens::is_callee_ident`]). Unlike
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
    is_call_head_pair(toks, open)
}

/// Build one element/argument: collapsed text, with any nested `{...}` or nested call `f(...)`
/// rendered as its own group so it collapses or explodes independently of its parent.
/// A statement or element at the top of its container: a chain or ternary here is bounded by
/// parentheses when it breaks, because nothing else bounds it. Its operands go through
/// [`build_expr_doc`], which never adds a token — a bounded operand would gain another pair as its
/// indent deepened, one per pass.
fn build_element_doc(toks: &[Token], headless: Bound) -> Doc {
    // The terminal fallback's collapse is the one path without a refusal: a bare `Ident : Number`
    // break reaches it on the for-clause and statement-expression paths, and joining it writes the
    // shape `space_bit_fields` tightens. The author's text, newline included, is the fixpoint the
    // refusal cannot write — the arms the group and call handlers already join to the canonical
    // tight form take no refusal here. The edges are trimmed to the non-trivia core: a container's
    // own separator owns those gaps, and the previous pass's indentation is trivia this pass would
    // otherwise double.
    if element_join_respaced(toks) {
        let first = toks.iter().position(|t| !is_trivia(t)).unwrap_or(0);
        let last = toks
            .iter()
            .rposition(|t| !is_trivia(t))
            .unwrap_or(toks.len());
        return Doc::text(
            toks[first..=last]
                .iter()
                .map(|t| t.text)
                .collect::<String>(),
        );
    }
    if is_balanced(toks)
        && let Some(bounded) = build_chain_doc(toks, headless)
    {
        return bounded;
    }
    build_expr_doc(toks)
}

/// The bracketing a `(` or `[` opens when it is a *group* this pass lays out. A call's `(` is matched
/// before this by [`call_head_before`] and a `{` body is a different container, so neither is here.
pub(super) fn group_bracketing(t: &Token) -> Option<&'static Bracketing> {
    match t.text {
        "(" => Some(&PARENS),
        "[" => Some(&BRACKETS),
        _ => None,
    }
}

/// Flush the text accumulated so far into `parts`, so a nested group can be pushed as its own [`Doc`].
/// `space` is whether a pending gap becomes one. A bracket §2.5 writes tight drops it instead: a call's
/// `(` against its callee, and whatever [`tight_against_previous`] recognizes.
fn flush_pending(text: &mut String, parts: &mut Vec<Doc>, pending: &mut bool, space: bool) {
    // A pending gap survives the flush even when the text before it was already pushed as a part —
    // a call's callee is its own part, and the space its `{` takes (`a() {`) is the spacing pass's,
    // not this flush's to drop (#121's search).
    if space && *pending {
        if !text.is_empty() {
            text.push(' ');
        } else if !parts.is_empty() {
            parts.push(Doc::text(" "));
        }
    }
    *pending = false;
    if !text.is_empty() {
        parts.push(Doc::Text(std::mem::take(text)));
    }
}

/// Whether the bracket at `open` is written tight against what precedes it (§2.5): a subscript's `[`
/// against the value it indexes, or a compound literal's `{` against its `(T)`.
///
/// The mirror of [`call_head_before`], and load-bearing for the same reason — see its doc. The `[`
/// arm is the shared predicate, the one spelling of what `space_subscripts` tightens.
fn tight_against_previous(toks: &[Token], open: usize) -> bool {
    let previous = prev_nontrivia(toks, open);
    match toks.get(open).map(|t| t.text) {
        Some("[") => is_subscript(toks, open),
        Some("{") => previous.is_some_and(|k| toks[k].text == ")" && closes_literal_type(toks, k)),
        _ => false,
    }
}

fn build_expr_doc(toks: &[Token]) -> Doc {
    if is_balanced(toks)
        && let Some((segments, ops)) = split_chain(toks)
    {
        // The same refusals the chain path makes in `is_boundable`: a span whose width the model
        // cannot describe (an unterminated literal spanning lines) or whose `#` a later pass
        // rewrites gets no conjunct parens either (#134's review).
        let unboundable = span_unmeasurable(toks);
        // #52's conjunct: a single comparison whose left operand is one whole call reads as one
        // term — its flat form is the call's, and its break belongs inside the call's arguments,
        // not at the operator, which stays with its right operand on the call's close line. The
        // headless position bounds it, an [`Bracketing::OnBreak`] in all but spelling.
        if !unboundable && let Some((elements, seps)) = conjunct_element(&segments, &ops) {
            return build_bounded_doc("", elements, seps, Fit::Measured, Bound::Parens);
        }
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
            flush_pending(
                &mut text,
                &mut parts,
                &mut pending_space,
                !tight_against_previous(toks, j),
            );
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
            flush_pending(&mut text, &mut parts, &mut pending_space, false);
            parts.push(build_call_body(&toks[j + 1..close], Fit::Measured));
            j = close + 1;
        } else if t.kind == TokenKind::Punct
            && let Some(bracketing) = group_bracketing(&t)
            && let Some(close) = match_bracket(toks, j)
        {
            if let Some(group) = build_bracketed_group(&toks[j + 1..close], bracketing) {
                flush_pending(
                    &mut text,
                    &mut parts,
                    &mut pending_space,
                    !tight_against_previous(toks, j),
                );
                parts.push(group);
            } else if respaced_when_joined_top(&toks[j + 1..close]) {
                // The group was refused, and joining its own break would hand the spacing pass a
                // shape it respaces — keep the author's text, newline included, instead of collapsing
                // it: the same refusal the chain head makes, one bracket in (#121's class). A nested
                // break is the nested group's own to refuse.
                flush_pending(
                    &mut text,
                    &mut parts,
                    &mut pending_space,
                    !tight_against_previous(toks, j),
                );
                text.push_str(&toks[j..=close].iter().map(|t| t.text).collect::<String>());
            } else {
                // The group was refused for a reason this text does not model; let the generic arm
                // collapse it, as before. A same-line `=` against a bracket needs no pad of its own:
                // `space_equals` runs first and pre-spaces every same-line `=`, and the collapse
                // preserves that trivia, so this pass cannot write the tight form (#121's search).
                if pending_space && !tight_against_previous(toks, j) {
                    text.push(' ');
                }
                pending_space = false;
                text.push_str(t.text);
                j += 1;
                continue;
            }
            j = close + 1;
        } else {
            // A bracket the author left a gap before is still tight (§2.5), even when it has nothing to
            // lay out and falls through to here: a space would be tightened on the next pass.
            if pending_space && !tight_against_previous(toks, j) {
                text.push(' ');
            }
            pending_space = false;
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

/// What separates a container's elements. Two cases, and the domain has only these two: an operator
/// chain's separator differs between elements (`a | b && c` cannot happen, but `a - b + c` can), while
/// every other construct here repeats one string — a comma list, a ternary's ` :`, a `for` header's `;`.
///
/// [`Each`](Seps::Each) owns its strings deliberately: [`chain_seps`] builds ` |` from the operator's
/// text plus a leading space, so those do not exist to borrow. Every other site names its separator as
/// a literal — except two, which name none: [`build_cond_doc`]'s single-element condition and
/// #52's parenthesised conjunct are not lists, and say so with an empty `Each` rather than a
/// string they would never read.
///
/// Which removes the *construction* and nothing further: `Doc::Text` owns its string, so
/// [`trailing_items`] still allocates one per gap for [`Every`](Seps::Every) where the `Vec<String>`
/// this replaced allocated the same count at the call site and moved them. The count is unchanged and
/// the claim in #76 was that the type states the rule; a `Doc` variant that could hold a `&'static
/// str` is what would remove them, and that is the IR's change to make, not this one's.
///
/// Consumed once, by value, and never cloned, compared or printed — hence no derives at all.
pub(super) enum Seps {
    Every(&'static str),
    Each(Vec<String>),
}

/// `segments` with a separator from `seps` trailing each one but the last: flat `a sep b`, or one
/// element per line with the separator ending each (§2.4, §2.7).
///
/// The gap after a separator is what the element following it is worth: a [`Doc::Line`] before content,
/// a [`Doc::SoftLine`] before nothing. So an empty element takes no space in the flat form — `for (;;)`,
/// not `for (; ; )` (#85) — while the broken form is untouched, because both break the same way.
///
/// A gap exists only *between* elements, so pairing the separators with the gaps is what says the last
/// element takes none — one rule, holding for both [`Seps`] and for a container of one, rather than a
/// bound each variant has to carry. The trailing separator a container does write is
/// `build_container`'s own, which knows its bracket.
///
/// Takes `seps` by value so [`Seps::Each`]'s strings move into their [`Doc::Text`]: they were built by
/// [`chain_seps`] one allocation each, and borrowing them here would have bought a second.
///
/// The empty-element rule reads [`Doc::is_empty`], which answers `false` for a [`Doc::Group`] however
/// empty its contents — so an element that is a group around nothing takes a [`Doc::Line`] and a space
/// the flat form does not need. No builder here produces one; the guarantee is `for (;;)`'s, and it
/// holds for the text and concat elements that reach this.
fn trailing_items(segments: Vec<Doc>, seps: Seps) -> Vec<Doc> {
    let gaps = segments.iter().skip(1).map(|next| {
        if next.is_empty() {
            Doc::SoftLine
        } else {
            Doc::Line
        }
    });
    let mut trailing = match seps {
        Seps::Every(sep) => gaps.map(|gap| [Doc::text(sep), gap]).collect::<Vec<_>>(),
        Seps::Each(each) => {
            // One per gap, not "as many as there are": a short `Each` would leave the elements past
            // it juxtaposed with neither a separator *nor* a gap — `a |bc` — which is a construct no
            // builder here has, and a silent merge rather than a visible mistake. `chain_seps` yields
            // exactly one operator per gap, and `build_cond_doc`'s condition and #52's conjunct
            // pair one element with none, so this holds for every producer; it is asserted so the
            // next one cannot quietly not.
            //
            // A real assertion rather than a `debug_assert`, which release builds compile out: the
            // failure it catches is *silently merged tokens*, and this file's whole subject is that
            // losing what the author wrote must never pass quietly. One integer compare per container
            // against a `Vec` this function already allocates is not a cost worth trading for it.
            assert_eq!(
                each.len(),
                segments.len().saturating_sub(1),
                "a separator per gap: {each:?} against {} elements",
                segments.len()
            );
            each.into_iter()
                .zip(gaps)
                .map(|(sep, gap)| [Doc::Text(sep), gap])
                .collect()
        }
    }
    .into_iter();
    let mut items = Vec::with_capacity(segments.len() * 3);
    for seg in segments {
        items.push(seg);
        if let Some(pair) = trailing.next() {
            items.extend(pair);
        }
    }
    items
}

/// A bracket's inner space in the flat form: `{1, 2}` and `f(a, b)` against `enum { A, B }`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Pad {
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
pub(super) enum Bracketing {
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
        open_pad: Pad,
        close_pad: Pad,
    },
    /// Brackets that appear only on the break, after `head` when there is one — the only tokens
    /// jphfmt writes, legal because the elements are already an implicit container.
    ///
    /// The head is a document, not text. It holds whatever preceded the operands — an assignment's
    /// left side, a `return` — and a call or a group in there is a construct with a width of its own.
    /// Rendering it flat measured nothing, so a call too long for the line stayed flat on the first
    /// pass and was laid out on the second, once the operands below it had broken and the span
    /// reached a different handler (#108).
    OnBreak { head: Doc },
}

/// The one layout every container in the language gets (§2.2): the elements in order, each `seps[i]`
/// *trailing* its element, `trailing` after the last one only when broken (§2.3's magic comma), all of
/// it bracketed per `bracketing` and flat-or-broken per `fit`.
///
/// An argument list, a `{}` or `enum` body, a `[…]` index, a `for` header, a condition, an operator
/// chain, a ternary's arms and a macro's parameters are the same construct: because the comma trails,
/// the operator trails and the ternary `:` trails. What differs between them is the four values passed
/// here, not the shape they are laid out in (#71).
fn build_container(
    bracketing: &Bracketing,
    elements: Vec<Doc>,
    seps: Seps,
    trailing: Option<&str>,
    fit: Fit,
) -> Doc {
    let mut items = trailing_items(elements, seps);
    items.extend(trailing.map(|text| Doc::IfBreak {
        broken: text.to_owned(),
        flat: String::new(),
    }));
    let nested =
        |lead: Doc, items: Vec<Doc>| Doc::nest(Doc::concat(std::iter::once(lead).chain(items)));
    match bracketing {
        Bracketing::Enclosing => fit.wrap(Doc::concat(items)),
        Bracketing::Hanging => fit.wrap(Doc::nest(Doc::concat(items))),
        Bracketing::Written {
            open,
            close,
            open_pad,
            close_pad,
        } => fit.wrap(Doc::concat([
            Doc::text(*open),
            nested(open_pad.doc(), items),
            close_pad.doc(),
            Doc::text(*close),
        ])),
        // The head sits *outside* the group the operands break as. Inside it, the head's own groups
        // would be measured together with the operands and break whenever the operands did — while
        // the next pass, reading the parentheses this wrote, measures each of them alone and reaches
        // the other answer. A head that is text cannot show that; a head that is a document can, so
        // the two must not share a fit.
        Bracketing::OnBreak { head } => Doc::concat(
            (!head.is_empty())
                .then(|| Doc::concat([head.clone(), Doc::text(" ")]))
                .into_iter()
                .chain([fit.wrap(Doc::concat([
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
                ]))]),
        ),
    }
}

/// The author's `(…)` around a clause run: a `for` header, a condition, a parenthesized chain.
pub(super) const PARENS: Bracketing = Bracketing::Written {
    open: "(",
    close: ")",
    open_pad: Pad::Tight,
    close_pad: Pad::Tight,
};

/// The author's `[…]` around an index.
pub(super) const BRACKETS: Bracketing = Bracketing::Written {
    open: "[",
    close: "]",
    open_pad: Pad::Tight,
    close_pad: Pad::Tight,
};

/// Whether an edge's pad flattens spaced: `space_equals` writes a space on both sides of every
/// same-line `=`, pad or no pad, so a tight pad beside one would be respaced on the next pass —
/// `a(= "")` is not a fixpoint of the spacing pass, `a( = "")` is. Each edge answers for its own
/// token, so a `=` on one edge spaces that edge alone and the other keeps §2.5's tight form.
fn edge_needs_pad(inner: &[Token], edge: Option<usize>) -> bool {
    edge.is_some_and(|k| inner[k].text == "=")
}

/// `bracketing`'s spelling, with the pads the edge tokens decide.
fn pad_for<'a>(inner: &[Token], bracketing: &Bracketing<'a>) -> Bracketing<'a> {
    let Bracketing::Written {
        open,
        close,
        open_pad,
        close_pad,
    } = bracketing
    else {
        return bracketing.clone();
    };
    Bracketing::Written {
        open,
        close,
        open_pad: if edge_needs_pad(inner, next_nontrivia(inner, 0)) {
            Pad::Spaced
        } else {
            *open_pad
        },
        close_pad: if edge_needs_pad(inner, prev_nontrivia(inner, inner.len())) {
            Pad::Spaced
        } else {
            *close_pad
        },
    }
}

/// `open`/`close` around `inner`'s collapsed text, with the pads the edge tokens decide — the
/// passthrough a hole or an imbalance takes instead of a layout, which must agree with the spacing
/// pass exactly as a laid-out container's pad does.
fn render_passthrough(open: &str, inner: &[Token], close: &str) -> Doc {
    // A passthrough is the author's text: when it holds a line break, collapsing it would join what
    // the author separated, and a later pass may respace the join — the same refusal the layouts
    // make, spelled as the author's own text, which already carries the spacing the pass wrote
    // (#121's class). Only the collapsed form takes a pad of its own: `space_equals` runs first and
    // pre-spaces every same-line `=` edge, so the verbatim form cannot write the tight one.
    let (text, open_pad, close_pad) = if inner.iter().any(|t| t.kind == TokenKind::Newline) {
        (inner.iter().map(|t| t.text).collect(), "", "")
    } else {
        (
            render_segment(inner),
            if edge_needs_pad(inner, next_nontrivia(inner, 0)) {
                " "
            } else {
                ""
            },
            if edge_needs_pad(inner, prev_nontrivia(inner, inner.len())) {
                " "
            } else {
                ""
            },
        )
    };
    Doc::Text(format!("{open}{open_pad}{text}{close_pad}{close}"))
}

/// Whether `toks` is one expression jphfmt may bound with parentheses of its own.
///
/// A depth-zero `,` means it is a list — a second declarator, or a comma expression — and
/// `(a | b, c)` is not `a | b, c`. A token carrying a line break is an unterminated literal, which the
/// width model does not describe ([`crate::doc::display_width`] measures one line), and a `#` means the
/// span is a directive fragment whose column a later pass rewrites. Bounding either would decide a
/// layout from a width that the next pass measures differently.
/// Whether a span's width is one this pass may decide from: a line break inside a *token* is an
/// unterminated literal spanning lines, which a one-line width cannot describe, and a `#` means a
/// directive fragment whose column a later pass rewrites. The one spelling — the chain path
/// (`is_boundable`) and the conjunct fallback both refuse the same spans (#134's review).
fn span_unmeasurable(toks: &[Token]) -> bool {
    spans_lines(toks) || toks.iter().any(|t| t.text == "#")
}

fn is_boundable(toks: &[Token], operands: &[Token]) -> bool {
    // A line break inside a *token* is an unterminated literal spanning lines, which a one-line
    // width cannot describe, and a `#` means a directive fragment whose column a later pass
    // rewrites. Either anywhere in the construct — head included — and the width this decides from
    // is not the width the next pass measures. A tab needs no refusal: `display_width` counts the
    // columns it occupies, the same as every other measure in the pipeline.
    //
    // Any `#`, not only [`super::tokens::holds_directive`]'s: narrowing this to a directive broke
    // idempotency on `]{'((.AA…'0}#A*:?` at width 57, where the `#` names nothing. Whatever a `#` here
    // is, its spacing is not settled until the passes that own it have run (#112's review).
    if span_unmeasurable(toks) {
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
fn build_bounded_doc(head: Doc, segments: Vec<Doc>, seps: Seps, fit: Fit, bound: Bound) -> Doc {
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
        Bound::Parens => Bracketing::OnBreak { head },
    };
    build_container(&bracketing, segments, seps, None, fit)
}

/// An operator chain or ternary with no parentheses of its own: flat, or one operand per line with the
/// operator trailing, bounded by parentheses [`build_bounded_doc`] adds on the break. A #52 conjunct —
/// one comparison whose left operand is one whole call — is the exception: it reads as a single term,
/// so it breaks inside its call's arguments and the operator stays with its right operand on the
/// call's close line, the single element of a one-element [`build_bounded_doc`].
pub(super) fn build_chain_doc(toks: &[Token], headless: Bound) -> Option<Doc> {
    let start = operand_span(toks);
    let operands = &toks[start..];
    if !is_boundable(toks, operands) {
        return None;
    }
    // The head renders as collapsed text. Collapsing a newline that separates an `Ident : Number` (or
    // a `;` from its predecessor) hands the spacing pass a shape it rewrites — the same refusal
    // `emit_brace` makes for a `{}` list, on the one path that lacked it (#121).
    if respaced_when_joined(&toks[..start]) {
        return None;
    }
    // Through the same builder the operands go through, not [`render_segment`]: whatever is in the
    // head is a construct with its own width, and rendering it flat measured none of them (#108).
    let head = build_expr_doc(&toks[..start]);
    // A head means these operands are only part of their container's span, so they are bounded
    // whatever they are; with no head it is the position that decides, and it decides the same for a
    // ternary and for a binary chain — unbounded operands read as elements of whatever list encloses
    // them either way (#59, #63).
    let bound = if start == 0 { headless } else { Bound::Parens };
    if let Some((segments, ops)) = split_chain(operands) {
        // A segment's collapse joins the break its span holds, so a segment that would join a
        // respaced pair is refused the way the head is — the canonical reading, since a segment's
        // group and call arms join those shapes themselves, and a nested construct inside the
        // segment refuses its own breaks (#121's search).
        if segments.iter().any(|s| element_join_respaced(s)) {
            return None;
        }
        // #52's conjunct, at the statement's own level: one bounded element, so the head leads
        // and the OnBreak parentheses the `bound` decides wrap the broken form — the pass the
        // clause branch re-reads later agrees.
        if let Some((elements, seps)) = conjunct_element(&segments, &ops) {
            return Some(build_bounded_doc(
                &head,
                elements,
                seps,
                Fit::Measured,
                bound,
            ));
        }
        return Some(build_bounded_doc(
            head,
            segment_docs(&segments),
            chain_seps(&ops),
            Fit::Measured,
            bound,
        ));
    }
    // §2.4's chain, with the `:` trailing, for a ternary the author left unparenthesized.
    let (arms, seps, fit) = ternary_layout(operands)?;
    Some(build_bounded_doc(head, arms, seps, fit, bound))
}

/// The trailing separators for an operator chain: ` |`, ` &&`, and so on. The one construct whose
/// separators differ between elements, so the one that owns a string per gap.
fn chain_seps(ops: &[&str]) -> Seps {
    Seps::Each(ops.iter().map(|op| format!(" {op}")).collect())
}

/// A ternary's arms as documents with the ` :` that trails each, and whether the width decides —
/// the whole of what the three places a ternary can appear need from one.
fn ternary_layout(inner: &[Token]) -> Option<(Vec<Doc>, Seps, Fit)> {
    let arms = ternary_arms(inner)?;
    Some((
        segment_docs(&arms),
        Seps::Every(" :"),
        Fit::of_ternary(inner),
    ))
}

/// The `:`-separated arms of a ternary, or `None` if any arm is missing its operand — a stranded
/// separator would put this layout's spacing where the author had none.
fn ternary_arms<'a, 'src>(inner: &'a [Token<'src>]) -> Option<Vec<&'a [Token<'src>]>> {
    if !has_top_level_question(inner) {
        return None;
    }
    // A `:` the spacing pass reads as a bit-field's is not an arm separator: laying these arms out
    // would write ` : ` where that pass writes `: `, and its output would be respaced (#121's search).
    // A label's colon whose statement opens with a number reads as the same shape, and is refused
    // with the rest — passing the whole statement through is the §6 cost of the over-broad reading.
    // A `*` arm's gap to a `:` is respaced the same way when the star reads as a declarator's —
    // `*:?` laid out as `* :` respaces to `*:` — so such an arm is refused too.
    let (_, all_cuts) =
        split_top_level_with_cuts(inner, |t| t.kind == TokenKind::Punct && t.text == ":");
    if all_cuts
        .iter()
        .any(|&j| is_bit_field_colon(inner, j) || star_gap_respaced(inner, j))
    {
        return None;
    }
    let arms = segments_at(inner, &all_cuts);
    // A separator cannot open an arm: laying `? : ;` out would put the ` :` gap before a `;` that
    // `space_semicolons` tightens on the next pass, and this pass's output would be respaced — the
    // same class, one separator over (#121's search). An arm that would join a respaced pair is
    // refused the way a chain's segment is.
    (arms.len() >= 2
        && arms.iter().all(|s| has_non_trivia(s))
        && arms.iter().all(|s| !opens_with_separator(s))
        && arms.iter().all(|s| !element_join_respaced(s)))
    .then_some(arms)
}

/// Each segment as its own expression, paired with the separators that trail them.
fn segment_docs(segments: &[&[Token]]) -> Vec<Doc> {
    segments.iter().map(|s| build_expr_doc(s)).collect()
}

/// #52's conjunct: [`split_chain`]'s shape where the chain is one comparison and the left operand
/// is one whole call. The flat form writes no parentheses, and the break writes one pair around
/// the call and the operator's right operand where the caller's bound is [`Bound::Parens`] — a
/// sole call argument ([`Bound::Enclosing`]) writes none, the enclosing call's own parens bounding
/// the operands instead.
/// #52's conjunct as a container's single element: the operator lives inside the term, so the
/// container separates nothing and names no separators. The three conjunct sites all build this
/// same one-element shape, differing only in the container they wrap it in.
fn conjunct_element(segments: &[&[Token]], ops: &[&str]) -> Option<(Vec<Doc>, Seps)> {
    comparison_conjunct(segments, ops).map(|conjunct| (vec![conjunct], Seps::Each(Vec::new())))
}

fn comparison_conjunct(segments: &[&[Token]], ops: &[&str]) -> Option<Doc> {
    let [left, right] = segments else {
        return None;
    };
    let [op] = ops else {
        return None;
    };
    if !is_comparison(op) {
        return None;
    }
    // The left segment is one whole call: a callee identifier followed by its matching close at
    // the segment's last non-trivia token. The pair check comes first — it inspects only `open`
    // and its predecessor, so a left like `a[i] == b` never pays the bracket walk.
    let callee = next_nontrivia(left, 0)?;
    let open = next_nontrivia(left, callee + 1)?;
    if !is_call_head_pair(left, open) {
        return None;
    }
    let close = match_bracket(left, open)?;
    if prev_nontrivia(left, left.len()) != Some(close) || element_join_respaced(right) {
        return None;
    }
    Some(Doc::concat([
        build_expr_doc(left),
        Doc::text(format!(" {op} ")),
        build_expr_doc(right),
    ]))
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

/// A chain or a ternary inside brackets the author wrote — what a parenthesized group, an
/// `if`/`while`/`switch` condition and a `[…]` index all are, differing only in `bracketing`.
///
/// A ternary belongs here as much as a chain does: [`build_chain_doc`] bounds a bare one with
/// parentheses, and this is the same content on the next pass, so both must reach the same layout or
/// neither is a fixpoint.
fn build_clause_contents(inner: &[Token], bracketing: &Bracketing) -> Option<Doc> {
    if let Some((segments, ops)) = split_chain(inner) {
        // The same conjunct, wrapped in the author's own parens — the form a previous pass's
        // layout re-reads, so it must lay out to the same shape.
        if let Some((elements, seps)) = conjunct_element(&segments, &ops) {
            return Some(build_container(
                bracketing,
                elements,
                seps,
                None,
                Fit::Measured,
            ));
        }
        // The gate the other segment consumers have, the canonical reading: a segment whose
        // collapse would join a respaced pair is refused, and the caller's own fallback keeps the
        // break (#121's search).
        if segments.iter().any(|s| element_join_respaced(s)) {
            return None;
        }
        return Some(build_container(
            bracketing,
            segment_docs(&segments),
            chain_seps(&ops),
            None,
            Fit::Measured,
        ));
    }
    let (arms, seps, fit) = ternary_layout(inner)?;
    Some(build_container(bracketing, arms, seps, None, fit))
}

/// A bracketed group the author wrote — `(…)` around an expression, `[…]` around an index. The
/// subscript is the same container an argument list is, in a different pair, so `arr[a ? b : c]` needs
/// no bound of its own and breaks on the same rule (#77).
///
/// The author's brackets do not exempt the span from the width model: a literal running to the end of
/// the file has no one-line width, so every group holding one passes through, exactly as
/// `is_boundable` refuses one a chain would have bounded. [`build_cond_doc`] deliberately does not
/// take that refusal — it has a fallback layout to reach instead of a passthrough — which is why the
/// guard lives here rather than in [`build_clause_contents`].
///
/// Takes no comment or balance guard of its own, and needs none: `super::structure::emit_tokens`
/// refuses a comment-bearing or unbalanced construct before any of this module runs, so a span that
/// reaches here has neither. That matters because flattening a `//` comment would put whatever
/// followed it on the comment's line and swallow it — the layout must never see one.
pub(super) fn build_bracketed_group(inner: &[Token], bracketing: &Bracketing) -> Option<Doc> {
    if spans_lines(inner) || holds_directive(inner) {
        return None;
    }
    build_clause_contents(inner, &pad_for(inner, bracketing))
}

/// `for (init; cond; step)` — one clause per line when broken (§2.4).
///
/// Each clause is an *element* of this container, not a bare expression, so a chain or ternary inside
/// one is bounded when it breaks (#77). Unbounded, its arms would sit at the clause indent and read as
/// further clauses — the same reason a call's arguments bound theirs (#59) — and a ternary chain
/// forces the break, so `for (i = a ? b : c ? d : e; …)` reads as the map it is.
pub(super) fn build_for_doc(inner: &[Token]) -> Doc {
    if !is_balanced(inner) {
        return render_passthrough("(", inner, ")");
    }
    let clauses = statement_segments(inner);
    let docs = clauses.iter().map(|c| build_statement_element(c)).collect();
    build_container(
        &pad_for(inner, &PARENS),
        docs,
        Seps::Every(";"),
        None,
        Fit::Measured,
    )
}

/// Split a `;`-separated run into its elements — a `for` header's clauses, or a statement-expression
/// body's statements. The two constructs are the same shape and `super::structure::format_stmt_expr`
/// splits the other one, so the predicate lives here rather than once in each.
pub(super) fn statement_segments<'a, 'src>(inner: &'a [Token<'src>]) -> Vec<&'a [Token<'src>]> {
    split_top_level(inner, |t| t.kind == TokenKind::Punct && t.text == ";")
}

/// One element of a `;`-separated run. Bounded, because it has siblings: unbounded, a chain's operands
/// would sit at the element indent and read as further clauses or statements, exactly as they would
/// read as further arguments in a call (#59, #63, #77). Both callers go through here so that decision
/// has one home and cannot drift between them.
pub(super) fn build_statement_element(toks: &[Token]) -> Doc {
    build_element_doc(toks, Bound::Parens)
}

/// An `if`/`while`/`switch` condition — split on its loosest-binding operator with that operator
/// trailing (§2.7), so `a | b | c` breaks on the same rule `&&` does; a condition with no operator at
/// depth zero explodes as a single indented element.
///
/// A ternary condition is the same span in the same parentheses [`build_bracketed_group`] would lay
/// out, so it splits at its arms here too — otherwise `while (a ? b : c ? d : e)` and
/// `x = (a ? b : c ? d : e)` would disagree about a construct that is bracket-for-bracket identical.
pub(super) fn build_cond_doc(inner: &[Token]) -> Doc {
    if !is_balanced(inner) {
        return render_passthrough("(", inner, ")");
    }
    build_clause_contents(inner, &pad_for(inner, &PARENS)).unwrap_or_else(|| {
        // No depth-zero operator to split at, so the whole condition is one element: an overlong one
        // still breaks away from the `if (` and the `) {` rather than overrunning them. A condition is
        // not a list, so it names no separator — where a call's sole argument still writes
        // [`Seps::Every`] because a comma list of one is still a comma list.
        //
        // The empty [`Seps::Each`] says that for *this* element and no others, and the `vec!` below is
        // where that holds: a second element would be juxtaposed against the first with nothing
        // between them, since the pairing runs out. Any element added here needs a separator named.
        build_container(
            &pad_for(inner, &PARENS),
            vec![build_element_doc(inner, Bound::Enclosing)],
            Seps::Each(Vec::new()),
            None,
            Fit::Measured,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tok(kind: TokenKind, text: &'static str) -> Token<'static> {
        Token { kind, text }
    }

    /// The separators and gaps `trailing_items` emitted, as text — `~` for a [`Doc::Line`] and `.` for
    /// a [`Doc::SoftLine`], so where a gap went is visible and not only that one did. Neither marker
    /// appears in any separator, so what is a gap and what is a separator cannot be confused.
    fn placed(segments: &[&str], seps: Seps) -> String {
        trailing_items(segments.iter().map(|s| Doc::text(*s)).collect(), seps)
            .iter()
            .map(|item| match item {
                Doc::Text(text) => text.clone(),
                Doc::Line => "~".to_owned(),
                Doc::SoftLine => ".".to_owned(),
                other => format!("{other:?}"),
            })
            .collect()
    }

    /// The one invariant the whole of [`Seps`] rests on: a separator goes in every *gap*, and a gap
    /// exists only between elements — so the last element takes none, whichever variant this is and
    /// however many elements there are. Asserted here because the `camas corpus` A/B that verified the
    /// conversion is a manual task, not part of the gate CI runs.
    #[test]
    fn trailing_items_separates_between_elements_only() {
        assert_eq!(placed(&["a", "b", "c"], Seps::Every(",")), "a,~b,~c");
        assert_eq!(placed(&["a"], Seps::Every(",")), "a");
        // The shape `build_cond_doc` emits: one element, no gap, and an `Each` naming nothing. The
        // same shape #52's conjunct sites emit — a condition and a conjunct are not lists, so the
        // "however many elements" claim above rests on them.
        assert_eq!(placed(&["a"], Seps::Each(Vec::new())), "a");
        assert_eq!(placed(&[], Seps::Every(",")), "");
        let each = Seps::Each(vec![" |".to_owned(), " &&".to_owned()]);
        assert_eq!(placed(&["a", "b", "c"], each), "a |~b &&~c");
        // An empty element takes a `SoftLine` rather than a `Line`, so `for (;;)` is not `for (; ; )`.
        assert_eq!(placed(&["a", "", "c"], Seps::Every(";")), "a;.;~c");
    }

    /// An [`Seps::Each`] as long as its gaps, which is the only shape there is one for — the length
    /// is a precondition `trailing_items` asserts, not a case it handles. A shorter one leaves the
    /// elements past it juxtaposed with neither a separator nor a gap, which no construct in the
    /// language is, so there is deliberately no test asserting that output: it would pin a merge as
    /// the expected answer where the `assert_eq!` says it is a caller's mistake.
    /// And it is a real assertion, not one release builds compile away — the failure it catches is
    /// silently merged tokens, which this suite may not let pass in the profile that ships.
    #[test]
    #[should_panic(expected = "a separator per gap")]
    fn a_short_each_is_refused_in_every_profile() {
        let _ = placed(&["a", "b", "c"], Seps::Each(vec![" |".to_owned()]));
    }

    #[test]
    fn trailing_items_pairs_each_operator_with_its_own_gap() {
        let ops = Seps::Each(vec![" |".to_owned(), " &&".to_owned(), " ^".to_owned()]);
        assert_eq!(placed(&["a", "b", "c", "d"], ops), "a |~b &&~c ^~d");
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
    fn build_call_body_recursively_explodes_nested_call() {
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
