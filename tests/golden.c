/*
 * SPDX-License-Identifier: Apache-2.0
 *
 * Copyright (c) 2026 Intercreate, Inc.
 * Author: J.P. Hutchins <jp@intercreate.io>
 *
 * C23 syntax showcase for the black-style .clang-format in this directory.
 *
 * This is a FORMATTING REFERENCE, not production code. It deliberately uses a
 * few constructs the house style discourages (casts, goto, #define constants,
 * a switch default) purely to show how the formatter renders them. Run
 *
 *     clang-format -i showcase.c
 *
 * and nothing should change: the file is checked in already formatted.
 *
 * Compiles with:  gcc -std=c2x -c showcase.c   (or clang -std=c23 -c ...).
 * _BitInt and #embed are guarded by feature tests for older toolchains.
 */

/* ============================= Preprocessor ============================= */

#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>

#include <stdbool.h>
#include <stdio.h>

/*
 * Zephyr/Arm-style attribute macros. The .clang-format lists these under
 * AttributeMacros, so it spaces them like attributes rather than variables.
 * Defined here, guarded, so this standalone file still compiles.
 */
#ifndef __packed
#define __packed __attribute__((packed))
#endif
#ifndef __aligned
#define __aligned(n) __attribute__((aligned(n)))
#endif

#define PI 3.14159265358979323846
#define MIN(a, b) ((a) < (b) ? (a) : (b))
#define LOG(level, ...) log_impl((level), __FILE__, __LINE__, __VA_ARGS__)
#define STRINGIFY(x) #x
#define CONCAT(a, b) a##b
#define SWAP(a, b) \
	do { \
		typeof(a) t = (a); \
		(a) = (b); \
		(b) = t; \
	} while (0)

#define PACKED_BEGIN _Pragma("pack(push, 1)")
#define PACKED_END _Pragma("pack(pop)")

#if defined(__STDC_VERSION__) && __STDC_VERSION__ >= 202000L
#define C23_OR_NEWER 1
#elif defined(__STDC_VERSION__)
#define C23_OR_NEWER 0
#else
#error "a standard C compiler is required" /* never taken */
#endif

#if 0
#warning "this conditional group is never compiled"
#error "neither is this line"
#endif

#ifdef __has_include
#if __has_include(<stdint.h>)
#define HAVE_STDINT 1
#endif
#endif

/* ======================== Compile-time assertions ======================= */

_Static_assert(sizeof(int) >= 2, "int is too small"); /* C11 spelling */
static_assert(sizeof(void *) >= 4, "pointer is too small"); /* C23 spelling */

/* ================================ Types ================================= */

enum color {
	COLOR_RED,
	COLOR_GREEN = 10,
	COLOR_BLUE,
};

/* C23: an enum with an explicit underlying type */
enum byte_code : uint8_t {
	OP_NOP = 0x00,
	OP_HALT = 0xFF,
};

enum { ANON_LIMIT = 64 }; /* anonymous enum as a constant */

typedef uint32_t handle_t;
typedef int (*comparator_t)(void const * lhs, void const * rhs);
typedef char line_buffer_t[256];

struct __packed packet_header {
	uint8_t version: 4; /* bit-fields */
	uint8_t flags: 4;
	uint16_t length;
	uint32_t: 0; /* anonymous zero-width aligner */
};

PACKED_BEGIN
struct wire_format { /* packed via the _Pragma-based macros above */
	uint16_t type;
	uint32_t value;
};
PACKED_END

struct sensor_sample {
	alignas(16) uint64_t timestamp;
	struct { /* anonymous struct member */
		int16_t x;
		int16_t y;
		int16_t z;
	};
	union { /* anonymous union member */
		uint32_t raw;
		float scaled;
	};
};

struct message {
	size_t length;
	uint8_t payload[]; /* flexible array member */
};

union scalar {
	int as_int;
	float as_float;
	void * as_ptr;
};

