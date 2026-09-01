//! Conformance suite. What must hold is idempotency, verbatim passthrough of call-free input, and the §2.2
//! layout for calls.

mod support;

use jphfmt::doc::display_width;
use jphfmt::format;
use jphfmt::format_with_width;
use support::significant;

const GOLDEN: &str = include_str!("golden.c");

#[test]
fn golden_is_a_fixpoint() {
    assert_eq!(format(GOLDEN), GOLDEN, "golden must be idempotent");
}

const MESSY: &str = include_str!("messy.c");

#[test]
fn messy_real_world_input_is_idempotent_and_safe() {
    let once = format(MESSY);
    assert_eq!(format(&once), once, "must be idempotent on messy input");
    assert_eq!(
        significant(&once),
        significant(MESSY),
        "must not change any token on messy input"
    );
    for (n, line) in once.lines().enumerate() {
        if let Some(rest) = line.strip_prefix(' ') {
            assert!(
                rest.trim_start().starts_with('*'),
                "messy line {} is space-indented code: {line:?}",
                n + 1
            );
        }
    }
}

#[test]
fn golden_has_no_space_indented_code() {
    // §7 cardinal rule: zero column alignment. Only sacred comment bodies (` * …`) may lead
    // with a space.
    for (n, line) in GOLDEN.lines().enumerate() {
        if let Some(rest) = line.strip_prefix(' ') {
            assert!(
                rest.trim_start().starts_with('*'),
                "line {} is space-indented code: {line:?}",
                n + 1
            );
        }
    }
}

#[test]
fn passthrough_for_call_free_input() {
    let snippets = [
        "int x = 1'000'000;\n",
        "/* block * / not the end */ x; // trailing\n",
        "char const * p = \"a\\\"b\\n\"; char c = '\\'';\n",
        "#define M(a) ((a) + 1) \\\n\t+ 2\n",
        "auto s = u\"\u{3b7} \u{3bc}\u{3ac}\u{3b8}\u{3b7}\u{3c3}\u{3b9}\u{3c2}\";\n",
        "a->b = c << 2; d.e = f ? g : h;\n",
        "",
    ];
    for s in snippets {
        assert_eq!(format(s), s, "call-free input must be unchanged: {s:?}");
    }
}

#[test]
fn short_call_stays_flat() {
    assert_eq!(format("foo(a, b, c);\n"), "foo(a, b, c);\n");
    assert_eq!(
        format("driver_deinit(void) {}\n"),
        "driver_deinit(void) {}\n"
    );
    assert_eq!(format("empty();\n"), "empty();\n");
}

#[test]
fn long_call_explodes_one_per_line() {
    let long = "result = some_function_with_a_fairly_long_name(first_argument_value, second_argument_value, third_argument_value);\n";
    let expected = "result = some_function_with_a_fairly_long_name(\n\tfirst_argument_value,\n\tsecond_argument_value,\n\tthird_argument_value\n);\n";
    assert_eq!(format(long), expected);
}

#[test]
fn collapses_a_call_that_now_fits() {
    assert_eq!(format("foo(\n    a,\n    b\n);\n"), "foo(a, b);\n");
}

#[test]
fn nested_paren_comma_is_not_a_split_point() {
    let src = "register_cb(int (*cb)(void * ctx, int status), int n);\n";
    assert_eq!(
        format(src),
        src,
        "inner comma must stay inside the nested parens"
    );
}

#[test]
fn control_headers_are_not_calls() {
    let src = "if (a && b) { return f(x); }\n";
    assert_eq!(format(src), src);
}

#[test]
fn short_initializer_stays_flat_and_tight() {
    assert_eq!(format("int v[] = {1, 2, 3};\n"), "int v[] = {1, 2, 3};\n");
    assert_eq!(format("int v[] = {0};\n"), "int v[] = {0};\n");
}

#[test]
fn magic_trailing_comma_forces_explosion_with_trailing_comma() {
    let src = "int v[] = {1, 2, 3,};\n";
    let expected = "int v[] = {\n\t1,\n\t2,\n\t3,\n};\n";
    assert_eq!(format(src), expected);
}

#[test]
fn collapses_initializer_without_trailing_comma() {
    let src = "int v[] = {\n    1,\n    2,\n    3\n};\n";
    assert_eq!(format(src), "int v[] = {1, 2, 3};\n");
}

#[test]
fn nested_initializer_collapses_independently() {
    let src = "int m[2][3] = {{1, 2, 3}, {4, 5, 6},};\n";
    let expected = "int m[2][3] = {\n\t{1, 2, 3},\n\t{4, 5, 6},\n};\n";
    assert_eq!(format(src), expected);
}

#[test]
fn enum_body_is_padded_when_flat() {
    assert_eq!(format("enum { A, B };\n"), "enum { A, B };\n");
    assert_eq!(
        format("enum color { A = 1, B };\n"),
        "enum color { A = 1, B };\n"
    );
}

#[test]
fn enum_magic_comma_explodes() {
    let src = "enum color { RED, GREEN, BLUE, };\n";
    let expected = "enum color {\n\tRED,\n\tGREEN,\n\tBLUE,\n};\n";
    assert_eq!(format(src), expected);
}

#[test]
fn initializer_with_comment_keeps_structure_but_retabs() {
    // comments defer to M7 (no comma reflow), but leading indentation is normalized to tabs
    let src = "int v[] = {\n    1, /* one */\n    2,\n};\n";
    let expected = "int v[] = {\n\t1, /* one */\n\t2,\n};\n";
    assert_eq!(format(src), expected);
}

/// #77's fourth item is now implemented — a `#define` body that is entirely one bracket is laid out
/// as a container — and this pins the two claimed shapes' exact continuation layout alongside the
/// shapes that still pass through, so the day a claim regresses, this test fails and says where to
/// look.
///
/// Claiming such a body previously produced five distinct regressions over three review rounds on
/// #103, each on a body the walk then measured as something it is not: a dropped `\` continuation;
/// a nested `({ … })` collapsed to a brace list, both adjacent and spaced; a two-cycle at width 40
/// on a nested parenthesized ternary, because the two passes measure the body at different columns
/// (#43/#108's defect, not the claim's); and a `\`-continued string literal gaining a ` \` inside
/// its own text on every pass, which changes what the macro expands to.
///
/// A body that is one bracket is still not one construct — it is whatever the macro's use makes of
/// it — and the claim's guards keep the shapes above passing through, the nested ternary's two-cycle
/// included. The `#104` fix that shipped with these findings is narrow and separate: a
/// statement-expression body is laid out only when its `)` is the body's last token.
#[test]
fn a_define_body_that_is_one_whole_bracket_is_claimed() {
    // #77's fourth item: a body that is one whole bracket is a container now — the ternary and the
    // chain break with `\` continuations — while a bare chain, a partial paren, the statement
    // expression shapes and the literal keep their passthrough.
    assert_eq!(
        format(
            "#define M(x) ((x) ? aaaaaaaaaaaaaaaaaaaaaa : bbbbbbbbbbbbbbbbbbbbbb ? cccccccccccccccccccc : dddddddddddddddddddd)\n"
        ),
        "#define M(x) ( \\\n\t(x) ? aaaaaaaaaaaaaaaaaaaaaa : \\\n\tbbbbbbbbbbbbbbbbbbbbbb ? cccccccccccccccccccc : \\\n\tdddddddddddddddddddd \\\n)\n"
    );
    assert_eq!(
        format(
            "#define N(x) ((x) + aaaaaaaaaaaaaaaaaaaaaa + bbbbbbbbbbbbbbbbbbbbbb + cccccccccccccccccccc + dddddddddddddddddddd)\n"
        ),
        "#define N(x) ( \\\n\t(x) + \\\n\taaaaaaaaaaaaaaaaaaaaaa + \\\n\tbbbbbbbbbbbbbbbbbbbbbb + \\\n\tcccccccccccccccccccc + \\\n\tdddddddddddddddddddd \\\n)\n"
    );
    // A whole `[` body is the same claim, one bracket over — and the nested-ternary two-cycle's
    // bracket form passes through exactly like the paren form (both pinned below).
    assert_eq!(
        format(
            "#define IDX [aaaaaaaaaaaaaaaaaaaaaa + bbbbbbbbbbbbbbbbbbbbbb ? cccccccccccccccccccc : dddddddddddddddddddd]\n"
        ),
        "#define IDX [ \\\n\taaaaaaaaaaaaaaaaaaaaaa + bbbbbbbbbbbbbbbbbbbbbb ? cccccccccccccccccccc : \\\n\tdddddddddddddddddddd \\\n]\n"
    );
    for claimed in [
        "#define M(x) ((x) ? aaaaaaaaaaaaaaaaaaaaaa : bbbbbbbbbbbbbbbbbbbbbb ? cccccccccccccccccccc : dddddddddddddddddddd)\n",
        "#define N(x) ((x) + aaaaaaaaaaaaaaaaaaaaaa + bbbbbbbbbbbbbbbbbbbbbb + cccccccccccccccccccc + dddddddddddddddddddd)\n",
        "#define IDX [aaaaaaaaaaaaaaaaaaaaaa + bbbbbbbbbbbbbbbbbbbbbb ? cccccccccccccccccccc : dddddddddddddddddddd]\n",
    ] {
        assert_eq!(
            format(&format(claimed)),
            format(claimed),
            "fixpoint: {claimed:?}"
        );
    }
    for src in [
        // A bare chain gets no parentheses of jphfmt's, however long it is: they would be tokens the
        // author did not write, in a body whose expansion is the author's to control.
        "#define Q(x) x + aaaaaaaaaaaaaaaaaaaaaa + bbbbbbbbbbbbbbbbbbbbbb + cccccccccccccccccccc + dddddddddddddddddddd\n",
        "#define R(x) ((x) + aaaaaaaaaaaaaaaaaaaaaa + bbbbbbbbbbbbbbbbbbbbbb + cccccccccccccccccccc) + d\n",
        // Each shape a claim regressed, at the width that showed it.
        "#define X (a + ({ int t = (x); t; }))\n",
        "#define X (({ int t = (x); t; }) + a)\n",
        "#define X (a + ( { int t = (x); t; } ))\n",
        "#define MSG (\"abc\\\n def\")\n",
    ] {
        assert_eq!(format(src), src, "{src:?}");
    }

    // The nested-ternary two-cycle showed at width 40, so the width it showed at is the one asserted
    // — both the paren form and the bracket form the claim's depth guard seeds for.
    for two_cycle in [
        "#define value(x) ((123 ? 0xff : (a)) ? (t))\n",
        // The spacing pass tightens the subscript's `[` against the params' `)`, so the bracket
        // form's canonical spelling is the tight one.
        "#define value(x)[(123 ? 0xff : (a)) ? (t)]\n",
    ] {
        assert_eq!(jphfmt::format_with_width(two_cycle, 40), two_cycle);
    }
}

/// A statement expression is a parenthesized group, so the container arm above claims a body that is
/// entirely one — and a body that is *not* entirely one passes through, rather than being claimed from
/// its first two tokens and rendered only as far as the `})`. That is what deleted the `+ 1` from
/// `#define M(x) ({ int t = (x); t; }) + 1`, silently changing the expansion on valid GNU C (#104), and
/// it is #81's shape in a third place: a handler reporting more consumed than it rendered.
///
/// Invisible to idempotency, like #81 — the truncated output is a fixpoint.
#[test]
fn a_statement_expression_body_keeps_what_follows_it() {
    let trailing = "#define M(x) ({ int t = (x); t; }) + 1\n";
    assert_eq!(format(trailing), trailing);

    // Whole-body, so the walk lays it out — the operand is what the arm above already does.
    let whole = "#define M(x) ({ int t = (x); t; })\n";
    assert_eq!(
        format(whole),
        "#define M(x) ({ \\\n\tint t = (x); \\\n\tt; \\\n})\n"
    );
    assert_eq!(format(&format(whole)), format(whole));

    // Neither does anything before it make the body a statement expression to lay out.
    let leading = "#define N(x) 1 + ({ int t = (x); t; })\n";
    assert_eq!(format(leading), leading);

    // Wrapping that in parentheses makes the *body* one whole group, so the container arm claims it —
    // but the `({` inside is then no longer where the walk tests for one, and reaches
    // `build_expr_doc`'s brace-list branch, which spaces a block as a list: `(a + ({int t = (x); t;}))`.
    // §6 prefers passthrough over a layout no handler owns.
    for operand in [
        "#define X (a + ({ int t = (x); t; }))\n",
        "#define X (({ int t = (x); t; }) + a)\n",
    ] {
        assert_eq!(format(operand), operand, "{operand:?}");
    }
}

/// A body holding a `\`-continued literal is not one to lay out. Such a literal is **one token** whose
/// text holds the newline (#110/#111), so a rendered body already carries it — and `emit_define` re-splits
/// at every `\n` to place the continuations, putting its ` \` *inside the literal*. Re-lexing that back
/// into the same token compounds it, once per pass:
///
/// ```text
/// #define M(x) f("a\        ->  f("a\ \      ->  f("a\ \ \
///  b")                          b")             b")
/// ```
///
/// Non-idempotent, and the macro expands to different text each time (#117). Both arms `define_body_layout`
/// retains carried it — pre-existing on `main`, which is why it was filed rather than folded into the
/// deferral of #77's fourth item, where the container arm had the same defect.
///
/// `spans_lines` is the refusal `is_boundable` and `build_bracketed_group` already make, for exactly this
/// reason: a token carrying a line break is a literal the one-line width model cannot describe.
#[test]
fn a_define_body_holding_a_continued_literal_passes_through() {
    for src in [
        "#define M(x) f(\"a\\\n b\")\n",
        "#define M(x) ({ f(\"a\\\n b\"); })\n",
        "#define M(x) ({ char * s = \"a\\\n b\"; s; })\n",
    ] {
        assert_eq!(format(src), src, "{src:?}");
    }

    // What the refusal must not take with it: the same two arms, with no continued literal, still lay out.
    let call = "#define M(x) fooooooooooooooo(aaaaaaaaaaaaaaaaaaa, bbbbbbbbbbbbbbbbbbb, ccccccccccccccccccccccccc, dddddddddddddddd)\n";
    assert!(format(call).contains(" \\\n"), "{:?}", format(call));
    let block = "#define M(x) ({ int t = (x); t; })\n";
    assert_eq!(
        format(block),
        "#define M(x) ({ \\\n\tint t = (x); \\\n\tt; \\\n})\n"
    );
}

/// A `#define` whose name is a line continuation is not one to split. `split_define` flattens the
/// continuations inside the head, so there is nothing to write this one back, and what follows it is
/// read as the body rather than as the name — `#define \` + `(})` came out as `#define (})`, which
/// `space_call_heads` then tightened to `#define(})` on the next pass.
///
/// A `\` touching the name is the same loss from the other side: the splice leaves nothing between the
/// lines, so `#define NAME\` + `(x)` defines the function-like `NAME(x)` and `#define NAME (x)` is a
/// different macro. `(x) x + 1` was already here and passed, because a body that is not one whole
/// bracket is not claimable — it took a claimable one to reach the gap.
///
/// Found by the 200k property run on the branch that made the body claimable; before that the body was
/// passed through, which masked the loss.
///
/// Only a *claimable* body reaches the split, so only an input with one pins the guard — everything else
/// keeps its `\` through `emit_define`'s verbatim fallback whether the guard is there or not. With #77's
/// fourth item deferred, the claimable shapes are a whole call and a whole statement expression, which
/// makes `f(x)` the input that pins the `toks[name + 1]` arm and `f()` the one that pins `toks[name]`.
/// `#define NAME\` + `f(x)` splices the two names into one, defining a function-like macro that takes
/// `x`; without the guard it came out as the object-like `#define NAME f(x)` — a different macro, which
/// `main` still writes.
///
/// The other four assert passthrough and pin no arm. They are kept because passthrough is worth holding,
/// not because they cover the guard, and were asserted with `contains('\\')` until the review found that
/// two of them stayed green with the guard deleted. All of them would.
#[test]
fn a_continued_macro_name_is_not_a_name_to_split() {
    // Claimable, so these reach the split and fail if the guard goes.
    assert_eq!(format("#define NAME\\\nf(x)\n"), "#define NAME\\\nf(x)\n");
    assert_eq!(format("#define\\\nf()\n"), "#define\\\nf()\n");
    assert_eq!(
        format("#define NAME\\\n({ x; })\n"),
        "#define NAME\\\n({ x; })\n"
    );

    // The only input without a trailing newline; formatting adds the one every other line has (§2.1).
    assert_eq!(format("#define\\\n(})"), "#define\\\n(})\n");
    for src in [
        "#define \\\nNAME 1\n",
        "#define NAME\\\n(x) x + 1\n",
        "#define NAME\\\n(x)\n",
    ] {
        assert_eq!(format(src), src);
    }
}

/// The gap between a `#define`'s name and a `(` is meaning, not spacing: `#define X (y)` defines `X` as
/// `(y)`, and `#define X(y)` a function-like macro taking `y`. Tightening it turned every object-like
/// macro with a parenthesized body into a function-like one, and the output did not compile.
///
/// Invisible to every check in the suite, which is why it survived: the character dropped is whitespace,
/// which every excuse set removes by design, and `#define X(y)` is a fixpoint. 478 of the 1200 corpus
/// files write the shape, and glibc's `elf.h` came out with 100 compiler errors.
#[test]
fn a_macro_name_keeps_the_gap_that_says_what_it_defines() {
    for src in [
        "#define X (y)\n",
        "#define SECURE_FLAG (1 << 3)\n",
        "#define WEOF (0xffffffffu)\n",
        // A function-like macro's name is tight, and stays tight.
        "#define F(x) ((x) + 1)\n",
        "#define MIN(a, b) ((a) < (b) ? (a) : (b))\n",
    ] {
        assert_eq!(format(src), src, "the author's gap is the definition");
    }
    // The `#`-to-keyword gap belongs to the scope pass, and still collapses.
    assert_eq!(format("# define H (c)\n"), "#define H (c)\n");
    // A comment is whitespace by the time the preprocessor reads the line, so it defines the same thing
    // — and the walk back to the `#` has to read past it, since a comment is a token of its own.
    for src in [
        "#define /* c */ X (y)\n",
        "#/* c */ define Y (z)\n",
        "#define /* c */ F(x) ((x) + 1)\n",
    ] {
        assert_eq!(
            format(src),
            src,
            "a comment does not change what is defined"
        );
    }
}

/// The structure pass measures a `#define` at the `#if` depth the scope pass will indent it to, and
/// nothing else asserts the two agree. This define's line is 94 columns: it fits at depth 0 and
/// overruns at depth 2, so it explodes only if those 8 columns were counted.
#[test]
fn a_define_is_measured_at_the_depth_the_scope_pass_indents_it_to() {
    const DEFINE: &str = "#define M(a) FFFFFFFFFFFF(aaaaaaaaaaaaaaaa, bbbbbbbbbbbbbbbb, \
                          cccccccccccccccc, ddddddddddddd)\n";
    assert_eq!(format(DEFINE), DEFINE);

    let nested = format(&format!("#if A\n#if B\n{DEFINE}#endif\n#endif\n"));
    assert_eq!(
        nested,
        "#if A\n#\tif B\n#\t\tdefine M(a) FFFFFFFFFFFF( \\\n\taaaaaaaaaaaaaaaa, \\\n\
         \tbbbbbbbbbbbbbbbb, \\\n\tcccccccccccccccc, \\\n\tddddddddddddd \\\n)\n#\tendif\n#endif\n"
    );
    assert_eq!(format(&nested), nested);
}

/// #93: every line of a continued `#define` ends in ` \`, and those two columns are the continuation's
/// rather than the layout's. Measured against the whole width, the nested `g(…)` here stayed flat on
/// the strength of columns it did not own, and the line as written overran §8.5 by exactly them.
///
/// One column narrower on each argument and the line lands exactly at the limit, which is the check
/// that the reservation is two columns and not a blanket explosion: the shortest form still wins when
/// it genuinely fits.
#[test]
fn a_continued_define_reserves_the_columns_its_continuation_takes() {
    for len in 40..=50 {
        let (a, b) = ("a".repeat(len), "b".repeat(len));
        let src = format!("#define P(x) f(x, g({a}, {b}), y)\n");
        let once = format(&src);
        for line in once.lines() {
            assert!(
                display_width(line) <= 100,
                "argument length {len}: {} columns: {line:?}",
                display_width(line)
            );
        }
        assert_eq!(format(&once), once, "argument length {len}");
    }
    let at_the_limit = format(&format!(
        "#define P(x) f(x, g({a}, {b}), y)\n",
        a = "a".repeat(44),
        b = "b".repeat(44)
    ));
    assert!(
        at_the_limit.contains("\tg(aaa"),
        "a nested group that fits must stay flat: {at_the_limit:?}"
    );
}

