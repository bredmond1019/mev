# Implementation Report — phase1-blockC-task2

**Date:** 2026-06-19
**Plan:** planning/phase1-blockC/tasks.md
**Scope:** Task 2 — Define and validate the `ModuleMeta` struct (`FileKind::LearnModuleJson`)

## What Was Built or Changed
- Added the strict module-`.json` serde model in `src/meta.rs`: `ModuleFile` (top-level
  `metadata` + `sections[]`), `ModuleMeta` (the metadata block), and `ModuleSection`. All
  required fields are modelled as `Option` so a *missing* key becomes a precise-locator
  diagnostic instead of aborting deserialization. Unknown keys are tolerated (no
  `deny_unknown_fields`) per the live-file note in the spec.
- Added `validate_module_json()` plus `validate_module_metadata()` and `validate_module_section()`
  helpers that emit one `error` `Diagnostic` per missing required field
  (`metadata.{id,pathId,title,description,duration,type,difficulty,order,objectives,tags,version,lastUpdated}`,
  `metadata`/`sections` blocks, non-empty `sections[]`, and per-section `id`/`type`/`order`),
  each with a precise locator (e.g. `metadata.duration`, `sections[0].id`).
- Added hand-rolled regex-equivalent check helpers (no `regex` crate, matching the `is_valid_*`
  style in `crawl.rs`): `is_kebab_case` (`^[a-z0-9]+(-[a-z0-9]+)*$`), `is_valid_duration`
  (`^\d+\s+(minutes?|hours?)$`), `is_valid_difficulty`, `is_valid_module_type`,
  `is_valid_section_type`, plus `require_str`/`non_empty`/`missing` presence helpers.
- Wired `validate_file()` to dispatch `FileKind::LearnModuleJson` to `validate_module_json`;
  `PathMetadataJson` (Task 3) and `ModuleMdx` (Task 4) remain no-ops until their tasks.
- Added 18 unit tests covering the good module, each broken case (missing duration, bad
  difficulty enum, non-kebab id, malformed duration, empty sections, section missing id, bad
  section type, bad module type, missing metadata/sections blocks, invalid JSON, all-fields-empty
  metadata), and the format/enum helpers — asserting exact locators and severities, not just counts.

## Files Created or Modified
| File | Action |
|---|---|
| src/meta.rs | modified |

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
cargo fmt --check        -> clean (FMT_OK)
cargo clippy -D warnings -> Finished, no warnings
cargo test               -> lib: 27 passed; crawl: 7 passed; smoke: 2 passed; doc: 0; 0 failed
cargo build --release    -> Finished `release` profile [optimized]
```
Status: PASSED

## Decisions and Trade-offs
- Modelled all required fields as `Option<T>` rather than relying on serde's own
  "missing field" error. This is the only way to emit a separate precise-locator diagnostic for
  each absence (serde stops at the first). Typed `Option<String>`/`Option<i64>`/`Option<Vec<..>>`
  keep the struct strict while remaining tolerant of the broken fixtures the spec enumerates
  (none of which break typed deserialization).
- `require_str`/`non_empty` treat whitespace-only strings as missing, so an empty `title: ""`
  is reported as missing — consistent with the "missing or empty" intent for required fields.
- Used edition-2024 let-chains (`if let Some(x) = .. && !valid`) for the enum/format checks to
  satisfy clippy's `collapsible_if` under `-D warnings`.
- Kept Task 3/4 dispatch as no-ops and Task 5 wiring (into `validate()`) untouched — strictly
  Task 2 scope.

## Follow-up Work
- Task 3: path `metadata.json` struct + validation.
- Task 4: MDX frontmatter YAML parsing.
- Task 5: wire `validate_file()` into `validate()`.
- Task 6: temp-dir fixture integration tests in `tests/`.

## git diff --stat
```
 src/meta.rs | 510 ++++++++++++++++++++++++++++++++++++++++++++++++++++++++--
 1 file changed, 499 insertions(+), 11 deletions(-)
```
