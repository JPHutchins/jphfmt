//! Conformance suite. M2 reformats call/declaration argument lists, so the byte-identity check
//! is gone; what must hold is idempotency, verbatim passthrough of call-free input, and the §2.2
//! layout for calls. The golden `showcase.c` check returns once the §2 set is complete and the
//! file is re-tabbed.

use cfmt::format;

const SHOWCASE: &str = include_str!("../showcase.c");

#[test]
fn idempotent_on_showcase() {
    let once = format(SHOWCASE);
    assert_eq!(format(&once), once, "format must be idempotent");
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
fn statement_expression_is_not_an_initializer() {
    let src = "int d = ({\n    int t = x;\n    t * 2;\n});\n";
    assert_eq!(
        format(src),
        src,
        "GNU statement-expression must pass through"
    );
}

#[test]
fn initializer_with_comment_passes_through() {
    let src = "int v[] = {\n    1, /* one */\n    2,\n};\n";
    assert_eq!(format(src), src, "comments in a list defer to M7");
}

#[test]
fn compound_literal_initializer_explodes() {
    let src = "p = &(struct shape){.tag = R, .rect = {.w = 3, .h = 4},};\n";
    let expected = "p = &(struct shape){\n\t.tag = R,\n\t.rect = {.w = 3, .h = 4},\n};\n";
    assert_eq!(format(src), expected);
}

#[test]
fn declaration_with_brace_explodes_and_keeps_brace_attached() {
    let src = "static int do_something_with_a_long_name(int first_parameter, int second_parameter, int third_parameter) {\n";
    let expected = "static int do_something_with_a_long_name(\n\tint first_parameter,\n\tint second_parameter,\n\tint third_parameter\n) {\n";
    assert_eq!(format(src), expected);
}
