---
type: Plan
title: State YAML Drift Plan
description: Mini-roadmap for automating state YAML frontmatter reconciliation — ad-hoc, not in master-plan.md.
doc_id: plan-state-yaml-drift
related: [managing-state-drift-yaml]
---

# State YAML Drift — Plan

*Mini-roadmap. Created 2026-07-05. Ad-hoc: not in `planning/master-plan.md`.
See `planning/decisions/D34-adhoc-planning-seam.md`.*

## The Goal, Stated Plainly
Currently, `mev emit-state --write` updates derived tables, but leaves `now`, `next`, and `blocked` YAML frontmatter scalars in `status.md` files untouched, leading to drift (since they are hand-maintained but rarely updated at the tier level). This plan extends the `mev` emit engine to parse the YAML block of `status.md` files and automatically rewrite these scalars to match the derived focus from `state.json`.

## The Destination
A zero-drift state where running `mev emit-state --write` guarantees that every `status.md` file's frontmatter `now`, `next`, and `blocked` fields are perfectly in sync with the derived focus in `state.json`.

## Architecture / Design Overview
We will introduce a new pure helper function `reconcile_status_scalars` (modeled after `reconcile_synced_from`) and a new planner `plan_status_frontmatter` in `core/mev/src/brain/emit.rs`. The planner will resolve each `state.json`'s sibling `status.md`, calculate the derived focus, and emit an action if the frontmatter requires updating. This planner will then be wired into `emit_state` in `src/lib.rs`.

---

## The Block Contract

`/generate-tasks` reads **only the target block's section** below. Every block is self-sufficient
and uses the same skeleton:

- **What** — the scope, in implementation terms.
- **Why** — the motivation (keeps the generator from over- or under-scoping).
- **Files** — *new* vs *modified*, named by path. Load-bearing: tasks sharing a file must be
  serialized (`dependsOn`) or append-only; tasks owning distinct files may run in parallel.
- **Interfaces / shared surface** *(optional)* — shared exports/APIs consumed or added. Omit when
  there is no shared layer.
- **Out of scope** — explicit boundaries; what belongs to a later block or a different effort.
- **Acceptance criteria** — true/false conditions checkable against the diff, ending with the
  project's gating checks passing.

---

## Phase 5 — Status Frontmatter Reconciliation

### Block A — Status Frontmatter Reconciler
- **Block ID:** MV.5.A
- **What:** Add a pure helper `reconcile_status_scalars(original: &str, focus: &Focus) -> String` to `src/brain/emit.rs` that replaces or adds `now`, `next`, and `blocked` fields in the YAML frontmatter block. Add a new planner `plan_status_frontmatter(files, graph, config)` which uses `derive_focus`/`derive_brain_focus` to get the focus for each state file, reads its `status.md` (resolved via `brain.toml` or fallback sibling), and adds an `EmitAction` if the frontmatter needs an update. Finally, wire `plan_status_frontmatter` into `emit_state` in `src/lib.rs`.
- **Why:** Solves the persistent issue where `status.md` YAML scalars drift from the true federated state captured in `state.json`, ensuring both humans and indexers see accurate information at the top of the file without manual intervention.
- **Files:**
  - *Modified* `core/mev/src/brain/emit.rs` — Implement `reconcile_status_scalars` and `plan_status_frontmatter`.
  - *Modified* `core/mev/src/lib.rs` — Call `plan_status_frontmatter` inside `emit_state`.
  - *Modified* `core/mev/tests/brain_emit.rs` — Add unit tests for `reconcile_status_scalars` and integration tests for the planner.
- **Out of scope:** Modifying `state.json` schemas; modifying `docs/projects/<slug>.md` cache files (these use `synced_from` instead of now/next/blocked).
- **Acceptance criteria:**
  - `mev emit-state --write` updates the `now`, `next`, and `blocked` scalars in the YAML frontmatter of `status.md` files to match the derived focus.
  - Re-running `mev emit-state --write` on already synchronized files yields no changes (fixed-point property).
  - Project's gating checks pass (see `planning/harness.json`).

---

## Quick Reference Sequence Table

| Phase | Block | What | Why | Role in destination |
|---|---|---|---|---|
| 5 | A | Status Frontmatter Reconciler | Automate `now`/`next`/`blocked` scalar updates | Eradicates status YAML drift |

---

*Ad-hoc mini-roadmap — run one block or the full train (see Report below).*
