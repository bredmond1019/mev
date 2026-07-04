---
type: Plan
title: "Task Spec — Phase 4, Block MV.4.A (Emit foundation)"
description: Lay the emit foundation for the state-sync-loop status generators — generated-marker name constants, a global repo:id→status map, and a fix to render_wave_table's cross-repo blocked bug — that MV.4.B/C/D build on.
doc_id: 4.A-emit-foundation
layer: [engine, factory]
project: mev
status: active
keywords: [emit, splice_generated, marker constants, render_wave_table, cross-repo blocked, state-sync-loop]
related: [state-sync-loop-master-plan, master-plan, status, state-json-schema]
---

# Task Spec — Phase 4, Block MV.4.A (Emit foundation)

**Status:** Done · **Last run:** 2026-07-03 (4 tasks, PASS)

## Goal
Lay the emit foundation for the state-sync-loop status generators: introduce generated-marker name
constants, add a global `repo:id → status` map helper, and fix `render_wave_table`'s cross-repo
`blocked` bug — the shared primitives that `MV.4.B` (project caches + tier rollups) and `MV.4.C`
(HQ board) build on.

## Context Pointers
- **Canonical block definition:** `core/planning/state-sync-loop/master-plan.md` — the cross-repo
  initiative table (MV.4.A row: "Emit foundation: marker-aware `splice_generated` +
  generated-marker constants; global `repo:id→status` map; fix `render_wave_table` cross-repo
  `blocked` bug; unit tests"). This block is Wave 1, no dependencies; it is the spine `MV.4.A →
  {B,C} → E`.
- **Block registration:** `planning/state.json` → Phase 4 track (this repo's mirror).
- **Code the block touches (all in one file):**
  - `src/brain/emit.rs` — `splice_generated` (line ~216, currently takes a free `marker: &str`;
    the sole call site at line ~511 hardcodes `"wave-table"`), `render_wave_table` (line ~90),
    and `plan_master_plan_tables` (line ~473, the caller that has `files` in scope).
  - `render_wave_table`'s **known bug** (lines ~162–172): cross-repo `depends_on` edges are
    *always* treated as unmet (`// Cross-repo: treat as unmet (conservative)`), so a block that
    depends on an already-**closed** cross-repo block is rendered `blocked` incorrectly. The `graph`
    param is accepted but unused (`let _ = graph;` line ~135).
- **Tests:** `tests/brain_emit.rs` — 41 existing tests; 7 call `render_wave_table("repo", &file,
  &graph)` and will need their call sites updated when the signature gains the status-map argument.
- **Standing rules:** `CLAUDE.md` — every code change ships with tests (rule 1); work the
  master-plan sequence (rule 3). Gated checks in `planning/harness.json`.

## Step-by-Step Tasks
See `tasks.json` in this directory — the task list is defined there, not here.

## Acceptance Criteria
- Named generated-marker constants exist in `src/brain/emit.rs` (at minimum `wave-table`, plus the
  marker names `MV.4.B/C` will target: project-cache, tier-rollup, hq-board), and the existing
  `plan_master_plan_tables` call site references the `wave-table` constant instead of a string
  literal — no behavioural change to the emitted table.
- A pure `global_status_map` helper maps every loaded block to its authored status keyed
  `"{repo_slug}:{block_id}"` across **all** state files (cross-repo, not just one repo), with unit
  tests covering multi-repo input and absent-status blocks.
- `render_wave_table` resolves a cross-repo `depends_on` edge against the global status map: a block
  depending on a **closed** cross-repo block renders `open` (not `blocked`); a block depending on
  an **open/absent** cross-repo block renders `blocked`. Same-repo derivation is unchanged.
- `plan_master_plan_tables` builds the global map and threads it into `render_wave_table`; the live
  `emit-state` output for this repo's `master-plan.md` is unchanged except where the cross-repo bug
  was previously mis-rendering `blocked`.
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