/// The body of a `#define` whose parameters explode is the *last* line, and `emit_define` writes ` \`
/// between lines — so that line takes no continuation and only its tab is reserved. Measured two
/// columns narrower, this 96-column body broke, `explode_params` refused a multi-line body, and the
/// parameter list stayed flat at 129 columns.
#[test]
fn a_params_exploded_define_measures_its_body_against_the_tab_alone() {
    let params = (1..=4)
        .map(|i| format!("{p}{i}", p = "p".repeat(25)))
        .collect::<Vec<_>>()
        .join(", ");
    let body = format!("f({a}, {b})", a = "a".repeat(45), b = "b".repeat(46));
    let once = format(&format!("#define PPPP({params}) {body}\n"));
    for line in once.lines() {
        assert!(
            display_width(line) <= 100,
            "{} columns: {line:?}",
            display_width(line)
        );
    }
    assert!(
        once.contains(&format!("\t{body}")),
        "the body keeps its own line whole: {once:?}"
    );
    assert_eq!(format(&once), once);
}

#[test]
fn comment_line_ends_are_trimmed() {
    let src = "int a; // after a line comment   \nint b; /* first   \n * interior   \n */\n";
    let expected = "int a; // after a line comment\nint b; /* first\n * interior\n */\n";
    assert_eq!(format(src), expected);
}

#[test]
fn a_continued_literal_keeps_the_spaces_in_its_value() {
    // The spaces before the `\` are part of the string, not a line ending to trim.
    let src = "char * s = \"keep   \\\n me\";\n";
    assert_eq!(format(src), src);
}

#[test]
fn indentation_is_normalized_to_tabs() {
    let src = "void f(void) {\n    int x = 1;\n        int y = 2;\n}\n";
    let expected = "void f(void) {\n\tint x = 1;\n\t\tint y = 2;\n}\n";
    assert_eq!(format(src), expected);
}

#[test]
fn call_with_line_comment_passes_through() {
    // a // comment in a call must not be collapsed onto one line (it would swallow later args)
    let src = "f(\n\t// keep me\n\tNULL,\n\t&x\n);\n";
    assert_eq!(
        format(src),
        src,
        "comment-bearing calls must not be reflowed"
    );
}

/// §2.5 tightens a call against its callee; a subscript is the same postfix operator on the same
/// value, so `[` is tight too (#79). `[[` is not a subscript — it opens an attribute, and both
/// `int x [[deprecated]];` and `int arr[10] [[deprecated]];` are valid C23.
#[test]
fn a_subscript_is_tight_against_what_it_indexes() {
    assert_eq!(format("int a = arr [i];\n"), "int a = arr[i];\n");
    assert_eq!(format("int b = m[i] [j];\n"), "int b = m[i][j];\n");
    assert_eq!(format("int c = f() [k];\n"), "int c = f()[k];\n");
    assert_eq!(format("int d = \"abc\" [1];\n"), "int d = \"abc\"[1];\n");
    assert_eq!(format("char port [127];\n"), "char port[127];\n");
    // Postfix `++`/`--` end their operand, and a compound literal is an lvalue a subscript indexes.
    assert_eq!(format("int e = p++ [i];\n"), "int e = p++[i];\n");
    assert_eq!(format("int f = q-- [j];\n"), "int f = q--[j];\n");
    assert_eq!(
        format("int g = (int[]){1, 2} [0];\n"),
        "int g = (int[]){1, 2}[0];\n"
    );
    for unchanged in [
        "int x [[deprecated]];\n",
        "int arr[10] [[deprecated]];\n",
        "[[nodiscard]] int f(void);\n",
    ] {
        assert_eq!(
            format(unchanged),
            unchanged,
            "an attribute is not a subscript"
        );
    }
}

#[test]
fn control_keyword_gets_one_space_before_paren() {
    assert_eq!(format("if(x) y;\n"), "if (x) y;\n");
    assert_eq!(format("while(y) z;\n"), "while (y) z;\n");
    assert_eq!(format("switch(c) {\n}\n"), "switch (c) {\n}\n");
}

#[test]
fn pointers_are_middle_spaced_after_type_keywords() {
    assert_eq!(format("int*p;\n"), "int * p;\n");
    assert_eq!(format("char const*const q;\n"), "char const * const q;\n");
    assert_eq!(format("void*f(void);\n"), "void * f(void);\n");
}

#[test]
fn pointer_declarator_stars_never_cluster() {
    // §2.5: only the dereference operator clusters with its operand, so each `*` in a declarator
    // stands alone.
    assert_eq!(format("int **p;\n"), "int * * p;\n");
    assert_eq!(format("int***q;\n"), "int * * * q;\n");
    assert_eq!(
        format("void py_release(PyObject ** const r);\n"),
        "void py_release(PyObject * * const r);\n"
    );
    assert_eq!(format("PyObject *const s;\n"), "PyObject * const s;\n");
    assert_eq!(format("PyObject **t;\n"), "PyObject * * t;\n");
    // A multiply keeps its own spacing: no declaration position, no declarator.
    assert_eq!(format("z = a ** b;\n"), "z = a ** b;\n");
}

#[test]
fn typedef_pointer_declarators_are_middle_spaced() {
    // A typedef name in declaration position is a type, so its `*` is a declarator (§6 only bars
    // the runs an expression could also produce).
    for src in [
        "uint32_t *p;\n",
        "size_t *p;\n",
        "FILE *p;\n",
        "PyObject *p;\n",
    ] {
        assert_eq!(format(src), src.replace('*', "* "));
    }
    assert_eq!(format("mytype*p;\n"), "mytype * p;\n");
    assert_eq!(
        format("static PyObject *probe(PyObject *self);\n"),
        "static PyObject * probe(PyObject * self);\n"
    );
    assert_eq!(
        format("void f(int a, PyObject *b);\n"),
        "void f(int a, PyObject * b);\n"
    );
    assert_eq!(
        format("typedef PyObject *ptr_t;\n"),
        "typedef PyObject * ptr_t;\n"
    );
}

#[test]
fn a_specifier_keyword_is_not_a_callee() {
    // `_Noreturn` introduces a declaration; it never takes an argument list, so its `(` must not be
    // tightened the way a call's is.
    assert_eq!(format("_Noreturn (void) f;\n"), "_Noreturn (void) f;\n");
    assert_eq!(
        format("_Noreturn void die(void);\n"),
        "_Noreturn void die(void);\n"
    );
}

#[test]
fn comma_separated_declarators_are_all_spaced() {
    // The second declarator's type is back past the comma, so its `*` is a declarator too.
    assert_eq!(format("int *p, *q, *r;\n"), "int * p, * q, * r;\n");
    assert_eq!(format("PyObject *x, *y;\n"), "PyObject * x, * y;\n");
    assert_eq!(format("struct foo *a, *b;\n"), "struct foo * a, * b;\n");
}

#[test]
fn a_declarator_after_a_broken_statement_expression_reads_the_same_both_passes() {
    // #130: the structure pass broke the author's `({=})` as a statement expression and wrote
    // the `;` that goes with it — and the next pass's spacing read `({ =; })` as a block, its
    // `=` hidden behind the `;`, respacing the author's `fx*f` as a declarator `fx * f`. The
    // verdict now reads tokens, bracket-aware: the `=` inside the group cannot mark an
    // initializer, and a brace is transparent so a real initializer's `=` stays visible.
    let once = format("({=}){fx*f");
    assert_eq!(
        format(&once),
        once,
        "the statement expression's broken form is a fixpoint"
    );
    assert!(
        once.contains("fx * f"),
        "the declarator read is the first pass's: {once:?}"
    );
}

#[test]
fn a_paren_group_the_layout_wrote_does_not_mask_the_initializers_equals() {
    // The layout wraps a labeled brace-list element in its own paren group, and the next pass's
    // bracket-aware scan read that `(` at depth zero as a group whose interior extends past the
    // brace — the depth went negative, masking the statement-level `=` and flipping the verdict to
    // a block, which respaced the element's multiply `A*a` as a declarator `A * a`. An expression
    // body's enclosing group is the layout's own and is transparent: the scan stays at statement
    // level and the `=` left of the group decides.
    let once = jphfmt::format_with_width("={0:{A*a}?\"\"?}", 1);
    assert_eq!(
        once,
        " = {\n\t(\n\t\t0 :\n\t\t{\n\t\t\tA*a,\n\t\t}?\"\"?\n\t),\n}\n"
    );
    assert_eq!(
        jphfmt::format_with_width(&once, 1),
        once,
        "and it is a fixpoint"
    );
    // The class members one level deeper: the labeled element's group is the layout's own, so
    // each width asserts the guard's own claim — the multiply is an initializer element's, kept
    // tight — by idempotency, the one property the pre-fix tree failed at these widths.
    for src in ["={0:{A*a}?\"\"?}", "={0:{A*b}?1:2}", "={0:{x*y}?a:b}"] {
        for width in 1..=32 {
            let once = jphfmt::format_with_width(src, width);
            assert_eq!(
                jphfmt::format_with_width(&once, width),
                once,
                "not a fixpoint at {width}: {src:?}"
            );
        }
    }
}

#[test]
fn a_statement_expressions_declarators_stay_spaced_across_the_enclosing_group() {
    // #143's review: the floor let a statement expression's own `=` decide its body's braces —
    // `S *p` went tight where §2.5 spells `S * p`. A body whose `;` has a statement after it is
    // a block, whose enclosing statement-expression group masks the `=` assigning the expression.
    for src in [
        "x = ({ q; S *p = q; p; });",
        "int a[] = { ({ S *p = q; p; }), 1 };",
        "x = f(({ q; S *p = q; }));",
        "x = ({ if (c) { S *p; } p; });",
        "f((x = y), ({ q; S *p = q; }));",
    ] {
        let once = format(src);
        assert!(once.contains("S * p"), "the declarator is spaced: {once:?}");
        assert_eq!(format(&once), once, "and it is a fixpoint");
    }
    // A single trailing `;`, a trailing comment, or a nested brace whose only `;` trails keeps
    // the expression verdict — the multiply stays tight, and the layout's magic trailing comma
    // after the `;` does not flip it on the next pass.
    for src in [
        "x = ({ A*a; });",
        "x = ({ A*a; /* c */ });",
        "x = ({ { S *p = q; } y; });",
    ] {
        let once = jphfmt::format_with_width(src, 1);
        assert!(
            !once.contains("A * a") && !once.contains("S * p"),
            "kept tight: {once:?}"
        );
        assert_eq!(
            jphfmt::format_with_width(&once, 1),
            once,
            "and it is a fixpoint"
        );
    }
}

#[test]
fn a_multiply_inside_braces_is_left_alone() {
    // An initializer element is an expression, whichever brace holds it — `=`, a compound literal
    // in `return` or in an argument, or a nested list.
    for src in [
        "int v[] = {a*b};\n",
        "int m[] = {{a*b}, {c*d}};\n",
        "f((struct Foo){a*b});\n",
    ] {
        assert_eq!(format(src), src, "must pass through: {src:?}");
    }
    assert_eq!(
        format("return (struct Foo){a*b};\n"),
        "return (struct Foo){a*b};\n"
    );
}

#[test]
fn a_declaration_after_a_brace_is_spaced() {
    // The `{` and `}` statement boundaries, not just `;` and start-of-input.
    assert_eq!(format("{ PyObject *p; }\n"), "{ PyObject * p; }\n");
    assert_eq!(
        format("void f(void) {}\nPyObject *p;\n"),
        "void f(void) {}\nPyObject * p;\n"
    );
}

#[test]
fn multiply_is_not_a_declarator() {
    // Every `Ident * Ident` an expression can produce must pass through (§6): the two are
    // token-level identical, so only declaration position tells them apart.
    for src in [
        "z = a*b;\n",
        "int n = f(a*b);\n",
        "x = arr[n*m];\n",
        "q = obj->fn(a*b);\n",
        "v = n*3;\n",
        "w = sizeof(int)*n;\n",
        "y = a * *p;\n",
        "foo(bar, baz*qux);\n",
    ] {
        assert_eq!(format(src), src, "expression must pass through: {src:?}");
    }
    assert_eq!(format("return f(a*b);\n"), "return f(a*b);\n");
    // Accepted §6 trade-off: an expression statement whose result is discarded is
    // token-indistinguishable from a declaration, so it is spaced as one.
    assert_eq!(format("a*b;\n"), "a * b;\n");
}

#[test]
fn ambiguous_star_is_left_alone() {
    // a multiply in expression position keeps whatever spacing it was written with (§6)
    assert_eq!(format("z = a*b;\n"), "z = a*b;\n");
    assert_eq!(format("z = a * b;\n"), "z = a * b;\n");
}

#[test]
fn function_pointer_star_is_not_spaced() {
    let src = "int (*cb)(void);\n";
    assert_eq!(format(src), src);
}

#[test]
fn struct_tag_pointer_is_middle_spaced() {
    assert_eq!(format("struct shape*s;\n"), "struct shape * s;\n");
    assert_eq!(format("union u*p;\n"), "union u * p;\n");
}

#[test]
fn casts_get_a_trailing_space() {
    assert_eq!(format("x = (int)y;\n"), "x = (int) y;\n");
    assert_eq!(format("p = (void *)q;\n"), "p = (void *) q;\n");
    assert_eq!(
        format("n = (unsigned char)b;\n"),
        "n = (unsigned char) b;\n"
    );
    // a grouped expression is not a cast
    assert_eq!(format("z = (a + b) * c;\n"), "z = (a + b) * c;\n");
    // a call is not a cast
    assert_eq!(format("v = sizeof(int);\n"), "v = sizeof(int);\n");
}

#[test]
fn a_prefix_operator_after_a_cast_stays_tight() {
    // A cast is not a value, so the operator after it takes one operand: `&x` is an address-of,
    // not a bitwise-and. `(T *)` proves itself a type by its trailing `*`, which no expression
    // can end with, so a typedef name needs no keyword.
    assert_eq!(
        format("g((PyObject *) &SomeType);\n"),
        "g((PyObject *) &SomeType);\n"
    );
    assert_eq!(format("a = (int) -x;\n"), "a = (int) -x;\n");
    assert_eq!(format("b = (int) +y;\n"), "b = (int) +y;\n");
    assert_eq!(format("c = (int) *p;\n"), "c = (int) *p;\n");
    // A `)` that closes anything else still ends a value, so a real binary stays spaced.
    assert_eq!(format("i = q & r;\n"), "i = q & r;\n");
    assert_eq!(format("j = (a + b) & mask;\n"), "j = (a + b) & mask;\n");
    assert_eq!(format("k = f(x) & mask;\n"), "k = f(x) & mask;\n");
    // Redundant parentheses around a lone name are indistinguishable from a cast without knowing
    // whether the name is a type, so that reading is left alone.
    assert_eq!(format("m = (count) & mask;\n"), "m = (count) & mask;\n");
    // `sizeof(int)` and friends take a parenthesized type and yield a *value*, so the operator
    // after them binds two operands and stays a chain cut.
    assert_eq!(
        format("x = sizeof(int) & mask;\n"),
        "x = sizeof(int) & mask;\n"
    );
    assert_eq!(
        format("z = _Alignof(int) - 1;\n"),
        "z = _Alignof(int) - 1;\n"
    );
    let long = "unsigned long value_with_a_long_name = sizeof(struct some_fairly_long_structure_name) & mask_with_a_long_name;\n";
    assert_eq!(
        format(long),
        "unsigned long value_with_a_long_name = (\n\tsizeof(struct some_fairly_long_structure_name) &\n\tmask_with_a_long_name\n);\n",
        "a value-yielding keyword group must still offer the chain a cut"
    );
}

#[test]
fn brace_attaches_for_functions_and_control() {
    assert_eq!(format("void f(void){}\n"), "void f(void) {}\n");
    assert_eq!(format("if(x){}\n"), "if (x) {}\n");
}

#[test]
fn compound_literal_padding_is_canonical() {
    // One construct, one rendering: the brace interior is emitted tight whatever the input had,
    // so a file cannot drift into a mix of `{ .x = 1 }` and `{.x = 1}` and stay that way.
    for src in [
        "return (struct s){ .x = 1 };\n",
        "return (struct s) { .x = 1 };\n",
        "return (struct s){.x = 1};\n",
    ] {
        assert_eq!(format(src), "return (struct s){.x = 1};\n", "input {src:?}");
    }
    // `return` heads a value, not a body, so the `){` stays tight like every other compound
    // literal (§8.4) — it is not a K&R brace attach.
    assert_eq!(
        format("int y = f((struct s) { .x = 1 });\n"),
        "int y = f((struct s){.x = 1});\n"
    );
}

#[test]
fn a_body_brace_after_an_extra_paren_group_is_not_a_literal() {
    // A `)` before a body's `{` is not enough: a function-pointer return type, a `__attribute__`,
    // and a commented callee all put one there, and none of them is a compound literal.
    for src in [
        "void (*signal(int sig, void (*handler)(int)))(int) { return handler; }\n",
        "void f(void) __attribute__((noreturn)) { g(); }\n",
        "void f /* c */ (void) { g(); }\n",
    ] {
        assert_eq!(format(src), src, "must pass through: {src:?}");
    }
}

#[test]
fn a_declarator_inside_the_type_is_still_a_literal() {
    // `(int (*)[10])` spells a type, parentheses and all, so its list canonicalizes like any other.
    assert_eq!(
        format("p = (int (*)[10]){ 1, 2, 3 };\n"),
        "p = (int (*)[10]){1, 2, 3};\n"
    );
    // A keyword that takes its own argument list does not make a type group.
    assert_eq!(
        format("y = (sizeof(int)) * 2;\n"),
        "y = (sizeof(int)) * 2;\n"
    );
}

#[test]
fn compound_literal_brace_stays_tight() {
    // §8.4: `&(struct shape){…}` has no space before `{` (it is not a function/control body)
    assert_eq!(
        format("p = &(struct shape){.x = 1};\n"),
        "p = &(struct shape){.x = 1};\n"
    );
}

#[test]
fn compound_literals_in_function_args() {
    // the inner `){` of a compound literal stays tight even inside a call's argument list
    assert_eq!(
        format("configure(&(struct opts){.mode = 1, .flags = 0}, count);\n"),
        "configure(&(struct opts){.mode = 1, .flags = 0}, count);\n"
    );
    assert_eq!(
        format("register_handler(handler, (struct event){.type = T, .data = d}, priority);\n"),
        "register_handler(handler, (struct event){.type = T, .data = d}, priority);\n"
    );
    // a long call carrying a compound-literal argument still explodes one-per-line, arg intact
    let long = "dispatch(&(struct request){.id = 1234567, .kind = KIND_READ}, &response_buffer_out, default_timeout_ms);\n";
    let expected = "dispatch(\n\t&(struct request){.id = 1234567, .kind = KIND_READ},\n\t&response_buffer_out,\n\tdefault_timeout_ms\n);\n";
    assert_eq!(format(long), expected);
}

#[test]
fn compound_literal_arg_explodes_its_initializer_when_long() {
    let src = "init(&(struct config){.alpha = 1111111111, .beta = 2222222222, .gamma = 3333333333, .delta = 4444444444});\n";
    let expected = "init(\n\t&(struct config){\n\t\t.alpha = 1111111111,\n\t\t.beta = 2222222222,\n\t\t.gamma = 3333333333,\n\t\t.delta = 4444444444,\n\t}\n);\n";
    assert_eq!(format(src), expected);
}

#[test]
fn bit_field_colon_spacing() {
    assert_eq!(
        format("struct s {\n\tint x:2;\n};\n"),
        "struct s {\n\tint x: 2;\n};\n"
    );
    // a ternary colon must not be touched
    assert_eq!(format("z = a ? b : 3;\n"), "z = a ? b : 3;\n");
}

#[test]
fn crlf_is_normalized_to_lf() {
    assert_eq!(format("int x;\r\nint y;\r\n"), "int x;\nint y;\n");
    // a construct jphfmt generates must not leave mixed endings
    let exploded = format(
        "r = f(\r\n\taaaaaaaaaa, bbbbbbbbbb, cccccccccc, dddddddddd, eeeeeeeeee, ffffffffff\r\n);\r\n",
    );
    assert!(
        !exploded.contains('\r'),
        "output must be pure LF: {exploded:?}"
    );
}

