---
type: Plan
title: "MV.3.P2 — Task 1 breakdown (v2 serde model migration)"
description: Atomic sub-step breakdown of MV.3.P2 task 1 — migrate src/brain/state.rs to the v2 state schema (depends_on/wave/origin, id rename, backlog[]).
doc_id: 3-P2-task1-breakdown
layer: [factory, brain]
project: mev
status: archived
keywords: [breakdown, state.json, serde, depends_on, id rename, backlog, MV.3.P2]
related: [3-P2-state-graph-validation-tasks, state-json-schema]
---

# Task Breakdown — MV.3.P2 Task 1 (v2 serde model migration)

> **Scope:** This breakdown covers **task 1 only** of `planning/3.P2-state-graph-validation/tasks.md`
> (the v2 serde model migration — the one task flagged as a breakdown candidate). Tasks 2–8 stay as
> written in the spec; run them straight from `tasks.md`.

## Source Spec
`planning/3.P2-state-graph-validation/tasks.md` — Step 1.

## Goal
Migrate `src/brain/state.rs` from the v1 model to the **v2 state schema**: add `depends_on` / `wave` /
`origin` to track blocks, standardize the focus/endpoint ID field on `id`, and add the HQ `backlog[]`
node type — all **without changing any validation behaviour** (that is tasks 2–5) and leaving the full
test suite green.

## How to Use
Work top to bottom. Each sub-step is a single atomic action. Run the inline **Verify** checks as you
go — do not batch them. Each check must pass before continuing. **This task is purely structural:** add
model fields + rename a field + mechanical fixture-key updates. Do **not** touch `build_state_graph`'s
edge sourcing, `check_schema`'s status enum, or any check logic — those changes belong to later tasks.

---

## Steps

### Step 1: Migrate `src/brain/state.rs` to the v2 model

All sub-steps edit one file: `src/brain/state.rs`. Real anchors are from the current (v1) source.

#### 1.1 Add the `Origin` struct
**File:** `src/brain/state.rs`
**Action:** Insert after the `TierEntry` definition (the block ending at the `// ---- StateFile ----`
section header, ~line 224), before the `StateFile` section.
```rust
// ---------------------------------------------------------------------------
// Origin — backlog→block promotion provenance (v2)
// ---------------------------------------------------------------------------

/// Provenance pointer on a block that was promoted from a backlog item (D1, Option B).
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Origin {
    /// Origin kind — `"backlog"` today.
    #[serde(rename = "type")]
    pub kind: String,
    /// The originating backlog node's stable `slug` key.
    pub slug: String,
}
```

#### 1.2 Add the `Backlog` struct
**File:** `src/brain/state.rs`
**Action:** Insert immediately after `Origin` (still before `StateFile`).
```rust
// ---------------------------------------------------------------------------
// Backlog — HQ queued-ideas graph node (v2)
// ---------------------------------------------------------------------------

/// One entry in the HQ brain `backlog[]` — a queued idea as a graph node.
///
/// `slug` is the stable node key. `depends_on` reuses [`BlockedBy`] (the same edge
/// form as blocks). On promotion the node persists with `status:"promoted"` + a
/// `block` pointer; the resulting block carries an [`Origin`] back-pointer.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Backlog {
    /// Stable node key (the notes-dir slug).
    pub slug: String,
    /// Human description.
    pub title: String,
    /// Repo the item will land in when promoted (or `"cross-repo"`).
    pub repo: String,
    /// Item kind (`improvement` / `feature` / `chore` / `decision` / …).
    #[serde(rename = "type")]
    pub kind: String,
    /// Lifecycle status: `idea` / `ready` / `promoted` (validated in task 2).
    pub status: String,
    /// What the idea is gated on — same edge forms as a block's `depends_on`.
    #[serde(default)]
    pub depends_on: Vec<BlockedBy>,
    /// Set only when `status == "promoted"`: the ID of the block it became.
    #[serde(default)]
    pub block: Option<String>,
    /// Path to the pre-plan notes doc.
    #[serde(default)]
    pub notes: Option<String>,
}
```

