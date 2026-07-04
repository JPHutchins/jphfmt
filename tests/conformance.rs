//! Conformance suite. What must hold is idempotency, verbatim passthrough of call-free input, and the §2.2
//! layout for calls.

use jphfmt::format;

const GOLDEN: &str = include_str!("golden.c");

#[test]
fn golden_is_a_fixpoint() {
    assert_eq!(format(GOLDEN), GOLDEN, "golden must be idempotent");
}

/// Significant content: everything but whitespace, commas (jphfmt may add a magic trailing comma),
/// and backslashes (continuations). Formatting must never alter anything else.
fn significant(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace() && *c != ',' && *c != '\\')
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
    assert_eq!(format("int **p;\n"), "int ** p;\n");
    assert_eq!(format("char const*const q;\n"), "char const * const q;\n");
    assert_eq!(format("void*f(void);\n"), "void * f(void);\n");
}

#[test]
fn ambiguous_star_is_left_alone() {
    // multiply and user-typedef pointers can't be told apart at the token level (§6)
    assert_eq!(format("z = a*b;\n"), "z = a*b;\n");
    assert_eq!(format("z = a * b;\n"), "z = a * b;\n");
    assert_eq!(format("mytype*p;\n"), "mytype*p;\n");
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

#[test]
fn preprocessor_scope_preserves_define_continuation() {
    // A #define with a \-continuation body: the #define line is at depth 0 (unchanged), and
    // the continuation line (previous line ends in \) is skipped by the scope pass.
    let src = "#define M(a) ((a) + 1) \\\n\t+ 2\n";
    assert_eq!(format(src), src);
}