#[test]
fn blank_line_runs_collapse_to_one_everywhere() {
    assert_eq!(format("int a;\n\n\nint b;\n"), "int a;\n\nint b;\n");
    // inside a function body too
    assert_eq!(
        format("void f(void) {\n\tint a;\n\n\n\tint b;\n}\n"),
        "void f(void) {\n\tint a;\n\n\tint b;\n}\n"
    );
    // a single blank, and adjacent lines, are left exactly as-is (never inserts)
    assert_eq!(
        format("int a;\nint b;\n\nint c;\n"),
        "int a;\nint b;\n\nint c;\n"
    );
}

#[test]
fn exactly_one_trailing_newline() {
    assert_eq!(format("int x;"), "int x;\n");
    assert_eq!(format("int x;\n\n\n"), "int x;\n");
    assert_eq!(format(""), "");
    assert_eq!(format("\n\n  \n"), "");
}

#[test]
fn block_comment_internals_are_untouched() {
    let src = "/*\n * aligned\n *   deeper\n */\nint x;\n";
    assert_eq!(format(src), src, "comment bodies are sacred (§2.1)");
}

#[test]
fn short_control_headers_unchanged() {
    assert_eq!(format("if (n < 0) {\n}\n"), "if (n < 0) {\n}\n");
    assert_eq!(
        format("while (total > 100) {\n}\n"),
        "while (total > 100) {\n}\n"
    );
    assert_eq!(format("switch (c) {\n}\n"), "switch (c) {\n}\n");
    assert_eq!(
        format("for (int i = 0; i < n; i++) {\n}\n"),
        "for (int i = 0; i < n; i++) {\n}\n"
    );
}

#[test]
fn long_for_header_explodes_one_clause_per_line() {
    let src = "for (size_t current_sample_index = 0; current_sample_index < total_number_of_samples; current_sample_index++) {\n}\n";
    let expected = "for (\n\tsize_t current_sample_index = 0;\n\tcurrent_sample_index < total_number_of_samples;\n\tcurrent_sample_index++\n) {\n}\n";
    assert_eq!(format(src), expected);
}

#[test]
fn long_if_condition_explodes_with_trailing_operators() {
    let src = "if (averaged_result > MINIMUM_ACCEPTABLE_THRESHOLD && averaged_result < MAXIMUM_ACCEPTABLE_THRESHOLD && averaged_result != 0) {\n}\n";
    let expected = "if (\n\taveraged_result > MINIMUM_ACCEPTABLE_THRESHOLD &&\n\taveraged_result < MAXIMUM_ACCEPTABLE_THRESHOLD &&\n\taveraged_result != 0\n) {\n}\n";
    assert_eq!(format(src), expected);
}

#[test]
fn condition_splits_on_the_outer_logical_operator() {
    let src = "if (alpha_value > 100 || bravo_value > 200 || charlie_value > 300 || delta_value > 400 || echo_v > 5) {\n}\n";
    let expected = "if (\n\talpha_value > 100 ||\n\tbravo_value > 200 ||\n\tcharlie_value > 300 ||\n\tdelta_value > 400 ||\n\techo_v > 5\n) {\n}\n";
    assert_eq!(format(src), expected);
}

#[test]
fn unbalanced_brackets_pass_through_verbatim() {
    // an inner `(` with no match makes the list unstructurable; it must pass through unchanged
    // rather than be mis-split (which previously accumulated commas across passes)
    assert_eq!(format("int v[] = {a, (b};\n"), "int v[] = {a, (b};\n");
    assert_eq!(format("f(a, [b);\n"), "f(a, [b);\n");
}

#[test]
fn for_header_is_not_treated_as_a_call() {
    // comma operator inside a for clause must not be split as call args
    let src = "for (int i = 0, j = N - 1; i < j; i++, j--) {\n}\n";
    assert_eq!(format(src), src);
}

#[test]
fn compound_literal_initializer_explodes() {
    let src = "p = &(struct shape){.tag = R, .rect = {.w = 3, .h = 4},};\n";
    let expected = "p = &(struct shape){\n\t.tag = R,\n\t.rect = {.w = 3, .h = 4},\n};\n";
    assert_eq!(format(src), expected);
}

#[test]
fn function_like_macro_body_opens_on_define_line() {
    let src = "#define DISPATCH_EVENT(handler, event) dispatch_incoming_event((handler), (event), read_monotonic_timestamp_ms(), current_execution_context_id())\n";
    let expected = "#define DISPATCH_EVENT(handler, event) dispatch_incoming_event( \\\n\t(handler), \\\n\t(event), \\\n\tread_monotonic_timestamp_ms(), \\\n\tcurrent_execution_context_id() \\\n)\n";
    assert_eq!(format(src), expected);
}

#[test]
fn statement_expression_macro_blocks_with_continuations() {
    let src =
        "#define MAX(a, b) ({ typeof(a) _a = (a); typeof(b) _b = (b); _a > _b ? _a : _b; })\n";
    let expected = "#define MAX(a, b) ({ \\\n\ttypeof(a) _a = (a); \\\n\ttypeof(b) _b = (b); \\\n\t_a > _b ? _a : _b; \\\n})\n";
    assert_eq!(format(src), expected);
}

#[test]
fn generic_macro_explodes_one_association_per_line() {
    let src = "#define type_name(x) _Generic((x), int: \"int\", long: \"long\", float: \"float\", double: \"double\", default: \"other\")\n";
    let expected = "#define type_name(x) _Generic( \\\n\t(x), \\\n\tint: \"int\", \\\n\tlong: \"long\", \\\n\tfloat: \"float\", \\\n\tdouble: \"double\", \\\n\tdefault: \"other\" \\\n)\n";
    assert_eq!(format(src), expected);
}

#[test]
fn short_object_macro_unchanged() {
    assert_eq!(format("#define PI 3.14159\n"), "#define PI 3.14159\n");
    assert_eq!(
        format("#define MIN(a, b) ((a) < (b) ? (a) : (b))\n"),
        "#define MIN(a, b) ((a) < (b) ? (a) : (b))\n"
    );
}

#[test]
fn do_while_macro_passes_through() {
    let src = "#define SWAP(a, b) \\\n\tdo { \\\n\t\tint t = a; \\\n\t} while (0)\n";
    assert_eq!(
        format(src),
        src,
        "do/while macro bodies are not yet structured"
    );
}

#[test]
fn statement_expression_in_code_block_indents() {
    let src = "int d = ({ int t = larger; t * 2; });\n";
    let expected = "int d = ({\n\tint t = larger;\n\tt * 2;\n});\n";
    assert_eq!(format(src), expected);
}

/// The statement-expression emitter consumed more than it rendered, so source was deleted outright:
/// it reported everything up to `)` while rendering only as far as `}`. Those spans pass through now —
/// §6 prefers passthrough to guessing, and no relayout may lose what the author wrote.
#[test]
fn a_statement_expression_the_emitter_cannot_own_passes_through() {
    for src in [
        // Something between `}` and `)`, which the emitter consumed without rendering.
        "({x}y)\n",
        "({\"\"}\"\")\n",
        "({ int t = 1; t; }/*c*/)\n",
        // The `}` is outside the `)` — `match_brace` and `match_bracket` disagree about the nesting.
        "({)}\n",
        // No statements at all, so there is no body to lay out.
        "({})\n",
        // #74's two inputs, which the property tests found twice in one day as a `){` that gained a
        // space on the second pass. The `){` was the symptom: what the emitter deleted between `}`
        // and `)` is what `space_braces` read on the first pass and no longer read on the second.
        // Deleting nothing leaves nothing for it to disagree with.
        "A''A({\"\"}]\"\"''\"\"){\n",
        "_({0\"\"}'']){\n",
    ] {
        assert_eq!(format(src), src, "must pass through unchanged");
    }
}

/// A `;` that opens no statement is still a statement, and the emitter writes exactly one `;` per
/// statement — so dropping the empty ones lost the `;` that produced them. Every leading segment is
/// kept for that reason; only a *trailing* empty one is dropped, since that is what a body ending in
/// `;` splits to and keeping it would write a `;` the author did not.
///
/// Asserted as exact output rather than token equality: both are needed, because a passthrough would
/// satisfy token equality too, and that is what made this indistinguishable before.
#[test]
fn every_statement_in_a_statement_expression_keeps_its_semicolon() {
    for (src, expected) in [
        ("({;,;})", "({\n\t;\n\t,;\n})\n"),
        ("({x;;y;})", "({\n\tx;\n\t;\n\ty;\n})\n"),
        ("({;})", "({\n\t;\n})\n"),
        // The canonical forms: a trailing `;` splits to an empty last segment, which is dropped, and
        // an unterminated last statement gains the `;` it needs.
        ("({ int t = 1; t; })", "({\n\tint t = 1;\n\tt;\n})\n"),
        ("({x;})", "({\n\tx;\n})\n"),
        ("({x})", "({\n\tx;\n})\n"),
    ] {
        assert_eq!(format(src), expected, "input {src:?}");
        assert_eq!(format(expected), expected, "must be a fixpoint: {src:?}");
    }
}

/// A comment-bearing group is never laid out, at any length. The builders take no comment guard of
/// their own — `emit_tokens` refuses the construct first — and this pins that, because flattening a
/// `//` comment would put the rest of the group on the comment's line and swallow it.
#[test]
fn a_comment_bearing_group_passes_through_however_long() {
    for src in [
        "int x = (aaaaaaaaaaaaaaaaaaaaaa /* c */ | bbbbbbbbbbbbbbbbbbbbbb | cccccccccccccccccccccc | dddddddddd);\n",
        "int y = (aaaaaaaaaaaaaaaaaaaaaa // c\n\t| bbbbbbbbbbbbbbbbbbbbbb | cccccccccccccccccccccc | ddddddddddddddd);\n",
        "int z = arr[aaaaaaaaaaaaaaaaaaaaaa /* c */ + bbbbbbbbbbbbbbbbbbbbbb + cccccccccccccccccccccc + ddddd];\n",
    ] {
        assert_eq!(format(src), src, "must pass through: {src:?}");
    }
}

/// #77: a `for` clause is an element of the header container, not a bare expression. A ternary chain
/// in one therefore reads as the map it is, and is bounded — unbounded, its arms would sit at the
/// clause indent and read as further clauses, exactly as they would read as further arguments (#59).
#[test]
fn a_for_clause_is_an_element_of_its_header() {
    assert_eq!(
        format("for (i = a ? b : c ? d : e; i < n; i++) {\n\tg();\n}\n"),
        "for (\n\ti = (\n\t\ta ? b :\n\t\tc ? d :\n\t\te\n\t);\n\ti < n;\n\ti++\n) {\n\tg();\n}\n"
    );
    // The step clause is the same element, so it lays out the same way.
    assert_eq!(
        format("for (i = 0; i < n; i = a ? b : c ? d : e) {\n\tg();\n}\n"),
        "for (\n\ti = 0;\n\ti < n;\n\ti = (\n\t\ta ? b :\n\t\tc ? d :\n\t\te\n\t)\n) {\n\tg();\n}\n"
    );
}

/// The clauses that were already right stay right: a header that fits is untouched, and a clause
/// holding a depth-zero `,` is a list rather than one expression, so nothing bounds it
/// (`is_boundable`).
///
/// An empty clause is an element like any other, and takes no space after the separator that ends it
/// (#85) — so the most idiomatic loop in C round-trips, wherever the hole is.
#[test]
fn an_ordinary_for_header_is_unchanged() {
    for src in [
        "for (i = 0; i < n; i++) {\n\tg();\n}\n",
        "for (int i = 0, j = n; i < j; i++, j--) {\n\tg();\n}\n",
        "for (i = a ? b : c; i < n; i++) {\n\tg();\n}\n",
        "for (i = 0; i < n; i++);\n",
        "for (;;) {\n\tg();\n}\n",
        "for (; i < n;) {\n\tg();\n}\n",
        "for (i = 0;;) {\n\tg();\n}\n",
        "for (;; i++) {\n\tg();\n}\n",
    ] {
        assert_eq!(format(src), src, "input {src:?}");
    }
}

/// #85: an empty clause takes no *space* after its separator, which is not the same as taking no
/// separator. The broken form still puts every clause on its own line — the header is one container,
/// and a clause that happens to be empty is still one of its elements.
#[test]
fn an_empty_for_clause_still_breaks_onto_its_own_line() {
    assert_eq!(
        format(
            "for (;; iiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiii++, jjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjj++) {\n\tg();\n}\n"
        ),
        "for (\n\t;\n\t;\n\tiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiii++, \
         jjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjjj++\n) {\n\tg();\n}\n"
    );
}

/// #77: a statement in a statement-expression body is an element of that body, not a bare expression.
/// A ternary chain in one therefore reads as the map it is, bounded — unbounded, its arms would sit at
/// the statement indent and read as further statements (#59).
#[test]
fn a_statement_expression_body_holds_elements() {
    assert_eq!(
        format("int x = ({ int t = a ? b : c ? d : e; t * 2; });\n"),
        "int x = ({\n\tint t = (\n\t\ta ? b :\n\t\tc ? d :\n\t\te\n\t);\n\tt * 2;\n});\n"
    );
    // A head-less chain is bounded by the same rule that bounds a `{}` element's (#63).
    assert_eq!(
        format("int y = ({ g(); a ? b : c ? d : e; });\n"),
        "int y = ({\n\tg();\n\t(\n\t\ta ? b :\n\t\tc ? d :\n\t\te\n\t);\n});\n"
    );
    // A body whose statements fit still explodes at its `;` — that is the statement-expression rule,
    // unchanged — but nothing inside a statement is bounded, and one `?` is one conditional.
    for (src, expected) in [
        (
            "int z = ({ int t = larger; t * 2; });\n",
            "int z = ({\n\tint t = larger;\n\tt * 2;\n});\n",
        ),
        (
            "int w = ({ int t = a ? b : c; t; });\n",
            "int w = ({\n\tint t = a ? b : c;\n\tt;\n});\n",
        ),
    ] {
        assert_eq!(format(src), expected, "input {src:?}");
    }
}

#[test]
fn long_binary_chain_explodes_with_trailing_operators() {
    // §2.2/§2.7: an operator chain is a container like any other, so it breaks one operand per line
    // with the operator trailing — bounded by parentheses jphfmt adds, since the operands after an
    // assignment are already an implicit container. Author-written parentheses reach the same form.
    let broken = "int x = (\n\tAAAAAAAAAAAAAAAA |\n\tBBBBBBBBBBBBBBBB |\n\tCCCCCCCCCCCCCCCC |\n\tDDDDDDDDDDDDDDDD |\n\tEEEEEEEEEEEEEEEE\n);\n";
    for src in [
        "int x = AAAAAAAAAAAAAAAA | BBBBBBBBBBBBBBBB | CCCCCCCCCCCCCCCC | DDDDDDDDDDDDDDDD | EEEEEEEEEEEEEEEE;\n",
        "int x = (AAAAAAAAAAAAAAAA | BBBBBBBBBBBBBBBB | CCCCCCCCCCCCCCCC | DDDDDDDDDDDDDDDD | EEEEEEEEEEEEEEEE);\n",
        broken,
    ] {
        assert_eq!(format(src), broken, "input {src:?}");
    }
}

#[test]
fn a_long_bare_ternary_is_bounded_too() {
    // The same rule, applied to §2.4: a ternary the author left unparenthesized is still an implicit
    // container, so it breaks with the `:` trailing rather than overrunning.
    let bare = "acc = status_code == 0 ? \"ok\" : status_code == 1 ? \"busy\" : status_code == 2 ? \"error\" : status_code < 0 ? \"fault\" : \"unknown\";\n";
    let broken = "acc = (\n\tstatus_code == 0 ? \"ok\" :\n\tstatus_code == 1 ? \"busy\" :\n\tstatus_code == 2 ? \"error\" :\n\tstatus_code < 0 ? \"fault\" :\n\t\"unknown\"\n);\n";
    assert_eq!(format(bare), broken);
    assert_eq!(format(broken), broken);
}

#[test]
fn a_container_that_already_bounds_gets_no_parens() {
    // A call's own parentheses bound its argument, so a chain there needs none of its own — the
    // parentheses exist to bound operands nothing else does, after an assignment or a `return`.
    let src = "call_something(AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA | BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB | CCCCCCCCCCCCCCCCCCCCCCCCCCCCCC);\n";
    let expected = "call_something(\n\tAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA |\n\tBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB |\n\tCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC\n);\n";
    assert_eq!(format(src), expected);
    assert_eq!(format(expected), expected);
}

#[test]
fn a_postfix_operand_ends_a_value() {
    // `i++ | x` splits at the `|`: the `++` ends its operand as much as the identifier does.
    let src = "int x = counter_value_here++ | BBBBBBBBBBBBBBBBBBBBBBBBBB | CCCCCCCCCCCCCCCCCCCCCCCCCCCCCC | DDDDDDDDDDDDDD;\n";
    let expected = "int x = (\n\tcounter_value_here++ |\n\tBBBBBBBBBBBBBBBBBBBBBBBBBB |\n\tCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC |\n\tDDDDDDDDDDDDDD\n);\n";
    assert_eq!(format(src), expected);
}

#[test]
fn a_list_is_never_bounded() {
    // A depth-zero `,` means the span is a list, not one expression: `(a | b, c)` is not `a | b, c`,
    // so a second declarator or a comma expression is left overrunning rather than changed.
    let src = "int aaaaaaaaaaaaaaaaaaaaaaaaaaaa = XXXXXXXXXXXXXXXXXXXXXXXXXXXX | YYYYYYYYYYYYYYYYYYYYYYYYYYYY | ZZZZZZZZZZZZZZZZZZZZZZZZ, b;\n";
    assert_eq!(format(src), src);
}

