# Review Report — phase1-blockC-task3

**Date:** 2026-06-20
**Spec:** planning/phase1-blockC/tasks.md
**Scope:** Task 3 — Define and validate path `metadata.json` (`FileKind::PathMetadataJson`)
**Verdict:** PASS

## Acceptance Criteria Check

| Criterion | Status | Evidence |
|---|---|---|
| Module `.json` files deserialize into a strict `ModuleMeta`; every missing required field emits the expected diagnostic | SKIP | Task 2 scope — not Task 3's step list |
| Enum violations (difficulty, module type, section type) and format violations each emit the expected diagnostic | SKIP | Task 2 scope — not Task 3's step list |
| Path `metadata.json` files require `id, title, description, level, duration, version, lastUpdated, topics, modules`; each missing field emits the expected diagnostic | MET | `src/meta.rs`: `validate_path_metadata_json()` calls `require_str` for all 7 string fields and explicit `is_none()` checks for `topics`/`modules`; `path_metadata_all_required_fields_reported_when_empty` test asserts all 9 locators |
| `level` validated case-insensitively; live capitalised values (`"Intermediate"`, `"Beginner"`, `"Advanced"`) accepted | MET | `src/meta.rs:324-329`: `is_valid_level()` uses `s.to_lowercase().as_str()`; tests `path_metadata_level_case_insensitive_lowercase` and `path_metadata_level_case_insensitive_capitalized` confirm this |
| Bad `level` value emits precise-locator error | MET | `src/meta.rs:296-304`; test `path_metadata_bad_level_emits_locator` confirms locator `"level"` and `Severity::Error` |
| MDX frontmatter parsed as real YAML; missing block, missing key, malformed YAML each emit error | SKIP | Task 4 scope — not Task 3's step list |
| New fixture-driven tests for path `metadata.json`: good case + each deliberately-broken case; existing Block B and smoke tests stay green | MET | 11 Task-3 tests in `meta::tests` (lines 718–846): good path, level case-insensitive x2, bad level, missing level, missing modules, missing topics, malformed duration, invalid JSON, all-required-when-empty, dispatch; 39/39 pass including all prior Block B and smoke tests |
| All four harness gates are green | MET | fmt exit 0, clippy exit 0, test exit 0 (39 passed), build --release exit 0 |
| CLAUDE.md standing rule 1: every task ships with tests | MET | 11 new unit tests covering Task 3 functionality |
| No fabricated metrics, quotes, or emoji in implementation | MET | Source reviewed; no violations found |

## Fresh Test Results

**fmt** (`cargo fmt --check`): EXIT 0 — PASS

**clippy** (`cargo clippy -- -D warnings`): EXIT 0 — PASS
```
Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.05s
```

**test** (`cargo test`): EXIT 0 — PASS
```
running 39 tests
... (all 39 tests passed, including 11 new Task-3 path-metadata tests)
test result: ok. 39 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

tests/crawl.rs: 7 passed
tests/smoke.rs: 2 passed
```

**build** (`cargo build --release`): EXIT 0 — PASS
```
Finished `release` profile [optimized] target(s) in 0.02s
```

## Verdict: PASS

All in-scope acceptance criteria for Task 3 are fully met. The `PathMeta` struct correctly models all 9 required fields as `Option` for per-field diagnostics. `validate_path_metadata_json()` enforces all required fields, validates `level` case-insensitively via `is_valid_level()`, and validates `duration` format. Eleven new unit tests cover the good case, each missing-field scenario, the case-insensitivity contract, invalid enum, malformed duration, and invalid JSON. All four harness gates pass. Criteria for Task 2 (Module JSON) and Task 4 (MDX frontmatter) are correctly deferred.

## Issues Found

None.

## Next Steps

Task 3 is complete. Proceed to Task 4: parse and validate MDX frontmatter as real YAML (`FileKind::ModuleMdx`) using `serde_yaml`, adding the required frontmatter key checks and integrating them into `validate_file()`.
