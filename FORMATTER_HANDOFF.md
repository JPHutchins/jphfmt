# Handoff: build an opinionated C formatter ("cfmt")

> **Audience:** an autonomous coding agent (or engineer) implementing the tool from scratch.
> **You do not need the originating conversation.** Everything required is here plus the two
> sibling files in this directory: `showcase.c` (the conformance suite) and `.clang-format`
> (reference for the ~90% an off-the-shelf tool already gets right).

---

## 0. Mission

Build a small, **zero-config, opinionated** C code formatter — gofmt/black philosophy applied
to C. It enforces **one uniform layout rule** and performs **no column alignment, ever**.

**Definition of done:** the formatter reproduces `showcase.c` byte-for-byte *after its five
`// clang-format off` … `// clang-format on` guarded regions are unguarded* — i.e. it natively
produces the hand-laid-out forms those guards currently protect — and `format(format(x)) ==
format(x)` for all inputs.

Non-goals: configurability beyond a tiny set (width, tab width), macro expansion, semantic
analysis, supporting C++/ObjC, or matching any existing tool's output.

---

## 1. Why this exists (it constrains the design — don't skip)

We exhausted clang-format 22.1.5. It is excellent and now covers calls, parameters,
declarations, initializers, `if`, `switch`, and `for`/`while` headers with uniform block-indent.
But the requester's rule is **absolute: column alignment is never acceptable**, and clang-format
hardwires alignment in constructs with **no option to disable it**:

| Construct | clang-format forces | Fixable in clang-format? |
| --- | --- | --- |
| Multi-line ternary | column-aligns `?` and the final clause | No — verified, no option |
| Wrapped `(bare paren expr)` | aligns operands to the open paren | No — no break option for bare parens |
| `for` clauses | fill, not one-per-line | No — no for-clause bin-pack option |
| Function-like / statement-expr macros | continuation-indents; body won't open on the `#define` line | No — open bug `llvm/llvm-project#82426`, unfixed since 2024 |

(The `for`-header *frame* was fixed in clang-format 22 via `BreakAfterOpenBracketLoop`, see
`llvm/llvm-project#79176` / PR `#108332`; the macro and alignment gaps remain.)

**Consequence:** the requester needs the gofmt/black answer — a separate tool with a single
non-negotiable rule. That rule is *far simpler* to implement than clang-format because there is
no penalty-based line-fitting optimizer and no option matrix.

---

## 2. The style specification (this is the real payload)

### 2.1 Global

- **Indentation: hard tabs**, one tab per nesting level. (The requester is moving to tabs.)
  Because there is **no column alignment**, tabs are unambiguous — there is never a need to mix
  tabs and spaces. This is a major simplification; lean on it.
- **Width limit: 100 columns.** Measure tab width as **4** for the fits/overflow decision
  (configurable). The limit drives the single rule in §2.2.
- **Line endings: LF.** Ensure exactly one trailing newline at EOF.
- **Comments are sacred:** never reflow, rewrap, move, or column-align them. Preserve text
  verbatim; only normalize the code around them.
- Examples in this doc are shown with 4 spaces for readability; **emit tabs**.

### 2.2 THE rule (the entire core of the formatter)

For every bracket group — `(...)`, `{...}`, `[...]` — and for `for`-headers:

1. If the whole construct **fits** within the width on its current line → keep it on one line.
2. Otherwise **explode**:
   - newline immediately after the open bracket;
   - **every top-level element on its own line**, indented exactly one level deeper
     (split on commas at this bracket depth; for a `for`-header, split on `;`);
   - newline before the close bracket;
   - the close bracket sits at the **parent's** indentation.
3. **Never align** a continuation to an opening delimiter, an operator, or a previous token.
   Indentation is always a whole number of tabs.

This is exactly the Wadler/Prettier `group` combinator (fits-flat **or** fully-broken) **without
Prettier's `fill` mode**. Implement `group` and you have the engine.

Applies uniformly to: function calls, function declaration/definition parameter lists, braced
initializers, compound-literal initializers, array initializers, `for`/`while`/`if`/`switch`
headers, **macro bodies**, and **GNU statement-expressions** `({ ... })`.

### 2.3 Magic trailing comma (`{}` only)

