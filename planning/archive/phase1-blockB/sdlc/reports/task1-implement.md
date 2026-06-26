# Fix Pass 2 — phase1-blockB-task1

**Date:** 2026-06-19
**Plan:** planning/phase1-blockB/tasks.md
**Fix pass:** 2

## Failures Addressed

| Criterion | Status Before | Fix Applied |
|---|---|---|
| `Corpus` enumerates content tree correctly (integration test) | PARTIAL | Added `tests/crawl.rs` with `good_modules_classified_correctly` test exercising `crawl()` against a real temp-dir tree with `metadata.json`, `.json` module, and `.mdx` module |
| Filename violations surface as `Diagnostic` with correct severity (integration test) | PARTIAL | Added three bad-filename tests: spaces, uppercase, missing `NN-` prefix — each asserts `Severity::Error` with the correct message substring |
| New fixture-driven tests cover good + deliberately-broken filenames | PARTIAL | Created `tests/crawl.rs` with 7 integration tests covering all four fixture scenarios from the spec |
| Non-content files skipped without error (integration test) | MET (unit) | Added `non_content_files_are_skipped` end-to-end test covering `README.md`, `schemas/`, `shared/`, and dotfiles |

## Changes Made

- Created `tests/crawl.rs` — fixture-driven integration tests exercising `crawl()` end-to-end against real temp-dir trees:
  1. `good_modules_classified_correctly` — 3-file fixture (metadata.json + 01-intro.json + 01-intro.mdx); asserts corpus size, path_ids(), FileKind, module_id, locale, modules_for()
  2. `bad_filename_spaces_emits_error` — space in filename; asserts Severity::Error with "spaces" in message
  3. `bad_filename_uppercase_emits_error` — uppercase in filename; asserts Severity::Error with "lowercase" in message
  4. `bad_filename_missing_nn_prefix_emits_error` — no NN- prefix; asserts Severity::Error with "NN-slug" in message
  5. `bad_filename_file_still_in_corpus` — bad filename still lands in corpus (not dropped)
  6. `non_content_files_are_skipped` — README.md, schemas/, shared/, dotfiles produce zero diagnostics and zero corpus entries
  7. `empty_tree_produces_empty_corpus_and_no_diagnostics` — empty tree is clean

## Files Created or Modified

| File | Action |
|---|---|
| src/crawl.rs | created (Phase 1 Block B implementation — prior pass) |
| src/lib.rs | modified (Phase 1 Block B wiring — prior pass) |
| tests/crawl.rs | created (this fix pass) |
| planning/phase1-blockB/sdlc/reports/task1-implement.md | updated |

## Validation Output

```
cargo fmt --check
(no diff — exit 0)

cargo clippy -- -D warnings
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.05s
(exit 0)

cargo test
running 7 tests
test crawl::tests::file_kind_eq ... ok
test crawl::tests::get_finds_exact_match ... ok
test crawl::tests::invalid_module_filenames ... ok
test crawl::tests::locale_eq ... ok
test crawl::tests::valid_module_filenames ... ok
test crawl::tests::path_ids_sorted_and_deduped ... ok
test crawl::tests::modules_for_filters_by_locale_and_excludes_metadata ... ok
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

Running tests/crawl.rs
running 7 tests
test empty_tree_produces_empty_corpus_and_no_diagnostics ... ok
test bad_filename_uppercase_emits_error ... ok
test bad_filename_file_still_in_corpus ... ok
test bad_filename_missing_nn_prefix_emits_error ... ok
test bad_filename_spaces_emits_error ... ok
test non_content_files_are_skipped ... ok
test good_modules_classified_correctly ... ok
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

Running tests/smoke.rs
running 2 tests
test diagnostic_severity_drives_failure ... ok
test empty_tree_produces_clean_report ... ok
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

cargo build --release
    Finished `release` profile [optimized] target(s) in 0.05s
(exit 0)
```

Status: PASSED

## git diff --stat

```
 tests/crawl.rs  (new file, ~240 lines)
```