struct shape { /* tagged union: a sum type, house-style */
	enum { SHAPE_CIRCLE, SHAPE_RECT } tag;
	union {
		struct {
			double radius;
		} circle;
		struct {
			double width;
			double height;
		} rect;
	};
};

#ifdef __BITINT_MAXWIDTH__
typedef _BitInt(24) i24_t; /* C23 bit-precise integer */
#endif

/* ========================= Storage & qualifiers ========================= */

int volatile g_hardware_register;
int const g_const_value = 42;
[[maybe_unused]] static int g_internal_counter = 0;
extern int g_external_symbol;
_Atomic int g_atomic_flag;
thread_local int g_per_thread;
[[maybe_unused]] constexpr int MAX_ITEMS = 16; /* C23 */

int const * const g_table_ptr = nullptr; /* east const, middle pointer */
int ** g_pointer_to_pointer = nullptr;

[[maybe_unused]] static uint8_t __aligned(32) g_dma_buffer[64];

char const * const g_names[] = {
	"alpha",
	"bravo",
	"charlie",
};

int const g_matrix[2][3] = {
	{1, 2, 3},
	{4, 5, 6},
};

struct sensor_sample const g_sample = {
	.timestamp = 1'000'000'000, /* digit separators */
	.x = -1,
	.y = 0,
	.z = 1,
	.raw = 0xDEAD'BEEF,
};

struct shape const g_unit_circle = {.tag = SHAPE_CIRCLE, .circle = {.radius = 1.0}};

#if defined(__has_embed)
#if __has_embed(__FILE__) == 1
static unsigned char const g_self_prefix[] = {
#embed __FILE__ limit(8)
};
#endif
#endif

/* ============================== Functions =============================== */

[[nodiscard]] int compute(int x);
[[deprecated("use compute() instead")]] int legacy_compute(int x);
[[noreturn]] void fatal(char const * msg);
int compare(int, int); /* prototype with unnamed parameters */
extern int log_impl(int level, char const * file, int line, char const * fmt, ...);

static inline int square(int const x) {
	return x * x;
}

int sum_variadic(int count, ...) {
	va_list args;
	va_start(args, count);
	int total = 0;
	for (int i = 0; i < count; i++) {
		total += va_arg(args, int);
	}
	va_end(args);
	return total;
}

int sum_array(size_t const n, int const values[n]) { /* VLA-syntax parameter */
	int total = 0;
	for (size_t i = 0; i < n; i++) {
		total += values[i];
	}
	return total;
}

void copy_words(int * restrict dst, int const * restrict src, size_t n) {
	for (size_t i = 0; i < n; i++) {
		dst[i] = src[i];
	}
}

#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Wfloat-equal"
[[maybe_unused]] static bool exactly_equal(double a, double b) {
	return a == b; /* intentional exact compare; pragma silences the warning */
}
#pragma GCC diagnostic pop

/* ============================ Literals ================================== */

/* These locals are intentionally unused; this section only shows literal forms. */
#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Wunused-variable"
[[maybe_unused]] static void literals(void) {
	int decimal = 1'000'000;
	int hexadecimal = 0xFF'FF;
	int octal = 0755;
	int binary = 0b1010'0101;
	unsigned int unsigned_lit = 4'000'000'000U;
	long long big = 9'000'000'000'000LL;
	unsigned long long bigger = 0xFFFF'FFFF'FFFF'FFFFULL;
	double scientific = 6.022e23;
	double hex_float = 0x1.8p3;
	float single = 3.14f;
	long double extended = 3.14159L;
	char letter = 'A';
	char escaped = '\n';
	char utf8_char = u8'$';
	char const * string = "tab\tnewline\n";
	char const * adjacent =
		"auto"
		"concatenated";
	auto utf16_string = u"η μάθησις"; /* type inferred via auto */
	auto utf32_string = U"unicode";
	auto wide_string = L"wide";
}
#pragma GCC diagnostic pop

/* ======================= Operators & expressions ======================== */