- A trailing comma before a `}` **forces** explosion (§2.2 step 2) even if it would otherwise fit.
- No trailing comma → collapse-if-fits.
- **Scope:** `{}` braced lists only. C forbids a trailing comma in `()` call/parameter lists and
  in `_Generic`, so those are **purely width-driven** (no magic-comma input is possible there).

### 2.4 Golden forms (copy these exactly; they are in `showcase.c`)

Function call / parameter list, exploded one-per-line, close paren on its own line, hug the
callee (`x = f(` stays together):

```c
int const averaged_result = compute_weighted_average(
    accumulated_signal_total,
    total_number_of_samples,
    0,
    true
);

int reconfigure_peripheral_clock_tree(
    struct shape * target_node,
    uint32_t requested_frequency_hz,
    uint32_t tolerance_parts_per_million,
    bool allow_fractional_dividers
);
```

`for` header — **each clause on its own line**, `)` on its own line, brace attached:

```c
for (
    size_t i = 0;
    i < total_number_of_samples;
    i++
) {
    ...
}
```

Ternary — when broken: **flat** (every clause at the same indent), **operators trailing**, no
alignment, wrapped in parens:

```c
return (
    status_code == 0 ? "ok" :
    status_code == 1 ? "busy" :
    status_code == 2 ? "error" :
    status_code < 0 ? "fault" :
    "unknown"
);
```

