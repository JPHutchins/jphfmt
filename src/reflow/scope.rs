//! The `#if` preprocessor scope-indentation pass: a pure text post-pass that runs after
//! [`super::structure::structure`] and indents directives between `#` and the keyword to show
//! `#if`/`#else`/`#endif` nesting. `#` stays at column 0; N tabs follow `#`, then the keyword
//! (GNU `#`-column style). Scope depth is independent of brace depth — a `#if` inside a function
//! body indents purely by its own `#if` nesting.
//!
//! * `#if` / `#ifdef` / `#ifndef`: emit at current depth, then `depth += 1`.
//! * `#else` / `#elif`: emit at `depth - 1`; `depth` unchanged.
//! * `#endif`: emit at `depth - 1`, then `depth -= 1`.
//! * All other directives: emit at current depth; no scope change.
//! * Continuation lines (previous line ends in `\`): skipped — no `#`-column indentation.
//! * Depth clamps at ≥ 0 (unbalanced `#endif` degrades gracefully).
//! * Idempotent: existing whitespace between `#` and keyword is stripped before re-inserting tabs.

struct DirectiveLine<'a> {
    leading_ws: &'a str,
    keyword: &'a str,
    rest: &'a str,
}

impl DirectiveLine<'_> {
    fn emit(&self, out: &mut String, depth: usize) {
        out.push_str(self.leading_ws);
        out.push('#');
        for _ in 0..depth {
            out.push('\t');
        }
        out.push_str(self.keyword);
        out.push_str(self.rest);
    }
}

/// Parse a directive line. Returns `None` for non-directive lines.
/// Recognizes `^(\s*)#(\s*)(keyword)(.*)$` — strips the `#`-keyword whitespace,
/// captures the rest so it can be re-emitted with scope tabs.
fn parse_directive(line: &str) -> Option<DirectiveLine<'_>> {
    let leading_ws_len = line.len() - line.trim_start().len();
    let leading_ws = &line[..leading_ws_len];
    let stripped = &line[leading_ws_len..];

    if !stripped.starts_with('#') {
        return None;
    }

    let after_hash = &stripped[1..];
    let after_hash_trimmed = after_hash.trim_start();
    if after_hash_trimmed.is_empty() {
        return None;
    }

    let keyword_end = after_hash_trimmed
        .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .unwrap_or(after_hash_trimmed.len());

    if keyword_end == 0 {
        return None;
    }

    Some(DirectiveLine {
        leading_ws,
        keyword: &after_hash_trimmed[..keyword_end],
        rest: &after_hash_trimmed[keyword_end..],
    })
}

fn scope_open(kw: &str) -> bool {
    matches!(kw, "if" | "ifdef" | "ifndef")
}

fn scope_close(kw: &str) -> bool {
    kw == "endif"
}

fn scope_alt(kw: &str) -> bool {
    matches!(kw, "else" | "elif")
}

/// True when `prev_line`, after trimming trailing whitespace, ends with a single `\`
/// (a `#define` continuation signal).
fn is_continuation(prev_line: &str) -> bool {
    let trimmed = prev_line.trim_end();
    trimmed.ends_with('\\') && !trimmed.ends_with("\\\\")
}

