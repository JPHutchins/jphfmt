//! A Wadler/Leijen pretty-printing document and a width-aware renderer implementing §2.2's
//! single rule: a group is laid out entirely flat if it fits the width, otherwise entirely
//! broken (every separator becomes a newline). There is deliberately no `fill` mode.

/// Columns a tab advances the cursor for the fits/overflow decision (§2.1, §8.5). Output still
/// emits literal tabs; this width is only used for measuring.
pub const TAB_WIDTH: usize = 4;

/// Display width of `s` in columns: one per `char`, [`TAB_WIDTH`] for a tab. A tab reaches here
/// inside a token — a string or character literal holding one — and counting it as a single column
/// measures the line narrower than it renders, which is how a line could pass the fits test and
/// still overrun §8.5's limit. A newline reaches here only from `fits`'s lookahead over a
/// passthrough text, where the over-count is deliberate — see the [`Doc::Text`] arm there.
pub fn display_width(s: &str) -> usize {
    s.chars()
        .map(|c| if c == '\t' { TAB_WIDTH } else { 1 })
        .sum()
}

/// A layout document. Built bottom-up, then rendered at a width.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Doc {
    /// Verbatim text. It may hold newlines — a passthrough keeps an author's break verbatim — and
    /// the render's cursor reads them: a line ends at each one, so the column after the text is the
    /// last line's tail, the column a group that follows it measures from (#134's review).
    Text(String),
    /// A space when the enclosing group is flat; a newline + indentation when broken.
    Line,
    /// Nothing when the enclosing group is flat; a newline + indentation when broken.
    SoftLine,
    /// Indent breaks inside the inner document by one further tab level.
    Nest(Box<Doc>),
    /// A left-to-right sequence.
    Concat(Vec<Doc>),
    /// Render flat if it fits the remaining width, otherwise broken.
    Group(Box<Doc>),
    /// `broken` when the enclosing group is broken, `flat` when flat — e.g. a trailing comma
    /// that appears only on explosion.
    IfBreak { broken: String, flat: String },
    /// Always broken, and reported as not-fitting so any enclosing group also breaks. Models the
    /// §2.3 magic trailing comma: its presence forces the list (and its parents) to explode.
    ForceBreak(Box<Doc>),
    /// A measurement boundary that renders as nothing: the fit lookahead stops here, so a group
    /// before it is measured with the budget its own line has and nothing past it. One measurement
    /// ends where another's begins — what follows has a fit of its own, and a width read across the
    /// boundary is a line the next pass does not keep (#108).
    Boundary,
}

