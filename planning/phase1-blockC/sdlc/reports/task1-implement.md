# Implementation Report — phase1-blockC-task1

**Date:** 2026-06-19
**Plan:** planning/phase1-blockC/tasks.md
**Scope:** Task 1 — Add a `validate` (struct/frontmatter) module

## What Was Built or Changed
- Added `src/meta.rs`: the Block C struct/frontmatter-validation module, kept separate from
  `crawl.rs` (which stays focused on the walk). It holds:
  - `read_content(&ContentFile) -> Result<String, Diagnostic>` — reads a classified file's
    contents and, on any read failure (missing file, permission error, non-UTF-8 bytes), returns
    a single `error`-severity `Diagnostic` located at the file's `rel` path instead of panicking
    or propagating `Err`, so one unreadable file never aborts the run.
  - `validate_file(&ContentFile) -> Vec<Diagnostic>` — the per-file dispatch entry point. Task 1
    reads the file and surfaces read failures only; per-kind serde/enum/format/frontmatter checks
    are layered on by Tasks 2-4, and Task 5 wires this into `validate()`.
- Re-exported `validate_file` from `src/lib.rs` (`mod meta;` + `pub use meta::validate_file;`).
- Added 4 unit tests in `src/meta.rs` covering the core Task 1 logic: read success, read failure
  diagnostic (severity/file/locator/message), `validate_file` single-error-on-read-failure, and
  readable-file-is-clean (Task 1 baseline).

## Files Created or Modified
| File | Action |
|---|---|
| src/meta.rs | created |
| src/lib.rs | modified |

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
cargo fmt --check        -> clean (FMT_CLEAN)
cargo clippy -- -D warnings -> Finished, no warnings
cargo test               -> lib: 11 passed (incl. 4 new meta::tests); crawl.rs: 7 passed;
                            smoke.rs: 2 passed; doc-tests: 0; 0 failed
cargo build --release    -> Finished `release` profile [optimized]
```
Status: PASSED

## Decisions and Trade-offs
- `validate_file` is intentionally NOT yet wired into `validate()` — wiring is Task 5 scope, so
  `main.rs` and the `validate()` public contract are untouched. `validate_file` is `pub` and
  re-exported, so it raises no dead-code warning under `clippy -D warnings`.
- `read_content` is `pub(crate)` (internal helper for later tasks) while `validate_file` is the
  public entry point. The per-kind match dispatch deliberately returns an empty `Vec` for now;
  the struct/frontmatter checks belong to Tasks 2-4.
- Tests follow the existing repo style (manual `std::env::temp_dir` fixtures, no new dev-deps);
  `ContentFile` is constructed directly since its fields are `pub`.

## Follow-up Work
- Tasks 2-4: define `ModuleMeta` / path `metadata.json` serde structs and MDX YAML frontmatter
  parsing with field/enum/format diagnostics.
- Task 5: dispatch `validate_file` over `corpus.files` inside `validate()` and append to `Report`.

## git diff --stat
```
 src/lib.rs | 2 ++
 1 file changed, 2 insertions(+)
```
(plus new untracked file `src/meta.rs`)
