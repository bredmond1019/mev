# Implementation Report — phase1-blockB-task1

**Date:** 2026-06-19
**Plan:** planning/phase1-blockB/tasks.md
**Scope:** Task 1 — Define classification + corpus types

## What Was Built or Changed

- Created `src/crawl.rs` with all classification, corpus, and walk logic for Phase 1 Block B.
  Includes:
  - `Locale` enum (`En`, `PtBr`)
  - `FileKind` enum (`LearnModuleJson`, `PathMetadataJson`, `ModuleMdx`)
  - `ContentFile` struct with `path`, `rel`, `kind`, `path_id`, `module_id`, `locale` fields
  - `Corpus` struct with `files: Vec<ContentFile>` and three accessors: `path_ids()`,
    `modules_for()`, `get()`
  - Private `classify(root, path) -> Option<ContentFile>` implementing the ground-truth
    derivation rule from the breakdown
  - Private `check_filename(&ContentFile) -> Vec<Diagnostic>` for the three naming conventions
    (no spaces, lowercase, `NN-slug.(json|mdx)` pattern)
  - Private `is_valid_module_filename(name) -> bool` hand-rolled regex check (no `regex` crate)
  - Public `crawl(root) -> (Corpus, Vec<Diagnostic>)` walk driver
  - 7 unit tests in `#[cfg(test)]` covering derive equality, corpus accessors, and filename validation
- Modified `src/lib.rs` to:
  - Register `mod crawl;` and re-export all public types via `pub use crawl::{...}`
  - Replace the Phase 0 stub `validate()` body with a real crawl call; return type preserved

## Files Created or Modified

| File | Action |
|---|---|
| src/crawl.rs | created |
| src/lib.rs | modified |
| planning/phase1-blockB/sdlc/reports/task1-implement.md | created |

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
cargo fmt --check
(no diff output — exit 0)

cargo clippy -- -D warnings
    Checking mev v0.1.0
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.14s

cargo test
running 7 tests
test crawl::tests::file_kind_eq ... ok
test crawl::tests::invalid_module_filenames ... ok
test crawl::tests::get_finds_exact_match ... ok
test crawl::tests::locale_eq ... ok
test crawl::tests::modules_for_filters_by_locale_and_excludes_metadata ... ok
test crawl::tests::path_ids_sorted_and_deduped ... ok
test crawl::tests::valid_module_filenames ... ok
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

Running tests/smoke.rs
running 2 tests
test diagnostic_severity_drives_failure ... ok
test empty_tree_produces_clean_report ... ok
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

cargo build --release
    Finished `release` profile [optimized] target(s) in 5.44s
```

Status: PASSED

## Decisions and Trade-offs

- `Locale` field added to `ContentFile` despite not being in the spec's type definition: the
  breakdown explicitly calls this "in-scope discovery, not scope creep" — the `pt-BR` nesting in
  the live tree cannot be correctly parsed without it, and Block E (pt-BR parity) will consume it.
- `BTreeSet` used in `path_ids()` to guarantee sorted, deduplicated output — test determinism
  across different filesystem orderings.
- `is_valid_module_filename` is hand-rolled (char-class checks) rather than using the `regex`
  crate — the spec and breakdown both prohibit new deps; the implementation is explicit and legible.
- All three naming convention checks (spaces, case, pattern) are errors, matching the TS
  `validateFileName` behavior. Files with bad names are still added to the corpus so downstream
  blocks see them.
- `_corpus` binding in `validate()` uses leading underscore to silence clippy — it documents
  that the corpus is built here and ready for Blocks C–E without being consumed yet.

## Follow-up Work

- Task 2: `crawl.rs` classification logic already implemented in this task (classify + crawl
  functions). Task 2 in the orchestration may be a no-op or minimal extension.
- Task 3: `check_filename` already implemented.
- Task 4: `validate()` wiring already done.
- Task 5: Integration test file `tests/crawl.rs` with fixture-tree tests remains to be created.
- Task 6: Optional live-tree sanity check (`cargo run -- validate ../learn-ai/content/learn`).

## git diff --stat

```
 src/lib.rs | 15 ++++++++++-----
 1 file changed, 10 insertions(+), 5 deletions(-))
src/crawl.rs  (new file, ~390 lines)
```
