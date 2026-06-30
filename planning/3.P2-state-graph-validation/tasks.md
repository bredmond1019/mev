---
type: Plan
title: "MV.3.P2 — State-graph expansion validation task spec"
description: Decomposed task spec for the v2 state-graph validator — depends_on DAG, cycle detection, derived-blocked enforcement, backlog nodes, and derivation-drift warnings, extending MV.3.P.
doc_id: 3-P2-state-graph-validation-tasks
layer: [factory, brain]
project: mev
status: active
keywords: [state.json, depends_on, DAG, cycle detection, backlog, derivation drift, validate-brain, Phase 3]
related: [master-plan, status, 3-P-state-integrity-tasks, state-json-schema]
---

# Task Spec — Phase 3, Block P2 (State-graph expansion validation)

**Status:** Not started · **Last run:** never

## Goal
Extend `mev validate-brain --state` to guard the **v2 state schema** — the full work-block DAG: validate `depends_on` resolution + acyclicity, reject the now-derived `blocked` status, check status consistency and backlog nodes, and warn on `focus`/rollup derivation drift.

## Context Pointers
- **Plan section:** `planning/master-plan.md` → Phase 3 → **MV.3.P2**. Governed by **D29** (mev is the single validation engine; read-only — D25) and brain **D36** (state.json is the work-block graph).
- **v2 schema (the contract this validates):** `core/planning/state-schema.md` (Schema v2, 2026-06-30). Decisions settled in `core/planning/state-graph-design-decisions/notes.md` — read the **Resolutions** section. Key points this block enforces:
  - `tracks[].blocks[]` carries `depends_on[]` (the authoritative DAG, same edge forms as the existing `BlockedBy` enum) + a `wave` integer + optional `origin`.
  - Authored block `status` ∈ `{open, in_progress, closed}` — **`blocked` is derived, never authored**.
  - `focus`, brain `repos[]`/`cross_repo[]` are **derived views**; drift is **warning-only** here (the warn→error flip is deferred to after the `/log-work` writer ships — do not make drift an error).
  - HQ-only `backlog[]` nodes (`slug` key, status `idea·ready·promoted`, `depends_on[]`, `block?` pointer on promote); promoted blocks carry `origin:{type:backlog,slug}`.
- **The module being extended:** `src/brain/state.rs` — currently the **v1** model. Real symbols to migrate/extend:
  - `TrackBlock { id, title, status: Option<String> }` — add `depends_on: Vec<BlockedBy>`, `wave: Option<i64>`, `origin: Option<Origin>`.
  - `Block` (focus entries) uses field name **`block`** — v2 standardizes on **`id`** (serde rename; cascades to `Endpoint.block`, `build_state_graph`, `check_state_graph`, `check_rollup`).
  - `BlockedBy::Block { repo, id, what } | External { what }` — **reuse verbatim** as the `depends_on` entry type (the edge form is identical).
  - `const VALID_STATUSES = ["open","in_progress","blocked","closed"]` — split into authored-block (no `blocked`) vs the derived view.
  - `build_state_graph` derives `BlockedBy` edges from `focus.*.blocked_by[]` today — v2 sources the DAG from `tracks[].blocks[].depends_on[]`.
  - `check_schema`, `check_state_graph`, `check_rollup`, `discover_state_files`, `load_state`, `StateGraph`/`StateNode`/`StateEdge`/`StateEdgeKind` — all in `state.rs`.
- **Pipeline wiring:** `src/lib.rs` → `validate_brain_state` runs discovery → schema → graph build/check → rollup. New checks append into the same `Report`. **No new CLI flag** — reuse `--state` (`src/main.rs` unchanged).
- **Sequencing (the chicken-and-egg the schema flagged):** the 5 live `state.json` files are still **v1**. Develop + test this block entirely against **v2 fixtures**; `mev validate-brain --state` on the *live* brain will (correctly) fail until the brain-side re-seed lands. The live-clean run is therefore **NOT** an acceptance criterion here — it belongs to the coordinated re-seed step.
- **Standing rules (`CLAUDE.md`):** every new fn ships with tests (rule 1); decisions append-only (rule 4); harness gates in `planning/harness.json`.

