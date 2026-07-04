# Implementation Report — 4.D-sync-comparator-hardening

**Date:** 2026-07-04
**Plan:** planning/4.D-sync-comparator-hardening/tasks.md
**Scope:** Full spec

## What Was Built or Changed
- `src/brain/sync.rs` — hardened `check_sync`'s watermark comparison (~line 211) to
  `source_dt.to_utc() != cache_dt.to_utc()` with a doc comment stating the instant-comparison
  invariant (a `-03:00` and a `Z` watermark denoting the same moment are in sync; no
  `E_SYNC_DRIFT`). The `E_SYNC_DRIFT` message and symmetric `!=` semantics are unchanged.
- `src/brain/sync.rs` — added two regression tests to the inline `#[cfg(test)] mod tests`:
  `same_instant_across_offsets_produces_no_e_sync_drift` (status `timestamp`
  `2026-06-27T00:00:00Z` vs cache `synced_from` `2026-06-26T21:00:00-03:00`, same instant, no
  diagnostics) and `different_instant_across_offsets_produces_e_sync_drift` (`Z` vs `-03:00`
  denoting different instants, exactly one `E_SYNC_DRIFT`).

## Files Created or Modified
| File | Action |
|---|---|
| src/brain/sync.rs | modified |
| planning/4.D-sync-comparator-hardening/sdlc/reports/implement.md | created |

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
$ cargo fmt --check
(no output — clean)

$ cargo clippy -- -D warnings
    Checking okf-core v0.1.0 (.../core/bastion/crates/okf-core)
    Checking mev v0.1.0 (.../core/mev)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.86s

$ cargo test --lib brain::sync
running 15 tests
test brain::sync::tests::date_only_is_rejected ... ok
test brain::sync::tests::datetime_without_offset_is_rejected ... ok
test brain::sync::tests::garbage_value_is_rejected ... ok
test brain::sync::tests::full_rfc3339_with_z_suffix_parses_ok ... ok
test brain::sync::tests::full_rfc3339_parses_ok ... ok
test brain::sync::tests::missing_status_file_produces_e_sync_file_missing ... ok
test brain::sync::tests::missing_cache_doc_produces_e_sync_file_missing ... ok
test brain::sync::tests::date_only_watermark_produces_e_sync_watermark_malformed ... ok
test brain::sync::tests::different_instant_across_offsets_produces_e_sync_drift ... ok
test brain::sync::tests::same_instant_across_offsets_produces_no_e_sync_drift ... ok
test brain::sync::tests::missing_timestamp_produces_e_sync_watermark_missing ... ok
test brain::sync::tests::in_sync_repo_produces_no_diagnostics ... ok
test brain::sync::tests::missing_synced_from_produces_e_sync_watermark_missing ... ok
test brain::sync::tests::drifted_repo_produces_e_sync_drift ... ok
test brain::sync::tests::re_aligning_cache_clears_drift_error ... ok
test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 299 filtered out

(full `cargo test` run: all unit + integration test binaries pass — lib tests plus
tests/brain_sync.rs, tests/brain_validate.rs, tests/crawl.rs, tests/meta.rs, tests/smoke.rs,
tests/structure.rs — 0 failures across the whole suite)

$ cargo build --release
   Compiling mev v0.1.0 (.../core/mev)
    Finished `release` profile [optimized] target(s) in 2.53s
```
Status: PASSED

## Decisions and Trade-offs
- Used `DateTime::to_utc()` (returns `DateTime<Utc>`) rather than `.timestamp()`/
  `.timestamp_nanos_opt()` for the explicit-instant comparison: `to_utc()` normalizes the offset
  away entirely and compares as a genuine UTC instant, which is the clearest, most legible way to
  express the invariant in a doc comment and satisfies the spec's own suggested options
  ("e.g. `source_dt.to_utc()` ... "). No behavior change versus the prior `!=` — chrono's
  `DateTime<FixedOffset>` `PartialEq` was already instant-based (confirmed by the spec's own
  investigation finding and re-confirmed here: all pre-existing sync tests pass unchanged).
- Left `parse_watermark`, the `E_SYNC_DRIFT` message text, and all other branches untouched per
  the spec's explicit scope limits (no directional staleness, no `--sync` wiring changes).

## Follow-up Work
None — out-of-scope items (directional staleness ordering, wiring `--sync` into session-start)
are tracked separately as `BT.1.B` / `BR.1.B` per the spec's Context Pointers.

## git diff --stat
```
 planning/status.md |  5 ++--
 src/brain/sync.rs  | 77 +++++++++++++++++++++++++++++++++++++++++++++++++++++-
 2 files changed, 79 insertions(+), 3 deletions(-)
```
