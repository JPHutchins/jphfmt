//! What formatting may write, in one place.
//!
//! Every assertion about the author's content has to excuse the characters the *layout* writes, and
//! different assertions can excuse different ones — a count can carry a character formatting may add,
//! an order cannot. So the set is named here rather than spelled out per call site, and the reason a
//! character is excused lives once.
//!
//! A directory module, so cargo does not compile it as a test binary of its own. Each test binary uses
//! the subset it needs, which is why the whole module allows dead code.
#![allow(dead_code)]

use std::collections::HashMap;

/// A rewrite jphfmt is allowed to make, whose characters an assertion must therefore excuse.
///
/// Not every assertion can afford every excuse, and the ones it makes are the ones it is blind to:
/// #88's corruption consisted entirely of a `(` and a `)` in the wrong place, which is invisible to
/// anything excusing [`Rewrite::Bounds`]. An excuse is a stated limit, not a formality.
#[derive(Clone, Copy)]
pub enum Rewrite {
    /// The parentheses bounding a broken chain or ternary (#59), legal exactly because the operands
    /// were already an implicit container.
    Bounds,
    /// The `;` a statement expression writes to terminate an unterminated last statement (#81).
    Terminator,
}

impl Rewrite {
    fn writes(self, c: char) -> bool {
        match self {
            Rewrite::Bounds => matches!(c, '(' | ')'),
            Rewrite::Terminator => c == ';',
        }
    }
}

/// `s` reduced to what the author wrote, `also` naming what this comparison must excuse beyond the
/// layout's own characters.
///
/// Whitespace, §2.3's magic trailing comma and a `#define`'s `\` continuations are always removed:
/// they are the layout's to place, and an all-empty `{,}` legitimately collapses to `{}`.
pub fn authored(s: &str, also: &[Rewrite]) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, ',' | '\\'))
        .filter(|c| !also.iter().any(|rewrite| rewrite.writes(*c)))
        .collect()
}

/// Significant content: what formatting must never alter, whatever it does to the layout.
pub fn significant(s: &str) -> String {
    authored(s, &[Rewrite::Bounds])
}

/// How many times each character formatting must not discard occurs. A *count* excuses nothing beyond
/// the layout's own characters — it can carry the `;` and the `()` formatting may add, and so still
/// catch a dropped one, which is what an order cannot do.
pub fn kept(s: &str) -> HashMap<char, usize> {
    authored(s, &[])
        .chars()
        .fold(HashMap::new(), |mut counts, c| {
            *counts.entry(c).or_default() += 1;
            counts
        })
}

/// The author's characters in order. Counting alone would accept `a + b` becoming `b + a`; this would
/// not — at the cost of excusing every character formatting may *write*, since an inserted one shifts
/// every position after it.
pub fn ordered(s: &str) -> String {
    authored(s, &[Rewrite::Bounds, Rewrite::Terminator])
}