#### 1.3 Extend `TrackBlock` with the v2 graph fields
**File:** `src/brain/state.rs` (struct at ~lines 140–149)
**Action:** Add three fields after the existing `status` field. Final struct:
```rust
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TrackBlock {
    /// Canonical block ID.
    pub id: String,
    /// Brief human description.
    pub title: String,
    /// Lifecycle status (authored: open/in_progress/closed — enforced in task 2).
    #[serde(default)]
    pub status: Option<String>,
    /// The block's full dependency edges (the authoritative DAG). Same forms as `BlockedBy`.
    #[serde(default)]
    pub depends_on: Vec<BlockedBy>,
    /// Execution-order rank for "what's next" (orthogonal to track grouping).
    #[serde(default)]
    pub wave: Option<i64>,
    /// Backlog-promotion provenance, when this block came from a backlog item.
    #[serde(default)]
    pub origin: Option<Origin>,
}
```

#### 1.4 Rename the focus `Block.block` field → `id` (with back-compat alias)
**File:** `src/brain/state.rs` (struct at ~lines 97–115; field `pub block: String,` at line 100)
**Action:** Rename the field and add a serde alias so existing `"block"` fixtures still deserialize:
```rust
    /// Canonical block ID. (`#[serde(alias)]` keeps v1 `"block"`-keyed files readable
    /// through the v2 transition; the canonical authored key is `id`.)
    #[serde(alias = "block")]
    pub id: String,
```
Leave the doc comment and the other fields (`title`, `status`, `note`, `repo`, `blocked_by`) unchanged.

#### 1.5 Rename the `Endpoint.block` field → `id` (with back-compat alias)
**File:** `src/brain/state.rs` (struct at ~lines 189–195; field `pub block: String,` at line 194)
**Action:**
```rust
    /// Canonical block ID.
    #[serde(alias = "block")]
    pub id: String,
```

#### 1.6 Add `backlog[]` to `StateFile`
**File:** `src/brain/state.rs` (struct at ~lines 238–264)
**Action:** Add one field (e.g. after `tiers`):
```rust
    /// HQ queued-ideas graph (brain HQ only; empty elsewhere).
    #[serde(default)]
    pub backlog: Vec<Backlog>,
```

**Verify:** `cargo build` → compiles (the struct/field additions alone should build; the renames in
1.4/1.5 will produce errors at the internal call sites until 1.7).

#### 1.7 Cascade the internal field references `.block` → `.id`
**File:** `src/brain/state.rs`
**Action:** Update every non-test reference the rename broke (these are the only six sites in `src/`):
- **`check_schema`** — line ~492 `block.block` → `block.id`; line ~511 `block.block` → `block.id`.
- **`build_state_graph`** — line ~661 `block.block` → `block.id`; lines ~678–679 `edge.from.block` →
  `edge.from.id`, `edge.to.block` → `edge.to.id`.
- **`check_state_graph`** — line ~766 `block.block` → `block.id`; line ~773 `block.block` → `block.id`.
- **`check_rollup`** — lines ~893, 894, 896, 898, 906 `b.block.as_str()` → `b.id.as_str()` (the
  `rollup.now/next/blocked` and `child.focus.*` iterators all use the focus `Block`).
> Do **not** change any logic here — only the field name. (`lib.rs` uses `f.kind` / `s.repo_slug`, not
> `.block`, so it needs no change.)

**Verify:** `cargo build` → compiles clean. `cargo clippy -- -D warnings` → no warnings.

#### 1.8 Mechanically migrate the in-file fixture JSON keys `"block"` → `"id"`
**File:** `src/brain/state.rs` (the `#[cfg(test)] mod tests` fixtures)
**Action:** In every fixture string, rename the JSON key `"block"` → `"id"` in **focus arrays**
(`now`/`next`/`blocked` entries) and in **`cross_repo` endpoints** (`from`/`to`). Affected lines:
981, 984, 1009, 1012, 1019, 1020, 1026, 1027, 1043, 1048, 1061, 1224, 1580, 1617, 1721, 1780, 1821,
1898, 1937, 1938. **Mechanical only** — do NOT change any other value:
- Keep the deliberately-bad fixtures intact: line 1580 `"status": "flying"` (bad-status test), line 1898
  `"AL.1.GHOST"` (dangling-focus test), line 1224 unknown `blocked_by` type — only their `"block"` key
  becomes `"id"`.
