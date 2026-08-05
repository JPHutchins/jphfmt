//! Conformance suite. What must hold is idempotency, verbatim passthrough of call-free input, and the §2.2
//! layout for calls.

mod support;

use jphfmt::doc::display_width;
use jphfmt::format;
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

/// #77's fourth item — a `#define` body that is entirely one group laid out as a container — is **not**
/// implemented, and this pins that it is not, so the day it is, this test fails and says where to look.
///
/// Claiming such a body is a two-line change and produced five distinct regressions over three review
/// rounds on #103, each on a body the walk then measured as something it is not: a dropped `\`
/// continuation; a nested `({ … })` collapsed to a brace list, both adjacent and spaced; a two-cycle at
/// width 40 on a nested parenthesized ternary, because the two passes measure the body at different
/// columns (#43/#108's defect, not the claim's); and a `\`-continued string literal gaining a ` \` inside
/// its own text on every pass, which changes what the macro expands to.
///
/// A body that is one bracket is not one construct — it is whatever the macro's use makes of it, and
/// §6 prefers passthrough. The `#104` fix that shipped with these findings is narrow and separate: a
/// statement-expression body is laid out only when its `)` is the body's last token.
#[test]
fn a_define_body_that_is_one_group_passes_through() {
    for src in [
        "#define M(x) ((x) ? aaaaaaaaaaaaaaaaaaaaaa : bbbbbbbbbbbbbbbbbbbbbb ? cccccccccccccccccccc : dddddddddddddddddddd)\n",
        "#define N(x) ((x) + aaaaaaaaaaaaaaaaaaaaaa + bbbbbbbbbbbbbbbbbbbbbb + cccccccccccccccccccc + dddddddddddddddddddd)\n",
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

    // The nested-ternary two-cycle showed at width 40, so the width it showed at is the one asserted.
    let ternary = "#define value(x) ((123 ? 0xff : (a)) ? (t))\n";
    assert_eq!(jphfmt::format_with_width(ternary, 40), ternary);
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
/// instead of its argument list. A block comment before it does not stop it being a directive — phase 3
/// makes the comment whitespace and phase 4 then reads the `#`.
#[test]
fn a_construct_does_not_measure_across_a_directive() {
    let header = "static int f(int a, int b) {\n\tif (a == 111111111\n#if defined(__APPLE__)\n\t\t|| b == 22222222\n#endif\n\t) {\n\t\treturn 1;\n\t}\n\treturn 0;\n}\n";
    assert_eq!(format(header), header, "the header passes through");

    let group =
        "int x = (aaaaaaaaaaaaaaaaaaaaa\n#if defined(X)\n\t| bbbbbbbbbbbbbbbbbbbbb\n#endif\n);\n";
    assert_eq!(format(group), group, "so does the group");

    // A directive anywhere in a call's arguments, wherever the newlines fall around it. The last three
    // are the ones a multi-line body hides: the directive is the argument's whole trimmed body, so
    // nothing is left in it for `has_middle_newline` to find.
    for src in [
        "void g(void) {\n\tfoo(x,\n#if X\n\t\ty\n#endif\n\t);\n}\n",
        "void g(void) {\n\tfoo(x, y\n#if X\n#endif\n\t);\n}\n",
        "void g(void) {\n\tfoo(\n#if X\n\t\tx\n#endif\n\t);\n}\n",
        "void g(void) {\n\tfoo(x\n#if X\n\t\t, y\n#endif\n\t);\n}\n",
        "void g(void) {\n\tfoo(\n#\n\t);\n}\n",
        "void g(void) {\n\tfoo(x,\n#pragma pack(1)\n\t\t);\n}\n",
        "void g(void) {\n\tfoo(\n#error nope\n\t);\n}\n",
    ] {
        assert_eq!(format(src), src, "{src:?}");
    }

    // A statement expression's statements go one per line, so a directive joined onto the next one
    // swallows it: `#pragma pack(1) int t = 1;` left `t` undeclared where the input compiled.
    let block = "void g(void) {\n\tint y = ({\n#pragma pack(1)\n\t\tint t = 1;\n\t\tt;\n\t});\n}\n";
    assert_eq!(format(block), block);

    // A `{}` list too, and a comment before the `#` does not make it something else.
    let list = "int t[] = {\n#if X\n\t1,\n#endif\n\t2,\n};\n";
    assert_eq!(format(list), list);
    let commented = "int x = (a\n/* c */ #if X\n\t| b\n#endif\n);\n";
    assert_eq!(format(commented), commented);

    // The stringize `#` is not a directive, and `##` is not even a `Punct`.
    let stringize = "#define STR(x) fooooooooooooooo(\n\t#x, \\\n\tbarrrrrrrrrrrrrrr, \\\n\tbazzzzzzzzzzzzzzz, \\\n\tquxxxxxxxxxxxxxxxxxxxxxxxxxxxx \\\n)\n";
    assert!(
        format(stringize).starts_with("#define STR(x) fooooooooooooooo("),
        "the argument list breaks, not the parameter list: {:?}",
        format(stringize)
    );
    assert_eq!(
        format("#define CAT(a, b) a##b\n"),
        "#define CAT(a, b) a##b\n"
    );
}