impl Doc {
    pub fn text(s: impl Into<String>) -> Doc {
        Doc::Text(s.into())
    }
    pub fn concat(items: impl IntoIterator<Item = Doc>) -> Doc {
        Doc::Concat(items.into_iter().collect())
    }
    pub fn group(inner: Doc) -> Doc {
        Doc::Group(Box::new(inner))
    }
    pub fn nest(inner: Doc) -> Doc {
        Doc::Nest(Box::new(inner))
    }
    /// Whether this renders as nothing at every width. Only text and the measurement boundary can:
    /// every other variant either writes something or is a break, which is not nothing.
    pub fn is_empty(&self) -> bool {
        match self {
            Doc::Text(text) => text.is_empty(),
            Doc::Concat(items) => items.iter().all(Doc::is_empty),
            Doc::Nest(inner) => inner.is_empty(),
            Doc::Boundary => true,
            Doc::Line
            | Doc::SoftLine
            | Doc::Group(_)
            | Doc::IfBreak { .. }
            | Doc::ForceBreak(_) => false,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Flat,
    Break,
}

/// The width of `s`'s last line — the columns a cursor after the text sits at — and whether the
/// text held a line break before it. One walk, so the render's hot path pays one scan whether the
/// text is a plain one or a passthrough holding breaks (#134's review).
fn last_line_width(s: &str) -> (usize, bool) {
    let (mut width, mut broken) = (0, false);
    for c in s.chars() {
        match c {
            '\n' | '\r' => {
                width = 0;
                broken = true;
            }
            '\t' => width += TAB_WIDTH,
            _ => width += 1,
        }
    }
    (width, broken)
}

/// Render `doc`: groups that fit within `width` columns stay flat, the rest break fully.
/// `start_col` is the cursor column before the document; `base_level` is the indentation, in tab
/// levels, that broken lines and the closing delimiter return to.
pub fn render(doc: &Doc, width: usize, start_col: usize, base_level: usize) -> String {
    let mut out = String::new();
    let mut col = start_col;
    let mut stack: Vec<(usize, Mode, &Doc)> = vec![(base_level, Mode::Break, doc)];
    while let Some((level, mode, d)) = stack.pop() {
        match d {
            Doc::Text(s) => {
                out.push_str(s);
                let (width, broken) = last_line_width(s);
                if broken {
                    col = width;
                } else {
                    col += width;
                }
            }
            Doc::Concat(items) => {
                for child in items.iter().rev() {
                    stack.push((level, mode, child));
                }
            }
            Doc::Nest(inner) => stack.push((level + 1, mode, inner)),
            Doc::Line | Doc::SoftLine => match mode {
                Mode::Flat => {
                    if matches!(d, Doc::Line) {
                        out.push(' ');
                        col += 1;
                    }
                }
                Mode::Break => {
                    out.push('\n');
                    for _ in 0..level {
                        out.push('\t');
                    }
                    col = level * TAB_WIDTH;
                }
            },
            Doc::Group(inner) => {
                let mode = if fits(width.saturating_sub(col), inner, &stack) {
                    Mode::Flat
                } else {
                    Mode::Break
                };
                stack.push((level, mode, inner));
            }
            Doc::IfBreak { broken, flat } => {
                let s = if mode == Mode::Break { broken } else { flat };
                out.push_str(s);
                col += display_width(s);
            }
            Doc::ForceBreak(inner) => stack.push((level, Mode::Break, inner)),
            Doc::Boundary => {}
        }
    }
    out
}

/// Does `doc`, laid out flat, fit in `remaining` columns before the line would break — including
/// the work still queued in `rest` up to the first newline or [`Doc::Boundary`]?
fn fits(mut remaining: usize, doc: &Doc, rest: &[(usize, Mode, &Doc)]) -> bool {
    let mut work: Vec<(Mode, &Doc)> = vec![(Mode::Flat, doc)];
    let mut rest_idx = rest.len();
    loop {
        let (mode, d, from_rest) = if let Some((mode, d)) = work.pop() {
            (mode, d, false)
        } else {
            if rest_idx == 0 {
                return true;
            }
            rest_idx -= 1;
            let (_, mode, d) = rest[rest_idx];
            (mode, d, true)
        };
        match d {
            Doc::Text(s) => {
                // The whole string, newlines and all, where the render counts only the last line:
                // the over-count is what makes a group holding a passthrough text break, the
                // decision both passes agree on — narrowing it to the tail re-opens #134's
                // pass-1/pass-2 flip.
                let w = display_width(s);
                if w > remaining {
                    return false;
                }
                remaining -= w;
            }
            Doc::Concat(items) => {
                for child in items.iter().rev() {
                    work.push((mode, child));
                }
            }
            Doc::Nest(inner) => work.push((mode, inner)),
            Doc::Group(inner) => work.push((Mode::Flat, inner)),
            // A boundary in the lookahead's own path has no width and nothing past it belongs to
            // this group's line — what follows has its own fit, and reading a width across it
            // would decide this group from a line the next pass does not keep (#108). Flattened
            // *through* by the lookahead — in `work` — it is the boundary of a group this one
            // encloses, whose flat form is part of this line, so the lookahead measures past it.
            Doc::Boundary => {
                if from_rest {
                    return true;
                }
            }
            Doc::Line => match mode {
                Mode::Flat => {
                    if remaining == 0 {
                        return false;
                    }
                    remaining -= 1;
                }
                Mode::Break => return true,
            },
            Doc::SoftLine => {
                if mode == Mode::Break {
                    return true;
                }
            }
            Doc::IfBreak { broken, flat } => {
                let s = if mode == Mode::Break { broken } else { flat };
                let w = display_width(s);
                if w > remaining {
                    return false;
                }
                remaining -= w;
            }
            Doc::ForceBreak(_) => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bracketed, comma-separated group shaped like §2.2: flat `(a, b)` or one-per-line broken.
    fn bracket_group(args: &[&str]) -> Doc {
        let mut items = vec![Doc::SoftLine];
        for (i, a) in args.iter().enumerate() {
            if i > 0 {
                items.push(Doc::text(","));
                items.push(Doc::Line);
            }
            items.push(Doc::text(*a));
        }
        Doc::group(Doc::concat([
            Doc::text("("),
            Doc::nest(Doc::concat(items)),
            Doc::SoftLine,
            Doc::text(")"),
        ]))
    }

    #[test]
    fn flat_when_it_fits() {
        assert_eq!(render(&bracket_group(&["a", "b"]), 100, 1, 0), "(a, b)");
    }

    #[test]
    fn fully_breaks_when_it_overflows() {
        assert_eq!(
            render(&bracket_group(&["a", "b"]), 3, 1, 0),
            "(\n\ta,\n\tb\n)"
        );
    }

    #[test]
    fn breaks_indent_relative_to_base_level() {
        assert_eq!(render(&bracket_group(&["a"]), 0, 0, 2), "(\n\t\t\ta\n\t\t)");
    }

    #[test]
    fn trailing_reserved_width_forces_a_break() {
        // `(a)` is 3 wide and would fit in 4 columns, but only 2 are available.
        assert_eq!(render(&bracket_group(&["a"]), 2, 0, 0), "(\n\ta\n)");
    }

    #[test]
    fn display_width_counts_a_tab_as_a_tab() {
        assert_eq!(display_width("abc"), 3);
        assert_eq!(display_width("a\tb"), 1 + TAB_WIDTH + 1);
    }

    #[test]
    fn a_tab_in_a_literal_is_measured_not_ignored() {
        // `("a\tb")` is 7 chars but 10 columns, so it overflows a width its char count would fit.
        let doc = bracket_group(&["\"a\tb\""]);
        assert_eq!(render(&doc, 7, 0, 0), "(\n\t\"a\tb\"\n)");
        assert_eq!(render(&doc, 10, 0, 0), "(\"a\tb\")");
    }

    #[test]
    fn fits_zero_remaining_no_panic() {
        // remaining=0 with empty doc -- does not panic
        assert!(fits(0, &Doc::Concat(vec![]), &[]));
        // remaining=0 with single-char text -- correctly returns false
        assert!(!fits(0, &Doc::text("x"), &[]));
        // remaining=0 with empty text -- does not underflow
        assert!(fits(0, &Doc::text(""), &[]));
    }

    #[test]
    fn fits_force_break_in_rest_returns_false() {
        let fb_doc = Doc::text(",");
        let rest = [(0, Mode::Break, &Doc::ForceBreak(Box::new(fb_doc)))];
        // The doc itself would fit, but a ForceBreak in the rest queue forces false.
        assert!(!fits(5, &Doc::text("hi"), &rest));
    }

    #[test]
    fn nest_inside_group_inside_forcebreak() {
        let doc = Doc::ForceBreak(Box::new(Doc::group(Doc::nest(Doc::concat([
            Doc::text("a"),
            Doc::Line,
            Doc::text("b"),
        ])))));
        // With width 80 the group fits flat -- no indentation from Nest.
        assert_eq!(render(&doc, 80, 0, 0), "a b");
        // With width 1 the group overflows, Line breaks, and Nest adds one tab.
        assert_eq!(render(&doc, 1, 0, 0), "a\n\tb");
    }

    #[test]
    fn ifbreak_broken_alternative_in_break_mode() {
        let doc = Doc::group(Doc::concat([
            Doc::text("call"),
            Doc::IfBreak {
                broken: "(".to_string(),
                flat: ".".to_string(),
            },
            Doc::text("x"),
        ]));
        // Flat group uses the flat alternative.
        assert_eq!(render(&doc, 80, 0, 0), "call.x");
        // Broken group uses the broken alternative.
        assert_eq!(render(&doc, 0, 0, 0), "call(x");
    }

    #[test]
    fn concat_nested_group_line_cascade() {
        let doc = Doc::group(Doc::concat([
            Doc::text("def"),
            Doc::Line,
            Doc::group(Doc::nest(Doc::concat([
                Doc::text("x"),
                Doc::Line,
                Doc::text("y"),
            ]))),
        ]));
        // Width 80: both outer and inner groups fit flat.
        assert_eq!(render(&doc, 80, 0, 0), "def x y");
        // Width 4: outer group overflows (def + space + x.. = 5 > 4), inner fits flat.
        assert_eq!(render(&doc, 4, 0, 0), "def\nx y");
        // Width 2: both groups overflow; inner Line uses Nest indentation (level 1).
        assert_eq!(render(&doc, 2, 0, 0), "def\nx\n\ty");
    }

    #[test]
    fn a_newline_bearing_text_sets_the_cursor_to_its_last_lines_tail() {
        // A passthrough text holds the author's break verbatim; a group after it must measure
        // from the column its last line ends at, not the whole string's width (#134's review).
        let doc = Doc::group(Doc::concat([
            Doc::text("f(\n\tx"),
            Doc::Line,
            Doc::text("y"),
        ]));
        // At width 4: the text's tail `x` puts the cursor at 5, so ` y` overflows and the group
        // breaks — the whole-string accounting would have said 7 and broken it either way, but
        // the column after the break is the tail's, which is what the next group measures from.
        assert_eq!(render(&doc, 4, 0, 0), "f(\n\tx\ny");
        // The cursor after the text is its tail's width: a group rendered after it starts there.
        let after = Doc::concat([
            Doc::text("a\nbb\nccc"),
            Doc::group(Doc::concat([Doc::Line, Doc::text("dddd")])),
        ]);
        assert_eq!(render(&after, 5, 0, 0), "a\nbb\nccc\ndddd");
        assert_eq!(render(&after, 2, 0, 0), "a\nbb\nccc\ndddd");
        // The distinguishing band: the tail `bc` leaves two columns, so the group after the text
        // fits flat — the old whole-string accounting read four and broke it. Without this width,
        // the test cannot fail if the fix is deleted.
        let band = Doc::concat([
            Doc::text("a\nbc"),
            Doc::group(Doc::concat([Doc::Line, Doc::text("x")])),
        ]);
        assert_eq!(render(&band, 4, 0, 0), "a\nbc x");
    }

    #[test]
    fn boundary_stops_the_rest_but_not_the_flattened_lookahead() {
        // The chain head's measurement boundary (#108). A group whose rest holds one stops its
        // fit there — what follows has its own fit — so the head's groups are measured with the
        // head's line and nothing past it.
        let head = Doc::group(Doc::concat([
            Doc::text("arr[a"),
            Doc::Line,
            Doc::text("b]"),
        ]));
        let doc = Doc::concat([head, Doc::Boundary, Doc::text(" = "), Doc::text("aaaaa")]);
        // Width 8: the head alone fits flat, and the rest past the boundary does not widen the
        // head's measurement — the old lookthrough measured 8 + 8 and broke it.
        assert_eq!(render(&doc, 8, 0, 0), "arr[a b] = aaaaa");

        // A lookahead flattened *through* the boundary — an enclosing group's — measures past it:
        // the boundary is this group's content, not its rest, and a forced break after it still
        // makes the whole line break.
        let forced = Doc::group(Doc::concat([
            Doc::text("for("),
            Doc::text("i"),
            Doc::Boundary,
            Doc::ForceBreak(Box::new(Doc::concat([Doc::Line, Doc::text("x")]))),
            Doc::text(")"),
        ]));
        assert_eq!(render(&forced, 80, 0, 0), "for(i\nx)");
    }

    #[test]
    fn is_empty_is_renders_as_nothing() {
        for doc in [
            Doc::text(""),
            Doc::concat([]),
            Doc::concat([Doc::text(""), Doc::text("")]),
            Doc::nest(Doc::text("")),
        ] {
            assert_eq!(render(&doc, 80, 0, 0), "");
            assert!(doc.is_empty(), "{doc:?}");
        }
        // A break renders as nothing only when flat, and the flat form is not the only one.
        for doc in [
            Doc::text(" "),
            Doc::SoftLine,
            Doc::Line,
            Doc::concat([Doc::text(""), Doc::SoftLine]),
            Doc::group(Doc::text("")),
            Doc::IfBreak {
                broken: ",".to_owned(),
                flat: String::new(),
            },
            Doc::ForceBreak(Box::new(Doc::text(""))),
        ] {
            assert!(!doc.is_empty(), "{doc:?}");
        }
    }
}