/// #52: a conjunct that is a single comparison of one whole call prefers its own break — the
/// call's arguments — over the comparison's, with the layout's parentheses around it, and the
/// operator stays with its right operand on the call's close line. The wrapped form the next pass
/// re-reads lays out to the same shape.
#[test]
fn a_comparison_conjunct_breaks_inside_its_call() {
    let src = "if (all_names != NULL\n\t&& new_names != NULL\n\t&& default_by_name != NULL\n\t&& append_inherited(base, all_names, default_by_name) == RESULT_OK\n\t&& append_declared(base, annotations, namespace, all_names, new_names, default_by_name) == RESULT_OK) {\n\treturn 1;\n}\n";
    let expected = "if (\n\tall_names != NULL &&\n\tnew_names != NULL &&\n\tdefault_by_name != NULL &&\n\tappend_inherited(base, all_names, default_by_name) == RESULT_OK &&\n\t(\n\t\tappend_declared(\n\t\t\tbase,\n\t\t\tannotations,\n\t\t\tnamespace,\n\t\t\tall_names,\n\t\t\tnew_names,\n\t\t\tdefault_by_name\n\t\t) == RESULT_OK\n\t)\n) {\n\treturn 1;\n}\n";
    assert_eq!(format(src), expected);
    assert_eq!(
        format(expected),
        expected,
        "the laid-out form is a fixpoint"
    );
    // A conjunct that fits keeps its flat form, no parentheses.
    assert_eq!(
        format("if (x && append(a, b) == RESULT_OK) { g(); }\n"),
        "if (x && append(a, b) == RESULT_OK) { g(); }\n"
    );
    // The head-bounded contexts converge to the same shape on both passes.
    for src in [
        "x = append_declared(base, annotations, namespace, all_names, new_names, default_by_name) == RESULT_OK;\n",
        "return append_declared(base, annotations, namespace, all_names, new_names, default_by_name) == RESULT_OK;\n",
        "int v = append_declared(base, annotations, namespace, all_names, new_names, default_by_name) == RESULT_OK;\n",
    ] {
        let once = format(src);
        assert_eq!(format(&once), once, "must be idempotent: {src:?}");
    }
    // A headless conjunct writes no stray head: it bounds itself, and it fits flat it stays
    // exactly as the author wrote it.
    assert_eq!(
        format("printf(\"%d\\n\", strcmp(a, b) == 0);\n"),
        "printf(\"%d\\n\", strcmp(a, b) == 0);\n"
    );
    assert_eq!(
        format("for (int i = 0; strcmp(a, b) == 0; i++) { f(i); }\n"),
        "for (int i = 0; strcmp(a, b) == 0; i++) { f(i); }\n"
    );
    assert_eq!(
        format("int arr[] = { strcmp(a, b) == 0 };\n"),
        "int arr[] = {strcmp(a, b) == 0};\n"
    );
    let headless: &[(&str, &str)] = &[
        (
            "printf(\"%d\\n\", strcmp(a, b) == 0);\n",
            "printf(\n\t\"%d\\n\",\n\t(\n\t\tstrcmp(\n\t\t\ta,\n\t\t\tb\n\t\t) == 0\n\t)\n);\n",
        ),
        (
            "for (int i = 0; strcmp(a, b) == 0; i++) {\n\tf(i);\n}\n",
            "for (\n\tint i = 0;\n\t(\n\t\tstrcmp(\n\t\t\ta,\n\t\t\tb\n\t\t) == 0\n\t);\n\ti++\n) {\n\tf(\n\t\ti\n\t);\n}\n",
        ),
        (
            "int arr[] = { strcmp(a, b) == 0 };\n",
            "int arr[] = {\n\t(\n\t\tstrcmp(\n\t\t\ta,\n\t\t\tb\n\t\t) == 0\n\t),\n};\n",
        ),
    ];
    for &(src, expected) in headless {
        assert_eq!(&format_with_width(src, 1), expected, "headless: {src:?}");
        assert_eq!(
            format_with_width(expected, 1),
            expected,
            "the laid-out form is a fixpoint: {src:?}"
        );
    }
}
#[test]
fn a_conjunct_whose_left_breaks_keeps_the_right_operand_on_the_close_line() {
    // The review's fuzz found the shape the pins could not: when the left call breaks, pass 2
    // re-reads it through the `has_middle_newline` passthrough as one multi-line text, and the
    // renderer's column accounting consumed the whole string — the right operand's group that fit
    // on the close line in pass 1 broke in pass 2. The renderer now reads a text's newlines: the
    // column after one is its last line's tail, the same column the structured doc's broken
    // [`Doc::Line`]s reached.
    let src = "x = p(check(*q, b(append), (T*)y)) > b(y);\n";
    let expected = "x = (\n\tp(\n\t\tcheck(\n\t\t\t*q,\n\t\t\tb(\n\t\t\t\tappend\n\t\t\t),\n\t\t\t(T*)y\n\t\t)\n\t) > b(y)\n);\n";
    let once = format_with_width(src, 20);
    assert_eq!(once, expected, "the pinned shape");
    assert_eq!(format_with_width(&once, 20), once, "and it is a fixpoint");

    // The head-bounded form, pinned exactly: the head leads, the call explodes, the operator
    // stays with its right operand on the close line.
    let head = "x = append_declared(base, annotations, namespace, all_names, new_names, default_by_name) == RESULT_OK;\n";
    let expected = "x = (\n\tappend_declared(\n\t\tbase,\n\t\tannotations,\n\t\tnamespace,\n\t\tall_names,\n\t\tnew_names,\n\t\tdefault_by_name\n\t) == RESULT_OK\n);\n";
    let once = format_with_width(head, 40);
    assert_eq!(once, expected, "the head-bounded broken form");
    assert_eq!(format_with_width(&once, 40), once, "and it is a fixpoint");

    // The sole-argument Enclosing form: the enclosing call's own parens bound the operands, and
    // the conjunct writes no pair of its own.
    let sole = "g(append_declared(base, annotations, namespace, all_names, new_names, default_by_name) == RESULT_OK);\n";
    let expected = "g(\n\tappend_declared(\n\t\tbase,\n\t\tannotations,\n\t\tnamespace,\n\t\tall_names,\n\t\tnew_names,\n\t\tdefault_by_name\n\t) == RESULT_OK\n);\n";
    let once = format_with_width(sole, 40);
    assert_eq!(
        once, expected,
        "the Enclosing form writes no pair of its own"
    );
    assert_eq!(format_with_width(&once, 40), once, "and it is a fixpoint");

    // A span whose width the model cannot describe — an unterminated literal spanning lines,
    // spelled with a *real* newline inside the string token so `spans_lines` sees it — takes no
    // conjunct parens: the same refusal `is_boundable` makes on the chain path.
    let literal = "int arr[] = { f(\"abc\\\n def\") == 0 };\n";
    let once = format_with_width(literal, 20);
    assert!(!once.contains(" ( f("), "no stray-spaced parens: {once:?}");
    assert_eq!(format_with_width(&once, 20), once, "and it is a fixpoint");
}

#[test]
fn a_chain_that_fits_stays_flat() {
    for src in [
        "int x = A | B | C;\n",
        "int x = (A | B | C);\n",
        "flags = a & b;\n",
        "total = first + second - third;\n",
    ] {
        assert_eq!(format(src), src, "must stay flat: {src:?}");
    }
}

#[test]
fn a_chain_splits_on_its_loosest_operator() {
    let src = "int x = aaaaaaaaaaaaaaaaaaaaaaaa * bbbbbbbbbbbbbbbbbbbbbbbb + cccccccccccccccccccccccc * dddddddddddddddddddddddd;\n";
    let expected = "int x = (\n\taaaaaaaaaaaaaaaaaaaaaaaa * bbbbbbbbbbbbbbbbbbbbbbbb +\n\tcccccccccccccccccccccccc * dddddddddddddddddddddddd\n);\n";
    assert_eq!(format(src), expected);
}

#[test]
fn a_declaration_is_never_a_chain() {
    // §6: `*` is not a chain operator, so a long declarator is left alone rather than broken
    // between its type and its name.
    let src =
        "static struct a_rather_long_type_name_here * const the_pointer_variable_name = nullptr;\n";
    assert_eq!(format(src), src);
}

#[test]
fn short_parenthesized_ternary_stays_flat() {
    assert_eq!(format("x = (b != 0 ? b : 1);\n"), "x = (b != 0 ? b : 1);\n");
}

/// #64: the lexer has no keyword kind, so `return` is an `Ident` and `space_casts` read it as a
/// value, leaving the cast after it tight. The layout's own bounding parenthesis then replaced
/// `return` as the preceding token, so the second pass spaced what the first had not.
#[test]
fn a_cast_after_return_is_spaced_on_the_first_pass() {
    assert_eq!(
        format("return (float)x + (float)y;\n"),
        "return (float) x + (float) y;\n"
    );
    let long = "return (float)d->c00 + xxxxxxxxxxxxxxxxxxxx * (float)d->c10 + \
                yyyyyyyyyyyyyyyyyyyy * (float)d->c01;\n";
    let once = format(long);
    assert_eq!(format(&once), once, "\n--- once ---\n{once}");
    assert!(
        once.contains("\t(float) d->c00 +"),
        "\n--- once ---\n{once}"
    );
}

/// A typedef name spells no type keyword, so `(size_t)` is not confidently a cast and is left alone
/// (§6) — the same residual #55 recorded, unchanged by the `return` carve-out.
#[test]
fn a_cast_through_a_typedef_is_still_left_alone() {
    assert_eq!(format("return (size_t)n;\n"), "return (size_t)n;\n");
}

#[test]
fn short_unparenthesized_ternary_is_left_alone() {
    assert_eq!(format("acc = a > b ? a : b;\n"), "acc = a > b ? a : b;\n");
}

#[test]
fn nested_ternary_reads_as_a_map_however_short_it_is() {
    // #59: two arms are one conditional and fit on a line; more are a `cond -> value` map, which
    // reads as one only broken — so the width does not decide, and the parens come with the break.
    assert_eq!(
        format("acc = a > b ? a : a < b ? b : 0;\n"),
        "acc = (\n\ta > b ? a :\n\ta < b ? b :\n\t0\n);\n"
    );
}

/// A depth-zero `:` that opens no arm gives a *single* conditional a third arm. Counting `?` rather
/// than arms is what keeps these two on their line.
#[test]
fn a_colon_that_is_not_a_ternary_arm_forces_nothing() {
    let bit_field = "struct s {\n\tint f : cond ? 1 : 2;\n};\n";
    assert_eq!(format(bit_field), bit_field);
    let labeled = "void f(void) {\ndone: p ? x() : y();\n}\n";
    assert_eq!(
        format(labeled),
        "void f(void) {\n\tdone : p ? x() : y();\n}\n"
    );
}

/// A chain's arms are bounded even where no head precedes them. Unbounded, a `{}` element's arms
/// read as elements of the list, a call argument's as sibling arguments, and a statement's are not
/// indented at all.
#[test]
fn a_chain_with_no_head_is_still_bounded() {
    assert_eq!(
        format("int f[] = {a ? b : c ? d : e, 1};\n"),
        "int f[] = {\n\t(\n\t\ta ? b :\n\t\tc ? d :\n\t\te\n\t),\n\t1,\n};\n"
    );
    assert_eq!(
        format("h(x, a ? b : c ? d : e, y);\n"),
        "h(\n\tx,\n\t(\n\t\ta ? b :\n\t\tc ? d :\n\t\te\n\t),\n\ty\n);\n"
    );
    assert_eq!(
        format("a ? b() : c ? d() : e();\n"),
        "(\n\ta ? b() :\n\tc ? d() :\n\te()\n);\n"
    );
}

/// #63: the bound is the position's, not the operator's — a binary chain in a head-less element is
/// bounded exactly as a ternary's arms are.
#[test]
fn a_head_less_binary_chain_is_bounded_like_a_ternary() {
    let src = "struct s v = {AAAAAAAAAAAAAAAA | BBBBBBBBBBBBBBBB | CCCCCCCCCCCCCCCC | \
               DDDDDDDDDDDDDDDD | EEEEEEEEEEEEEEEE | FFFFFFFFFFFFFFFF, 1};\n";
    let expected = "struct s v = {\n\t(\n\t\tAAAAAAAAAAAAAAAA |\n\t\tBBBBBBBBBBBBBBBB |\n\
                    \t\tCCCCCCCCCCCCCCCC |\n\t\tDDDDDDDDDDDDDDDD |\n\t\tEEEEEEEEEEEEEEEE |\n\
                    \t\tFFFFFFFFFFFFFFFF\n\t),\n\t1,\n};\n";
    assert_eq!(format(src), expected);
}

/// #102: whitespace that ends the file never reaches the output — `normalize_endings` trims it — so
/// reserving for it measures a line this pass is about to shorten, and the next pass reaches a different
/// verdict. `trailing_reserved` says exactly that about a whitespace *run*, but an unterminated string or
/// char literal carries the whitespace *inside* the token, where that guard cannot see it.
///
/// A third piece of #84's fallout: before an index was a container, nothing here had a decision to be
/// inconsistent about. Reduced from a `proptest` failure at 200k cases.
#[test]
fn whitespace_that_ends_the_file_reserves_nothing() {
    for (src, width) in [
        ("A_[A * a < aA *]\"xxxx ", 21),
        ("A_[A * a < aA *]\'x ", 19),
        // The same tail with a newline after it is already fine, and must stay so.
        ("A_[A * a < aA *]\"xxxx \n", 21),
    ] {
        let once = jphfmt::format_with_width(src, width);
        assert_eq!(
            jphfmt::format_with_width(&once, width),
            once,
            "{src:?} at width {width} -> {once:?}"
        );
    }
}

/// The gap before a subscript is tight however it was written (§2.5) — a newline included, because the
/// layout collapses it and `space_subscripts` would tighten what the layout wrote. #84 added the spacing
/// rule without the layout's half, so `A\n[0] + b;` became `A [0] + b;` on the first run and `A[0] + b;`
/// on the second: the output was a fixpoint of the *spacing* pass rather than of itself.
///
/// The same trap `call_head_before` documents for a call's `(`, which is why the two now read the same
/// way. Found by `proptest` at 200k cases, and reduced from `'':A\n[]?;`.
#[test]
fn a_subscript_is_tight_across_a_newline_too() {
    for src in [
        "A\n[0] + b;\n",
        "arr\n[0] + arr\n[1];\n",
        // The index has nothing to lay out, so it falls through to the plain token path.
        "A\n[] + b;\n",
        // An index that *does* lay out takes the group path instead.
        "A\n[a ? b : c] + d;\n",
        // Reduced from the proptest failures, which is what a fixpoint-of-another-pass looks like.
        "'':A\n[]?;",
        "({[[]&x\n[]]",
    ] {
        let once = format(src);
        assert_eq!(format(&once), once, "{src:?} -> {once:?}");
        assert!(
            !once.contains(" ["),
            "a gap before a subscript: {src:?} -> {once:?}"
        );
    }
    // An attribute is not a subscript, and a designator ends no value, so neither is tightened. The
    // second `[` may carry a gap of its own — `space_subscripts` reads a trivia-stripped list, so the
    // layout has to look past the gap as well or the two passes disagree about what this is.
    assert_eq!(
        format("int x\n[[deprecated]];\n"),
        "int x\n[[deprecated]];\n"
    );
    let spaced_attribute = "int f(void) {\n\tif (x [ [aaaa]] && bbbbbbbbbbbbbbbbbbbbbb) { return 1; }\n\treturn 0;\n}\n";
    assert_eq!(format(spaced_attribute), spaced_attribute);
}

/// A compound literal's `{` is tight against its `(T)` for the same reason a subscript's `[` is: the
/// review on #99 found the `{` branch carrying the trap this fixed for `[`, and it reproduced —
/// `(struct s)⏎{1, 2}.a` in a condition became `(struct s) {1, 2}.a` on the first run and
/// `(struct s){1, 2}.a` on the second, because `space_braces` tightens what the layout wrote.
#[test]
fn a_compound_literal_brace_is_tight_across_a_newline_too() {
    let src = "int f(void) {\n\tif ((struct s)\n\t{1, 2}.a && bbbbbbbbbbbbbbbbbbbbbb) { return 1; }\n\treturn 0;\n}\n";
    let once = format(src);
    assert_eq!(format(&once), once, "{once:?}");
    assert!(
        once.contains("(struct s){1, 2}.a"),
        "the literal's brace is tight: {once:?}"
    );
}

/// #88: a compound literal is a value like any other, so the `}` that ends one ends a value and not a
/// statement. Read as a statement boundary, what followed the literal became a statement of its own,
/// and the parentheses bounding it (#59) landed against the `}` — `(struct s){1, 2}(.a + …)`, a *call*
/// on the literal, which does not compile.
///
/// Asserted as the absence of that call at every width rather than as exact output: what jphfmt should
/// write here is a layout question, and pinning today's answer would make tomorrow's improvement read
/// as a regression. No layout may ever put a `(` against a literal's `}`.
///
/// Nothing else in the suite can see this. Idempotency cannot: the call is a fixpoint. [`significant`]
/// cannot: it excludes parentheses, because jphfmt legitimately writes the pair bounding a broken
/// chain — which is exactly what makes a *mis-placed* one invisible.
#[test]
fn a_compound_literal_is_never_called_by_what_follows_it() {
    for src in [
        "int * p = (int[]){1, 2} + aaaaaaaaaaaaaaaaaaaa + bbbbbbbbbbbbbbbbbbbb + cccccccccccccccccccc;\n",
        "int q = (struct s){1, 2}.a + aaaaaaaaaaaaaaaaaaaa + bbbbbbbbbbbbbbbbbbbb + cccccccccccccccccc;\n",
        "int * r = ((int[]){1, 2} + aaaaaaaaaaaaaaaaaaaa) + bbbbbbbbbbbbbbbbbbbb + cccccccccccccccccc;\n",
        "void f(void) {\n\tg((int[]){1, 2} + aaaaaaaaaaaaaaaaaaaa + bbbbbbbbbbbbbbbbbbbb, 1);\n}\n",
        "int * s = (int[]){1, 2} + a;\n",
        // The type is a typedef name, so no keyword in the group says it is a type. A parenthesized
        // single name before a `{` can be nothing else, which is what makes it provable.
        "int t = (vec2_t){1, 2}.x + aaaaaaaaaaaaaaaaaaaa + bbbbbbbbbbbbbbbbbbbb + cccccccccccccccccc;\n",
        // A braceless `if` body: the `)` before the type is a control header's, not a declarator's.
        "if (c) (struct s){1, 2}.a + aaaaaaaaaaaaaaaaaaaa + bbbbbbbbbbbbbbbbbbbb + cccccccccccccccccc;\n",
        "while (c) (vec2_t){1, 2}.x + aaaaaaaaaaaaaaaaaaaa + bbbbbbbbbbbbbbbbbbbb + ccccccccccccccccc;\n",
        // A comment between the type and the body, which trivia-only skipping would stop at.
        "int u = (int[]) /* c */ {1, 2}[0] + aaaaaaaaaaaaaaaaaaaa + bbbbbbbbbbbbbbbbbbbb + cccccccccc;\n",
        // An anonymous type: its own braces come first, and they spell a `{` no type-token test accepts
        // (#95). The tag keyword opening the group is what says it is a type.
        "int v = (struct { int x; }){1}.x + aaaaaaaaaaaaaaaaaaaa + bbbbbbbbbbbbbbbbbbbb + cccccccccccc;\n",
        "int w = (union { int a; float b; }){.a = 2}.a + aaaaaaaaaaaaaaaaaaaa + bbbbbbbbbbbbbbbbbbbb;\n",
    ] {
        for width in 1..=120 {
            let once = jphfmt::format_with_width(src, width);
            assert!(
                !once.contains("}(") && !once.contains("} ("),
                "width {width}: a call on the literal: {once:?}"
            );
            assert_eq!(
                jphfmt::format_with_width(&once, width),
                once,
                "width {width}"
            );
        }
    }
}

/// #95: the type of an anonymous compound literal is a definition, not a list. Laid out as one, its
/// members gained §2.3's magic comma when the line broke — `{ int x;, }` — so the output did not compile
/// at *any* width, before or after the call-on-the-literal half of this was fixed.
///
/// Members are `;`-terminated, so there is no comma list to lay out and the body is written as the author
/// wrote it. Only the literal's own `{…}` is a container.
#[test]
fn an_anonymous_literal_type_is_a_definition_and_not_a_list() {
    // The commented spelling is the same definition, and a walk that stopped at the comment read it as a
    // list — `prev_nontrivia` skips whitespace, not comments.
    for (src, body) in [
        (
            "int v = (struct { int x; }){1}.x + aaaa;\n",
            "(struct { int x; }){",
        ),
        (
            "int w = (struct /* c */ { int x; }){1}.x + aaaa;\n",
            "(struct /* c */ { int x; }){",
        ),
        (
            "int u = (union { int a; float b; }){.a = 2}.a + aaaa;\n",
            "(union { int a; float b; }){",
        ),
    ] {
        for width in 1..=120 {
            let once = jphfmt::format_with_width(src, width);
            assert!(
                once.contains(body),
                "width {width}: the member list is the author's: {once:?}"
            );
            assert!(
                !once.contains(";,"),
                "width {width}: a magic comma in a member list: {once:?}"
            );
            assert_eq!(
                jphfmt::format_with_width(&once, width),
                once,
                "width {width}"
            );
        }
    }
    let src = "int v = (struct { int x; }){1}.x + aaaa;\n";
    // The literal's own body is still a container, and still explodes when it must.
    assert_eq!(
        jphfmt::format_with_width(src, 20),
        "int v = (struct { int x; }){\n\t1,\n}.x + aaaa;\n"
    );
}

