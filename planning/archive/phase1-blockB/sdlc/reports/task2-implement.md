# Implementation Report — phase1-blockB-task2

**Date:** 2026-06-19
**Plan:** planning/phase1-blockB/tasks.md
**Scope:** Task 2 — Classify files during the walk

## What Was Built or Changed

Task 2's scope (per the execution plan) is adding `classify()` and `crawl()` to `src/crawl.rs`.
Both functions were already present in the merged Task 1 implementation; this task verified the
implementation is complete and correct per the spec, ran all validation gates, and recorded the
report.

Key implementation verified in `src/crawl.rs`:

- `classify(root, entry_path) -> Option<ContentFile>` (private free function, ~50 lines):
  - Strips the path prefix to get `rel`, collects UTF-8 components (returns `None` on non-UTF8).
  - First component must be `"paths"` — anything else returns `None` (skips `schemas/`, `shared/`,
    top-level `*.md`, etc.).
  - Second component is `path_id`.
  - If third component is `"pt-BR"`, sets `locale = PtBr` and advances tail start; else `locale = En`.
  - Tail matched as: `["metadata.json"]` → `PathMetadataJson`; `["modules", file]` with `.json` →
    `LearnModuleJson`; with `.mdx` → `ModuleMdx`; anything else → `None`.
  - `module_id` = file stem (without extension) for module files; `None` for `PathMetadataJson`.
  - Path derivation logic is documented in the module-level doc comment for Block D pairing.

- `crawl(root: &Path) -> (Corpus, Vec<Diagnostic>)` (public entry point):
  - Uses `walkdir::WalkDir::new(root)` to walk the tree.
  - Walk errors are surfaced as `Diagnostic::error` and do not panic or abort the walk.
  - Skips directories (files only).
  - Calls `classify()` per file; skips `None` results (non-content files produce no diagnostics).
  - Calls `check_filename()` on each classified file (implemented in full by Task 1; stub phase was
    skipped since Task 1 implemented all of crawl.rs in a single pass).
  - Returns `(Corpus, Vec<Diagnostic>)` — corpus accumulates all `ContentFile`s; diagnostics
    accumulate filename violations and any walk errors.

## Files Created or Modified

| File | Action |
|---|---|
| src/crawl.rs | verified (classify + crawl already present from Task 1 merge) |
| planning/phase1-blockB/sdlc/reports/task2-implement.md | created |

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
(no diff — exit 0)

cargo clippy -- -D warnings
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.06s
(exit 0)

cargo test
running 7 tests
test crawl::tests::file_kind_eq ... ok
test crawl::tests::get_finds_exact_match ... ok
test crawl::tests::modules_for_filters_by_locale_and_excludes_metadata ... ok
test crawl::tests::locale_eq ... ok
test crawl::tests::invalid_module_filenames ... ok
test crawl::tests::path_ids_sorted_and_deduped ... ok
test crawl::tests::valid_module_filenames ... ok
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

Running tests/crawl.rs
running 7 tests
test empty_tree_produces_empty_corpus_and_no_diagnostics ... ok
test bad_filename_missing_nn_prefix_emits_error ... ok
test bad_filename_file_still_in_corpus ... ok
test bad_filename_uppercase_emits_error ... ok
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
    Finished `release` profile [optimized] target(s) in 0.07s
(exit 0)
```

Status: PASSED

## Decisions and Trade-offs

- **Task 1 over-implementation:** Task 1's implementation created `src/crawl.rs` with the full
  classify + crawl + check_filename + unit tests in a single pass, rather than stopping at the
  type definitions. This means Task 2's scope arrived already merged. The approach here is to
  verify, not re-implement.
- **No stub for check_filename:** The breakdown specified crawl() should stub check_filename during
  Task 2, replacing it in Task 3. Since Task 1 wrote the full implementation, the stub phase was
  never needed and the code is functionally complete earlier than planned.
- **Non-UTF8 paths:** classify() returns `None` for any path with a non-UTF8 component, silently
  skipping it. This is acceptable for the content tree (all files are ASCII-named); walk errors
  (permissions, etc.) still produce diagnostics.

## Follow-up Work

- Task 3: filename checks are already implemented (no-op task — verify and report).
- Task 4: validate() wiring is already done in src/lib.rs (no-op task — verify and report).
- Task 5: tests/crawl.rs already created with 7 tests (no-op task — verify and report).
- Task 6: validate gate run.

## git diff --stat

```
(no source changes — implementation already present from Task 1 merge)
 planning/phase1-blockB/sdlc/reports/task2-implement.md | new file
```
