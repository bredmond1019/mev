---
type: Plan
title: "Task Spec — Phase 4, Block MV.4.C (HQ Operating Board generator)"
description: Add the plan_hq_board emit planner — render a NOW/NEXT/BLOCKED Operating Board from the brain's derived focus + cross_repo[] edges and splice it into the HQ board doc's hq-board sentinel, pulling the board bastion status previously produced into mev.
doc_id: 4.C-hq-board-emit
layer: [engine, factory]
project: mev
status: active
keywords: [emit, plan_hq_board, operating board, derive_brain_focus, cross_repo, state-sync-loop]
related: [state-sync-loop-master-plan, 4.A-emit-foundation, master-plan, status, state-json-schema]
---

# Task Spec — Phase 4, Block MV.4.C (HQ Operating Board generator)

**Status:** Done · **Last run:** 2026-07-04 (PASS, 3 tasks)

## Goal
Add the `plan_hq_board` emit planner: render a NOW/NEXT/BLOCKED Operating Board from the brain's
derived `focus` + `cross_repo[]` edges and splice it into the HQ board doc's `hq-board` sentinel —
pulling the board `bastion status` previously produced into mev.

## Context Pointers
- **Canonical block definition:** `core/planning/state-sync-loop/master-plan.md` — MV.4.C row:
  "`plan_hq_board` — NOW/NEXT/BLOCKED Operating Board from brain `focus` + `cross_repo[]` (pulls
  `bastion status`/Block V into mev)". Wave 41; `depends_on: MV.4.A` (done) — reuses A's
  marker-aware splice + `global_status_map`. "Block V" = `MV.3B.V` (`emit-graph` ships resolved
  cross-repo edges); the board consumes those resolved edges.
- **Foundation from MV.4.A (`planning/4.A-emit-foundation/`, Done):** `src/brain/emit.rs` now has
  `pub mod markers` with `HQ_BOARD = "hq-board"` (docstring: "the cross-repo HQ status board").
- **The pattern to follow:** `plan_master_plan_tables` (`src/brain/emit.rs` ~line 473) — locate the
  target doc relative to the brain root, `read_to_string`, `splice_generated(&original,
  markers::HQ_BOARD, &rendered)`, push an `EmitAction` only when content changed (fixed-point), and
  emit `W_EMIT_NO_SENTINEL` when the file/sentinels are absent. Separate the pure renderer from the
  planner, mirroring `render_wave_table` (pure) + `plan_master_plan_tables` (locate/splice/diagnose).
- **Data sources (in `src/brain/state.rs` + okf-core):** `derive_brain_focus(scope, config, graph,
  files) -> Focus` (the repo-tagged union of in-scope children's focus, HQ scope =
  `TierScope::All`); `derive_cross_repo(files) -> Vec<CrossRepoEdge>`; `tier_scope_for`. Types:
  `Focus { now, next, blocked }`, `Block { id, title, status, note, repo, blocked_by }`,
  `CrossRepoEdge { from: Endpoint, to: Endpoint, note }`, `Endpoint { repo, id }`.
- **Target-doc resolution:** the HQ Operating Board doc lives at the brain root (the HQ, not a tier
  sub-brain). Resolve the brain root from the brain-kind `StateSource` / `brain.toml` via `config`,
  as the sibling planners do. Do NOT wire `plan_hq_board` into `emit_state` — that is MV.4.E's job;
  this block only defines the renderer + planner + their tests.
- **Standing rules:** `CLAUDE.md` — every code change ships with tests (rule 1). Gated checks in
  `planning/harness.json`.

## Step-by-Step Tasks
See `tasks.json` in this directory — the task list is defined there, not here.

## Acceptance Criteria
- A pure `render_hq_board` function exists in `src/brain/emit.rs` taking the brain-derived `Focus`
  (from `derive_brain_focus`) and the `cross_repo[]` edges (from `derive_cross_repo`) and returning
  the Operating Board Markdown: a NOW, a NEXT, and a BLOCKED section, each listing its repo-tagged
  blocks (`repo:id — title`), with blocked entries annotated by their cross-repo `blocked_by` /
  matching `cross_repo[]` edge. Rendered deterministically (stable order) with no trailing newline,
  matching the `render_wave_table` convention.
- A `plan_hq_board` planner exists in `src/brain/emit.rs`: it locates the HQ board doc at the brain
  root, builds the board via `render_hq_board(derive_brain_focus(...), derive_cross_repo(...))`, and
  splices it into the `markers::HQ_BOARD` sentinel — pushing an `EmitAction` only when content
  changed (fixed-point) and emitting `W_EMIT_NO_SENTINEL` when the doc/sentinels are absent.
- Both are covered by tests in `tests/brain_emit.rs` over synthetic multi-repo fixtures: a
  render-produces-expected-NOW/NEXT/BLOCKED case, a splice-into-sentinel case, a
  missing-sentinel-warns case, and an already-correct-no-action (fixed-point) case.
- `plan_hq_board` is not wired into `emit_state` yet (MV.4.E owns that); the new functions are
  `pub` and called only from tests.
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