/// #90: an empty macro argument is a hole the author wrote, and the comma is what spells it. Dropping
/// the empty element dropped its comma too, so the call lost an argument and the output did not compile
/// — `PICK_MID(x, , y)` became `PICK_MID(x, y)`, "macro requires 3 arguments, but only 2 given".
///
/// A comma count, not the exact bytes: whether a hole is written `, ,` or `,,` is a layout question this
/// pins nothing about, but the number of arguments is not a layout question at all. `significant`
/// excuses commas — §2.3's magic one is the layout's to write — so this is one of the assertions that
/// filter cannot make.
#[test]
fn an_empty_macro_argument_keeps_the_comma_that_spells_it() {
    for src in [
        "int p = PICK_MID(x, , y);\n",
        "int q = PICK_FIRST(, x, y);\n",
        "int r = PICK_LAST(x, y, );\n",
        "int s = F(, , );\n",
        "int t = G(aaaaaaaaaaaaaaaaaaaa, , bbbbbbbbbbbbbbbbbbbb, cccccccccccccccccccc, dddddddddddd);\n",
    ] {
        for width in 1..=120 {
            let once = jphfmt::format_with_width(src, width);
            assert_eq!(
                once.matches(',').count(),
                src.matches(',').count(),
                "width {width}: {src:?} -> {once:?}"
            );
            assert_eq!(
                jphfmt::format_with_width(&once, width),
                once,
                "width {width}"
            );
        }
    }
    // An empty argument list is the one empty element that is not a hole: there is nothing between the
    // parentheses to keep, and §2.5 writes them tight.
    assert_eq!(format("int u = f();\n"), "int u = f();\n");
    assert_eq!(format("int v = f( );\n"), "int v = f();\n");

    // How a hole is spaced is the author's, because the layout has no rule for it: `F(a,, b)` is what
    // #85's rule would write and no other C formatter does. So a list with a hole passes through, which
    // the comma count above cannot see — it is satisfied by any spacing.
    assert_eq!(
        format("int p = PICK_MID(x, , y);\n"),
        "int p = PICK_MID(x, , y);\n"
    );
    // Only the trailing trivia inside the parens goes, which is `render_segment` trimming its own edges.
    assert_eq!(
        format("int r = PICK_LAST(x, y, );\n"),
        "int r = PICK_LAST(x, y,);\n"
    );
    // A `{}` list holds a hole the same way, and the call-level passthrough cannot see one: the braces put
    // it a bracket deeper, so `split_on_commas` at the call level sees a non-empty argument. The review on
    // #96 found this, and `MACRO({a, , b}, c)` is the shape that reaches it from valid-ish C.
    for src in [
        "int x[] = {1, , 2};\n",
        "int f(void) {\n\treturn MACRO({a, , b}, c);\n}\n",
    ] {
        let once = format(src);
        assert_eq!(
            once.matches(',').count(),
            src.matches(',').count(),
            "{src:?} -> {once:?}"
        );
        assert_eq!(format(&once), once);
    }
    // The trailing empty element is §2.3's magic comma, not a hole — it still forces the break and is
    // still written. And `{,}` holds nothing apart, so it is still the empty list.
    assert_eq!(
        format("int a[] = {1, 2,};\n"),
        "int a[] = {\n\t1,\n\t2,\n};\n"
    );
    assert_eq!(format("int b[] = {,};\n"), "int b[] = {};\n");

    // Passing through costs the layout: an over-width list with a hole stays over-width (§6).
    let long = format!(
        "int t = G({a}, , {b}, {c}, dddddddddddd);\n",
        a = "a".repeat(20),
        b = "b".repeat(20),
        c = "c".repeat(20)
    );
    assert_eq!(format(&long), long);
}

/// A sole argument's span is the call's own parentheses, so it is already bounded. A sole `{}`
/// element is not: its list writes a trailing comma on the break, which is what made unbounded arms
/// read as elements in the first place.
#[test]
fn a_sole_call_argument_is_not_bounded_twice() {
    assert_eq!(
        format("h(a ? b : c ? d : e);\n"),
        "h(\n\ta ? b :\n\tc ? d :\n\te\n);\n"
    );
    assert_eq!(
        format("struct s w = {a ? b : c ? d : e};\n"),
        "struct s w = {\n\t(\n\t\ta ? b :\n\t\tc ? d :\n\t\te\n\t),\n};\n"
    );
}

/// #77: an index is the same container the author's other brackets hold, so it needs no bound of its
/// own — `[` and `]` are the bound — and its contents obey every rule a parenthesized span does.
#[test]
fn an_index_is_a_container_like_any_other_bracket() {
    assert_eq!(
        format("int j = arr[a ? b : c ? d : e];\n"),
        "int j = arr[\n\ta ? b :\n\tc ? d :\n\te\n];\n"
    );
    let chain = "int n = table[AAAAAAAAAAAAAAAAAAAAAAAA + BBBBBBBBBBBBBBBBBBBBBBBB + \
                 CCCCCCCCCCCCCCCCCCCCCCCC + DDDDDDDDDDDDDDDDDDDDDDDD];\n";
    let broken = "int n = table[\n\tAAAAAAAAAAAAAAAAAAAAAAAA +\n\tBBBBBBBBBBBBBBBBBBBBBBBB +\n\
                  \tCCCCCCCCCCCCCCCCCCCCCCCC +\n\tDDDDDDDDDDDDDDDDDDDDDDDD\n];\n";
    assert_eq!(format(chain), broken);
    // An operator inside brackets is spaced as one inside parentheses always was — `[…]` was the
    // only pair this did not reach.
    assert_eq!(format("int b = arr[i-1];\n"), "int b = arr[i - 1];\n");
}

/// An index with no operator to break at is text, exactly as it was: a subscript, a declarator's
/// bound, a designator, and an attribute all pass through untouched.
#[test]
fn an_index_with_nothing_to_break_is_left_alone() {
    for src in [
        "int d = arr[i];\n",
        "int e = arr[-1];\n",
        "int h = m[i][j];\n",
        "struct s {\n\tint arr[10];\n};\n",
        "static const int t[] = {\n\t[A] = 1,\n\t[B] = 2,\n};\n",
    ] {
        assert_eq!(format(src), src);
    }
}

#[test]
fn nested_ternary_condition_breaks_at_its_arms() {
    // The same span in the same parentheses as `x = (a ? b : c ? d : e)`, so it lays out the same.
    assert_eq!(
        format("if (a ? b : c ? d : e) {\n\tf();\n}\n"),
        "if (\n\ta ? b :\n\tc ? d :\n\te\n) {\n\tf();\n}\n"
    );
}

#[test]
fn long_ternary_chain_explodes_flat_with_trailing_colons() {
    let src = "return (status_code == 0 ? \"ok\" : status_code == 1 ? \"busy\" : status_code == 2 ? \"error\" : status_code < 0 ? \"fault\" : \"unknown\");\n";
    let expected = "return (\n\tstatus_code == 0 ? \"ok\" :\n\tstatus_code == 1 ? \"busy\" :\n\tstatus_code == 2 ? \"error\" :\n\tstatus_code < 0 ? \"fault\" :\n\t\"unknown\"\n);\n";
    assert_eq!(format(src), expected);
}

#[test]
fn declaration_with_brace_explodes_and_keeps_brace_attached() {
    let src = "static int do_something_with_a_long_name(int first_parameter, int second_parameter, int third_parameter) {\n";
    let expected = "static int do_something_with_a_long_name(\n\tint first_parameter,\n\tint second_parameter,\n\tint third_parameter\n) {\n";
    assert_eq!(format(src), expected);
}

#[test]
fn function_params_break_before_inner_call_in_body() {
    // §2.7 eager break: function bodies always break — newline after `{`, indented body,
    // newline before `}`. This ensures the inner call stays flat because the body is on its
    // own lines, so the inner call has plenty of room.
    let src = "int study_point_debug(Point const *const s, char *const b, size_t const n) { return Point_debug(s, b, n); }\n";
    let expected = "int study_point_debug(Point const * const s, char * const b, size_t const n) {\n\treturn Point_debug(s, b, n);\n}\n";
    assert_eq!(format(src), expected);
}

#[test]
fn constructs_inside_a_function_body_are_structured() {
    // §2.2 applies wherever the construct is: a body is walked, not passed through.
    let long_call = "void f(void) {\n\tresult = some_function_with_a_fairly_long_name(first_argument_value, second_argument_value, third_argument_value);\n}\n";
    let expected = "void f(void) {\n\tresult = some_function_with_a_fairly_long_name(\n\t\tfirst_argument_value,\n\t\tsecond_argument_value,\n\t\tthird_argument_value\n\t);\n}\n";
    assert_eq!(format(long_call), expected);

    // A nested block no longer stops the walk, and a leading-operator condition is re-laid out
    // with the operators trailing (§2.7).
    let nested = "void f(void) {\n\tif (x) {\n\t\tif (alpha_value > 100\n\t\t\t&& bravo_value > 200\n\t\t\t&& charlie_value > 300\n\t\t\t&& delta_value > 400\n\t\t\t&& echo_value > 500\n\t\t\t&& foxtrot_value > 600) {\n\t\t\tg();\n\t\t}\n\t}\n}\n";
    let once = format(nested);
    assert!(
        once.contains("alpha_value > 100 &&\n"),
        "condition must re-lay out with trailing operators: {once}"
    );
    assert_eq!(format(&once), once);
}

#[test]
fn a_literal_inside_a_body_canonicalizes_too() {
    // #16's own repro wraps its compound literal in a one-line function, so it needed the body to be
    // walked at all — the padding rule and this one only meet here.
    for src in [
        "int f(void) { return (struct s){ .x = 1 }; }\n",
        "int f(void) { return (struct s) { .x = 1 }; }\n",
        "int f(void) { return (struct s){.x = 1}; }\n",
    ] {
        assert_eq!(
            format(src),
            "int f(void) {\n\treturn (struct s){.x = 1};\n}\n",
            "input {src:?}"
        );
    }
}

#[test]
fn a_directive_last_in_a_body_leaves_no_blank_line() {
    // A directive brings its own line break; the one before `}` must not be added on top of it.
    let src = "void f(void) {\n\tg();\n#define M(a) call(a)\n}\n";
    assert_eq!(format(src), src);
}

#[test]
fn a_comment_on_the_brace_line_stays_there() {
    // §2.1: comments are never moved, so the forced break after `{` goes after the comment.
    let src = "int f(int n) { /* VLA-syntax parameter */\n\treturn n;\n}\n";
    assert_eq!(format(src), src);
    assert_eq!(
        format("void f(void) { /* nothing */ }\n"),
        "void f(void) { /* nothing */ }\n"
    );
    assert_eq!(format("void f(void) {\n\n}\n"), "void f(void) {}\n");
}

#[test]
fn preprocessor_scope_indents_between_hash_and_keyword() {
    let src = "#if a\n#define thing\n#else\n#if b\n#define thing\n#if c\n#define thing\n#endif\n#endif\n#endif\n";
    let expected = "#if a\n#\tdefine thing\n#else\n#\tif b\n#\t\tdefine thing\n#\t\tif c\n#\t\t\tdefine thing\n#\t\tendif\n#\tendif\n#endif\n";
    assert_eq!(format(src), expected);

    // Depth-2 nesting: body of an inner #if is one tab deeper than the inner #if's own line.
    let nested = "#if A\n#if B\n#define x\n#endif\n#endif\n";
    let expected_nested = "#if A\n#\tif B\n#\t\tdefine x\n#\tendif\n#endif\n";
    assert_eq!(format(nested), expected_nested);
}

#[test]
fn preprocessor_scope_is_idempotent() {
    let src = "#if a\n#define thing\n#else\n#if b\n#define thing\n#if c\n#define thing\n#endif\n#endif\n#endif\n";
    let once = format(src);
    assert_eq!(format(&once), once, "scope pass must be idempotent");
}

// A `#define` has no fixture shape: `tests/cases` mutates trivia, and any newline inserted into a
// directive ends it — leaving a malformed macro whose trailing code lands on a fits boundary that
// merged `main` is already unstable on. These conformance cases pin the behavior instead.
#[test]
fn define_params_explode_when_the_line_overruns() {
    // §2.2: a macro's parameter list is a container like a call's, so it breaks one per line and
    // the body starts after the `)`.
    let src = "#define __pldx_range(access_kind, retention_policy, length, \\\n                    metadata, addr) \\\n  __builtin_arm_range_prefetch(addr, access_kind, retention_policy, metadata)\n";
    let expected = "#define __pldx_range( \\\n\taccess_kind, \\\n\tretention_policy, \\\n\tlength, \\\n\tmetadata, \\\n\taddr \\\n) \\\n\t__builtin_arm_range_prefetch(addr, access_kind, retention_policy, metadata)\n";
    assert_eq!(format(src), expected);
    assert_eq!(format(expected), expected);
    // A list whose line fits keeps the body on the `#define` line.
    assert_eq!(
        format("#define M(a, b) f(a, b)\n"),
        "#define M(a, b) f(a, b)\n"
    );
}

#[test]
fn continued_define_params_do_not_accumulate_backslashes() {
    // A `\` left in the parameter list is not a continuation but an invalid token, and
    // `significant()` filters backslashes out, so nothing else in the suite would notice.
    let src = "#define M(a, \\\n\t\tb) f(a, b)\n";
    let once = format(src);
    assert_eq!(once, "#define M(a, b) f(a, b)\n");
    assert_eq!(format(&once), once);
    assert!(!once.contains("\\ \\"), "stray backslash: {once:?}");
}

#[test]
fn preprocessor_scope_preserves_define_continuation() {
    // A #define with a \-continuation body: the #define line is at depth 0 (unchanged), and
    // the continuation line (previous line ends in \) is skipped by the scope pass.
    let src = "#define M(a) ((a) + 1) \\\n\t+ 2\n";
    assert_eq!(format(src), src);
}

#[test]
fn a_braceless_control_body_is_a_statement_of_its_own() {
    // The `)` of the header ends the previous statement as much as a `;` does: the chain that
    // follows it is the loop's whole body, and nothing else would lay it out.
    let src = "for (;;) aaaaaaaaaaaaa | bbbbbbbbbbbbbbb | ccccccccccccccc | ddddddddddddddd;\n";
    let once = jphfmt::format_with_width(src, 40);
    assert!(
        once.contains("aaaaaaaaaaaaa |\n"),
        "the body chain must break: {once:?}"
    );
    assert_eq!(jphfmt::format_with_width(&once, 40), once);
    assert_eq!(significant(&once), significant(src));
}

#[test]
fn a_comma_operator_after_a_ternary_is_never_bounded() {
    // `x = (a ? b : c, d)` assigns `d` where `x = a ? b : c, d` assigns the ternary. The operands
    // are a list, so they are not an implicit container and parentheses would not be free.
    let src = "xxxxxxxxxxxxxxx = conditionaaaaaaaaaa ? valuebbbbbbbbbbbb : valuecccccccccccc, otherdddddddddd;\n";
    assert_eq!(format(src), src);
}

#[test]
fn a_depth_zero_colon_is_never_a_chain_to_split() {
    // `<` and `>` are the relational class, but `Type<T>::member` is not a comparison — the `:`
    // says the layout belongs to something else, whatever the operands look like.
    let src = "using decay_t = typename decay<T>::type;\n";
    assert_eq!(format(src), src);
}

#[test]
fn a_brace_less_initializer_macro_keeps_its_own_line() {
    // `PyVarObject_HEAD_INIT(a, b)` expands to initializers that must lead the list, so what
    // follows it is juxtaposed rather than comma-separated. Every CPython static type is written
    // this way, and joining the next designator onto it reads as a typo.
    let src = "static PyTypeObject T = {\n\tPyVarObject_HEAD_INIT(NULL, 0)\n\t.tp_name = \"x\",\n\t.tp_basicsize = 0,\n};\n";
    assert_eq!(format(src), src);
}

#[test]
fn a_designator_tight_against_the_paren_is_left_alone() {
    // `f().field = v` is a member assignment, token-for-token the shape a juxtaposed designator
    // has. Only the gap tells them apart, so no gap means no split (§6).
    assert_eq!(
        format("int x = {get_ptr().field = 1};\n"),
        "int x = {get_ptr().field = 1};\n"
    );
}

#[test]
fn an_operator_with_no_right_operand_is_not_a_chain_cut() {
    // Bounding a chain moves its operators inside the parentheses, so what is left at depth zero on
    // the next pass can be an operator that never had a right operand. Splitting there put the `=`
    // in a segment of its own and the two layouts alternated forever.
    let src = "={A/ =00aa.|*}AA=0] aa";
    let once = jphfmt::format_with_width(src, 1);
    assert_eq!(jphfmt::format_with_width(&once, 1), once);
}

#[test]
fn a_depth_zero_chain_is_not_cut_before_an_assignment() {
    // #43. Bounding an assignment's right-hand side puts the whole assignment back through
    // `split_chain` on the next pass, and an operator in the *left* side is not one of those
    // operands' separators: cutting `0/a = A & A` at the `/` moved the break, and the layout
    // alternated between the two spellings forever.
    //
    // Depth zero is the whole of it, and the name says so because the invariant does not hold one
    // bracket in: `s = (a | b) = c | d` moved its break between passes on every main before #122,
    // since nothing about a depth-zero rule reaches a `|` inside parentheses and `build_bracketed_group`
    // lays that group out on pass 2 with no idea it sits in a left side. That is #125 — #122's head
    // gate refuses the double assignment's head as the second-construct class, so the shape is now
    // laid out on the first pass and pinned by the test below, and `(a | b)` is no lvalue so no C
    // program reaches it. Not asserted here either way: pinning a two-step settling would pin a bug.
    for (src, width) in [
        ("A''={0/a=A&A}\"\"\" _#\ta0", 1),
        // The same shape without the unterminated literals that reduced it.
        ("x = {0 / a = A & A};\n", 12),
        // And with a compound assignment, whose left side is reached the same way. Each of the three
        // was checked to oscillate on its own without the fix, not merely as a group — a loop that
        // dies on its first input says nothing about the ones after it.
        ("x = {a / b = c ^ d};\n", 8),
    ] {
        let once = jphfmt::format_with_width(src, width);
        assert_eq!(
            jphfmt::format_with_width(&once, width),
            once,
            "must be idempotent: {src:?} at width {width}"
        );
        assert_eq!(significant(&once), significant(src));
    }
}

#[test]
fn a_chain_is_still_cut_on_an_assignments_right_side() {
    // The other half of #43's rule, and what a blanket refusal of assignment spans would cost:
    // everything after the last depth-zero `=` is still the chain's, so a long right-hand side still
    // breaks one operand per line.
    //
    // Asserted as a layout at a width, not only as a fixpoint: a flat over-wide line is idempotent
    // and preserves every significant token, so neither of those catches a lost break. Ablating the
    // restriction leaves this passing — it is the *other* direction — while refusing the span
    // outright fails it, which is the mistake it exists to catch.
    assert_eq!(
        jphfmt::format_with_width("x = aaaa | bbbb | cccc;\n", 12),
        "x = (\n\taaaa |\n\tbbbb |\n\tcccc\n);\n"
    );
}

/// A corpus pin, not a guard: sqlite writes `(j = i/2)` and the chain container's flat form spaces
/// the `/`, so a change that stopped claiming this span would show up here as churn across real
/// files. It passes on the merge base and under every ablation of #43's rule — including
/// `operand_span` stubbed to zero — because what it exercises is §2.5 spacing inside a parenthesized
/// group rather than where the cut goes. Labelled so it is not read as guarding the cut.
#[test]
fn a_parenthesized_assignment_keeps_its_operator_spaced() {
    assert_eq!(
        format("void f(void) {\n\twhile ((j = i/2) > 0) {\n\t\tx = 1;\n\t}\n}\n"),
        "void f(void) {\n\twhile ((j = i / 2) > 0) {\n\t\tx = 1;\n\t}\n}\n"
    );
}

/// Format `src` at `width` once and assert the three things every head pin asserts: the exact
/// output, the fixpoint, and every line within the width. The one recipe — a case that forgets the
/// width bound would otherwise regress silently.
fn assert_laid_out(src: &str, width: usize, expected: &str) {
    let once = jphfmt::format_with_width(src, width);
    assert_eq!(once, expected, "{src:?}");
    assert_eq!(
        jphfmt::format_with_width(&once, width),
        once,
        "and it is a fixpoint"
    );
    for line in once.lines() {
        assert!(display_width(line) <= width, "over the limit: {line:?}");
    }
}

