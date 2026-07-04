//! Folder-based shape fixtures. Each subdirectory of `tests/cases/` is one *shape*: a canonical
//! `out.c` plus one or more `in*.c` inputs (`in.c`, or `in_1.c`/`in_2.c` for several variants) that
//! must all format to it. The folder name carries the shape's meaning, so the file names stay
//! uniform and free of redundant semantic suffixes.
//!
//! * **Tier 1** — every `in*.c` formats to its folder's `out.c`, exactly.
//! * **Tier 2** — whitespace mutants of each input stay idempotent and significant-token-equal
//!   under formatting (the universal safety invariant from `tests/conformance.rs`'s `messy.c`).
//!   Arbitrary whitespace mutation does *not* always preserve exact `format(x)` — a newline in a
//!   call arg hits `has_middle_newline` passthrough, a trailing comma flips magic-comma explosion
//!   — so Tier 2 asserts the universal properties, not exact equality.
//! * **Tier 3** — opt-in per shape via a `.fuzz-equality` sentinel in the folder: mutants must
//!   also format *exactly* to `out.c`. Few shapes qualify (passthrough/explosion decisions must
//!   be stable under whitespace mutation), so the sentinel is added only to shapes verified stable.

use jphfmt::format;
use jphfmt::lexer::{TokenKind, tokenize};
use std::fs;
use std::path::{Path, PathBuf};

const CASES_DIR: &str = "tests/cases";
const SEED: u64 = 0x00C0_FFEE;
const MUTANTS_PER_INPUT: usize = 128;

struct Case {
    shape: String,
    expected: String,
    inputs: Vec<(String, String)>,
    /// A `.fuzz-equality` sentinel lives in this shape's folder → Tier 3 exact-equality applies.
    fuzz_equality: bool,
}

/// Discover every `tests/cases/<shape>/` folder, sorted for deterministic iteration. Each folder
/// contributes its `out.c` and every `in*.c` input (sorted by name).
fn discover_cases() -> Vec<Case> {
    let mut dirs: Vec<PathBuf> = fs::read_dir(CASES_DIR)
        .unwrap_or_else(|e| panic!("read {CASES_DIR}: {e}"))
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();

    dirs.into_iter()
        .map(|dir| {
            let shape = dir.file_name().unwrap().to_string_lossy().into_owned();
            let expected = fs::read_to_string(dir.join("out.c"))
                .unwrap_or_else(|e| panic!("read {}: {e}", dir.join("out.c").display()));
            let mut inputs = read_inputs(&dir);
            inputs.sort();
            let inputs = inputs
                .into_iter()
                .map(|p| {
                    let name = p.file_name().unwrap().to_string_lossy().into_owned();
                    let src = fs::read_to_string(&p)
                        .unwrap_or_else(|e| panic!("read {}: {e}", p.display()));
                    (name, src)
                })
                .collect();
            Case {
                shape,
                expected,
                inputs,
                fuzz_equality: dir.join(".fuzz-equality").exists(),
            }
        })
        .collect()
}

fn read_inputs(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_stem().is_some_and(|n| {
                let n = n.to_string_lossy();
                n == "in" || n.starts_with("in_")
            }) && p.extension().is_some_and(|x| x == "c")
        })
        .collect()
}

/// Significant content: everything but whitespace, commas (jphfmt may add a magic trailing comma),
/// and backslashes (line continuations). Formatting must never alter anything else.
fn significant(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace() && *c != ',' && *c != '\\')
        .collect()
}

/// A tiny deterministic `SplitMix64` generator — no new dependency, fully reproducible from the fixed
/// [`SEED`]. Pure (no global state); the only mutation is the cursor's own advancing word count.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn pick_str(&mut self, slice: &[&'static str]) -> &'static str {
        let idx = self.next_u64() % slice.len() as u64;
        slice[usize::try_from(idx).expect("index bounded by slice length")]
    }
}

/// Whitespace runs a mutant may substitute for existing trivia or insert between tokens. Newlines
/// exercise `has_middle_newline` and blank-line collapse. No empty string — deletion can merge tokens
/// into a different valid identifier (e.g. `voidf`), which the formatter must not split back apart.
const TRIVIA_RUNS: &[&str] = &[" ", "  ", "\t", "\n", "\n\n", " \t ", "\n\t"];

/// Spacing-only runs for Tier 3: spaces and tabs, never a newline. A mutant built from these keeps
/// each statement on one line, so the formatter's spacing normalization (not its line-break
/// passthrough) is what's under test — and `format(mutant) == out.c` becomes reachable for shapes
/// whose fits/explode decision isn't spacing-driven. No empty string — deletion can merge tokens
/// (`voidf`), which the formatter must not split back apart.
const SPACING_RUNS: &[&str] = &[" ", "  ", "\t", " \t "];

