//! Conformance suite. What must hold is idempotency, verbatim passthrough of call-free input, and the §2.2
//! layout for calls.

use jphfmt::format;

const GOLDEN: &str = include_str!("golden.c");

#[test]
fn golden_is_a_fixpoint() {
    assert_eq!(format(GOLDEN), GOLDEN, "golden must be idempotent");
}

/// Significant content: everything but whitespace, commas (jphfmt may add a magic trailing comma),
/// backslashes (continuations), and parentheses — a chain or ternary that breaks is bounded by
/// parentheses jphfmt writes, which is legal exactly because the operands were already an implicit
/// container. Formatting must never alter anything else.
fn significant(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, ',' | '\\' | '(' | ')'))
        .collect()
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

#[test]
fn unparenthesized_ternary_is_left_alone() {
    // §8.2: jphfmt does not insert parens; a bare ternary passes through
    assert_eq!(
        format("acc = a > b ? a : a < b ? b : 0;\n"),
        "acc = a > b ? a : a < b ? b : 0;\n"
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