#[test]
fn a_parenthesized_chain_in_a_double_assignment_head_is_cut_on_the_first_pass() {
    // #125: a chain inside parentheses in an assignment's *left* side — the head of the second
    // `=` — was cut only on the second pass: pass 1 kept `(a | b)` flat, pass 2 broke it, and
    // only pass 3 was stable. #108's head gate refuses the double assignment's head as the
    // second-construct class — pass 1's parens would read back as construct two on pass 2 — so
    // the walk lays the parenthesized chain out on the first pass and every pass agrees. The
    // issue's own widths and the deeper variants, pinned.
    for (src, width, expected) in [
        (
            "s = (a | b) = c | d;\n",
            8,
            "s = (\n\ta |\n\tb\n) = (\n\tc |\n\td\n);\n",
        ),
        (
            // The issue's width 4, same layout as width 8. Exact-output and fixpoint only: its
            // first line `s = (` is five columns of unbreakable prefix.
            "s = (a | b) = c | d;\n",
            4,
            "s = (\n\ta |\n\tb\n) = (\n\tc |\n\td\n);\n",
        ),
        (
            "s = (a | b) = c | d;\n",
            12,
            "s = (\n\ta |\n\tb\n) = c | d;\n",
        ),
    ] {
        let once = jphfmt::format_with_width(src, width);
        assert_eq!(once, expected, "{src:?} at {width}");
        assert_eq!(
            jphfmt::format_with_width(&once, width),
            once,
            "must be a fixpoint: {src:?} at {width}"
        );
    }
    // The one-bracket-deeper member: the walk lays the head's subscript out but the RHS chain
    // stays flat — its `)] = c | d;` tail is 11 columns at width 8, a breakable line the
    // paren-head member cuts at the same width. Recorded rather than pinned wide: the refusal's
    // tail is the same §6 passthrough the head-gate pins record, and a fix that cuts it will
    // turn this exact pin red, which is the point of pinning it exactly.
    let subscript_head = "s = x[(a | b)] = c | d;\n";
    let subscript_layout = "s = x[(\n\ta |\n\tb\n)] = c | d;\n";
    let once = jphfmt::format_with_width(subscript_head, 8);
    assert_eq!(once, subscript_layout, "the subscript head at width 8");
    assert_eq!(
        jphfmt::format_with_width(&once, 8),
        once,
        "and it is a fixpoint"
    );
    // The class the issue named, one bracket deeper and with longer operands: each variant is
    // pinned at one width its own layout satisfies, and all six are asserted stable at every
    // width 1-32 on the merged gate.
    for (src, width, expected) in [
        (
            "s = ((a | b)) = c | d;\n",
            12,
            "s = ((\n\ta |\n\tb\n)) = c | d;\n",
        ),
        (
            "s = (aaaa | bbbb) = cc | dd;\n",
            12,
            "s = (\n\taaaa |\n\tbbbb\n) = cc | dd;\n",
        ),
        (
            "s = (a ? b : c) = d | e;\n",
            12,
            "s = (\n\ta ? b :\n\tc\n) = d | e;\n",
        ),
        (
            "s = (a | b) = c | d = e | f;\n",
            16,
            "s = (\n\ta |\n\tb\n) = c | d = (\n\te |\n\tf\n);\n",
        ),
        (
            "s = (f(x)) = c | d;\n",
            8,
            "s = (f(\n\tx\n)) = (\n\tc |\n\td\n);\n",
        ),
        (
            "s = (a | b) + e = c | d;\n",
            12,
            "s = (\n\ta |\n\tb\n) + e = (\n\tc |\n\td\n);\n",
        ),
    ] {
        assert_laid_out(src, width, expected);
        for width in 1..=32 {
            let once = jphfmt::format_with_width(src, width);
            assert_eq!(
                jphfmt::format_with_width(&once, width),
                once,
                "{src:?} at {width}"
            );
        }
    }
}

#[test]
fn a_hash_fragment_in_a_bracket_group_measures_the_same_both_passes() {
    // #131: a `#` fragment inside a bracket group was measured one way on pass 1 and another on
    // pass 2 — the group broke on the first pass and rejoined on the second. The guard that
    // closed it is the bracketed-group handler's any-`#` refusal outside a define body, merged in
    // #135, whose own pins live in `a_construct_does_not_measure_across_a_directive`. The issue's
    // own shape and width, pinned — the walk keeps the author's form.
    assert_laid_out(
        "[# _0<a\"A&_&.aA]a&A&AA\t#&]0&\"]A\t",
        35,
        "[# _0<a\"A&_&.aA]a&A&AA\t#&]0&\"]A\n",
    );
    // The class members one level deeper: the refusal is a passthrough, so each width asserts the
    // guard's own claim — the author's form, unchanged — not only the fixpoint the pre-fix tree
    // could also settle on.
    for src in ["x = [#a & b] + c | d;\n", "x = [f(#a) | b] = c | d;\n"] {
        for width in 1..=32 {
            let once = jphfmt::format_with_width(src, width);
            assert_eq!(once, src, "the author's form passes through at {width}");
        }
    }
}

#[test]
fn a_subscript_in_a_chain_head_is_laid_out_on_the_first_pass() {
    // #127: the subscript in a chain's head — `.[0:?]` — rendered as flat text on pass 1 and
    // was laid out only on pass 2, once the broken operands had made the statement a different
    // shape. The head gate *passes* this head — the `?` marks only the subscript's own frame,
    // and the boundary covers exactly that one construct — so `build_chain_doc`'s boundary path
    // lays the head out through `build_expr_doc` on the first pass, and pass 2 re-reads the
    // written operand parens through the walk's group arms: the same two-path taxonomy the
    // measured-head pin's comment names. The pin's value over that admitted-path class is the
    // issue's exact shape as contract.
    //
    // The width bound is meaningless at width 1 — `.[` is two columns of unbreakable content —
    // so this test pins the layout and the fixpoint, not the width.
    let src = ".[0:?]=A&\"\ta&&a0aa0A0A\"_A;";
    let expected = ".[\n\t0 :\n\t?\n] = (\n\tA &\n\t\"\ta&&a0aa0A0A\"_A\n);\n";
    let once = jphfmt::format_with_width(src, 1);
    assert_eq!(
        once, expected,
        "the head's subscript breaks on the first pass"
    );
    assert_eq!(
        jphfmt::format_with_width(&once, 1),
        once,
        "and it is a fixpoint"
    );
    // The wide counterpart: a head that fits is still written flat.
    assert_eq!(
        jphfmt::format_with_width(src, 100),
        ".[0 : ?] = A & \"\ta&&a0aa0A0A\"_A;\n"
    );
}

#[test]
fn a_chain_head_is_measured_like_its_operands() {
    // #108. The head held whatever precedes the operands — an assignment's left side — and was
    // rendered flat, so no width reached the call or subscript inside it. That overruns §8.5's
    // limit outright: at width 40 the head alone is 50 columns.
    //
    // The gate passes this head — the `,` list leaves the call unmarked, so the exemption admits
    // the call in the head's outermost subscript — and `build_chain_doc` lays the head out through
    // the boundary mechanism. The fallback path — a refused head the statement walker's subscript
    // arm lays out — is pinned by the chain-marked heads in
    // [`a_chain_head_does_not_alternate_with_the_wrapped_operands`] (`arr[a | f(y)] =`).
    //
    // Asserted three ways, because the layout alone is not enough: a pass-1 layout that no second
    // pass reproduces is what #108 *is*, so an exact-output test that never formats its own output
    // is green on the very defect it names. The review of this change found exactly that.
    for (src, width, expected) in [
        (
            "void f(void) {\n\tarr[index_of(first_argument, second_argument)] = alpha | beta;\n}\n",
            40,
            "void f(void) {\n\tarr[index_of(\n\t\tfirst_argument,\n\t\tsecond_argument\n\t)] = alpha | beta;\n}\n",
        ),
        // The head fits on its own line and stays flat; the operands take their own parens. Sharing
        // one fit made the head break whenever the operands did, and the next pass — measuring the
        // head alone, its trailing reserve stopping at the operands' bracket — disagreed (#108's
        // review). A `Doc::Boundary` after the head is what keeps the two verdicts the same.
        (
            "void f(void) {\n\tarr[a + b] = a | b;\n}\n",
            18,
            "void f(void) {\n\tarr[a + b] = (\n\t\ta |\n\t\tb\n\t);\n}\n",
        ),
    ] {
        assert_laid_out(src, width, expected);
        // Measured is not the same as broken: a head that fits is still written flat.
        assert_eq!(jphfmt::format_with_width(src, 100), src);
    }
}

#[test]
fn a_chain_head_does_not_alternate_with_the_wrapped_operands() {
    // The draft of #108 recorded 211 shapes that still alternated: pass 1 breaks the head on the
    // operands' flat width, pass 2 measures the head alone — its trailing reserve stops at the
    // operands' bracket — and joins it back. The boundary after the head stops the head's own
    // groups' measurement there, the same verdict the next pass reaches either way.
    for (src, width) in [
        (
            "void f(void) {\n\tarr[a + b] = aaaa | bbbb | cccc;\n}\n",
            18,
        ),
        (
            "void f(void) {\n\tarr[a + b] = aaaa | bbbb | cccc;\n}\n",
            20,
        ),
        (
            "void f(void) {\n\tarr[a + b] = aaaa | bbbb | cccc;\n}\n",
            24,
        ),
        ("void f(void) {\n\tarr[a + b] = a ? b : c ? d : e;\n}\n", 18),
        (
            "void f(void) {\n\tarr[a + b] = a ? b : c ? d : e;\n}\n",
            100,
        ),
    ] {
        let once = jphfmt::format_with_width(src, width);
        assert_eq!(
            jphfmt::format_with_width(&once, width),
            once,
            "must be a fixpoint: {src:?} at width {width}"
        );
        for line in once.lines() {
            assert!(display_width(line) <= width, "over the limit: {line:?}");
        }
    }
    // A head with juxtaposed brackets — `arr[index_of(a, b)][c]` — is measured one handler at a
    // time on the next pass, each against a trailing reserve that stops at the second bracket,
    // while this pass's single lookahead crosses it. The two disagree, so the chain is refused and
    // the author's form passes through (§6): the same span must read the same either way.
    let juxtaposed = "void f(void) {\n\tarr[index_of(a, b)][c] = a | b | c | d;\n}\n";
    assert_eq!(jphfmt::format_with_width(juxtaposed, 24), juxtaposed);
    assert_eq!(jphfmt::format_with_width(juxtaposed, 100), juxtaposed);
    // The refusal is a passthrough: at a width the verbatim form satisfies, every line stays
    // within it — the width rule is guarded even for the shapes the gate refuses.
    for (src, width) in [
        (juxtaposed, 100),
        ("void f(void) {\n\tarr[a + b] = f(x) = c | d;\n}\n", 40),
        ("void f(void) {\n\tarr[f(a | b)] = x | y;\n}\n", 30),
        ("void f(void) {\n\tarr[f(g(x))] = a | b;\n}\n", 30),
    ] {
        let once = jphfmt::format_with_width(src, width);
        for line in once.lines() {
            assert!(display_width(line) <= width, "{src:?} at {width}: {line:?}");
        }
    }
    // The review's residual shapes, now the same refusal: a second construct after the first
    // (a double assignment's head) and a breakable construct a bracket deep (a chain argument
    // inside the head's call). Each passes through and stays there — the two passes measured
    // different budgets before.
    let double_assign = "void f(void) {\n\tarr[a + b] = f(x) = c | d;\n}\n";
    let deep_break = "void f(void) {\n\tarr[f(a | b)] = x | y;\n}\n";
    for (src, width) in [(double_assign, 19), (double_assign, 20), (deep_break, 14)] {
        let once = jphfmt::format_with_width(src, width);
        assert_eq!(
            jphfmt::format_with_width(&once, width),
            once,
            "{src:?} at {width}"
        );
    }
    // A nested call at depth two is the same class the gate is now structural for: pass 1's
    // lookahead flattened through `g(x)`'s own fits where pass 2's handlers reserve stops at its
    // bracket, and the two laid `g(x)` two ways. The chain is refused; the structure's own
    // handlers lay the head out one construct at a time, the same path on every pass.
    let nested_call = "void f(void) {\n\tarr[f(g(x))] = a | b;\n}\n";
    for width in 12..=20 {
        let once = jphfmt::format_with_width(nested_call, width);
        assert_eq!(
            jphfmt::format_with_width(&once, width),
            once,
            "the nested-call head at {width}"
        );
    }
    // The gate's other half: unbreakable nested content — a cast, parens around an atom, a call
    // through a parenthesized callee — measures the same on both passes and is allowed, so the
    // operands still wrap and every line stays within the width.
    for (src, expected, width) in [
        (
            "void f(void) {\n\tarr[(size_t)i] = a | b;\n}\n",
            "void f(void) {\n\tarr[(size_t)i] = (\n\t\ta |\n\t\tb\n\t);\n}\n",
            24,
        ),
        (
            "void f(void) {\n\t(*fp)(x) = aaaa | bbbb;\n}\n",
            "void f(void) {\n\t(*fp)(x) = (\n\t\taaaa |\n\t\tbbbb\n\t);\n}\n",
            24,
        ),
        (
            "void f(void) {\n\ta[0][1] = aaaa | bbbb;\n}\n",
            "void f(void) {\n\ta[0][1] = (\n\t\taaaa |\n\t\tbbbb\n\t);\n}\n",
            24,
        ),
        (
            "void f(void) {\n\ta[0].b[1] = aaaa | bbbb;\n}\n",
            "void f(void) {\n\ta[0].b[1] = (\n\t\taaaa |\n\t\tbbbb\n\t);\n}\n",
            24,
        ),
        (
            "void f(void) {\n\tarr[i](j) = a | b;\n}\n",
            "void f(void) {\n\tarr[i](j) = a | b;\n}\n",
            24,
        ),
        (
            "void f(void) {\n\tarr[(a)][0] = x | y;\n}\n",
            "void f(void) {\n\tarr[(a)][0] = x | y;\n}\n",
            24,
        ),
        (
            "void f(void) {\n\tarr[x[a, b]] = c | d;\n}\n",
            "void f(void) {\n\tarr[x[a, b]] = (\n\t\tc |\n\t\td\n\t);\n}\n",
            24,
        ),
        (
            "void f(void) {\n\t(a) + b = c | d;\n}\n",
            "void f(void) {\n\t(a) + b = (\n\t\tc |\n\t\td\n\t);\n}\n",
            16,
        ),
        (
            "void f(void) {\n\tx[0] + b = c | d;\n}\n",
            "void f(void) {\n\tx[0] + b = (\n\t\tc |\n\t\td\n\t);\n}\n",
            16,
        ),
        (
            "void f(void) {\n\t(f(x)) + b = c | d;\n}\n",
            "void f(void) {\n\t(f(x)) + b = (\n\t\tc |\n\t\td\n\t);\n}\n",
            19,
        ),
        (
            "void f(void) {\n\tx[a + b] + c = d | e;\n}\n",
            "void f(void) {\n\tx[\n\t\ta +\n\t\tb\n\t] + c = (\n\t\td |\n\t\te\n\t);\n}\n",
            14,
        ),
        (
            "void f(void) {\n\tx[f(y)] + z = c | d;\n}\n",
            "void f(void) {\n\tx[f(\n\t\ty\n\t)] + z = (\n\t\tc |\n\t\td\n\t);\n}\n",
            16,
        ),
    ] {
        assert_laid_out(src, width, expected);
    }
    // The round-12 classes. A call in the author's group with a second construct after it —
    // `(f(x))(y)`, `(f(x))[y]` — and a ternary after the exempted call's close are the
    // second-construct class the gate refuses: pass 1's operand parens would read back as
    // construct two on pass 2. The exemption re-arms the breakability it exempts, so each is
    // refused again and the next pass keeps whatever the refusal laid out — pinned at the widths
    // in the band that used to alternate or overrun.
    let juxtaposed_call = "void f(void) {\n\t(f(x))(y) = c | d;\n}\n";
    let juxtaposed_subscript = "void f(void) {\n\t(f(x))[y] = c | d;\n}\n";
    let ternary_after_exempt = "void f(void) {\n\t(f(x)) ? a : b = c | d;\n}\n";
    for (src, width) in [
        (juxtaposed_call, 12),
        (juxtaposed_call, 16),
        (juxtaposed_subscript, 12),
        (juxtaposed_subscript, 16),
        (ternary_after_exempt, 16),
    ] {
        let once = jphfmt::format_with_width(src, width);
        assert_eq!(
            jphfmt::format_with_width(&once, width),
            once,
            "{src:?} at {width}"
        );
    }
    // The same shapes at a width the refusal's own layout satisfies, with every line within it.
    assert_laid_out(juxtaposed_call, 22, juxtaposed_call);
    assert_laid_out(juxtaposed_subscript, 22, juxtaposed_subscript);
    assert_laid_out(
        ternary_after_exempt,
        23,
        "void f(void) {\n\t(f(\n\t\tx\n\t)) ? a : b = c | d;\n}\n",
    );
    // The depth-2 residual: `f(` after a `(` reads as a group, not a call, so the exemption
    // cannot reach it and the chain stays refused — stable, with the compliant band pinned.
    assert_laid_out(
        "void f(void) {\n\t((f(x))) + b = c | d;\n}\n",
        20,
        "void f(void) {\n\t((f(\n\t\tx\n\t))) + b = c | d;\n}\n",
    );
    // A chain operator in the enclosing bracket — before the call or after it — is the deep
    // class: pass 1's lookahead crosses the call's bracket where pass 2's reserve stops, so the
    // chain refuses and the walk lays the head's subscript out on every pass. The w=14 pin sits
    // beside the band that alternated — w=13 for this member, w=13-15 for the longer operands
    // (pass 1 wrote `f(y)` flat, pass 2 broke it): the exact broken call asserts the stable
    // side, and the width bound guards the refused layout. The w=26 companion is the verbatim
    // form at a width it satisfies.
    assert_laid_out(
        "void f(void) {\n\tarr[a | f(y)] = x | y;\n}\n",
        14,
        "void f(void) {\n\tarr[\n\t\ta |\n\t\tf(\n\t\t\ty\n\t\t)\n\t] = x | y;\n}\n",
    );
    assert_laid_out(
        "void f(void) {\n\tarr[a | f(y)] = x | y;\n}\n",
        26,
        "void f(void) {\n\tarr[a | f(y)] = x | y;\n}\n",
    );
    // The mark-after order exercises the arm the mark-before pins cannot: the call closes before
    // the `|` marks the enclosing bracket, and the frame's own close refuses
    // (`chain_marked && has_call`). w=16 sits in the band that alternated on the round-12
    // binary (w=15-17); the laid form is the one every pass reaches.
    assert_laid_out(
        "void f(void) {\n\tarr[f(y) | a] = aa | bb;\n}\n",
        16,
        "void f(void) {\n\tarr[\n\t\tf(\n\t\t\ty\n\t\t) |\n\t\ta\n\t] = aa | bb;\n}\n",
    );
    // The same mark in a group: refused, compliant from the width its broken group satisfies.
    // At w=24-26 the whole statement renders flat at 27 columns — a stable §6 passthrough the
    // gate's refusal keeps — recorded here as the cost the suite states rather than hides.
    assert_laid_out(
        "void f(void) {\n\t(a | f(x)) + b = c | d;\n}\n",
        22,
        "void f(void) {\n\t(\n\t\ta |\n\t\tf(x)\n\t) + b = c | d;\n}\n",
    );
    // The ternary refusal's trade, stated: at w=16 the pinned output's tail is 23 columns — the
    // refusal widened the admitted form's 18-column overrun for the `?`-arm's categorical rule —
    // and w=23 above is the first width the refused form satisfies.
    //
    // A `?` in the enclosing bracket is the same deep class in both orders: the review's probe
    // found the short-operand member stable, but the width sweep shows `x[a ? f(y) : b] =` and
    // `x[f(y) ? a : b] =` alternate at w=19-21 once the operands grow — the walk's ternary arms
    // and the head's own groups measure the call against different budgets. The categorical
    // refusal stays; this pin is the refused layout, compliant from w=14, and the flat form
    // overruns w=25-27 at 28 columns (recorded, per the round-9 convention).
    assert_laid_out(
        "void f(void) {\n\tx[a ? f(y) : b] = c | d;\n}\n",
        14,
        "void f(void) {\n\tx[\n\t\ta ? f(\n\t\t\ty\n\t\t) :\n\t\tb\n\t] = c | d;\n}\n",
    );
    // The justified refusals' §8.5 cost across the chain-marked family, per the round-9
    // convention: each member pins a width its passthrough satisfies. The bands the flat tail
    // overruns, measured on the built binary: `aa | bb | cc` at 21 columns through w=20 and the
    // flat form at 33 from w=24; the ternary operand at 18 through w=17; the `&&` continuation
    // at 19 through w=18 and the flat form at 31 at w=28-30. The `==` head with the long
    // operands has no compliant width below 40 — the broken form overruns through w=26 at 27
    // columns and the flat form from w=27 at 40 — so its pin is the verbatim passthrough at its
    // natural width, the shape the walk keeps when the gate refuses.
    for (src, width, expected) in [
        (
            "void f(void) {\n\tarr[f(y) | a] = aa | bb | cc;\n}\n",
            22,
            "void f(void) {\n\tarr[\n\t\tf(y) |\n\t\ta\n\t] = aa | bb | cc;\n}\n",
        ),
        (
            "void f(void) {\n\tarr[a | f(y)] = a ? b : c;\n}\n",
            18,
            "void f(void) {\n\tarr[\n\t\ta |\n\t\tf(\n\t\t\ty\n\t\t)\n\t] = a ? b : c;\n}\n",
        ),
        (
            "void f(void) {\n\tarr[f(y) == a] = aaaa | bbbb | cccc;\n}\n",
            40,
            "void f(void) {\n\tarr[f(y) == a] = aaaa | bbbb | cccc;\n}\n",
        ),
        (
            "void f(void) {\n\tarr[f(y) | a] && b = x | y;\n}\n",
            19,
            "void f(void) {\n\tarr[\n\t\tf(\n\t\t\ty\n\t\t) |\n\t\ta\n\t] && b = x | y;\n}\n",
        ),
    ] {
        assert_laid_out(src, width, expected);
    }
}

