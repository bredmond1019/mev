---
type: Handoff
title: "Handoff — MV.3B.T done; next is MV.3.L or MV.3B.Q"
description: MV.3B.T (emit-state subcommand, single-derivation engine) is merged (PR #8); next is MV.3.L (structural coverage) or MV.3B.Q (manifest emit).
created: 2026-06-30
---

# Handoff — MV.3B.T done; next is MV.3.L or MV.3B.Q

> **For the next agent:** Read this immediately after `/prime`. Delete this file once consumed.

## What we're doing and why

`mev` is a Rust CLI tool that validates Markdown/MDX for two consumers: the learn-ai site and
the Bastion Brain OKF corpus. Phase 3 / 3B is the Brain integrity layer. This session completed
**MV.3B.T** — the single-derivation emit engine for all generated views declared by the v2
state schema (`emit-state` subcommand, `src/brain/emit.rs`). The emit is designed to be the
fixed point of the validator drift checks: running `mev emit-state --write` then
`mev validate-brain --state` on the same corpus reports zero `W_STATE_FOCUS_DRIFT` /
`W_STATE_ROLLUP_DRIFT`. The project is clean on `main` with 275 passing tests.

## Completed this session

- Ran `/sdlc-flow 3B.T-state-table-rollup-emit` — **6 tasks, all PASS**, 275 tests green.
- **Task 1:** Extracted `derive_focus`, `derive_rollup`, `derive_cross_repo` from
  `check_focus_drift` into standalone public functions in `src/brain/state.rs`; added
  `DerivedFocus { now, next, blocked }` struct; 8 new integration tests in `tests/brain_state.rs`.
- **Task 2:** Created `src/brain/emit.rs` with `EmitError`, `wave_order`, `render_wave_table`,
  `splice_generated`; 18 integration tests in `tests/brain_emit.rs`.
- **Task 3:** Added `EmitAction`/`EmitPlan` types and planners `plan_state_json`,
  `plan_master_plan_tables`, `apply_plan`; 14 more tests (fixed-point, idempotency, dry-run/write).
- **Task 4:** Added `emit_state` library driver in `src/lib.rs` + `emit-state` CLI subcommand
  in `src/main.rs` with `--write` flag (default dry-run); 4 integration tests.
- **Task 5:** Updated `docs/cli.md` (full subcommand reference, sentinel contract, diagnostic
  codes) and `docs/architecture.md` (emit module map, `derive_*` function table, `DerivedFocus`
  type).
- **Task 6:** Confirmed all four harness gates green: `cargo fmt --check`, `cargo clippy -D
  warnings`, `cargo test` (275 tests, 0 failures), `cargo build --release`.
- `/code-review low` — no findings.
- PR #8 opened and merged (merge commit `75abaae`); worktree `trees/3B.T-state-table-rollup-emit-flow`
  removed; branch deleted; `main == origin/main`.

## Remaining work

In priority order:

1. **Brain-side v2 `state.json` re-seed** (5 files in the company-brain repo) — a brain-side
   coordination step, not a mev blocker. Live `mev validate-brain --state` on the real brain
   will fail until those files are re-seeded from v1 → v2 schema. Not blocking next mev blocks.
2. **`MV.3.L`** — structural coverage (`index.md` ↔ directory, governed by D17). Not started.
   Check `planning/master-plan.md` for the spec and wave ordering.
3. **`MV.3B.Q`** — manifest emit: mev emits a canonical file-list + metadata JSON that
   `index_brain.py` consumes (kills the double crawl, D5 extract-once refactor). Not started.
   Depends on `MV.3.J-crawl` (done). Check ordering vs `MV.3.L` in master-plan.
4. Phase 3B blocks `MV.3B.R` (graph emit → Postgres edges) and `MV.3B.S` (graph-aware RAG)
   are further out and depend on `MV.3B.Q`.

## Open questions / choices

- **3.L vs 3B.Q ordering:** `planning/master-plan.md` is the authority. Check the wave/ordering
  table there before picking the next block. Both are unblocked.
- **Sentinel contract for `emit-state`:** `plan_master_plan_tables` only splices if the
  `<!-- BEGIN generated:wave-table -->` sentinels are already present — never invents them. The
  brain-side master-plan files need to be seeded with the sentinel pair before live emit touches
  them.

## Context the next agent needs

- Test count is **275** at session end (`cargo test` from the project root).
- `render_wave_table` (`src/brain/emit.rs:91`) accepts `graph` for API symmetry but uses a
  conservative "treat cross-repo deps as unmet" rule since it only receives one repo's `StateFile`.
  This is by design (recorded in Task 2 decision log).
- `I_EMIT_WROTE` uses `Warning` severity (no `Info` level in `Diagnostic`) — intentional; docs
  already reflect this accurately.
- `AGENT.md` and `GEMINI.md` are untracked files in the repo root — do not commit them unless
  the user asks. They appeared during this session and are not part of mev source.
- The sdlc-flow state for this block lives at `planning/3B.T-state-table-rollup-emit/sdlc/` —
  the spec is fully closed; no residual state to clean up.
- Dispatch precedence in `src/main.rs`: `links → state → graph → sync`.

## First command after `/prime`

`cat planning/master-plan.md` — check the wave/ordering table to confirm whether `MV.3.L` or
`MV.3B.Q` is next, then run `/plan <chosen-block>`.
