//! Conformance suite. M2 reformats call/declaration argument lists, so the byte-identity check
//! is gone; what must hold is idempotency, verbatim passthrough of call-free input, and the §2.2
//! layout for calls. The golden `showcase.c` check returns once the §2 set is complete and the
//! file is re-tabbed.

use cfmt::format;

const SHOWCASE: &str = include_str!("../showcase.c");

const GOLDEN: &str = include_str!("golden.c");

/// `showcase.c` with the ten `// clang-format off|on` guard lines removed — the §4 acceptance
/// input. cfmt must natively produce the hand-laid forms those guards protected.
fn unguarded_showcase() -> String {
    SHOWCASE
        .lines()
        .filter(|l| {
            let t = l.trim();
            t != "// clang-format off" && t != "// clang-format on"
        })
        .fold(String::new(), |mut acc, l| {
            acc.push_str(l);
            acc.push('\n');
            acc
        })
}

#[test]
fn idempotent_on_showcase() {
    let once = format(SHOWCASE);
    assert_eq!(format(&once), once, "format must be idempotent");
}

#[test]
fn golden_acceptance_unguarded_showcase() {
    assert_eq!(
        format(&unguarded_showcase()),
        GOLDEN,
        "cfmt must reproduce the golden from the guard-removed showcase"
    );
}

#[test]
fn golden_is_a_fixpoint() {
    assert_eq!(format(GOLDEN), GOLDEN, "golden must be idempotent");
}

/// Significant content: everything but whitespace, commas (cfmt may add a magic trailing comma),
/// and backslashes (continuations). Formatting must never alter anything else.
fn significant(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace() && *c != ',' && *c != '\\')
        .collect()
}

#[test]
fn showcase_content_is_preserved() {
    assert_eq!(
        significant(&format(SHOWCASE)),
        significant(SHOWCASE),
        "formatting must be whitespace/comma/continuation-only — no token may change"
    );
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
        "char const *p = \"a\\\"b\\n\"; char c = '\\'';\n",
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
    // §8.2: cfmt does not insert parens; a bare ternary passes through
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