/// Mutate only the trivia of `src`, drawing replacement/inserted runs from `runs`: replace each
/// existing trivia token (`Whitespace` or `Newline`) with a random run, and occasionally insert a
/// run between adjacent non-trivia tokens. No commas or backslashes are ever added, and
/// comment/string interiors are single tokens so they are untouched — therefore
/// `significant(mutant) == significant(src)` by construction, and the assertions test the formatter
/// rather than the mutator. `runs` decides whether newlines are in play (Tier 2 yes, Tier 3 no).
fn mutate_trivia(src: &str, rng: &mut Rng, runs: &[&'static str]) -> String {
    let toks = tokenize(src);
    let mut out = String::with_capacity(src.len() + 16);
    for (i, t) in toks.iter().enumerate() {
        match t.kind {
            TokenKind::Whitespace | TokenKind::Newline => out.push_str(rng.pick_str(runs)),
            _ => {
                out.push_str(t.text);
                if i + 1 < toks.len() && rng.next_u64().is_multiple_of(4) {
                    out.push_str(rng.pick_str(runs));
                }
            }
        }
    }
    out
}

/// Tier 2 mutant: trivia (incl. newlines) mutated freely — exercises `has_middle_newline`
/// passthrough and blank-line collapse.
fn mutate_whitespace(src: &str, rng: &mut Rng) -> String {
    mutate_trivia(src, rng, TRIVIA_RUNS)
}

/// Tier 3 mutant: spacing only (spaces/tabs, never newlines) — keeps each statement on one line
/// so spacing normalization is what's under test, making `format(mutant) == out.c` reachable.
fn mutate_spacing(src: &str, rng: &mut Rng) -> String {
    mutate_trivia(src, rng, SPACING_RUNS)
}

/// Derive a per-input seed so a failing mutant is reproducible from shape + input name alone,
/// independent of discovery order or other cases.
fn seed_for(shape: &str, name: &str) -> u64 {
    let mut z = SEED;
    for b in shape.bytes() {
        z = z.wrapping_mul(31).wrapping_add(u64::from(b));
    }
    z = z.wrapping_mul(31).wrapping_add(u64::from(b'/'));
    for b in name.bytes() {
        z = z.wrapping_mul(31).wrapping_add(u64::from(b));
    }
    z
}

#[test]
fn call_with_middle_newline_is_idempotent() {
    // A call whose args contain a nested call with an intra-arg newline must pass through verbatim
    // as a whole; reflowing the nested call would strip that newline and flip the outer call's
    // fits/explode decision on the next pass (regression found by the macro-wrapper fuzz, jphfmt
    // 0.1.2).
    let src = "dispatch_incoming_event((handler), (event), read_monotonic_timestamp_ms(\n\t), current_execution_context_id())\n";
    let once = format(src);
    assert_eq!(format(&once), once, "\n--- once ---\n{once}");
}

#[test]
fn curated_inputs_match_expected() {
    for case in discover_cases() {
        for (name, src) in &case.inputs {
            assert_eq!(
                format(src),
                case.expected,
                "shape `{shape}` input `{name}` did not format to expected",
                shape = &case.shape,
            );
        }
    }
}

#[test]
fn whitespace_mutants_are_idempotent_and_significant() {
    for case in discover_cases() {
        for (name, src) in &case.inputs {
            let mut rng = Rng::new(seed_for(&case.shape, name));
            for k in 0..MUTANTS_PER_INPUT {
                let mutant = mutate_whitespace(src, &mut rng);
                let once = std::panic::catch_unwind(|| format(&mutant))
                    .unwrap_or_else(|_| {
                        panic!(
                            "formatter panicked: shape `{shape}` input `{name}` mutant #{k}\n--- mutant ---\n{mutant}",
                            shape = &case.shape,
                        )
                    });
                assert_eq!(
                    format(&once),
                    once,
                    "idempotency broke: shape `{shape}` input `{name}` mutant #{k}\n--- mutant ---\n{mutant}",
                    shape = &case.shape,
                );
                assert_eq!(
                    significant(&once),
                    significant(&mutant),
                    "significance broke: shape `{shape}` input `{name}` mutant #{k}\n--- mutant ---\n{mutant}",
                    shape = &case.shape,
                );
            }
        }
    }
}

#[test]
fn spacing_mutants_match_expected() {
    for case in discover_cases() {
        if !case.fuzz_equality {
            continue;
        }
        for (name, src) in &case.inputs {
            let mut rng = Rng::new(seed_for(&case.shape, name));
            for k in 0..MUTANTS_PER_INPUT {
                let mutant = mutate_spacing(src, &mut rng);
                let once = std::panic::catch_unwind(|| format(&mutant))
                    .unwrap_or_else(|_| {
                        panic!(
                            "formatter panicked: shape `{shape}` input `{name}` mutant #{k}\n--- mutant ---\n{mutant}",
                            shape = &case.shape,
                        )
                    });
                assert_eq!(
                    once,
                    case.expected,
                    "tier 3 exact-equality broke: shape `{shape}` input `{name}` mutant #{k}\n--- mutant ---\n{mutant}",
                    shape = &case.shape,
                );
            }
        }
    }
}