Function-like macro — body **opens on the `#define` line**, args at +1, close paren at column 0,
`\` continuations one space after the content (never aligned):

```c
#define DISPATCH_EVENT(handler, event) dispatch_incoming_event( \
    (handler), \
    (event), \
    read_monotonic_timestamp_ms(), \
    current_execution_context_id() \
)
```

Statement-expression macro — `({` on the `#define` line, body +1, `})` at column 0:

```c
#define MAX(a, b) ({ \
    typeof(a) _a = (a); \
    typeof(b) _b = (b); \
    _a > _b ? _a : _b; \
})
```

Compound literal / designated initializer (trailing comma → explode):

```c
struct shape * shape_ptr = &(struct shape){
    .tag = SHAPE_RECT,
    .rect = {.width = 3, .height = 4},
};
```

### 2.5 Spacing & tokens

- **Pointers, middle-aligned:** `T * p`, and `T const * const p` (space on both sides of `*`).
- **East const, preserved:** keep `int const * const` as written. Do **not** reorder qualifiers
  (we chose preserve over enforce; revisit only if §8 says so).
- **C-style casts get a space:** `(int) x`, `(void *) p`.
- **Control keywords get a space before `(`:** `if (`, `for (`, `while (`, `switch (`.
  Function calls do not: `foo(`.
- **Braces attach (K&R):** `) {` on the same line for functions and control statements,
  including multi-line headers.
- **`switch`:** `case` labels indented one level under `switch`; a braced case attaches:
  `case X: {`.
- **Bit-fields:** `uint8_t ready: 1;` (space after the colon, none before).
- **Short enums** stay inline (`enum { A, B }`); a trailing comma explodes them (§2.3).

### 2.6 Preprocessor (the differentiator — clang-format gets this wrong)

- A `#define` body is formatted **as if it began at the macro's base indentation** (column 0 for
  a file-scope `#define`), then a `\` is appended to every line but the last, **one space after
  the content** (never column-aligned). This is the rule that lets a function-like macro's body
  open on the `#define` line (§2.4).
- `#if` / `#ifdef` / `#elif` / `#else` / `#endif`: preserve the directives; format the contained
  code if it parses, otherwise pass it through untouched. Do not attempt macro expansion.

### 2.7 Operator line-breaking

- Ternary operators **trail** (the `:` ends the line) — see §2.4.
- Binary operators: the reference `.clang-format` currently breaks **before** them
  (operator leads the continuation). **This is an open decision (§8) — pick one and apply it
  uniformly.** For internal consistency with the ternary, the requester may prefer trailing
  everywhere; confirm.

---

## 3. Architecture

### 3.1 Recommended: Rust, token-stream pipeline ("Architecture A")

```
source bytes
  → Lexer            (logos: C tokens + comments + whitespace/newlines as trivia)
  → Structurer       (split into logical lines: stmt / decl / preprocessor directive;
                      match brackets; tag the few special constructs: for, #define, ({, ?:)
  → Doc builder      (emit a Wadler document: text / line / group / nest)
  → Renderer         (lay out at width W using the fits-or-break rule)
  → bytes
```

- **Lexer:** [`logos`](https://crates.io/crates/logos) — hand-write the C token rules (~100
  lines). Keep comments and newlines as trivia so they can be reattached losslessly.
- **Pretty-printer:** the Wadler/Leijen algorithm ("A prettier printer", Wadler 2003). Either the
  [`pretty`](https://crates.io/crates/pretty) crate or ~200 lines of your own `Doc` enum +
  `group`/`nest`/`line`/`text` + a width-aware renderer. **Do not implement `fill`.**
- **Error tolerance:** if a logical line can't be structured confidently, **emit it verbatim**
  (passthrough). This de-risks v1 enormously — partial coverage is still useful and never
  corrupts code.
- **Distribution:** single static binary via `cargo`; trivial CI and editor integration. The
  requester already uses Rust.

### 3.2 Why token-stream over a full parse

You do **not** need a C parser or type resolution — the rules in §2 are local and structural.
clang-format itself works on tokens + bracket nesting, not a Clang AST. The one place semantics
leak in is `*` (pointer vs. multiply) for spacing; a heuristic (previous token is a type/ident,
next is an ident/`*`/`(`) covers it — same approach clang-format uses. Macros and `for` headers —
the sore spots — are easiest to control on a raw token stream, which is exactly why this beats a
CST approach here.

### 3.3 Alternatives (consider, then likely reject)

- **tree-sitter-c + `pretty`:** robust, error-tolerant parse for free; but you inherit
  tree-sitter's preprocessor model and get *less* control over the macro layout that is the whole
  point. Good fallback if hand-structuring proves fiddly.
- **OCaml** (`ocamllex` + the `Format` module / `pprint`): the most elegant implementation —
  pretty-printing is OCaml's home turf — but packaging a CLI for an embedded team is more work.
- **Python** (`pygments` lexer or tree-sitter): fastest prototype, worst distribution. Avoid
  `pycparser` (needs preprocessed input; mangles macros).
- **Topiary / extending clang-format:** rejected. Topiary's width-reflow is uncertain;
  clang-format's alignment is unfixable (§1).

---

## 4. Conformance suite & verification

In this directory:

- **`showcase.c`** — the golden file and the spec-by-example. It is currently formatted by
  clang-format 22 **plus five `// clang-format off` … `on` guarded regions** (3 macros, 2
  ternaries — `grep -nE '^[[:space:]]*// clang-format (off|on)$' showcase.c`). Your formatter must
  produce those guarded forms **natively**, so the acceptance test is: *delete the ten guard
  lines, run cfmt, and the result must equal the unguarded content.* Everything outside the guards is already the target output.
- **`.clang-format`** — documents the spacing/brace/wrapping decisions clang-format *can* express;
  use it to resolve any spacing ambiguity in §2.

Tests to implement:

1. **Idempotency:** `cfmt(cfmt(x)) == cfmt(x)` on the whole corpus. Non-negotiable.
2. **Golden:** `cfmt(showcase.c sans guards) == (showcase.c sans guards)`.
3. **Compiles:** `gcc -std=c2x -Wall -Wextra -c showcase.c` must pass before and after
   (semantics unchanged; whitespace-only edits). `gcc-13` is sufficient; `_BitInt`/`#embed` in the
   showcase are behind feature guards.
4. **Passthrough safety:** feeding a file cfmt can't fully structure must change nothing outside
   the parts it understands.

Reference tooling used to build the suite: `uv tool install clang-format` (gives 22.1.x);
`gcc -std=c2x`.

---

## 5. Build milestones

- **M1 — Skeleton + identity.** Lex → reprint verbatim. Prove byte-identity and idempotency on
  `showcase.c`. CLI: read stdin / `-i` in place / `--check`.
- **M2 — The engine + calls.** Wadler `Doc`, the §2.2 rule, applied to function call argument
  lists only. Hug the callee.
- **M3 — Params, initializers, magic comma.** Declaration/definition parameter lists; `{}`
  braced/array/designated initializers; trailing-comma explosion (§2.3).
- **M4 — Control headers.** `for` (one clause per line), `while`, `if`, `switch` — block-indent
  with attached `) {`.
- **M5 — Macros.** `#define` body at base indent, `\` continuations one-space-after; function-like
  macro bodies and statement-expressions `({ })` (§2.6). This is the feature clang-format lacks.
- **M6 — Ternaries.** Flat, trailing operators, parens (§2.4). Decide insert-vs-require parens (§8).
- **M7 — Long tail.** Comment attachment/placement; `#if` interleaving; idempotency hardening;
  the `*` spacing heuristic.
- **M8 — Polish.** Config (width, tab width), editor integration, docs.

Ship M1–M4 early; they already cover most of a real file and are safe (passthrough for the rest).

---

## 6. Hard parts & pitfalls (where the real time goes)

- **Comments (biggest):** deciding where a trailing/leading/dangling comment attaches and keeping
  it there across a reflow is the classic formatter tar-pit. Budget for it. Rule of thumb: attach
  to the nearest following token; keep trailing comments trailing; never move a comment across a
  bracket it was inside.
- **Preprocessor interleaving:** `#if` blocks that open/close mid-construct. Safest v1: treat any
  directive line as its own logical line and don't reflow across it.
- **Idempotency:** the binary fits-or-break rule makes this tractable, but watch the boundary case
  where re-formatting a just-exploded `{}` (now with a trailing comma you didn't add) would
  re-collapse — define magic-comma handling so the second pass is stable.
- **Width with tabs:** pick a tab width (4) purely for the overflow measurement; emitted
  indentation is still tabs.
- **`*` pointer vs multiply:** heuristic only; document the cases it gets wrong and prefer
  passthrough over guessing when ambiguous.

---

## 7. Acceptance criteria

- [ ] Idempotent on the corpus.
- [ ] Reproduces `showcase.c` (guards removed) exactly.
- [ ] `gcc -std=c2x -Wall -Wextra -c` passes on the output.
- [ ] **Zero column alignment** anywhere in any output (the cardinal rule).
- [ ] Single uniform bracket rule (§2.2) applied to calls, params, initializers, control headers,
      macros, statement-expressions, ternaries.
- [ ] CLI: stdin→stdout, `-i` in place, `--check` (non-zero exit if changes needed).
- [ ] Unstructurable input passes through unchanged.

---

## 8. Open decisions — confirm with J.P. before/while building

1. **Binary operator placement:** lead (current `.clang-format`: `BreakBeforeBinaryOperators:
   NonAssignment`) vs. trail. Ternary is **trail**; pick binary for consistency.
2. **Ternary parentheses:** does cfmt **insert** the wrapping `( … )` automatically, or require the
   author to write them? (clang-format can't insert them; cfmt could.)
3. **East const:** preserve as-written (current) vs. **enforce** (`const int` → `int const`).
4. **Compound-literal brace spacing:** `&(struct foo){ … }` vs. `&(struct foo) { … }` (cast-space
   coupling). The showcase currently shows `&(struct shape){` (no space before `{`).
5. **Tab width** for the overflow measurement (default 4) and the **column limit** (default 100).
6. **Tool name** and repo location (new crate, separate from this `utils` repo?).

---

## 9. References


- Philip Wadler, *A prettier printer* (2003) — the layout algorithm; `group` = fits-or-break.
- Prettier's IR (`group`/`indent`/`line`/`softline`) — the same model, minus the `fill` you won't use.
- `logos`, `pretty` (Rust crates); `tree-sitter-c`; OCaml `Format`/`pprint`.
- LLVM clang-format, the tool this replaces for the gaps in §1:
  - `llvm/llvm-project#82426` — multi-line C macro over-indentation (open, unfixed).
  - `llvm/llvm-project#79176` + PR `#108332` — `for`-header block-indent added in clang-format 22.
  - clang-format 22 deprecated `AlignAfterOpenBracket: BlockIndent` → bool + per-construct
    `BreakAfterOpenBracket*` / `BreakBeforeCloseBracket*` options.
- Sibling files: `./showcase.c` (golden), `./.clang-format` (spacing reference).
```
