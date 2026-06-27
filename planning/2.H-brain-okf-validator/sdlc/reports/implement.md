# Implementation Report — 2.H-brain-okf-validator

**Date:** 2026-06-26
**Plan:** planning/2.H-brain-okf-validator/tasks.md
**Scope:** Full spec

## What Was Built or Changed

- `src/brain/okf.rs` (new): `OkfFrontmatter` serde struct with all OKF fields as `Option`, `layer` typed as `Option<Vec<String>>`, extras tolerated; `validate_md_file` entry point implementing read → extract → parse → field-check pipeline; `is_valid_layer`, `is_valid_project`, `is_valid_status` vocab helpers; full `#[cfg(test)]` unit test module (30 tests covering all rules and helpers).
- `src/brain/mod.rs` (modified): wired `pub mod okf;` and defined `BrainValidator` implementing `ContentValidator` (`crawl` delegates to `crawl_brain`, `validate_item` delegates to `okf::validate_md_file`); added unit tests for crawl/run.
- `src/lib.rs` (modified): re-exported `BrainValidator`, `OkfFrontmatter`, and `validate_md_file` from `brain`.
- `tests/brain_okf.rs` (new): 14 integration tests driving `.md` fixtures through `validate_md_file` directly and through `BrainValidator::run` end-to-end, including the nested-git pruning integration test.

## Files Created or Modified

| File | Action |
|---|---|
| `src/brain/okf.rs` | created |
| `src/brain/mod.rs` | modified |
| `src/lib.rs` | modified |
| `tests/brain_okf.rs` | created |

## Validation Output

**Commands run:**
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

**Results:**
```
cargo fmt --check    -> no diff (clean)
cargo clippy -- -D warnings -> Finished `dev` profile — 0 errors, 0 warnings
cargo test          -> 91 unit tests + 14 integration tests (brain_okf) — all passed
cargo build --release -> Finished `release` profile
```

Status: PASSED

## Decisions and Trade-offs

- **Collapsible-if pattern for project/status/doc_id checks:** clippy `-D warnings` requires the `if let Some(...) = ... && !condition { ... }` form rather than the double-nested `if`. This is consistent with how `learn_ai::meta` handles `duration`/`difficulty` checks.
- **`!(3..=7).contains(&count)` for keywords range:** clippy prefers this over `count < 3 || count > 7`. Clearer intent.
- **`related` field tolerated but not validated:** per spec scope boundary — `related` is an optional structural edge field with no closed set to validate. It is deserialized as `Option<Vec<String>>` and silently ignored.
- **No scalar-coercion for `layer`:** the spec settled this question definitively — `layer` is always a YAML list in the live corpus. The struct models it as `Option<Vec<String>>` with no fallback path.
- **`type` is presence-only:** spec explicitly states "type is presence-only (open vocab — never check its value)". The unit test `type_value_is_not_vocab_checked` confirms this.

## Follow-up Work

- Block I: `validate-brain` subcommand in `src/main.rs` + `--json` output flag. Deliberately excluded from this block per the spec scope boundary ("Do not touch `src/main.rs`").

## git diff --stat

```
 src/brain/mod.rs | 74 ++++++++++++++++++++++++++++++++++++++++++++++++++++++--
 src/lib.rs       |  2 ++
 2 files changed, 74 insertions(+), 2 deletions(-)
```
(New files `src/brain/okf.rs` and `tests/brain_okf.rs` not shown in diff —stat since they are untracked at diff time; shown as new in git status.)
