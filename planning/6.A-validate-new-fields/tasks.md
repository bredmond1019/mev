---
type: LocalContext
title: Task Spec — MV.6.A Validate the new routing/priority fields + schema doc
description: Decomposed task spec for MV.6.A — add mev validate-brain --state policy checks (priority range, due ISO date, sdlc_workflow/model enums) for the four fields okf-core OK.1.A introduced, plus schema-doc coverage.
doc_id: spec-6a-validate-new-fields
layer: [factory]
project: mev
status: active
keywords: [MV.6.A, state validation, priority, due, sdlc_workflow, model, E_STATE]
related: [master-plan, state-json-schema]
---

# Task Spec — Phase 6, Block MV.6.A

**Status:** Not started · **Last run:** never

## Goal
Add `mev validate-brain --state` policy validation for the four fields okf-core `OK.1.A`
introduced — `priority` (0–3), `due` (ISO `YYYY-MM-DD`), `sdlc_workflow` (enum), `model` (enum) —
with their `E_STATE_*` error codes, and document all four in the canonical schema doc.

## Context Pointers
- **Plan:** `planning/master-plan.md` → Phase 6 → **MV.6.A** (the authoritative block definition,
  incl. the exact enum sets and error codes). Also the Execution Guide row (mechanical block; cargo
  gates catch mistakes).
- **Upstream dependency (satisfied):** okf-core `OK.1.A` has landed — `priority: Option<u8>`,
  `due: Option<String>`, `sdlc_workflow: Option<String>`, `model: Option<String>` already exist on
  `okf_core::TrackBlock` (`../okf-core/src/state.rs:90–150`), re-exported into mev at
  `src/brain/state.rs:48`. This block adds **only the validation policy + docs**, not the struct
  fields.
- **Where policy lives:** the existing `E_STATE_*` guards are in `src/brain/state.rs` (`check_schema`,
  `check_state_graph`, `check_status_consistency`, …), each a pure `fn(...) -> Vec<Diagnostic>` wired
  into `validate_brain_state` in `src/lib.rs:358`. The new check follows the same shape.
- **Authoritative surface:** validate `tracks[].blocks[]` (the authored DAG). `focus` entries are a
  **derived** view (`plan_state_json`) — do not separately validate them (avoids double-reporting).
- **Enum sets (from the block, authoritative — note these differ from okf-core's lenient doc
  examples):**
  - `sdlc_workflow` ∈ {`none`, `patch`, `task`, `run`, `flow`} → `E_STATE_SDLC_WORKFLOW_ENUM`
  - `model` ∈ {`sonnet`, `gemini-pro`, `gemini-flash`, `either`} → `E_STATE_MODEL_ENUM`
  - `priority` ∈ 0..=3 → `E_STATE_PRIORITY_RANGE`
  - `due` parses as `%Y-%m-%d` (use `chrono::NaiveDate`, already a dep — see `src/brain/sync.rs`) →
    `E_STATE_DUE_FORMAT`
- **Standing rules:** every code change ships with tests (Rule 1); the schema doc + its `index.md`
  live cross-repo in `core/planning/` (one tier up from this repo) and must carry OKF frontmatter.
- **Out of scope (from the block):** the struct fields (okf-core `OK.1.A`); priority inheritance /
  effective-priority (MV.7.A); the board emit + carry-through (MV.6.B); board columns for
  `model`/`sdlc_workflow`; teaching plan commands to author the fields (BR.2.B); the per-block
  *rationale* (stays in plan markdown, not a state field).

## Step-by-Step Tasks
See `tasks.json` in this directory — the task list is defined there, not here.

## Acceptance Criteria
- A `tracks[].blocks[]` block with `priority` 0–3, a valid ISO `due`, and valid `sdlc_workflow`/
  `model` values passes `mev validate-brain --state` (no new diagnostics).
- `priority: 4` raises `E_STATE_PRIORITY_RANGE`.
- `due: "2026-13-99"` raises `E_STATE_DUE_FORMAT` (invalid calendar date), and a non-ISO string like
  `"Q3"` also raises it.
- `sdlc_workflow: "pipeline"` raises `E_STATE_SDLC_WORKFLOW_ENUM`.
- `model: "gpt"` raises `E_STATE_MODEL_ENUM`.
- Each of the four fields is validated only when present (absent = no diagnostic — the fields are
  optional).
- `core/planning/state-schema.md` documents all four fields, the P0–P3 priority rubric, the model
  rule-of-thumb, and the recurring-work exclusion; the doc's `index.md` row is updated if the scope
  changed.
- `cargo test`, `cargo clippy -- -D warnings`, `cargo fmt --check`, `cargo build --release` all pass.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes
- The schema-doc + `index.md` edits (Task 4) live in `core/planning/`, **one tier up from the mev git
  repo**. If this spec runs in an isolated worktree (`/sdlc-flow`), those two files sit outside the
  worktree — apply the doc edit in the main `core/` checkout as a companion step, or run this block
  with an in-place engine (`/sdlc-task` / `/sdlc-run`) so the cross-repo path is reachable.

## Amendment Log
<!-- Append-only. Pipeline stages append one dated line here when they deviate from the spec. -->
_No amendments yet._
