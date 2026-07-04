---
type: Plan
title: "Task Spec — Phase 4, Block MV.4.D (--sync comparator hardening)"
description: Harden validate-brain --sync's watermark comparator to compare explicit UTC instants (never a raw-string or offset-sensitive compare) and close the offset-mismatch test gap — a -03:00 watermark and a Z watermark denoting the same instant must not report E_SYNC_DRIFT.
doc_id: 4.D-sync-comparator-hardening
layer: [engine, factory]
project: mev
status: active
keywords: [sync, watermark, timestamp, instant comparison, E_SYNC_DRIFT, rfc3339, state-sync-loop]
related: [state-sync-loop-master-plan, master-plan, status, D29-mev-brain-validation-engine]
---

# Task Spec — Phase 4, Block MV.4.D (--sync comparator hardening)

**Status:** Not started · **Last run:** never

## Goal
Harden `validate-brain --sync`'s watermark comparator so it compares parsed UTC **instants** (never
a raw-string or offset-sensitive compare) and add the missing regression coverage: a `-03:00`
watermark and a `Z` watermark denoting the same instant must not report `E_SYNC_DRIFT`.

## Context Pointers
- **Canonical block definition:** `core/planning/state-sync-loop/master-plan.md` — MV.4.D row:
  "`--sync` timestamp comparator hardening (parse to instant, never string-compare `-03:00` vs
  `Z`)". Wave 40, **no dependencies** — one of the four immediately-startable fronts.
- **The comparator:** `src/brain/sync.rs` → `check_sync` (~line 87). The two watermarks are parsed
  by `parse_watermark` (~line 31, `DateTime::parse_from_rfc3339` → `DateTime<FixedOffset>`) into
  `source_dt` / `cache_dt`, then compared at **line ~211** (`if source_dt != cache_dt { … E_SYNC_DRIFT … }`).
- **Investigation finding (verified this session):** chrono's `DateTime<FixedOffset>` `PartialEq`/`Ord`
  is already **instant-based** — `2026-06-27T00:00:00Z` and `2026-06-26T21:00:00-03:00` compare
  *equal*; `2026-06-27T00:00:00Z` and `2026-06-27T00:00:00-03:00` compare *unequal*. So the current
  comparison is offset-correct. This block therefore **hardens and documents** that guarantee rather
  than fixing a live bug: make the instant comparison explicit and legible (compare via an explicit
  UTC/`timestamp`-based form + a doc comment stating the invariant) so a future refactor cannot
  silently regress it into a string/offset-sensitive compare, and **close the test gap** — every
  existing sync test (`in_sync_repo_produces_no_diagnostics`, `drifted_repo_produces_e_sync_drift`,
  etc., inline `#[cfg(test)] mod tests` from ~line 231) uses `Z`-suffixed watermarks only; none
  exercises a cross-offset pair.
- **Out of scope:** directional staleness (cache-older-than-source ordering) and wiring `--sync`
  into session-start — those are `BT.1.B` / `BR.1.B`, not this block. Keep the drift check
  symmetric (`!=`), only hardening *how* the two instants are compared.
- **Standing rules:** `CLAUDE.md` — every code change ships with tests (rule 1); decisions are
  append-only (rule 4). Gated checks in `planning/harness.json`.

## Step-by-Step Tasks
See `tasks.json` in this directory — the task list is defined there, not here.

## Acceptance Criteria
- `check_sync`'s watermark comparison compares explicit UTC instants (e.g. `source_dt.to_utc()` /
  `.timestamp()`-based) rather than a bare `DateTime` `!=`, with a doc comment stating the invariant
  ("compared as instants — a `-03:00` and a `Z` watermark denoting the same moment are in sync").
  Behaviour for all currently-passing cases is unchanged (same `E_SYNC_DRIFT` / no-diagnostic
  outcomes for the existing tests).
- New regression tests in `src/brain/sync.rs` cover the offset-mismatch cases:
  - a status `timestamp` of `2026-06-27T00:00:00Z` against a cache `synced_from` of
    `2026-06-26T21:00:00-03:00` (same instant, different offset) produces **no** `E_SYNC_DRIFT`;
  - a `Z` vs `-03:00` pair denoting **different** instants produces `E_SYNC_DRIFT`.
- The existing sync test suite continues to pass unchanged (no regressions to
  `in_sync_repo_produces_no_diagnostics`, `drifted_repo_produces_e_sync_drift`,
  `date_only_watermark_produces_e_sync_watermark_malformed`, etc.).
- All four gated checks pass: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`,
  `cargo build --release`.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes
<filled in as work happens>

## Amendment Log
<!-- Append-only. Pipeline stages append one dated line here when they deviate from the spec. -->
_No amendments yet._
