//! The `#if` preprocessor scope-indentation pass: a token post-pass that runs after
//! [`super::structure::structure`] and indents directives between `#` and the keyword to show
//! `#if`/`#else`/`#endif` nesting. `#` stays at column 0; N tabs follow `#`, then the keyword
//! (GNU `#`-column style). Scope depth is independent of brace depth — a `#if` inside a function
//! body indents purely by its own `#if` nesting.
//!
//! * `#if` / `#ifdef` / `#ifndef`: emit at current depth, then `depth += 1`.
//! * `#else` / `#elif`: emit at `depth - 1`; `depth` unchanged.
//! * `#endif`: emit at `depth - 1`, then `depth -= 1`.
//! * All other directives: emit at current depth; no scope change.
//! * A `#` is a directive's by [`super::tokens::begins_directive`], the walk's reading: one on a
//!   line a `\` splices in, in a comment, or after code is text, and one after a comment on its line
//!   is a directive (#194). Text passes through verbatim, line endings and all; `post_process`
//!   normalizes them after this pass.
//! * Only blanks between `#` and the keyword are rewritten: `# /* c */ if` keeps its spelling and
//!   moves no depth, as it did when this pass read text lines.
//! * Depth clamps at ≥ 0 (unbalanced `#endif` degrades gracefully).
//! * Idempotent: existing whitespace between `#` and keyword is stripped before re-inserting tabs.

use super::tokens::begins_directive;
use crate::lexer::{TokenKind, tokenize};

/// The depth a directive is emitted at, and the one it leaves behind.
pub(super) struct Scoped {
    pub at: usize,
    pub after: usize,
}

/// Apply the nesting rule to `keyword` met at `depth`. Shared with
/// `structure::emit_define`, which measures a `#define`'s prefix at the depth this pass
/// will indent it to — so the rule lives here, once.
pub(super) fn scoped(keyword: &str, depth: usize) -> Scoped {
    match keyword {
        "if" | "ifdef" | "ifndef" => Scoped {
            at: depth,
            after: depth + 1,
        },
        "endif" => {
            let at = depth.saturating_sub(1);
            Scoped { at, after: at }
        }
        "else" | "elif" => Scoped {
            at: depth.saturating_sub(1),
            after: depth,
        },
        _ => Scoped {
            at: depth,
            after: depth,
        },
    }
}