- Do NOT add `depends_on`/`wave` or remove `status:"blocked"` here — those are semantic changes owned by
  tasks 2–3 (this task must not alter validation behaviour).
> The `#[serde(alias)]` from 1.4/1.5 means this step is belt-and-suspenders for green, but migrating the
> keys keeps the in-file fixtures in canonical v2 form (the spec's task-1 intent).

**Verify:** `cargo test` → all existing tests pass (same count as before task 1; no behaviour changed).

#### 1.9 Final task-1 gate
**File:** — (commands only)
**Action:** Run the full harness suite.

**Verify:**
```
cargo fmt --check && cargo clippy -- -D warnings && cargo test && cargo build --release
```
→ all four green; test count unchanged from pre-task-1 (237). No new behaviour, so no new tests yet.

---

## Acceptance Criteria
<!-- Task-1 slice of the spec's acceptance criteria (verbatim where applicable): -->
- The serde model deserializes v2 files: `id` in focus, `depends_on`/`wave`/`origin` on track blocks, `backlog[]` on brain files; the full pre-existing test suite passes after fixture migration.
- All harness gates green (`fmt`, `clippy -D warnings`, `cargo test`, release build).
> The remaining spec acceptance criteria (cycle detection, authored-blocked rejection, status
> consistency, backlog dangling/promotion, focus drift, `ready_order` reusability) are delivered by
> tasks 2–6 and are **not** in scope for task 1.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes
- **`#[serde(alias = "block")]` is the migration linchpin.** Renaming `Block.block`/`Endpoint.block` →
  `id` would otherwise make *every* existing fixture — in `state.rs` **and** `tests/brain_state.rs` — and
  the five live v1 `state.json` files fail to deserialize (missing required `id`). The alias keeps them
  all readable through the multi-task transition, and lets live v1 files reach the v2 *checks* (rich
  diagnostics) instead of dying at a parse error. The canonical authored key is `id`; the alias can be
  removed in a later cleanup once all files are re-seeded.
- **Disjoint-ownership flag (task 1 ↔ task 6):** `tests/brain_state.rs` holds `"block"`-keyed fixtures
  (focus + cross_repo endpoints) but is **owned by task 6** (integration tests). Task 1 deliberately does
  **not** edit it — the alias keeps it green; task 6 migrates those fixtures to `id` when it adds the v2
  integration cases. In the sequential `/sdlc-flow` this is safe (one worktree, task 1 before task 6).
- **`BlockedBy` is reused verbatim** as the `depends_on` entry type — its `{type:block,repo,id,what?}` |
  `{type:external,what}` shape already matches the v2 `depends_on` schema. No new edge enum.
- **Behaviour is frozen in task 1.** `build_state_graph` still sources edges from `focus.*.blocked_by[]`
  here; re-pointing it at `tracks[].blocks[].depends_on[]` is **task 2**. `VALID_STATUSES` still includes
  `"blocked"` here; splitting the authored vs derived enums is **task 2**. Keeping task 1 behaviour-neutral
  is what lets the existing test suite pass unchanged.
- `type` is a Rust keyword → `Origin.kind` and `Backlog.kind` use `#[serde(rename = "type")]`.
```
