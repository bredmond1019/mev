# SDLC Workflow Report — 4.D-sync-comparator-hardening

**Date:** 2026-07-04
**Spec:** 4.D-sync-comparator-hardening
**Task scope:** All tasks
**Pipeline started from:** implement
**Review attempts:** 1 of 3 max

## Final Verdict
PASS — `check_sync`'s watermark comparison was hardened to an explicit UTC-instant compare with a documented invariant, both required offset-mismatch regression tests were added and pass, the pre-existing sync suite is unchanged, and all four gated checks passed fresh in review.

## Stage Results

| Stage | Status | Report | Commit | Notes |
|---|---|---|---|---|
| implement | completed | planning/4.D-sync-comparator-hardening/sdlc/reports/implement.md | c991a30 | Hardened check_sync's watermark comparison to explicit UTC-instant compare (`source_dt.to_utc() != cache_dt.to_utc()`) with an invariant doc comment; added 2 regression tests |
| test (attempt 1) | completed | planning/4.D-sync-comparator-hardening/sdlc/reports/test.md | — | All validation gates passed: fmt, clippy, test (314 tests), build --release |
| review (attempt 1) | PASS | planning/4.D-sync-comparator-hardening/sdlc/reports/review.md | — | All acceptance criteria MET; all four gating checks (fmt, clippy, test, build) re-run fresh and green; no issues found |
| ui-test | SKIPPED | — | — | uiTest disabled in harness.json |
| document | completed | planning/4.D-sync-comparator-hardening/sdlc/reports/document.md | b0dea52 | Patched docs/cli.md's --sync section to describe instant-based comparison and updated the E_SYNC_DRIFT locator wording |

## Key Findings
This block hardened (not fixed) `check_sync`'s watermark comparator in `src/brain/sync.rs`. Investigation confirmed chrono's `DateTime<FixedOffset>` `PartialEq`/`Ord` was already instant-based — a `-03:00` and a `Z` watermark denoting the same moment already compared equal — so there was no live drift-detection bug. The change makes that guarantee explicit via `.to_utc()` normalization plus a doc comment, closing the test-coverage gap the spec identified (no existing test exercised a cross-offset pair). Decision: `to_utc()` was chosen over `.timestamp()`/`.timestamp_nanos_opt()` as the clearest way to express "compare as instants" in code and documentation, per the spec's own suggested options. Scope was kept tight per the spec: no changes to `parse_watermark`, the `E_SYNC_DRIFT` message text, directional staleness ordering, or `--sync` wiring into session-start (those remain `BT.1.B`/`BR.1.B`). No bilingual-parity concerns apply (source-only, no learn-ai content touched).

## Files Modified
- `src/brain/sync.rs` — hardened the watermark comparison to `source_dt.to_utc() != cache_dt.to_utc()` with an invariant doc comment; added 2 regression tests (`same_instant_across_offsets_produces_no_e_sync_drift`, `different_instant_across_offsets_produces_e_sync_drift`)

## Docs Updated
- `docs/cli.md` — `--sync` section: clarified that `timestamp`/`synced_from` are compared as explicit UTC instants (each side normalized via `.to_utc()`), not raw strings; updated the `E_SYNC_DRIFT` locator row wording from "their values differ" to "denote different instants". No NEEDS_REVIEW flags — docs audit found `docs/okf-schema.md`, `docs/brain-toml.md`, and `docs/architecture.md` all clean (no changes needed).

## Commits (this pipeline run)
```
b0dea52 docs: update docs for 4.D-sync-comparator-hardening
c991a30 feat: implement 4.D-sync-comparator-hardening
caf52f1 chore: add spec for 4.D-sync-comparator-hardening
```
