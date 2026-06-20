# Implementation Report — phase1-blockC-task4

**Date:** 2026-06-20
**Plan:** planning/phase1-blockC/tasks.md
**Scope:** Task 4

## What Was Built or Changed

- `src/meta.rs` — added `MdxFrontmatter` struct, `extract_frontmatter` helper, and
  `validate_module_mdx` function; updated `validate_file` dispatch to route
  `FileKind::ModuleMdx` to the new validator; updated the placeholder test
  `validate_file_mdx_is_clean_until_task4` to `validate_file_dispatches_mdx_to_frontmatter_validator`;
  added 9 new MDX-specific unit tests.

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
**Result:** PASSED

## Decisions and Trade-offs

- `extract_frontmatter` handles the edge case where the closing `---` fence immediately follows
  the opening fence (empty frontmatter block) by checking if `rest` starts with `---` before
  searching for `\n---` patterns. This avoids a false "unterminated" error on `---\n---\n`.
- `serde_yaml` is used for real YAML parsing per spec; no custom YAML parser.
- `MdxFrontmatter` uses `Option<String>` for all required fields (same pattern as
  `ModuleMeta`/`PathMeta`) so each absence yields its own precise-locator diagnostic rather than
  a single deserialization failure.
- `difficulty` and `duration` format validation reuse the same `is_valid_difficulty` and
  `is_valid_duration` helpers already shared by the JSON validators.
- `validate_file` dispatch is now exhaustive (no `_ => Vec::new()` wildcard) — the compiler will
  catch any future `FileKind` variant that lacks a handler.

## Follow-up Work

- Task 5: wire `validate_file` into `validate()` so the diagnostics appear in the `Report`
  (currently `validate()` in `lib.rs` only collects filename diagnostics from the crawl).

## git diff --stat

```
 src/meta.rs | 327 ++++++++++++++++++++++++++++++++++++++++++++++++++++++++++--
 1 file changed, 320 insertions(+), 7 deletions(-)
```
