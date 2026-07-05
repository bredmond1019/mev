---
type: LocalContext
title: Task Spec — MV.6.B Unified priority-ranked HQ board view + DUE-SOON lane
description: Decomposed task spec for MV.6.B — carry priority/due onto derived focus Blocks and emit a unified NOW/NEXT/BLOCKED/DUE-SOON HQ board region unioning engineering + business blocks, tagged [BIZ]/[ENG], sorted by priority/due/wave.
doc_id: spec-6b-generate-hq-board
layer: [factory, console]
project: mev
status: active
keywords: [MV.6.B, unified board, DUE-SOON, priority, due, emit-state, BIZ, ENG]
related: [master-plan, spec-6a-validate-new-fields, statify-business-master-plan]
---

# Task Spec — Phase 6, Block MV.6.B

**Status:** Done · **Last run:** 2026-07-05 — 6/6 tasks passed, review PASS

## Goal
Carry `priority`/`due` onto the derived focus `Block`s and extend `mev emit-state` with one unified
HQ Operating Board region — NOW / NEXT / BLOCKED / **DUE-SOON** unioning every repo's blocks
(including the business track), each row tagged `[BIZ]`/`[ENG]`, `NEXT` sorted by
`(priority asc, due asc, wave asc)`.

## Context Pointers
- **Plan:** `planning/master-plan.md` → Phase 6 → **MV.6.B** (authoritative block: the two parts,
  the sort key, the DUE-SOON window, the untouched surfaces). Execution Guide row: `/sdlc-flow`,
  **Sonnet** — "hardest v1 block; must match the sentinel/emit architecture precisely, union across
  repos, render idempotently."
- **Cross-repo program seam:** `../../planning/statify-business/master-plan.md` (HQ program plan,
  D43). Load-bearing facts grounded there: **business ops live as the `business`-tier brain's own
  `tracks[]`** (line 84 — "No pseudo-repo, no engine change for structure; HQ already carries its own
  `BR.*` track this exact way"); the `[BIZ]`/`[ENG]` tag is **derived from the source repo's tier**
  (`business` tier → `[BIZ]`, everything else → `[ENG]`).
- **Depends on:** MV.6.A (the `priority`/`due` fields validated) and — transitively — okf-core
  `OK.1.A`, which already added `priority: Option<u8>` / `due: Option<String>` to both
  `okf_core::TrackBlock` and `okf_core::Block` (re-exported at `src/brain/state.rs:48–54`). The fields
  exist; this block only **copies** them onto the derived focus Block and consumes them.
- **Part 1 — carry-through (two code paths).** A focus `Block` is rehydrated from a `TrackBlock` id in
  **two** places, both currently hardcoding `due: None, priority: None`:
  - `src/brain/emit.rs::derived_focus_for` (~emit.rs:497) — leaf/per-repo focus.
  - `src/brain/state.rs::derive_brain_focus` (~state.rs:1544) — the **brain/HQ union** focus that
    `plan_hq_board` and the new unified board consume (this is the load-bearing one for the board
    sort; the block's `~emit.rs:497` pointer is approximate).
  Both build a `title_map`/`id_index` from the child's `tracks[]`; extend that lookup to also carry
  the source block's `priority`/`due` and set them on the constructed `Block`.
- **Part 2 — unified region.** New sentinel region (`markers::UNIFIED_BOARD = "unified-board"`),
  **separate** from the existing `hq-board` sentinel (which MV.4.C renders and which stays untouched,
  along with the per-domain lanes and `/biz-status`). The union source is `derive_brain_focus` at
  `TierScope::All` (already dedups + tags each Block with its source repo slug — see
  `derive_brain_focus_tags_each_block_with_its_source_repo`). Map repo slug → tier via `config.repos`
  for the `[BIZ]`/`[ENG]` tag.
- **Board host:** `../../planning/status.md` (HQ root, **two tiers up from this repo**) already hosts
  the `hq-board` sentinel (lines 40–103); the new `unified-board` sentinel is added there too. Follow
  `plan_hq_board`'s pattern: locate the HQ brain file (`tier_scope_for(...) == TierScope::All`), splice
  with the same fixed-point / `W_EMIT_NO_SENTINEL` semantics, then wire into `emit_state` (`src/lib.rs`,
  ~line 492) via `apply_plan` in stable order after `hq_board_plan`.
- **NEXT sort without a `wave` field.** The focus `Block` carries no `wave`, but `derive_focus`'s
  `next` list is already wave-ordered (via `ready_order`). So a **stable** sort of `next` keyed
  `(priority asc [None last], due asc [None last])` yields `(priority, due, wave)` with wave as the
  implicit tiebreak — no new field needed.
- **DUE-SOON determinism.** DUE-SOON = blocks in the unified union whose `due` parses (ISO
  `YYYY-MM-DD`) and is ≤ `today + 14 days` (overdue included, surfaced louder). The pure renderer
  takes `today: chrono::NaiveDate` as a parameter (tests pass a fixed date); the plan layer supplies
  `chrono::Local::now().date_naive()`. chrono is already a dep (see `src/brain/sync.rs`).
- **Standing rules:** every code change ships with tests (Rule 1). The HQ `status.md` edit is
  cross-repo (see Notes).

## Step-by-Step Tasks
See `tasks.json` in this directory — the task list is defined there, not here.

## Acceptance Criteria
- Derived focus `Block`s (from both `derived_focus_for` and `derive_brain_focus`) carry the source
  `TrackBlock`'s `priority`/`due` (asserted by unit tests), where present.
- `mev emit-state --write` renders a **new** `unified-board` region unioning business + engineering
  rows, each correctly tagged `[BIZ]` (business-tier source) or `[ENG]` (any other tier).
- `NEXT` is ordered by `priority`, then `due`, then `wave`; a P1 business block sorts above a P2
  engineering block.
- `DUE-SOON` lists open blocks whose `due` is within ~14 days of the reference date; an overdue open
  block is listed (surfaced louder).
- The existing `hq-board` sentinel region, the per-domain lanes, and `/biz-status` are unchanged.
- Re-running `emit-state` is idempotent — the `unified-board` region is byte-identical on a second
  pass (fixed point), asserted by an integration test.
- The `unified-board` sentinel exists in `../../planning/status.md` so a live emit populates it.
- `cargo test`, `cargo clippy -- -D warnings`, `cargo fmt --check`, `cargo build --release` all pass.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes
- **Cross-repo edit (Task 5):** the `unified-board` sentinel goes into `../../planning/status.md` —
  the HQ root, **two tiers above** the mev git repo. In an isolated worktree (`/sdlc-flow`) that path
  is outside the tree. Apply it in the main company-brain checkout as a companion step, or run this
  block with an in-place engine so the path is reachable. All Rust code + tests (Tasks 1–4) stay
  inside the mev repo and gate normally.
- The routing enums (`sdlc_workflow`/`model`) have **no board consumer in v1** — do not add columns
  for them (Out of scope). This block sorts by **raw** `priority` (effective-priority inheritance is
  MV.7.A).

## Amendment Log
<!-- Append-only. Pipeline stages append one dated line here when they deviate from the spec. -->
_No amendments yet._
