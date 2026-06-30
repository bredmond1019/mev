---
type: Handoff
title: "Handoff — MV.3.P2 merged; brain-side v2 re-seed is the next thread"
description: MV.3.P2 (state-graph v2 validator) is merged (PR #7); the gating next step is the brain-side re-seed of the 5 live state.json files to v2, then MV.3.L / MV.3B.Q / MV.3B.T.
created: 2026-06-30
---

# Handoff — MV.3.P2 merged; brain-side v2 re-seed is the next thread

> **For the next agent:** Read this immediately after `/prime`. Delete this file once consumed.

## What we're doing and why
`MV.3.P2` (state-graph v2 expansion validator) is **complete and merged** (PR #7). `src/brain/state.rs`
now validates the **v2** `state.json` schema: an authored `depends_on[]` DAG on track blocks, cycle
detection, derived-`blocked` enforcement, backlog-node integrity, and focus-drift warnings — `focus`,
rollups, and master-plan tables become *derived views* over the authored DAG (brain decision **D36**).
The validator was built and tested entirely against **v2 fixtures**, so it passes clean — but the
**five live `state.json` files are still v1**, so a live `mev validate-brain --state` against the real
brain will (correctly) fail until those files are re-seeded to v2. That re-seed is the active next thread.

## Completed this session
- Ran `/sdlc-flow 3.P2-state-graph-validation` to completion — **8 tasks, PASS verdict, 275 tests** green.
  v2 serde migration (`block`→`id` with `#[serde(alias = "block")]`, `TrackBlock.depends_on`/`wave`/`origin`,
  `Backlog`/`Origin` structs) → DAG edges re-sourced from `tracks[].blocks[].depends_on[]` →
  `detect_cycles` (DFS, `E_STATE_CYCLE`) + reusable `ready_order` → `check_status_consistency` +
  `check_backlog_integrity` → `check_focus_drift` (`W_STATE_FOCUS_DRIFT`) → pipeline wiring in
  `validate_brain_state` (9 steps) → docs → validate.
- `/code-review low` on the integrated branch → **no code findings**. Full gating suite (fmt, clippy
  `-D warnings`, 275 tests) green.
- **Post-review doc fix** (commit `1edbd21`): corrected `docs/architecture.md` — `check_focus_drift`
  signature was missing its 4th `files` arg; backlog integrity was wrongly credited to `check_state_graph`
  (it's the separate `check_backlog_integrity`); added the missing `check_status_consistency` +
  `check_backlog_integrity` rows; fixed the pipeline-step description.
- Merged PR #7 (merge commit `460d0cd`); fast-forwarded local `main` (clean FF this time, no unpushed
  local commits). Removed worktree `trees/3.P2-state-graph-validation-flow`, deleted the branch.
  Local + remote `main` both at `460d0cd`.
- Refreshed `planning/status.md` (MV.3.P2 Done, PR #7 + doc-fix note).

## Remaining work
In priority order (state-graph expansion is the active thread per D36):
1. **Re-seed the 5 live `state.json` files to v2** (brain-side: `core` repo + nested leaf repos):
   transcribe each repo's master-plan "Depends-on" prose into `depends_on` JSON; rename focus `block`→`id`
   (the serde alias keeps v1 readable, but author canonical `id`); drop any authored `status:"blocked"`;
   add `wave`. Run `mev validate-brain --state` after each file to catch cycles/dangling/drift as you go.
2. **Brain-side writers (after the re-seed):** `/generate-master-plan`, `/plan`, `/chore` populate
   `depends_on`+`wave` at block authoring; `/backlog-ticket` + promote write `backlog[]`; `/log-work`
   emits derived `focus`. Then **`MV.3B.T`** (table/rollup emit) — the block that lets `MV.3.P2` flip
   focus-drift from warning → error.
3. **Independent alternatives** if not continuing the state-graph thread: `MV.3.L` (structural coverage,
   `index.md` ↔ dir, D17) and `MV.3B.Q` (manifest emit) remain the other unstarted Phase 3 / 3B blocks.

## Open questions / choices
- **Sequencing:** continue the state-graph thread (re-seed → `MV.3B.T`), or pick `MV.3.L` / `MV.3B.Q`?
  All are unblocked; only P2's *live-clean* `--state` run is gated (on the re-seed).
- **`E_STATE_STATUS_INCONSISTENT` severity:** the spec makes "closed block depending on a non-closed dep"
  an **error**; it can legitimately occur if a dep is reopened. Downgrade to a warning if it proves noisy.

## Context the next agent needs
- 275 tests green on `main`. Harness: `cargo fmt --check && cargo clippy -- -D warnings && cargo test`.
- `mev validate-brain --state` failing on the live brain is **expected** until the v2 re-seed (the
  validator and fixtures are v2; the live files are v1).
- The v2 schema contract lives in the **`core` repo** at `core/planning/state-schema.md` (a *separate*
  git repo, not in the mev tree). Dispatch precedence in `src/main.rs` is `links → state → graph → sync`.
- The brain cache (`core/docs/projects/mev.md`) + `core` tier rollup were synced by `/log-work` and live
  in the parent brain repo — commit those there per your normal flow.

## First command after `/prime`
`cat ../planning/state-schema.md` (review the v2 contract, then start the 5-file re-seed; run
`cargo run -- validate-brain --state ~/Dev/agentic-portfolio` after each file). Alternatively, pick the
next mev block with `cat planning/master-plan.md`.
