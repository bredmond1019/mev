---
type: Plan
title: Task Spec — MV.4.E (spine terminus — wire all planners into emit_state)
description: Wire plan_project_caches / plan_tier_rollups / plan_hq_board into emit_state, preserve the fixed-point, and add a brain-wide close-A-unblocks-B ripple integration test across every generated surface.
doc_id: 4.E-emit-state-wiring-tasks
layer: [factory, brain]
project: mev
status: active
keywords: [emit-state, state-sync-loop, planners, ripple test, fixed-point, sentinels]
related: [state-sync-loop-master-plan, master-plan, status]
---

# Task Spec — Phase 4, Block MV.4.E

**Status:** Not started · **Last run:** never

## Goal
Wire the three already-built planners (`plan_project_caches`, `plan_tier_rollups`, `plan_hq_board`) into the `emit_state` driver so a single `mev emit-state --write` refreshes every human-read status surface, preserving the fixed-point guarantee, and prove it with a brain-wide close-A-unblocks-B ripple integration test.

## Context Pointers
- **Block definition:** `core/planning/state-sync-loop/master-plan.md` — MV.4.E ("Wire all planners into `emit_state`; preserve fixed-point; brain-wide ripple integration test (close-A-unblocks-B across every surface)", Wave 42, depends on MV.4.B/C/D — all Done).
- **Driver to change:** `src/lib.rs::emit_state` (~line 482) — currently runs only `plan_state_json` + `plan_master_plan_tables` (steps 4–5). The three MV.4.B/MV.4.C planners exist and are unit/integration-tested but were deliberately left unwired ("MV.4.E's job").
- **Planner signatures** (`src/brain/emit.rs`): `plan_project_caches(root, files, graph, config)` (809), `plan_tier_rollups(files, graph, config)` (934), `plan_hq_board(files, graph, config)` (1021). Each returns an `EmitPlan`; each is already fixed-point + emits `W_EMIT_NO_SENTINEL` on a missing sentinel. `apply_plan(&plan, write)` applies/dry-runs.
- **Existing tests:** `tests/brain_emit.rs` `mod task4_emit_state` and `mod task4_tier_scoping_integration` already exercise `emit_state` end-to-end (incl. a fixed-point test). New MV.4.E module goes here.
- **Docs:** `docs/cli.md` §`emit-state` (line 488) describes the derivation engine — must be updated to list the newly wired surfaces.
- **Standing rules:** `CLAUDE.md` — every behaviour change ships with tests; all four harness gates green.

## Step-by-Step Tasks
See `tasks.json` in this directory — the task list is defined there, not here.

## Acceptance Criteria
- `emit_state` (`src/lib.rs`) calls all five planners — `plan_state_json`, `plan_master_plan_tables`, `plan_project_caches`, `plan_tier_rollups`, `plan_hq_board` — and applies each via `apply_plan(&plan, write)`, threading `root`/`&config`/`&graph`/`&loaded` correctly (`plan_project_caches` needs `root`).
- The `emit_state` doc comment is updated to name all five generated surfaces (leaf `focus`, brain rollup, master-plan tables, project caches, tier rollups, HQ board).
- A new integration test in `tests/brain_emit.rs` builds a multi-repo fixture corpus (brain HQ + tier + ≥2 leaf project repos with `state.json` + the sentinel-bearing Markdown surfaces), flips a block another repo `depends_on` to `closed`, runs `emit_state(dir, true)` **once**, and asserts the ripple landed in **every** surface: the leaf cache focus line + `synced_from`, the tier rollup row, the HQ board BLOCKED→NEXT move, the dependent repo's `focus`, and the master-plan wave/status cell.
- A fixed-point assertion: a second `emit_state(dir, true)` over the already-emitted corpus produces no further file changes (no `I_EMIT_WROTE`).
- `docs/cli.md` §`emit-state` lists the project-cache, tier-rollup, and HQ-board surfaces the command now writes.
- All four harness gates pass: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, `cargo build --release`.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```
<!-- Add any spec-specific checks above the standard project checks. -->

## Notes
<filled in as work happens>

## Amendment Log
<!-- Append-only. Pipeline stages append one dated line here when they deviate from the spec. -->
_No amendments yet._