pub(super) fn scope_directives(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth: usize = 0;
    let mut prev_line: &str = "";

    for line in s.lines() {
        if is_continuation(prev_line) {
            out.push_str(line);
            out.push('\n');
            prev_line = line;
            continue;
        }

        if let Some(d) = parse_directive(line) {
            if scope_open(d.keyword) {
                d.emit(&mut out, depth);
                depth += 1;
            } else if scope_close(d.keyword) {
                depth = depth.saturating_sub(1);
                d.emit(&mut out, depth);
            } else if scope_alt(d.keyword) {
                d.emit(&mut out, depth.saturating_sub(1));
            } else {
                d.emit(&mut out, depth);
            }
        } else {
            out.push_str(line);
        }
        out.push('\n');
        prev_line = line;
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_continuation_backslash() {
        assert!(is_continuation("foo \\"));
        assert!(is_continuation("#define M(a) ((a) + 1) \\"));
    }

    #[test]
    fn is_continuation_not_backslash() {
        assert!(!is_continuation("foo bar"));
        assert!(!is_continuation(""));
    }

    #[test]
    fn is_continuation_double_backslash_is_not() {
        assert!(!is_continuation("foo \\\\"));
    }

    #[test]
    fn parse_directive_hash_define() {
        let d = parse_directive("#define PI 3.14").unwrap();
        assert_eq!(d.keyword, "define");
        assert_eq!(d.rest, " PI 3.14");
        assert_eq!(d.leading_ws, "");
    }

    #[test]
    fn parse_directive_hash_if() {
        let d = parse_directive("#if a").unwrap();
        assert_eq!(d.keyword, "if");
        assert_eq!(d.rest, " a");
    }

    #[test]
    fn parse_directive_hash_with_spaces() {
        let d = parse_directive("#  define PI 3.14").unwrap();
        assert_eq!(d.keyword, "define");
        assert_eq!(d.rest, " PI 3.14");
    }

    #[test]
    fn parse_directive_hash_with_tabs() {
        let d = parse_directive("#\tdefine PI 3.14").unwrap();
        assert_eq!(d.keyword, "define");
        assert_eq!(d.rest, " PI 3.14");
    }

    #[test]
    fn parse_directive_hash_with_mixed_ws() {
        let d = parse_directive("# \tdefine PI 3.14").unwrap();
        assert_eq!(d.keyword, "define");
        assert_eq!(d.rest, " PI 3.14");
    }

    #[test]
    fn parse_directive_hash_with_leading_indent() {
        let d = parse_directive("\t#define PI 3.14").unwrap();
        assert_eq!(d.keyword, "define");
        assert_eq!(d.rest, " PI 3.14");
        assert_eq!(d.leading_ws, "\t");
    }

    #[test]
    fn parse_directive_hash_endif() {
        let d = parse_directive("#endif").unwrap();
        assert_eq!(d.keyword, "endif");
        assert_eq!(d.rest, "");
    }

    #[test]
    fn parse_directive_hash_else() {
        let d = parse_directive("#else").unwrap();
        assert_eq!(d.keyword, "else");
        assert_eq!(d.rest, "");
    }

    #[test]
    fn parse_directive_hash_ifdef() {
        let d = parse_directive("#ifdef FOO").unwrap();
        assert_eq!(d.keyword, "ifdef");
        assert_eq!(d.rest, " FOO");
    }

    #[test]
    fn parse_directive_hash_ifndef() {
        let d = parse_directive("#ifndef FOO").unwrap();
        assert_eq!(d.keyword, "ifndef");
        assert_eq!(d.rest, " FOO");
    }

    #[test]
    fn parse_directive_hash_elif() {
        let d = parse_directive("#elif defined(BAR)").unwrap();
        assert_eq!(d.keyword, "elif");
        assert_eq!(d.rest, " defined(BAR)");
    }

    #[test]
    fn parse_directive_hash_pragma() {
        let d = parse_directive("#pragma GCC diagnostic push").unwrap();
        assert_eq!(d.keyword, "pragma");
        assert_eq!(d.rest, " GCC diagnostic push");
    }

    #[test]
    fn parse_directive_hash_include() {
        let d = parse_directive("#include <stdint.h>").unwrap();
        assert_eq!(d.keyword, "include");
        assert_eq!(d.rest, " <stdint.h>");
    }

    #[test]
    fn parse_directive_hash_error() {
        let d = parse_directive("#error \"compiler required\"").unwrap();
        assert_eq!(d.keyword, "error");
        assert_eq!(d.rest, " \"compiler required\"");
    }

    #[test]
    fn parse_directive_non_directive_line() {
        assert!(parse_directive("int x = 0;").is_none());
        assert!(parse_directive("// #define PI").is_none());
        assert!(parse_directive("").is_none());
    }

    #[test]
    fn parse_directive_hash_only() {
        // `#` with no keyword after it is not a directive
        assert!(parse_directive("#").is_none());
    }

    #[test]
    fn scope_open_keywords() {
        assert!(scope_open("if"));
        assert!(scope_open("ifdef"));
        assert!(scope_open("ifndef"));
        assert!(!scope_open("endif"));
        assert!(!scope_open("else"));
        assert!(!scope_open("define"));
    }

    #[test]
    fn scope_close_keywords() {
        assert!(scope_close("endif"));
        assert!(!scope_close("if"));
        assert!(!scope_close("else"));
    }

    #[test]
    fn scope_alt_keywords() {
        assert!(scope_alt("else"));
        assert!(scope_alt("elif"));
        assert!(!scope_alt("if"));
        assert!(!scope_alt("endif"));
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