/* Guarded: clang-format would push _Generic onto a continuation line (#82426). */
#define type_name(x) _Generic( \
	(x), \
	_Bool: "bool", \
	char: "char", \
	int: "int", \
	unsigned: "unsigned", \
	long: "long", \
	long long: "long long", \
	float: "float", \
	double: "double", \
	default: "other" \
)

[[maybe_unused]] static int operators(int a, int b) {
	register int accumulator = 0;
	accumulator = a + b - a * b / (b != 0 ? b : 1) % 7;
	accumulator += (a & b) | (a ^ b) | ~a;
	accumulator ^= (a << 2) >> 1;
	bool logic = ((a < b) && (a <= b)) || ((a > b) && (a >= b)) || ((a == b) && (a != b));
	accumulator = logic ? a : b;
	accumulator = a > b ? a : a < b ? b : 0; /* nested ternary */
	++accumulator;
	accumulator--;

	int values[ANON_LIMIT] = {0};
	values[0] = (int) sizeof values; /* cast: the house style avoids these */
	values[1] = (int) alignof(max_align_t);
	values[2] = (int) offsetof(struct sensor_sample, raw);
	int * cursor = &values[3];
	*cursor = 99;

	for (int i = 0, j = ANON_LIMIT - 1; i < j; i++, j--) { /* comma operator */
		accumulator += values[i];
	}
	return accumulator;
}

/* ========================== Control flow ================================ */

int control_flow(enum color c, int n) {
	int total = 0;

	if (n < 0) {
		return -1;
	} else if (n == 0) {
		total = 0;
	} else {
		total = n;
	}

	switch (c) {
		case COLOR_RED:
			total += 1;
			[[fallthrough]];
		case COLOR_GREEN:
			total += 10;
			break;
		case COLOR_BLUE: {
			int const bonus = 100;
			total += bonus;
			break;
		}
		default:
			total = 0;
	}

	while (total > 100) {
		total -= 100;
	}

	do {
		total += 1;
	} while (total < 5);

	for (int i = 0; i < n; i++) {
		if (i == 2) {
			continue;
		}
		if (i == 8) {
			break;
		}
		total += i;
	}

	if (total < 0) {
		goto cleanup;
	}
	return total;

cleanup:
	total = 0;
	return total;
}

/* =========================== C23 features =============================== */

#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Wunused-variable"
[[maybe_unused]] static void c23_features(void) {
	int * null_pointer = nullptr;
	auto inferred_int = 10;
	auto inferred_double = 1.5;
	typeof(inferred_int) same_type = 20;
	typeof_unqual(g_const_value) mutable_copy = 5;

	constexpr int table_size = 4;
	int table[table_size] = {1, 2, 3, 4};

	struct shape circle = {.tag = SHAPE_CIRCLE, .circle = {.radius = 2.5}};
	struct shape * shape_ptr = &(struct shape){
		.tag = SHAPE_RECT,
		.rect = {.width = 3, .height = 4},
	};

	SWAP(inferred_int, same_type);
}
#pragma GCC diagnostic pop

/* ===================== File-scope definitions =========================== */

static int driver_init(void) {
	return 0;
}

static int driver_read([[maybe_unused]] void * buf, size_t len) {
	return (int) len;
}

static void driver_deinit(void) {}

struct ops {
	int (*init)(void);
	int (*read)(void * buf, size_t len);
	void (*deinit)(void);
};

[[maybe_unused]] static struct ops const g_driver = {
	.init = driver_init,
	.read = driver_read,
	.deinit = driver_deinit,
};

/* ==================== const placement & cast spacing ==================== */

/*
 * East const throughout: `const` follows the type and pointers are middle-
 * aligned, so a const pointer to const data reads as `int const * const`.
 * clang-format preserves this ordering; it does not rewrite qualifiers.
 */
int const immutable_value = 0;
int const * pointer_to_const_data;
int * const const_pointer_to_data = nullptr;
int const * const const_pointer_to_const = nullptr;
int const * const * const handle_to_const_handle = nullptr;
char const * const status_strings[] = {"ready", "busy", "error"};