pub(super) fn scope_directives(s: &str) -> String {
    let toks = tokenize(s);
    let mut out = String::with_capacity(s.len());
    let mut depth = 0usize;
    let mut i = 0;
    while i < toks.len() {
        let keyword = begins_directive(&toks, i)
            .then(|| {
                (i + 1..toks.len()).find(|&k| {
                    toks[k].kind == TokenKind::Newline || !toks[k].text.trim().is_empty()
                })
            })
            .flatten()
            .filter(|&k| matches!(toks[k].kind, TokenKind::Ident | TokenKind::Number));
        if let Some(k) = keyword {
            let scope = scoped(toks[k].text, depth);
            out.push('#');
            out.extend(std::iter::repeat_n('\t', scope.at));
            depth = scope.after;
            i = k;
        } else {
            out.push_str(toks[i].text);
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blanks_between_hash_and_keyword_become_the_depth() {
        for line in [
            "#  define PI 3.14\n",
            "#\tdefine PI 3.14\n",
            "# \t define PI 3.14\n",
        ] {
            assert_eq!(scope_directives(line), "#define PI 3.14\n", "{line:?}");
        }
        assert_eq!(
            scope_directives("\t#define PI 3.14\n"),
            "\t#define PI 3.14\n"
        );
    }

    #[test]
    fn a_hash_that_begins_no_directive_is_text() {
        for src in [
            "#if a\nint x = 0;\n// #define PI\n#endif\n",
            "#if a\n/*\n#define PI\n*/\n#endif\n",
            "#if a\nint b; \\\n#define PI\n#endif\n",
            "#\n",
            "#if a\n# /* c */ define PI\n#endif\n",
        ] {
            assert_eq!(scope_directives(src), src, "{src:?}");
        }
    }

    #[test]
    fn a_comment_before_the_hash_is_white_space() {
        assert_eq!(
            scope_directives("#if a\n/* c */ #define PI\n/* a\n b */ #define E\n#endif\n"),
            "#if a\n/* c */ #\tdefine PI\n/* a\n b */ #\tdefine E\n#endif\n"
        );
    }

    fn at_after(keyword: &str, depth: usize) -> (usize, usize) {
        let scope = scoped(keyword, depth);
        (scope.at, scope.after)
    }

    #[test]
    fn scoped_opens_a_level_below_itself() {
        assert_eq!(at_after("if", 1), (1, 2));
        assert_eq!(at_after("ifdef", 1), (1, 2));
        assert_eq!(at_after("ifndef", 1), (1, 2));
    }

    #[test]
    fn scoped_closes_at_the_level_it_leaves() {
        assert_eq!(at_after("endif", 2), (1, 1));
        assert_eq!(at_after("endif", 0), (0, 0));
    }

    #[test]
    fn scoped_alternative_keeps_the_level_open() {
        assert_eq!(at_after("else", 2), (1, 2));
        assert_eq!(at_after("elif", 2), (1, 2));
        assert_eq!(at_after("else", 0), (0, 0));
    }

    #[test]
    fn scoped_plain_directive_changes_nothing() {
        assert_eq!(at_after("define", 1), (1, 1));
        assert_eq!(at_after("include", 0), (0, 0));
    }

    #[test]
    fn flat_define_unchanged() {
        assert_eq!(scope_directives("#define PI 3.14\n"), "#define PI 3.14\n");
    }

    #[test]
    fn simple_if_endif_scopes_body() {
        let input = "#if a\n#define thing\n#endif\n";
        let expected = "#if a\n#\tdefine thing\n#endif\n";
        assert_eq!(scope_directives(input), expected);
    }

    #[test]
    fn nested_if_scope() {
        let input = "#if a\n#define thing\n#else\n#if b\n#define thing\n#if c\n#define thing\n#endif\n#endif\n#endif\n";
        let expected = "#if a\n#\tdefine thing\n#else\n#\tif b\n#\t\tdefine thing\n#\t\tif c\n#\t\t\tdefine thing\n#\t\tendif\n#\tendif\n#endif\n";
        assert_eq!(scope_directives(input), expected);
    }

    #[test]
    fn user_example() {
        let input = concat!(
            "#if a\n",
            "#define thing\n",
            "#else\n",
            "#if b\n",
            "#define thing\n",
            "#if c\n",
            "#define thing\n",
            "#endif\n",
            "#endif\n",
            "#endif\n",
        );
        let expected = concat!(
            "#if a\n",
            "#\tdefine thing\n",
            "#else\n",
            "#\tif b\n",
            "#\t\tdefine thing\n",
            "#\t\tif c\n",
            "#\t\t\tdefine thing\n",
            "#\t\tendif\n",
            "#\tendif\n",
            "#endif\n",
        );
        assert_eq!(scope_directives(input), expected);
    }

    #[test]
    fn idempotent() {
        let input = concat!(
            "#if a\n",
            "#\tdefine thing\n",
            "#else\n",
            "#\tif b\n",
            "#\t\tdefine thing\n",
            "#\t\tif c\n",
            "#\t\t\tdefine thing\n",
            "#\t\tendif\n",
            "#\tendif\n",
            "#endif\n",
        );
        assert_eq!(scope_directives(input), input);
    }

    #[test]
    fn continuation_lines_skipped() {
        let input = "#define M(a) ((a) + 1) \\\n\t+ 2\n";
        assert_eq!(scope_directives(input), input);
    }

    #[test]
    fn other_directives_at_current_depth() {
        let input = concat!(
            "#if a\n",
            "#include <stdio.h>\n",
            "#define PI 3.14\n",
            "#pragma once\n",
            "#endif\n",
        );
        let expected = concat!(
            "#if a\n",
            "#\tinclude <stdio.h>\n",
            "#\tdefine PI 3.14\n",
            "#\tpragma once\n",
            "#endif\n",
        );
        assert_eq!(scope_directives(input), expected);
    }

    #[test]
    fn elif_at_scope_depth() {
        let input = concat!(
            "#if a\n",
            "#define thing\n",
            "#elif b\n",
            "#define other\n",
            "#endif\n",
        );
        let expected = concat!(
            "#if a\n",
            "#\tdefine thing\n",
            "#elif b\n",
            "#\tdefine other\n",
            "#endif\n",
        );
        assert_eq!(scope_directives(input), expected);
    }

    #[test]
    fn unbalanced_endif_does_not_panic() {
        // Depth would go below zero — clamps and degrades gracefully.
        let result = scope_directives("#endif\n");
        // At depth 0, saturating_sub(1) = 0, depth stays 0.
        assert_eq!(result, "#endif\n");
    }

    #[test]
    fn non_directive_lines_untouched() {
        let input = "int x = 0;\n#if a\n#define thing\n#endif\nint y = 1;\n";
        let expected = "int x = 0;\n#if a\n#\tdefine thing\n#endif\nint y = 1;\n";
        assert_eq!(scope_directives(input), expected);
    }

    #[test]
    fn ifdef_ifndef_work_like_if() {
        let input = concat!(
            "#ifdef FOO\n",
            "#define thing\n",
            "#endif\n",
            "#ifndef BAR\n",
            "#define other\n",
            "#endif\n",
        );
        let expected = concat!(
            "#ifdef FOO\n",
            "#\tdefine thing\n",
            "#endif\n",
            "#ifndef BAR\n",
            "#\tdefine other\n",
            "#endif\n",
        );
        assert_eq!(scope_directives(input), expected);
    }

    #[test]
    fn error_and_warning_at_current_depth() {
        let input = concat!("#if 0\n", "#error \"should not compile\"\n", "#endif\n",);
        let expected = concat!("#if 0\n", "#\terror \"should not compile\"\n", "#endif\n",);
        assert_eq!(scope_directives(input), expected);
    }

    #[test]
    fn else_branch_content_at_content_depth() {
        // Content directives inside #else are at the same depth as if-branch content.
        let input = concat!(
            "#if a\n",
            "#define x\n",
            "#else\n",
            "#define y\n",
            "#endif\n",
        );
        let expected = concat!(
            "#if a\n",
            "#\tdefine x\n",
            "#else\n",
            "#\tdefine y\n",
            "#endif\n",
        );
        assert_eq!(scope_directives(input), expected);
    }

    #[test]
    fn if_after_else_indents_uniformly() {
        // A #if directly after #else indents under the #else (one tab), and a #if after
        // content in the #else branch indents at the SAME level — no special-casing.
        let input = concat!(
            "#if a\n",
            "#else\n",
            "#if b\n",
            "#endif\n",
            "#define x\n",
            "#if c\n",
            "#endif\n",
            "#endif\n",
        );
        let expected = concat!(
            "#if a\n",
            "#else\n",
            "#\tif b\n",
            "#\tendif\n",
            "#\tdefine x\n",
            "#\tif c\n",
            "#\tendif\n",
            "#endif\n",
        );
        assert_eq!(scope_directives(input), expected);
    }
}