#[test]
fn a_call_in_a_chain_head_is_laid_out_on_the_first_pass() {
    // #108's other half. A head rendered flat is laid out anyway on the *next* pass — the operands
    // below it have broken by then, so the span reaches a handler that does measure it — and the
    // two passes disagreed about the same tokens. The layout is pinned, not only the fixpoint: a
    // flat head that both passes converge on is green on the very defect this test names.
    for (src, expected) in [
        ("0(A<0)=0:??;", "0(\n\tA <\n\t0\n) = (\n\t0 :\n\t??\n);\n"),
        (
            "a\tA(*)\'\'00 .=a<A;",
            "a A(\n\t*\n)''00 . = (\n\ta <\n\tA\n);\n",
        ),
    ] {
        let once = jphfmt::format_with_width(src, 1);
        assert_eq!(once, expected, "the head's call breaks on the first pass");
        assert_eq!(
            jphfmt::format_with_width(&once, 1),
            once,
            "and it is a fixpoint"
        );
    }
    // The width bound is meaningless at width 1 — `0(` is two columns of unbreakable content —
    // so this test pins the layout and the fixpoint, not the width.
}

#[test]
fn a_floating_exponent_keeps_its_sign() {
    // The sign is part of the number (C11 §6.4.8), not an operator to space. Splitting it produced
    // `1e - 5`, which does not compile — and musl's math sources are full of `0x1p-1022`.
    let src = "double a = 1e-5;\ndouble y = 0x1p-1022 * 0x1p53;\n";
    assert_eq!(format(src), src);
}

#[test]
fn a_brace_list_is_not_joined_where_a_later_pass_would_respace_it() {
    // #28: joining these onto one line hands `space_bit_fields` an `Ident : Number` and
    // `space_semicolons` a space before a `;` — both of which they rewrite, so the layout's own
    // output would be a fixpoint of a different pass. Neither is valid C in a `{}` list.
    // The indented forms too: a trivia run is a `Newline` and then the indentation, so a guard
    // that reads only the token beside the punctuator misses every element that is laid out.
    for src in [
        "x = {A\n:0};\n",
        "x = {A\n;};\n",
        "x = {A\n\t:0};\n",
        "x = {A\n\t;};\n",
    ] {
        let once = format(src);
        assert_eq!(format(&once), once, "must be idempotent: {src:?}");
        assert_eq!(significant(&once), significant(src));
    }
}

#[test]
fn a_chain_head_is_not_joined_where_a_later_pass_would_respace_it() {
    // #121: the chain head renders collapsed text, so joining a break onto a `:` hands
    // `space_bit_fields` a same-line `Ident : Number` to reinterpret — this pass's output would be
    // a fixpoint of a different pass. The `{}` list already refuses for the same reason; the head
    // path lacked the refusal. Neither shape is valid C, so refusing the layout costs no real
    // code (§6); a label whose statement opens with a number reads the same way at a span start and
    // is spared by the ternary layout, pinned by
    // a_label_whose_statement_opens_with_a_number_keeps_its_label.
    for src in ["_\n:0=0&A;\n", "int y = a\n:0 = b & c;\n"] {
        let once = format(src);
        assert_eq!(format(&once), once, "must be idempotent: {src:?}");
        assert_eq!(significant(&once), significant(src));
    }
}

#[test]
fn a_group_in_a_chain_operand_keeps_the_break_a_later_pass_would_respace() {
    // #121's class, one bracket in: a refused group inside a chain operand used to collapse its
    // newline — `(A\n:0)` to `(A :0)` — and the spacing pass respaces the joined pair. The group
    // keeps the break instead, one element per line.
    for src in ["x = y + (A\n:0);\n", "x = y + [A\n:0];\n"] {
        let once = format(src);
        assert_eq!(format(&once), once, "must be idempotent: {src:?}");
        assert_eq!(significant(&once), significant(src));
    }
}

#[test]
fn a_bracket_and_a_brace_join_by_tokens_not_the_authors_gap() {
    // The juxtaposed-bracket join the group doc writes tight. A refused bracket group falls back
    // to the token walk, and a gap kept there re-reads as the author's own on the next pass and
    // joins then — two passes for one line, keyed on whitespace where the doc keys on tokens
    // (#108's fresh draw found `[ {}x&x\n;]` alternating).
    for src in [
        "[ {}x&x\n;]",
        "[ {}x & x;]",
        "[{}x & x;]",
        "arr[ {1, 2}] = x;",
    ] {
        let once = format(src);
        assert_eq!(format(&once), once, "must be idempotent: {src:?}");
        assert!(!once.contains("[ {"), "the join is tight: {src:?}");
    }
}

#[test]
fn a_call_a_pass_broke_re_reads_with_its_forced_break() {
    // #108's fresh draw: `({[x<<case({[],})]`. The case's `{[],}` carries the magic trailing
    // comma, which forces the call broken on every pass. The middle-newline verbatim re-read
    // dropped that force — its text form has no ForceBreak — so the enclosing bracket group
    // measured a doc without it, joined what the previous pass broke, and the two passed
    // alternated. A forced break has no fits decision to flip: the re-laid form is the one every
    // pass reaches, so the passthrough yields to it.
    for (src, width) in [("({[x<<case({[],})]", 1), ("({[x<<case({[],})]", 100)] {
        let once = jphfmt::format_with_width(src, width);
        assert_eq!(
            jphfmt::format_with_width(&once, width),
            once,
            "must be idempotent: {src:?} at {width}"
        );
    }
    // A `#` or `##` fragment in the arguments keeps the verbatim either way — its lines are not
    // the layout's to own — and the same draw that surfaced the case shape also surfaced the
    // re-lay overreaching into one: the hash guard is the difference between the two.
    for (src, width) in [
        ("0\"\"''\"\"a(\"\"{##}{.,}=0+_)", 1),
        ("0\"\"''\"\"a(\"\"{##}{.,}=0+_)", 100),
    ] {
        let once = jphfmt::format_with_width(src, width);
        assert_eq!(
            jphfmt::format_with_width(&once, width),
            once,
            "must be idempotent: {src:?} at {width}"
        );
    }
}

#[test]
fn a_wrap_after_a_broken_chain_head_nests_where_the_next_pass_reads_it() {
    // #108's fresh draw: `x&return""x+f;` at width 1. The chain's head broke, but the operand
    // wrap's indent stayed at the head's own level, while the next pass — reading the written
    // parentheses as the operand's own group — nested one deeper. The wrap now follows the
    // head's outcome: a broken head indents the wrap with its own lines, the two passes agree.
    for width in [1, 3] {
        let once = jphfmt::format_with_width("x&return\"\"x+f;", width);
        assert_eq!(
            jphfmt::format_with_width(&once, width),
            once,
            "must be idempotent at {width}"
        );
    }
}

#[test]
fn the_chain_and_ternary_segments_join_call_head_and_subscript_breaks() {
    // Every segment renders through `build_expr_doc`, whose call and group arms join these to the
    // tight form the spacing pass canonicalizes, so the segment gate takes the canonical reading.
    let call = format_with_width("x = f\n(x) + y;\n", 12);
    assert_eq!(format_with_width(&call, 12), call, "must be idempotent");
    assert!(call.contains("f(x) +"), "the call joins: {call:?}");
    assert_eq!(format("x = a\n[0] + y;\n"), "x = a[0] + y;\n");
}

#[test]
fn the_clause_and_group_segments_keep_a_declarator_star_break_too() {
    // `build_clause_contents` takes the gate the other segment consumers have: a declarator-star
    // break in a condition or a bracketed group's segment is the caller's fallback to keep.
    for src in ["if (int *\n2 && b) { g(); }\n", "x = (int *\n2 && b);\n"] {
        let once = format(src);
        assert_eq!(format(&once), once, "must be idempotent: {src:?}");
        assert_eq!(significant(&once), significant(src));
    }
    // A value-predecessor star is provably a multiply and joins for every follower the spacing
    // pass leaves alone, and a newline-separated call head goes to the call handler, which writes
    // the tight `f(` the spacing pass canonicalizes.
    assert_eq!(
        format("for (i = p *\n~q; i; i++) { g(); }\n"),
        "for (i = p * ~q; i; i++) { g(); }\n"
    );
    assert_eq!(format("x = {a *\n(y)};\n"), "x = {a * (y)};\n");
    assert_eq!(format("f\n(long + chain + x);\n"), "f(long + chain + x);\n");
}

#[test]
fn a_chain_or_ternary_segment_keeps_a_declarator_star_break_too() {
    // The segment gate is the head's: a segment whose collapse would join a respaced pair is
    // refused — the top reading, so a nested construct inside the segment keeps its own breaks.
    for src in [
        "x = int *\n2 + y;\n",
        "x = y ? int *\n2 : 3;\n",
        "x = {struct s *\n2};\n",
        "x = {int **\n2};\n",
        "for (i = int a, *\n2; i; i++) { g(); }\n",
    ] {
        let once = format(src);
        assert_eq!(format(&once), once, "must be idempotent: {src:?}");
        assert_eq!(significant(&once), significant(src));
    }
}

#[test]
fn a_pad_spaces_only_the_edge_beside_an_equals() {
    // `space_equals` writes a space on both sides of a same-line `=`, so the `=`-adjacent edge
    // flattens spaced and the other edge keeps §2.5's tight form.
    assert_eq!(format("f(a, b =);\n"), "f(a, b = );\n");
    assert_eq!(format("x = {1, y = };\n"), "x = {1, y = };\n");
    assert_eq!(format("f(=, a);\n"), "f( =, a);\n");
}

#[test]
fn a_star_break_joins_only_where_no_declarator_verdict_could_fire() {
    // `space_pointers` tightens a declarator star's gap to a number, so `int *\n2` is refused —
    // including at a span start, where a comma-list declarator's head is outside the span — while
    // a star preceded by a value is provably a multiply and joins to `x * 2`.
    for src in [
        "x = {int *\n2};\n",
        "x = {int a, *\n2};\n",
        "if (int *\n2) { g(); }\n",
    ] {
        let once = format(src);
        assert_eq!(format(&once), once, "must be idempotent: {src:?}");
        assert_eq!(significant(&once), significant(src));
    }
    assert_eq!(
        format("for (i = x *\n2; i; i++) { g(); }\n"),
        "for (i = x * 2; i; i++) { g(); }\n"
    );
    // The colon arm keeps the break-adjacency gate: a label whose joined form is canonical joins.
    assert_eq!(
        format("x = ({ lbl: 3\n+ 4; });\n"),
        "x = ({\n\tlbl: 3 + 4;\n});\n"
    );
    // The declined pad shape itself: the first pass's space and the break both survive the
    // call's verbatim passthrough.
    {
        let src = "x = y + f(=\n\"\");\n";
        let once = format(src);
        assert_eq!(format(&once), once, "must be idempotent: {src:?}");
        assert!(once.contains("( ="), "the pad: {src:?} -> {once:?}");
    }
}

#[test]
fn the_element_fallbacks_join_call_subscript_and_star_breaks_canonically() {
    // The element fallback's refusal covers only the joins its collapse writes wrong — a bit-field
    // colon, an ambiguous declarator star, a `;` — while the group and call arms already join the
    // canonical tight form, so these lay out rather than freeze.
    assert_eq!(
        format("for (i = f\n(x); i; i++) { g(); }\n"),
        "for (i = f(x); i; i++) { g(); }\n"
    );
    assert_eq!(format("if (a\n[0]) { g(); }\n"), "if (a[0]) { g(); }\n");
    assert_eq!(format("int a[] = {f\n(x)};\n"), "int a[] = {f(x)};\n");
    assert_eq!(format("x = ({ f\n(1); });\n"), "x = ({\n\tf(1);\n});\n");
    assert_eq!(
        format("for (i = x *\n2; i; i++) { g(); }\n"),
        "for (i = x * 2; i; i++) { g(); }\n"
    );
    // A nested `{}` list's break is its own element's to refuse, so the outer container lays out.
    assert_eq!(format("x = { {a\n:0} };\n"), "x = {{a\n:0}};\n");
}

#[test]
fn a_bare_bit_field_colon_break_passes_the_element_fallbacks_through_verbatim() {
    // The builders' terminal fallback collapses any unclaimed span, and a bare `Ident : Number`
    // break collapsed to `A :0` is what `space_bit_fields` tightens — the one path without a
    // refusal until the element builder took one. The author's text, newline included, is the
    // fixpoint; the edges are trimmed so a container's own separator and the previous pass's
    // indentation are not doubled.
    for src in [
        "for (i = A\n:0; i; i++) { g(); }\n",
        "if (A\n:0) { g(); }\n",
        "while (A\n:0) { g(); }\n",
        "switch (A\n:0) { g(); }\n",
        "x = ({ A\n:0; });\n",
        "x = ({ lbl\n: 1 ? a : b; });\n",
    ] {
        let once = format(src);
        assert_eq!(format(&once), once, "must be idempotent: {src:?}");
        assert_eq!(significant(&once), significant(src));
    }
}

#[test]
fn a_nested_bit_field_colon_break_is_the_nested_constructs_to_keep() {
    // `space_bit_fields` reads at any depth, so each nested construct keeps its own colon break:
    // the chain head's all-depth refusal, the call's has-middle-newline contract (the structure
    // pass's own, for calls that reach here nested), and the `{}` list's depth-zero refusal.
    for src in [
        "int (A\n:0) = x + y;\n",
        "x = f(a\n:0) + y;\n",
        "x = (f({A\n:0}) + y);\n",
        "int v = { {A\n:0} };\n",
    ] {
        let once = format(src);
        assert_eq!(format(&once), once, "must be idempotent: {src:?}");
        assert_eq!(significant(&once), significant(src));
    }
    // A refused group's collapse preserves the spacing pass's own pre-written pad — `space_equals`
    // spaces every same-line `=` first, so the layout never writes one against a bracket — and its
    // nested subscript stays tight.
    assert_eq!(format("x = y + (=\n);\n"), "x = y + ( = );\n");
    assert_eq!(format("x = y + (a [0]);\n"), "x = y + (a[0]);\n");
    // A call whose `{` follows is spaced by `space_braces`, and the callee is its own part, so the
    // flush must carry the gap across the parts boundary rather than drop it on an empty text
    // buffer — the pinned search's `''[_/a(){"\""}]` at width 1, one flush over.
    for src in ["x[_/a(){\"x\"}]\n", "''[_/a(){\"\"}]\n"] {
        let once = format(src);
        assert_eq!(format(&once), once, "must be idempotent: {src:?}");
        assert!(
            once.contains(") {"),
            "the body's space: {src:?} -> {once:?}"
        );
    }
}

#[test]
fn a_declarator_stars_break_joins_by_the_spacing_passes_verdict() {
    // `space_pointers` writes `* p` before an identifier, so joining that break agrees with it and
    // the head lays out — a real declarator the round-3 over-broad refusal used to freeze verbatim.
    assert_eq!(format("int (*\ncb) = x + y;\n"), "int (* cb) = x + y;\n");
    // The head renders as one text, joining every break nested included, so a subscript break it
    // would join into a respaced pair still refuses the whole statement — the head has no nested
    // arm to keep it, and verbatim is the only spelling that stays a fixpoint.
    {
        let src = "int (f)\n[2] = x + y;\n";
        let once = format(src);
        assert_eq!(format(&once), once, "must be idempotent: {src:?}");
        assert_eq!(significant(&once), significant(src));
    }
    // A break below the span's own depth is the nested group's own to refuse, so the enclosing
    // container now lays it out rather than freezing: the join writes the tight `b[0]` the spacing
    // pass canonicalizes.
    assert_eq!(format("x = y + (a + b\n[0]);\n"), "x = y + (a + b[0]);\n");
}

#[test]
fn a_label_whose_statement_opens_with_a_number_keeps_its_label() {
    // `lbl: 1 ? a : b` reads as the bit-field shape even at a span start, where the colon is the
    // label's; laying the arms out would write ` : ` where the spacing pass writes `: `. The whole
    // statement passes through instead — the §6 cost — keeping the label and its colon.
    {
        let src = "lbl: 1 ? a : b;\n";
        let once = format(src);
        assert_eq!(format(&once), once, "must be idempotent: {src:?}");
        assert_eq!(significant(&once), significant(src));
    }
    // And a narrow width passes it through whole rather than writing the respaced shape.
    let src = "lbl: 11111111111111111111 ? 22222222222222222222 : 33333333333333333333;\n";
    assert_eq!(jphfmt::format_with_width(src, 30), src);
}

#[test]
fn a_leading_equals_keeps_the_spacing_passes_space() {
    // `space_equals` puts a space before every same-line `=`, pad or no pad, so the layout
    // dropping an element's leading gap wrote `a(= "")`, which the spacing pass respaced to
    // `a( = "")` — this pass's output as a fixpoint of a different pass. Found by the
    // random-input spacing-fixpoint search (#121's property) as `a(="" )` at width 7.
    for src in [
        "a(=\"\");\n",
        "x = {= \"\"};\n",
        "''[()?:=]\n",
        "x = {*A:0?};\n",
    ] {
        let once = format(src);
        assert_eq!(format(&once), once, "must be idempotent: {src:?}");
        assert_eq!(significant(&once), significant(src));
    }
}

#[test]
fn a_ternary_arm_in_a_brace_list_still_lays_out() {
    // The bit-field guard is a `?` earlier in the statement, so a ternary arm ending in a number
    // is not the shape that refuses.
    let src = "x = {a ? b : 0, c};\n";
    assert_eq!(format(src), src);
}

/// A `)` that closes a *call's* argument list closes no type, so the `{` after it is not a compound
/// literal's brace. `#if !defined(X)` ends in `)`, and `defined` is an excluded callee — which the
/// hand-written guard here did not reject, only a plain callee — so the block on the next line was laid
/// out as a `{}` initializer list: statements joined onto one line, and §2.3's trailing comma written
/// into statement position when that line overflows. The output does not compile (#109).
///
/// `can_precede_cast` is the question a cast already asks of the same position, and #64 was the cost of
/// two spellings of it drifting apart. This is the third.
#[test]
fn a_block_after_a_directive_is_not_a_compound_literal() {
    let src = "static int f(int nBuf, int pid) {\n#if !defined(X)\n\t{\n\t\tlong tttttttttttttttt;\n\t\tnBuf = sizeof(tttttttttttttttt) + sizeof(pid) + nBuf + pid + nBuf + pid + nBuf;\n\t}\n#endif\n\treturn nBuf;\n}\n";
    assert_eq!(format(src), src, "the block is a block");

    // The short form was stable and compiled; it only joined the statements onto one line.
    let short =
        "void f(void) {\n\tint pid;\n#if X\n\t{\n\t\tint fd;\n\t\tfd = 1;\n\t}\n#endif\n}\n";
    assert_eq!(format(short), short);

    // What the predicate exists to accept still passes: a literal after a control header, and after
    // every token a cast may follow.
    for src in [
        "int g(int x) {\n\tif (x)\n\t\treturn (struct s){1, 2}.a;\n\treturn 0;\n}\n",
        "int * p = (int[]){1, 2};\n",
    ] {
        let once = format(src);
        assert_eq!(format(&once), once, "{src:?}");
    }
}