## Step-by-Step Tasks

### 1. Migrate the serde model to v2
- In `src/brain/state.rs`: add `depends_on: Vec<BlockedBy>` (`#[serde(default)]`), `wave: Option<i64>`, and `origin: Option<Origin>` to `TrackBlock`; define `Origin { #[serde(rename="type")] kind: String, slug: String }` (plain struct; `kind` is `"backlog"` today).
- Rename the focus `Block.block` field → `id` and `Endpoint.block` → `id` (standardize on `id` per v2); update every internal reference so the crate compiles (`build_state_graph`, `check_state_graph`, `check_rollup`, `check_schema`). Pure mechanical cascade.
- Add the `Backlog` struct (`slug, title, repo, #[serde(rename="type")] kind, status, #[serde(default)] depends_on: Vec<BlockedBy>, block: Option<String>, notes`) and `backlog: Vec<Backlog>` (`#[serde(default)]`) on `StateFile`.
- Migrate the in-file `#[cfg(test)]` fixtures (`leaf_json`, `core_brain_json`, `hq_brain_json`, etc.) to v2: `id` in focus, `depends_on` on track blocks, no authored `status:"blocked"`.
- **Owns:** `src/brain/state.rs` (model + cascading rename + its unit-test fixtures).

### 2. Source the DAG from `depends_on` + split the status enums
- `build_state_graph`: build `BlockedBy`-kind edges from each `tracks[].blocks[].depends_on[]` `{type:block}` entry (`from` = the owning block's `repo:id`, `to_ref` = `{entry.repo}:{entry.id}`); skip `{type:external}` entries (leaves, not edges/nodes). Keep `CrossRepo` edges from brain `cross_repo[]`. Focus `blocked_by` is no longer an edge source (it is a derived view in v2).
- `check_schema`: authored `tracks[].blocks[].status` ∈ `{open, in_progress, closed}` — emit **`E_STATE_AUTHORED_BLOCKED`** when a track block's status is `"blocked"` (it must derive); validate `depends_on` entry well-formedness (reuse the existing `{type:block}` repo/id non-empty check); validate `backlog[].status` ∈ `{idea, ready, promoted}` (**`E_STATE_SCHEMA_BAD_STATUS`**).
- Unit tests: depends_on edges built correctly; external entries excluded; authored `status:"blocked"` flagged; bad backlog status flagged; clean v2 file passes.
- **Owns:** `src/brain/state.rs` (`build_state_graph` + `check_schema`).

### 3. Cycle detection + reusable readiness/topo ordering
- Add `detect_cycles(graph: &StateGraph) -> Vec<Diagnostic>`: DFS over `BlockedBy` edges; on a back-edge emit **`E_STATE_CYCLE`** naming the cycle path (e.g. `A → B → A`).
- Add a **reusable** `ready_order(graph, files) -> Vec<String>` (or similar): a block is *ready* iff every `type:block` dep is `closed` AND it has zero `type:external` deps; order ready+`open` blocks by `wave`, tiebreak track order then array order. **Build it standalone** (forward-compat: `MV.3B.T` serializes this exact ordering — do not bury it inside a check).
- Unit tests: a cyclic `depends_on` chain is flagged with its path; an acyclic DAG passes; `ready_order` returns the correct wave-ordered ready set (incl. external-dep exclusion).
- **Owns:** `src/brain/state.rs` (new `detect_cycles` + `ready_order`).

### 4. Status consistency + backlog-node integrity
- Add a status-consistency check: a `closed` block with a `type:block` `depends_on` target that is not `closed` → **`E_STATE_STATUS_INCONSISTENT`**.
- Extend the graph/dangling checks to **backlog nodes**: a backlog `depends_on` `{type:block}` that resolves to no node → `E_STATE_DANGLING_BLOCKED_BY` (existing family, backlog as the source); a `status:"promoted"` backlog node whose `block` pointer resolves to no `tracks[]` node → **`E_STATE_DANGLING_PROMOTION`**.
- Unit tests: closed-depends-on-open flagged; dangling backlog dep flagged; orphan promoted-node flagged; a clean promote (node `block` matches a real block carrying `origin`) passes.
- **Owns:** `src/brain/state.rs` (new check fns; may extend `check_state_graph` to fold in backlog edge sources).

### 5. Derivation-drift warnings (focus recompute) (in progress)
- Add `check_focus_drift(file, graph) -> Vec<Diagnostic>`: recompute the expected `focus` from authored `tracks[]` (`now` = `in_progress` blocks; `blocked` = blocks with an unmet `depends_on`; `next` = `ready_order` ∩ `open`) and compare to the stored `focus` (block-id sets only, mirroring `check_rollup`'s set comparison). On mismatch emit **`W_STATE_FOCUS_DRIFT`** (warning — exit 0). Reuse `ready_order` from task 3.
- Leave `check_rollup` (`W_STATE_ROLLUP_DRIFT`) as-is for now; note in code that v2 will eventually derive `repos[]` from child `tracks[]` (deferred to `MV.3B.T`).
- Unit tests: a stored `focus` that disagrees with the derived view → one `W_STATE_FOCUS_DRIFT`; an in-sync `focus` → none; drift never raises exit code.
- **Owns:** `src/brain/state.rs` (new `check_focus_drift`).

### 6. Wire into the pipeline + integration tests
- `src/lib.rs` → `validate_brain_state`: after the existing schema/graph/rollup passes, append `detect_cycles`, the status-consistency check, the backlog-node checks, and `check_focus_drift` into the same `Report`. (No `--state` flag change; `src/main.rs` untouched.)
- `tests/brain_state.rs`: migrate existing integration fixtures to v2 and add end-to-end cases — a cyclic `depends_on` → exit 1 with `E_STATE_CYCLE`; authored `status:"blocked"` → `E_STATE_AUTHORED_BLOCKED`; closed-depends-on-non-closed → `E_STATE_STATUS_INCONSISTENT`; dangling backlog dep + orphan promotion flagged; `focus` drift → warning (exit 0); a clean v2 corpus passes; `--json` envelope well-formed.
- **Owns:** `src/lib.rs`, `tests/brain_state.rs`.

### 7. Documentation
- `docs/cli.md`: add the new diagnostic codes (`E_STATE_CYCLE`, `E_STATE_AUTHORED_BLOCKED`, `E_STATE_STATUS_INCONSISTENT`, `E_STATE_DANGLING_PROMOTION`, `W_STATE_FOCUS_DRIFT`) to the diagnostics reference; note that `--state` now validates the v2 `depends_on` DAG (acyclicity + derived-blocked) and warns on focus drift.
- `docs/architecture.md`: update the `src/brain/state.rs` one-liner to mention DAG/cycle/backlog if it enumerates the module's checks.
- **Owns:** `docs/cli.md`, `docs/architecture.md`.

### 8. Validate
- Run the Validation Commands listed below and confirm all pass.

## Acceptance Criteria
- The serde model deserializes v2 files: `id` in focus, `depends_on`/`wave`/`origin` on track blocks, `backlog[]` on brain files; the full pre-existing test suite passes after fixture migration.
- A `depends_on` cycle is flagged `E_STATE_CYCLE` with the cycle path (exit 1); an acyclic DAG passes.
- An authored `tracks[].blocks[].status: "blocked"` is flagged `E_STATE_AUTHORED_BLOCKED` (blocked is derived, not authored).
- A `closed` block whose `type:block` `depends_on` target is not `closed` is flagged `E_STATE_STATUS_INCONSISTENT`.
- A backlog `depends_on` resolving to no block is flagged (`E_STATE_DANGLING_BLOCKED_BY`); a `promoted` backlog node whose `block` resolves to nothing is flagged (`E_STATE_DANGLING_PROMOTION`).
- `focus` that disagrees with its derivation from `tracks[]` produces `W_STATE_FOCUS_DRIFT` (warning, exit 0); never an error.
- `ready_order` is a standalone reusable function (the `MV.3B.T` topo-emit input), not buried in a check.
- All harness gates green (`fmt`, `clippy -D warnings`, `cargo test`, release build).
- **Out of scope / not an AC:** a clean `--state` run on the *live* (still-v1) brain — that follows the coordinated brain-side re-seed; and the drift warn→error flip (deferred until the `/log-work` derived-view writer exists).

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
