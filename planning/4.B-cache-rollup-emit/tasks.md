---
type: Plan
title: "Task Spec — Phase 4, Block MV.4.B (project-cache + tier-rollup generators)"
description: Add the plan_project_caches and plan_tier_rollups emit planners — splice each project's derived focus-line + synced_from watermark into its docs/projects/<slug>.md project-cache sentinel, and each tier's rollup rows into its tier-rollup sentinel.
doc_id: 4.B-cache-rollup-emit
layer: [engine, factory]
project: mev
status: active
keywords: [emit, plan_project_caches, plan_tier_rollups, synced_from, project cache, tier rollup, state-sync-loop]
related: [state-sync-loop-master-plan, 4.A-emit-foundation, master-plan, status, state-json-schema]
---

# Task Spec — Phase 4, Block MV.4.B (project-cache + tier-rollup generators)

**Status:** Done · **Last run:** 2026-07-04 (3 tasks, PASS)

## Goal
Add two emit planners — `plan_project_caches` and `plan_tier_rollups` — that write each project's
derived focus-line + `synced_from` watermark into its `docs/projects/<slug>.md` project-cache
sentinel, and each tier's rolled-up status rows into its tier-rollup sentinel.

## Context Pointers
- **Canonical block definition:** `core/planning/state-sync-loop/master-plan.md` — MV.4.B row:
  "`plan_project_caches` + `plan_tier_rollups` generators (write cache focus-line + `synced_from`,
  and tier rollup rows, into sentinels)". Wave 41; `depends_on: MV.4.A` (done) — reuses A's
  marker-aware splice + `global_status_map`.
- **Foundation from MV.4.A (`planning/4.A-emit-foundation/`, Done):** `src/brain/emit.rs` now has
  `pub mod markers` with `PROJECT_CACHE = "project-cache"` (docstring: `docs/projects/<slug>.md` in
  the brain) and `TIER_ROLLUP = "tier-rollup"`, plus `global_status_map`.
- **The pattern to follow:** `plan_master_plan_tables` (`src/brain/emit.rs` ~line 473) — locate a
  target doc relative to the state file / brain root, `read_to_string`, `splice_generated(&original,
  markers::X, &rendered)`, push an `EmitAction` only when content changed (fixed-point), and emit
  `W_EMIT_NO_SENTINEL` when the file/sentinels are absent. Planners return an `EmitPlan`
  (`actions` + `diagnostics`); no IO until `apply_plan`.
- **Data sources (in `src/brain/state.rs` + okf-core):** `derive_focus` (leaf headline →
  now/next/blocked ids); `derive_rollup(scope, config, existing, graph, files) -> Vec<RepoRollup>`
  (tier-scoped, non-destructive) for tier rows; `tier_scope_for`; `RepoRollup { repo, tier, now,
  next, blocked }`, `Block { id, title, status, note, repo, blocked_by }`, `Focus { now, next,
  blocked }`. The `synced_from` watermark (D29) is the child project state file's `updated` field
  (`StateFile.updated`); the frontmatter field is `OkfFrontmatter.synced_from`
  (`core/bastion/crates/okf-core/src/frontmatter.rs`).
- **Target-doc resolution:** project cache = `<brain-root>/docs/projects/<slug>.md`; tier rollup =
  the tier sub-brain's rollup doc. Resolve the brain root the way the sibling planners do (from the
  brain-kind `StateSource` / `brain.toml` via `config`). Do NOT wire these planners into
  `emit_state` — that is MV.4.E's job; this block only defines the planner functions + their tests.
- **Standing rules:** `CLAUDE.md` — every code change ships with tests (rule 1). Gated checks in
  `planning/harness.json`.

## Step-by-Step Tasks
See `tasks.json` in this directory — the task list is defined there, not here.

## Acceptance Criteria
- A `plan_project_caches` planner exists in `src/brain/emit.rs`: for each project-kind repo it
  locates `docs/projects/<slug>.md`, splices the repo's derived focus-line (from `derive_focus`)
  into the `markers::PROJECT_CACHE` sentinel region, and updates the doc's frontmatter
  `synced_from` to the child state file's `updated` watermark. It returns an `EmitPlan` whose
  `actions` cover only docs whose content actually changed (fixed-point: re-running over
  already-correct content yields no action), and it emits `W_EMIT_NO_SENTINEL` for a target doc
  that is missing or lacks the sentinels — never splicing into arbitrary prose.
- A `plan_tier_rollups` planner exists in `src/brain/emit.rs`: for each tier it renders the
  tier-scoped rollup rows (from `derive_rollup`) and splices them into the tier doc's
  `markers::TIER_ROLLUP` sentinel, with the same fixed-point + missing-sentinel behaviour.
- Both planners are covered by unit/integration tests in `tests/brain_emit.rs` over synthetic
  fixtures with the sentinels present: a splice-produces-expected-content case, a
  missing-sentinel-warns case, and an already-correct-no-action (fixed-point) case for each.
- Neither planner is wired into `emit_state` yet (MV.4.E owns that); `git grep` shows the new
  functions are `pub` and called only from tests.
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