/// A `\`-continued literal is one token under either line ending. The `String` rule's escape was
/// `\\.`, and the regex crate's `.` matches `\r` but not `\n`, so `"a\` + CRLF was one token and
/// `"a\` + LF was a stray `"` followed by loose identifiers. Formatting normalizes the endings
/// (§2.1), so the second pass lexed what the first pass wrote *differently* — and every spacing rule
/// was then free to disagree with itself across passes, inside the literal's own text (#110).
///
/// Found by the corpus check on a CRLF file; the LF half of the same defect is why glibc's
/// `pthread.h` has a `("\`-continued deprecation message that no rule here treated as a string.
#[test]
fn a_continued_literal_is_one_token_under_either_line_ending() {
    let crlf = "#error \"a\\\r\n b=0.\"\r\n";
    let lf = "#error \"a\\\n b=0.\"\n";
    // The tokenization cannot depend on the line ending, so neither can the output.
    assert_eq!(format(crlf), format(lf));
    assert_eq!(format(crlf), lf, "the message keeps its own spacing");
    assert_eq!(format(&format(crlf)), format(crlf));

    // The `("\` shape glibc writes, with the message's text untouched.
    let attribute = "int f(void) __attribute_deprecated_msg__(\"\\\nf is deprecated, use g\");\n";
    assert_eq!(format(attribute), attribute);
    // A character literal takes the same escape.
    assert_eq!(format("char c = '\\\n';\n"), "char c = '\\\n';\n");

    // The consequence, not just the lexing. An unterminated literal desynchronizes every string
    // boundary after it, so a construct further down is measured from tokens the source never wrote —
    // here a `#define` body whose continuation lines were joined, leaving the `\` mid-line. A stray `\`
    // in a macro *body* only reaches the compiler when the macro is used, which `sqlite3.c` does and
    // this reduction does not: the snippet alone compiles either way, and the errors #114 reports are
    // from the invocation. In `sqlite3.c` the literal and the macro are 60,000 lines apart, which is why
    // no reduction of the *macro* ever reproduced it.
    //
    // Also `tests/cases/continued-literal-desync`, which holds the same bytes to their exact output and
    // runs them through the whitespace-mutant harness.
    let desync = "const char *s = \"a\\\n b\";\n#define D(P) \\\n   if( ((P)->flags&E)!=0 \\\n       && f(P) ){ goto no_mem;}\n";
    let laid_out = "const char * s = \"a\\\n b\";\n#define D(P) \\\n   if ( ((P)->flags&E)!=0 \\\n\t   && f(P) ) { goto no_mem;}\n";
    assert_eq!(format(desync), laid_out);
    assert_eq!(format(laid_out), laid_out);
}

/// The lines a `#if` spans are the preprocessor's alternatives, not one expression. A construct that
/// measures across one lays the directive out as its own content and writes the `#` mid-line, which
/// does not compile (#112) — `if (a == 1 #if defined(X) || b == 2 #endif) {`. `contains_comment` is
/// the same refusal for the same reason, and directives had no counterpart to it.
///
/// Four handlers reach the shape. The control header and the bracketed group inside it are what
/// `sqlite3.c` needs — removing either takes it from 0 `gcc` errors back to 45. The other two were
/// found by the review, and the multi-line shapes that made them look unreachable are why: a directive
/// that is an argument's *whole* trimmed body has no newline left in that body, so
/// `has_middle_newline` does not decline it and `foo(` + `#` + `)` came out as `foo(#)`. In a statement
/// expression the statements are joined one per line, and a directive joined onto the statement after
/// it swallows that statement — `#pragma pack(1)` + `int t = 1;` compiled to `t` undeclared.
///
/// A directive begins its line; the stringize `#` of `foo(#x, y)` does not, and that call still lays
/// out. Refusing on any `#` cost real layout: `#define STR(x) f(#x, …)` exploded its *parameter list*
/// instead of its argument list — which is why #118 splits the verdict on context instead: any `#`
/// refuses outside a `#define` body, and inside one the name list decides ([`DIRECTIVE_FORMS`] is
/// pinned there). A block comment before it does not stop it being a directive — phase 3
/// makes the comment whitespace and phase 4 then reads the `#`.
/// Every form [`names_directive`] knows — the plain names, a line marker's number (GNU's
/// preprocessor-output form, not C11 §6.10.4's `#line`), the null directive, a name phase 2 splices
/// back together, and a name phase 3 does not — a comment ends one where a continuation does not.
const DIRECTIVE_FORMS: &[&str] = &[
    "#if defined(X)",
    "#ifdef X",
    "#ifndef X",
    "#elif defined(X)",
    "#elifdef X",
    "#elifndef X",
    "#else",
    "#endif",
    "#define Q 1",
    "#undef Q",
    "#include <f.h>",
    "#embed \"f.bin\"",
    "#line 42",
    "#error nope",
    "#warning hi",
    "#pragma pack(1)",
    "#include_next <f.h>",
    "#import \"f.h\"",
    "#ident \"v\"",
    "#sccs \"x\"",
    "#assert x(y)",
    "#unassert x",
    "#system_header",
    "#using <f>",
    "#region x",
    "#endregion",
    "#push_macro(\"X\")",
    "#pop_macro(\"X\")",
    "#42 \"gen.c\"",
    "#",
    "#\\\ndefine Q 1",
    "# /* c */ define Q 1",
    "#in\\\nclude <f.h>",
    "#in/*c*/clude <f.h>",
    "#if/*c*/defined(X)",
];

#[test]
fn a_construct_does_not_measure_across_a_directive() {
    let header = "static int f(int a, int b) {\n\tif (a == 111111111\n#if defined(__APPLE__)\n\t\t|| b == 22222222\n#endif\n\t) {\n\t\treturn 1;\n\t}\n\treturn 0;\n}\n";
    assert_eq!(format(header), header, "the header passes through");

    let group =
        "int x = (aaaaaaaaaaaaaaaaaaaaa\n#if defined(X)\n\t| bbbbbbbbbbbbbbbbbbbbb\n#endif\n);\n";
    assert_eq!(format(group), group, "so does the group");

    // Only these three pin the call handler's guard — verified by ablating it. A directive that is the
    // argument's *whole* trimmed body leaves no newline in that body, so `has_middle_newline` does not
    // decline and `foo(` + `#` + `)` came out as `foo(#)`.
    for src in [
        "void g(void) {\n\tfoo(\n#\n\t);\n}\n",
        "void g(void) {\n\tfoo(x,\n#pragma pack(1)\n\t\t);\n}\n",
        "void g(void) {\n\tfoo(\n#error nope\n\t);\n}\n",
    ] {
        assert_eq!(format(src), src, "{src:?}");
    }

    // These four are already declined by `has_middle_newline` and pass with the call guard removed, so
    // they assert passthrough rather than the guard. Kept because that is the coverage that makes the
    // three above the *only* reachable shapes — but the comment used to claim they pinned the guard, and
    // ablation says otherwise.
    for src in [
        "void g(void) {\n\tfoo(x,\n#if X\n\t\ty\n#endif\n\t);\n}\n",
        "void g(void) {\n\tfoo(x, y\n#if X\n#endif\n\t);\n}\n",
        "void g(void) {\n\tfoo(\n#if X\n\t\tx\n#endif\n\t);\n}\n",
        "void g(void) {\n\tfoo(x\n#if X\n\t\t, y\n#endif\n\t);\n}\n",
    ] {
        assert_eq!(format(src), src, "{src:?}");
    }

    // A statement expression's statements go one per line, so a directive joined onto the next one
    // swallows it: `#pragma pack(1) int t = 1;` left `t` undeclared where the input compiled.
    let block = "void g(void) {\n\tint y = ({\n#pragma pack(1)\n\t\tint t = 1;\n\t\tt;\n\t});\n}\n";
    assert_eq!(format(block), block);

    // A `{}` list too — `emit_brace`'s blanket `#`, not this PR's guard.
    let list = "int t[] = {\n#if X\n\t1,\n#endif\n\t2,\n};\n";
    assert_eq!(format(list), list);

    // A comment before the `#` does not make it something else. This one is declined by the enclosing
    // `contains_comment` guards before `holds_directive`'s comment-skip is consulted, so it pins the
    // *behaviour* and not the skip — ablating `build_bracketed_group`'s guard leaves it green. The plain
    // group above is what pins that guard.
    let commented = "int x = (a\n/* c */ #if X\n\t| b\n#endif\n);\n";
    assert_eq!(format(commented), commented);

    // #118: outside a `#define` body the list is not consulted at all — a stringize cannot occur
    // there, so *any* `#` refuses. These pass on the catch-all now; the loop below, inside a
    // `#define` body, is what pins the list itself.
    for directive in DIRECTIVE_FORMS {
        let src = format!(
            "static int f(int a, int b) {{\n\tif (a == 111111111\n{directive}\n\t\t|| b == 22222222\n\t) {{\n\t\treturn 1;\n\t}}\n\treturn 0;\n}}\n"
        );
        assert_eq!(format(&src), src, "{directive:?}");
    }

    // Spaced, which `DirectiveLine::emit` tightens, so byte-identity is the wrong assertion — what matters is
    // that the `#` keeps its own line. Whitespace between the `#` and the name ends the name, so these
    // also pin that `# region x` names `region` and not `regionx`.
    for directive in ["# region x", "# endregion", "# using <f>", "# 42 \"gen.c\""] {
        let src = format!(
            "static int f(int a, int b) {{\n\tif (a == 111111111\n{directive}\n\t\t|| b == 22222222\n\t) {{\n\t\treturn 1;\n\t}}\n\treturn 0;\n}}\n"
        );
        let once = format(&src);
        assert!(
            !once
                .lines()
                .any(|line| line.contains("if (") && line.contains('#')),
            "{directive:?} keeps its own line: {once:?}"
        );
        assert_eq!(format(&once), once, "{directive:?}");
    }

    // A stringize inside a nonzero `#if` scope, where the narrow guards' width-flip exposure would show
    // if it showed anywhere: `scope_directives` re-indents a line-start `#`+keyword, so a `#x` the layout
    // moved there could be measured at one width and rewritten to another. **It does not reach that
    // re-indentation** — the scope pass skips continuation lines, and the `#x` is on one — so this
    // asserts stability of the shape rather than exercising the mechanism. Closing the mechanism needs a
    // `#x` at a line start that is *not* a continuation, which no valid input this pass sees produces.
    let scoped = "#if X\n#define STR(x) fooooooooooooooo(#x, barrrrrrrrrrrrrrr, bazzzzzzzzzzzzzzz, quxxxxxxxxxxxxxxxxxxxxxxxxxxxx)\n#endif\n";
    let broken = format(scoped);
    assert!(broken.contains("#x"), "the stringize survives: {broken:?}");
    assert_eq!(format(&broken), broken, "and the result is a fixpoint");

    // Which line a `#` is on is what the layout decides, so the test for a directive may not read it.
    // Breaking this argument list puts the stringize `#x` at the start of a line, and a position-based
    // test then called it a directive and refused on pass 2 what it laid out on pass 1.
    let moved = "#define STR(x) fooooooooooooooo(#x, barrrrrrrrrrrrrrr, bazzzzzzzzzzzzzzz, quxxxxxxxxxxxxxxxxxxxxxxxxxxxx)\n";
    let broken = format(moved);
    assert!(
        broken.contains("\n\t#x, \\\n"),
        "the `#` lands at a line start: {broken:?}"
    );
    assert_eq!(format(&broken), broken, "and is still not a directive");

    // The stringize `#` is not a directive, and `##` is not even a `Punct`. Already-broken input, so the
    // assertion is byte-identity: `starts_with` alone left the `#x` and the fixpoint unchecked.
    let stringize = "#define STR(x) fooooooooooooooo(\n\t#x, \\\n\tbarrrrrrrrrrrrrrr, \\\n\tbazzzzzzzzzzzzzzz, \\\n\tquxxxxxxxxxxxxxxxxxxxxxxxxxxxx \\\n)\n";
    assert_eq!(
        format(stringize),
        stringize,
        "the argument list stays broken, and the stringize with it"
    );
    assert_eq!(
        format("#define CAT(a, b) a##b\n"),
        "#define CAT(a, b) a##b\n"
    );

    // The same forms, now where the list is actually consulted: inside a `#define` body a `#` may
    // be a stringize, so `holds_unsafe_hash` falls back to the name list there. Each form alone in
    // the body's argument span, and the body overflows, so a call the list let through would
    // explode the directive onto a `\t`-indented line of its own — the refusal passes the body
    // through instead, whose width-cut lands mid-token and never before a `#`. Ablating any single
    // name must fail here.
    for directive in DIRECTIVE_FORMS {
        let src = format!(
            "#define M(x) {f}(x, {b}, {directive})\n",
            f = "f".repeat(120),
            b = "b".repeat(40),
        );
        let once = format(&src);
        assert!(
            !once
                .lines()
                .skip(1)
                .any(|l| l.trim_start().starts_with('#')),
            "{directive:?} keeps its place in the passthrough: {once:?}"
        );
        assert_eq!(format(&once), once, "{directive:?}");
    }

    // A name the list does not know, in the same body: the stringize-side error, which is the
    // reason the list survives inside defines at all. The argument list explodes and the `#` lands
    // on a continuation line, where the splice keeps it from reading as a directive on the next
    // pass — §6 prefers the false positive, but a false negative must stay a fixpoint.
    let unknown = format!(
        "#define M(x) {f}(x, {b}, #pragma_mark x)\n",
        f = "f".repeat(120),
        b = "b".repeat(40),
    );
    let once = format(&unknown);
    assert!(once.contains("\n\t#pragma_mark x \\\n"), "{once:?}");
    assert_eq!(format(&once), once, "{once:?}");
}

#[test]
fn an_unknown_directive_name_is_refused_outside_a_define() {
    // #118: the name list is open-ended — three rounds found twelve missing names — so outside a
    // `#define` replacement list, where a stringize cannot occur, any `#` refuses. Each of the
    // four handlers that flatten a span gets one: a control header, a call's arguments, a
    // bracketed group, and a statement expression.
    for name in ["#link foo", "#suppress xyz", "#pragma_mark x"] {
        let src = format!(
            "static int f(int a, int b) {{\n\tif (a == 111111111\n{name}\n\t\t|| b == 22222222\n\t) {{\n\t\treturn 1;\n\t}}\n\treturn 0;\n}}\n"
        );
        assert_eq!(format(&src), src, "control: {name:?}");

        let src = format!("void g(void) {{\n\tfoo(x,\n{name}\n\t\t);\n}}\n");
        assert_eq!(format(&src), src, "call: {name:?}");

        let src =
            format!("int x = (aaaaaaaaaaaaaaaaaaaaa\n{name}\n\t| bbbbbbbbbbbbbbbbbbbbb\n);\n");
        assert_eq!(format(&src), src, "group: {name:?}");

        let src =
            format!("void g(void) {{\n\tint y = ({{\n{name}\n\t\tint t = 1;\n\t\tt;\n\t}});\n}}\n");
        assert_eq!(format(&src), src, "stmt-expr: {name:?}");
    }
}

#[test]
fn a_brace_element_chain_wears_its_parens_when_a_segment_refusal_reads_across_the_break() {
    // #148: pass 1's chain broke between segments (no refusal, parens on the break) where pass 0's
    // chain held the break *inside* its star segment — read span-local, the star looked
    // declarator-possible and the chain was refused, falling back to a bound-less form without
    // parens. The refusal now re-reads the segment with the operand before it in view; the `&`
    // proves a multiply, the refusal clears, and the first pass writes the paren-bound broken chain
    // every pass keeps. The first line overruns width 1 — the formatter's own best-effort output —
    // so this pins the exact form and the fixpoint, not the width bound.
    let once = jphfmt::format_with_width("=''\"\"\"\"{A&*\n.}", 1);
    assert_eq!(once, " = ''\"\"\"\"{\n\t(\n\t\tA &\n\t\t* .\n\t),\n}\n");
    assert_eq!(
        jphfmt::format_with_width(&once, 1),
        once,
        "and it is a fixpoint"
    );
}

#[test]
fn a_brace_reserve_measures_a_call_head_the_walk_will_attach() {
    // #146: a newline between a callee and its `(` stopped `trailing_reserved` early on the first
    // pass, so the brace measured a short tail and stayed inline though the line overran; the walk
    // attached `a(` and the next pass measured the attached form, whose extra column flipped the
    // brace's fits verdict — pass 1 inline, pass 2 exploded, pass 3 stable. The reserve now
    // measures the attached form the walk will write, so the first pass decides what every pass
    // keeps. The issue's seed and its minimized member, each pinned at width 22. The full seed's
    // stable last line overruns 22 — the formatter's own best-effort output — so its hand-inlined
    // assertions below check the exact form and the fixpoint, not the width bound.
    assert_laid_out(
        ": ?=, ,)A{*=}?,::?:\ta\n()",
        22,
        ": ? =, ,)A{\n\t*=,\n}?,::?: a()\n",
    );
    let once = jphfmt::format_with_width(": ?=, ,)A{*=}?,::?:\ta\n(aa() /)A;)}=\\\"\\\")aa", 22);
    assert_eq!(
        once,
        ": ? =, ,)A{\n\t*=,\n}?,::?: a(aa() /)A;)} = \\\"\\\")aa\n"
    );
    assert_eq!(
        jphfmt::format_with_width(&once, 22),
        once,
        "and it is a fixpoint"
    );
}

#[test]
fn a_segment_reread_starts_at_the_cut_and_stays_inside_the_window() {
    // The re-read's window must end at the segment's true end — the off-by-one dropped the follower
    // a refusal keys on and cleared refusals vacuously — and it must start at the segment's own cut
    // operator, which sits at depth zero: a window starting at the previous segment's closer went
    // bracket-negative and disabled every depth-gated refusal. A type-context star inside the
    // segment keeps the refusal standing (`int * .` respaces to `int *.`), and a call-ended previous
    // segment changes nothing.
    for src in ["x = a | int *\n.;\n", "x = f(a) | int *\n[0];\n"] {
        let once = format(src);
        assert_eq!(format(&once), once, "must be idempotent: {src:?}");
        assert_eq!(
            significant(&once),
            significant(src),
            "no tokens lost: {src:?}"
        );
    }
}

#[test]
fn a_ternary_arm_rereads_its_star_from_the_cut() {
    // #153: a fresh 1M draw found the ternary's arm refusal firing only on the broken form — the
    // arm `*\n0 "" ?` read its star span-initial, the ternary was refused, and the generic fallback
    // joined the break it meant to protect, writing a tight `"":` the next pass's ternary path
    // re-laid with ` : `. The arm's refusal now re-reads from its own `:` cut, which proves the
    // star belongs to a ternary arm, not a declarator; the first pass writes the ternary form
    // every pass keeps.
    let once = format("=\"\"\"\"{\"\":*\n0\"\"?}");
    assert_eq!(once, " = \"\"\"\"{\"\" : * 0\"\"?}\n");
    assert_eq!(format(&once), once, "and it is a fixpoint");
}

#[test]
fn a_chain_head_recontextualizes_the_first_segments_star() {
    // #153's draw, the chain form: the first segment `*\n0` read its star span-initial and refused
    // the chain, so the generic fallback collapsed the break and wrote a form the next pass's chain
    // path re-laid with its own separators. The first segment's refusal now re-reads with the
    // head's last token in view — the `=` after `?` proves the operand the star is — and the first
    // pass writes the chain form every pass keeps.
    let once = format("=\"\"{?=*\n0&\"\"}");
    assert_eq!(once, " = \"\"{? = * 0 & \"\"}\n");
    assert_eq!(format(&once), once, "and it is a fixpoint");
}

#[test]
fn a_brace_reserve_counts_the_chain_separator_the_layout_writes() {
    // #153's draw, the reserve form: pass 1's tail was the author's tight `0a&a` (the reserve
    // measured 9 and the brace fit exactly), the layout respaced it to `0a & a`, and pass 2's
    // reserve measured 10 — one column past the boundary, flipping the brace's fits verdict. The
    // reserve now counts the chain separator the layout will write — one space — the author's own
    // gap notwithstanding. The stable form's string interior overruns the width, so this pins the
    // exact form and the fixpoint, not the width bound.
    let once = format(
        "=.{_/aAA|a 'A.a.0A_Aa00a0\ta\"\"A(A(\"'/\"a_\t&\t0a.A0#&a&\n00\nA0aA0&__&\na\taa\"}A_A=0a&a;",
    );
    assert_eq!(
        once,
        " = .{\n\t_ / aAA | a 'A.a.0A_Aa00a0\ta\"\"A(A(\"' / \"a_\t&\t0a.A0#&a&\n00\nA0aA0&__&\na\taa\",\n}A_A = 0a & a;\n"
    );
    assert_eq!(format(&once), once, "and it is a fixpoint");
}
