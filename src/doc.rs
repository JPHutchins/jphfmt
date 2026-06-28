//! A Wadler/Leijen pretty-printing document and a width-aware renderer implementing §2.2's
//! single rule: a group is laid out entirely flat if it fits the width, otherwise entirely
//! broken (every separator becomes a newline). There is deliberately no `fill` mode.

/// Columns a tab advances the cursor for the fits/overflow decision (§2.1, §8.5). Output still
/// emits literal tabs; this width is only used for measuring.
pub const TAB_WIDTH: usize = 4;

/// Display width of `s` in columns. Text never contains tabs or newlines (indentation is emitted
/// separately), so one column per `char` is exact for the measurement.
pub fn display_width(s: &str) -> usize {
    s.chars().count()
}

/// A layout document. Built bottom-up, then rendered at a width.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Doc {
    /// Verbatim text containing no newline.
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
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Flat,
    Break,
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
                col += display_width(s);
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
        }
    }
    out
}

/// Does `doc`, laid out flat, fit in `remaining` columns before the line would break — including
/// the work still queued in `rest` up to the first newline?
fn fits(mut remaining: usize, doc: &Doc, rest: &[(usize, Mode, &Doc)]) -> bool {
    let mut work: Vec<(Mode, &Doc)> = vec![(Mode::Flat, doc)];
    let mut rest_idx = rest.len();
    loop {
        let (mode, d) = if let Some(item) = work.pop() {
            item
        } else {
            if rest_idx == 0 {
                return true;
            }
            rest_idx -= 1;
            let (_, mode, d) = rest[rest_idx];
            (mode, d)
        };
        match d {
            Doc::Text(s) => {
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
}
