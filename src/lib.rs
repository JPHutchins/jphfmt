//! cfmt — a zero-config, opinionated C formatter.
//!
//! Pipeline: a lossless [`lexer`] feeds a pass that builds Wadler [`doc`] documents for the
//! constructs it understands and renders them with the fits-or-fully-break rule (§2.2), emitting
//! everything else verbatim. Milestone M2 covers function-call and declaration argument lists;
//! later milestones extend the same engine to initializers, control headers, macros, and ternaries.

pub mod doc;
pub mod lexer;
mod reflow;

/// Format C source. Idempotent: `format(format(src)) == format(src)` for every input.
pub fn format(src: &str) -> String {
    reflow::format(src)
}