void takes_const_pointers(int const * const input, size_t const length, char const * const label);

[[maybe_unused]] static int cast_spacing(double measurement) {
	int whole = (int) measurement; /* C-style casts take a space: (int) x */
	void * opaque = (void *) &whole;
	int const * view = (int const *) opaque;
	unsigned narrowed = (unsigned char) whole;
	return whole + (int) (measurement * 2.0) + *view + (int) narrowed;
}

/* =================== Wrapping: long names & many args =================== */

enum {
	MINIMUM_ACCEPTABLE_THRESHOLD = 10,
	MAXIMUM_ACCEPTABLE_THRESHOLD = 1000,
};

extern int compute_weighted_average(long long total, size_t count, int rounding_mode, bool clamp);
extern int dispatch_incoming_event(void * handler, int event, uint64_t timestamp_ms, int context);
extern uint64_t read_monotonic_timestamp_ms(void);
extern int current_execution_context_id(void);

/* A declaration whose parameters do not fit breaks one per line, ) on its own line. */
int reconfigure_peripheral_clock_tree(
	struct shape * target_node,
	uint32_t requested_frequency_hz,
	uint32_t tolerance_parts_per_million,
	bool allow_fractional_dividers
);

void register_completion_callback(
	int (*on_complete)(void * user_context, int result_status),
	void * user_context,
	int priority_level,
	char const * debug_label
);

/*
 * Macros are clang-format's blind spot (LLVM issue #82426): it forces the body
 * onto a continuation line and over-indents it, so a function-like macro will
 * NOT block-indent like the equivalent call. To keep macros consistent with the
 * rest of the style, hand-lay-out inside a clang-format off/on guard - the one
 * override clang-format honors.
 */
#define DISPATCH_EVENT(handler, event) dispatch_incoming_event( \
	(handler), \
	(event), \
	read_monotonic_timestamp_ms(), \
	current_execution_context_id() \
)

[[maybe_unused]] static int demonstrate_wrapping(
	int const measurement_samples[],
	size_t const total_number_of_samples
) {
	long long accumulated_signal_total = 0;
	for (
		size_t current_sample_index = 0;
		current_sample_index < total_number_of_samples;
		current_sample_index++
	) {
		accumulated_signal_total += measurement_samples[current_sample_index];
	}

	int const averaged_result = compute_weighted_average(
		accumulated_signal_total,
		total_number_of_samples,
		0,
		true
	);

	if (
		averaged_result > MINIMUM_ACCEPTABLE_THRESHOLD &&
		averaged_result < MAXIMUM_ACCEPTABLE_THRESHOLD &&
		averaged_result != 0
	) {
		return averaged_result;
	}
	return -1;
}

/* ====================== Nested ternary (black-ish) ====================== */

/* Guarded: clang-format column-aligns multi-line ternaries no matter the
 * settings; this is the flat, no-alignment form with trailing operators. */
[[maybe_unused]] static char const * classify_status_code(int status_code) {
	return (
		status_code == 0 ? "ok" :
		status_code == 1 ? "busy" :
		status_code == 2 ? "error" :
		status_code < 0 ? "fault" :
		"unknown"
	);
}

[[maybe_unused]] static int saturating_clamp(
	int candidate_value,
	int lower_bound,
	int upper_bound
) {
	return (
		candidate_value < lower_bound ? lower_bound :
		candidate_value > upper_bound ? upper_bound :
		candidate_value
	);
}

/* ===================== Statement expressions (GNU) ====================== */

/* Guarded: clang-format would push ({ onto its own line and over-indent (#82426). */
#define MAX(a, b) ({ \
	typeof(a) _a = (a); \
	typeof(b) _b = (b); \
	_a > _b ? _a : _b; \
})

[[maybe_unused]] static int statement_expression(int x, int y) {
	int const larger = MAX(x, y);
	int const doubled = ({
		int t = larger;
		t * 2;
	});
	return larger + doubled;
}
