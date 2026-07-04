# Review Report — 4.D-sync-comparator-hardening

**Date:** 2026-07-04
**Spec:** planning/4.D-sync-comparator-hardening/tasks.md
**Scope:** Full spec
**Verdict:** PASS

## Acceptance Criteria Check
| Criterion | Status | Evidence |
|---|---|---|
| `check_sync`'s watermark comparison compares explicit UTC instants (e.g. `source_dt.to_utc()`/`.timestamp()`-based) rather than a bare `DateTime` `!=`, with a doc comment stating the invariant | MET | `src/brain/sync.rs:206-215` — `if source_dt.to_utc() != cache_dt.to_utc()` preceded by a doc comment stating the instant-comparison invariant ("compared as instants ... a `-03:00` and a `Z` watermark denoting the same moment are in sync") |
| Behaviour for all currently-passing cases is unchanged (same `E_SYNC_DRIFT`/no-diagnostic outcomes) | MET | Pre-existing tests `in_sync_repo_produces_no_diagnostics`, `drifted_repo_produces_e_sync_drift`, `date_only_watermark_produces_e_sync_watermark_malformed`, `re_aligning_cache_clears_drift_error` all pass unchanged (fresh `cargo test` run) |
| New regression test: `Z` vs `-03:00` same instant → no `E_SYNC_DRIFT` | MET | `src/brain/sync.rs:582-609` `same_instant_across_offsets_produces_no_e_sync_drift` — asserts `diags.is_empty()`; passes |
| New regression test: `Z` vs `-03:00` different instants → `E_SYNC_DRIFT` | MET | `src/brain/sync.rs:611-638` `different_instant_across_offsets_produces_e_sync_drift` — asserts exactly one diagnostic; passes |
| Existing sync test suite continues to pass unchanged | MET | Fresh `cargo test` run — all `brain::sync::tests` (15 tests incl. 2 new) pass, 0 failed |
| All four gated checks pass: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, `cargo build --release` | MET | Re-ran all four fresh in this review — all exit 0 (see below) |

CLAUDE.md standing rules: rule 1 (every change ships with tests) satisfied — two new regression tests added covering the exact scenarios described in the spec. No OKF/frontmatter or identity-integrity concerns apply to this source-only change (no new `.md` docs added beyond the SDLC report artifacts already tracked by the pipeline).

## Fresh Test Results
```
$ cargo fmt --check
(no output — clean, exit 0)

$ cargo clippy -- -D warnings
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.07s
(exit 0, no warnings)

$ cargo test
... (full workspace suite: lib tests, tests/brain_sync.rs, tests/brain_validate.rs,
     tests/crawl.rs, tests/meta.rs, tests/smoke.rs, tests/structure.rs, doc-tests)
brain::sync::tests — 15 passed; 0 failed (includes both new offset-mismatch tests)
Overall: 0 failed across every test binary (exit 0)

$ cargo build --release
    Finished `release` profile [optimized] target(s) (exit 0)
```

## Verdict: PASS
All acceptance criteria are fully met: the comparator now explicitly normalizes both watermarks to UTC via `.to_utc()` before comparing, with a doc comment stating the instant-comparison invariant; the two required regression tests (same-instant-cross-offset → no drift; different-instant-cross-offset → drift) were added and pass; the full pre-existing sync test suite passes unchanged; and all four gated checks (`fmt`, `clippy -D warnings`, `test`, `build --release`) were freshly re-run in this review and all pass with exit 0.

## Issues Found
None.

## Next Steps
Proceed to `/document` and `/log-work` to close out this block.
