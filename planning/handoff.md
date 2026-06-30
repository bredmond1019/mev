---
type: Handoff
title: "Handoff — State-graph expansion: schema v2 settled, MV.3.P2 specced (gated on re-seed)"
description: State-graph expansion decisions settled + state-schema v2 written (core repo); MV.3.P2 + MV.3B.T added to plan and MV.3.P2 specced. Next is the v2 re-seed + the MV.3.P2 flow.
created: 2026-06-30
---

# Handoff — State-graph expansion: schema v2 settled, MV.3.P2 specced (gated on re-seed)

> **For the next agent:** Read this immediately after `/prime`. Delete this file once consumed.

## What we're doing and why

This session was a cross-repo **design + planning** pass on the **state-graph expansion** (brain
decision **D36**): promoting `state.json` from a focus-snapshot-with-partial-edges into the
*authoritative work-block dependency graph*, where `focus` / brain rollup / master-plan tables become
**derived views** over an authored `depends_on` DAG, and backlog tickets join the same graph. The four
open design decisions were deliberated and settled, the canonical schema was rewritten to **v2** (in the
`core` repo), and the two mev follow-on blocks were added to the master-plan and `MV.3.P2` was specced.
The next agent picks up at the **mev implementation** of the validator (`MV.3.P2`) plus the coordinated
**re-seed** of the five live `state.json` files. (`MV.3.K` link integrity also landed this session via a
parallel worktree — PR #6 merged, 237 tests — but that is concurrent work, not part of this thread.)

## Completed this session

- **Settled 4 state-graph decisions + 7 refinements** — recorded in the **Resolutions** section of
  `core/planning/state-graph-design-decisions/notes.md` (core repo, commit `4693dce`):
  - D1 backlog identity → **Option B** (slug key; node persists `status:promoted` + `block:` pointer; the
    block carries `origin:{type:backlog,slug}`); D2 backlog scope → **HQ-only**; D3 table generation →
    **mev emits** (sentinel-delimited); D4 `produces`/`consumes` → **deferred**.
  - Refinements: standardize on `id` (not `block`) everywhere; **`blocked` is derived, never authored**
    (authored status enum = `open·in_progress·closed`); **one authored edge vocab = `depends_on`**
    (`blocked_by` survives only as a derived focus view); external deps are hand-cleared + skipped by
    dangling/cycle checks; **derivation drift is warning-only** until the writer ships; `wave` is
    orthogonal to `tracks[]`; `cross_repo[]` is generated; re-seed is the risk moment (run mev after each).
- **Rewrote `core/planning/state-schema.md` to v2** (core repo, `4693dce`): Authored-vs-derived table;
  block vocab gains `depends_on`/`wave`/`origin`; `backlog[]` vocab; both templates rewritten; derivation
  rules + external-dep + maintenance notes.
- **Added MV.3.P2 + MV.3B.T to the mev master-plan** (mev, commit `b1fb953`) + Quick Reference rows.
- **Specced `MV.3.P2`** (mev, commit `7f20ca8`): `planning/3.P2-state-graph-validation/tasks.md` — 8 tasks
  (v2 serde migration → DAG-from-`depends_on` → cycle detection + reusable `ready_order` →
  status-consistency + backlog checks → focus-drift warnings → pipeline wiring + integration tests →
  docs → validate). `planning/index.md` updated.
- **(Concurrent)** `MV.3.K` link integrity implemented, reviewed, merged (PR #6); a post-review fix moved
  `--links` to highest dispatch precedence; test count now **237**.

## Remaining work

In priority order (per the user's stated intent — state-graph expansion is the active thread):

1. **Re-seed the 5 live `state.json` files to v2** (brain-side: `core` repo + the nested leaf repos):
   transcribe each repo's master-plan "Depends-on" prose into `depends_on` JSON; rename `block`→`id` in
   focus; drop authored `status:"blocked"`; add `wave`. **Chicken-and-egg:** `MV.3.P2`'s validator and
   this re-seed must land in the same window — until the re-seed, `mev validate-brain --state` on the
   *live* brain will fail against the v1 files (expected, not a regression).
2. **Run `/sdlc-flow 3.P2-state-graph-validation`** — consider `/breakdown` on task 1 first (the v2 model
   migration is a wide cascading `block`→`id` rename + in-file fixture migration). Build/test against v2
   fixtures; the live re-seed is the parity check, not an acceptance criterion.
3. **Brain-side writers (separate, after the validator):** `/generate-master-plan`, `/plan`, `/chore`
   populate `depends_on`+`wave` at block-authoring; `/backlog-ticket` + promote write `backlog[]`;
   `/log-work` emits derived `focus`. Then **`MV.3B.T`** (table/rollup emit) — the block that lets
   `MV.3.P2` flip derivation drift from warning → error.
4. **Independent alternatives:** `MV.3.L` (structural coverage) and `MV.3B.Q` (manifest emit) remain the
   other unstarted Phase 3 / 3B blocks if the state-graph thread isn't the next pick.

## Open questions / choices

- **Sequencing:** is `MV.3.P2` (+ re-seed) the next block to build, or `MV.3.L` / `MV.3B.Q`? The user
  signalled state-graph (P2) is the intent, but the master-plan's strict order still lists L/Q earlier.
  Not a blocker — all three are unblocked (only P2's *live-clean* run is gated, on the re-seed).
- **`E_STATE_STATUS_INCONSISTENT` severity:** the spec makes "closed block depending on a non-closed dep"
  an **error**; it can legitimately occur if a dep was reopened. Downgrade to a warning if it proves noisy
  (flagged in the spec).

## Context the next agent needs

- **Two repos were touched.** mev commits (`b1fb953`, `7f20ca8`) are on mev `main`. The schema +
  decisions live in the **`core` repo** (commit `4693dce`) — a *separate* git repo at
  `/Users/brandon/Dev/agentic-portfolio/core`. `state-schema.md` is **not** in the mev tree.
- **`MV.3.P2` extends `src/brain/state.rs`**, currently the **v1** model. The spec's Context Pointers name
  every real symbol to migrate (`TrackBlock`, focus `Block.block`→`id`, `BlockedBy` reused as the
  `depends_on` type, `VALID_STATUSES`, `build_state_graph`, `check_*`). Read the spec before the file.
- **Tests:** 237 green on mev `main`. Harness: `cargo fmt --check && cargo clippy -- -D warnings && cargo test && cargo build --release`.
- The `.agents/skills/prime/` untracked dir is an unrelated skill-sync artifact — commit or ignore per your flow.

## First command after `/prime`

`/breakdown planning/3.P2-state-graph-validation/tasks.md` — then `/sdlc-flow 3.P2-state-graph-validation`, **coordinated with** the brain-side v2 re-seed of the 5 `state.json` files (see Remaining work #1).
