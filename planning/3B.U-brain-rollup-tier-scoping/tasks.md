---
type: Plan
title: Task Spec — MV.3B.U — Brain rollup tier-scoping + brain-focus aggregation
description: Make mev emit-state safe and correct for brain-kind state.json — tier-scope the repos[] rollup, preserve sourceless entries, populate tier, and derive brain focus as a repo-tagged union.
doc_id: 3B.U-brain-rollup-tier-scoping-tasks
layer: [factory]
project: mev
status: draft
keywords: [emit-state, rollup, tier-scoping, brain focus, state.json, corruption fix, brain.toml]
related: [master-plan, status, 3B.T-state-table-rollup-emit-tasks, state-schema]
---

# Task Spec — Phase 3B, Block MV.3B.U — Brain rollup tier-scoping + brain-focus aggregation

**Status:** Draft (0 tasks passed) · **Last run:** —

## Goal
Make `mev emit-state --write` **safe and correct for brain-kind `state.json` files** (`core/planning/state.json`,
the HQ root `planning/state.json`). Today it is not: `derive_rollup` rebuilds `repos[]` from a **global** scan of every
loadable `kind:"project"` file, so (a) the rollup is not tier-scoped (every brain file gets an identical flat
rollup) and (b) any tier repo without a loadable `state.json` — none authored yet, or a parse failure — is
**silently dropped** from `repos[]`. This corrupted `core/planning/state.json` and the HQ root file live this
session (the `bastion` entry vanished when its `state.json` hit `E_STATE_MALFORMED_JSON`; both files were manually
restored from `git show HEAD`). This block fixes it by:

1. **Tier-scoping** the rollup via `brain.toml` — each brain file's `repos[]` reflects only its own tier's repos
   (HQ = the full set).
2. **Preserving** the brain file's existing hand-authored `repos[]` entry for any in-scope repo that has no loadable
   child `state.json` (non-destructive; nothing silently dropped). A stub entry (tier + empty vecs) is emitted only
   when neither a child `state.json` **nor** an existing entry exists.
3. **Populating** `RepoRollup.tier` from config (it is hardcoded `None` today).
4. **Deriving brain `focus.now/next/blocked`** as the **repo-tagged union** of the in-scope children's derived focus
   (currently left untouched with a "not yet settled" note).

Same pure-compiler model as `MV.3B.T` (files in → files out; no DB, no network). Preserves the fixed-point property:
`emit-state --write` immediately followed by re-emit is a no-op.

## Context Pointers
- **Root-cause code (confirmed this session):**
  - `src/brain/emit.rs::plan_state_json` (~L384-405): `children` is a **global** `kind == "project"` filter, not
    tier-scoped; brain branch calls `derive_rollup(&children, …)` and leaves `derived.focus` untouched.
  - `src/brain/state.rs::derive_rollup` (~L1775-1844): maps over loaded children only → sourceless repos dropped;
    `RepoRollup { tier: None, … }` hardcoded at ~L1837.
  - `src/brain/state.rs::RepoRollup` (~L180): already has `pub tier: Option<String>` — just never populated.
  - `src/brain/config.rs::RepoEntry` (~L70): already parses `slug` + `tier` from `[[repos]]`; `BrainConfig.repos`
    is the tier map. No config change needed — just thread it into the emit path.
  - `src/lib.rs::emit_state` (~L433): resolves `BrainConfig` via `find_brain_config`, then calls
    `plan_state_json(&loaded, &graph)` — the config is available at the call site but not passed down.
  - `src/brain/emit.rs::apply_plan` (~L546): writes each action independently, no pre-write gate — which is why the
    partial-write landed. The **preserve** rule (task 1) fixes the corruption at source; task 4 adds a regression
    test proving a malformed child can no longer truncate a rollup.
- **Tier-scope mapping (decided):** a brain file self-identifies by its `repo` slug. If that slug matches a `tier`
  value in `brain.toml` (`core` → tier `"core"`), scope to that tier. If it matches no tier (the HQ root, `repo: "hq"`),
  scope to **all** repos. Encapsulate in a `tier_scope_for(brain_file, config) -> TierScope` helper. Both live brain
  files already carry `repo` fields on their focus blocks, so the union tagging is a natural fit.
- **v2 schema contract:** `../planning/state-schema.md` (the `core` repo) — *Authored vs derived* + **Derivation
  rules**. This block **extends** those rules for the brain rollup (tier-scoping + preserve) and **adds** the
  brain-focus derivation rule. Update that doc (task 5).
- **Prior block:** `planning/3B.T-state-table-rollup-emit/tasks.md` — the emit engine this fixes; decisions are
  append-only (append a `decisions/` note, do not edit MV.3B.T).
- **CLAUDE.md standing rules:** every new fn/module ships with tests (rule 1); new `.md` docs carry OKF frontmatter
  and update the directory `index.md` (rule 2); decisions are append-only (rule 4).

## Step-by-Step Tasks

### 1. Tier-scoped, non-destructive `derive_rollup` + `tier_scope_for` (`state.rs`)
- Add `TierScope` (an enum or slug-set) and `tier_scope_for(brain_file: &StateFile, config: &BrainConfig) -> TierScope`:
  match `brain_file.repo` against the set of `config.repos[].tier` values; a match yields that single tier, no match
  yields "all tiers" (HQ root).
- Change `derive_rollup` to accept the config + the brain file's existing `repos[]` (for preservation) + the tier
  scope. New behaviour, iterating the **in-scope config repos** (filtered by tier), in config order:
  - If a loadable child `state.json` exists for that slug → derive the headline as today, and set
    `tier: Some(<config tier>)`.
  - Else if the brain file already has a `repos[]` entry for that slug → **preserve it verbatim** (backfill `tier`
    from config if it was `None`).
  - Else → emit a stub `RepoRollup { repo, tier: Some(...), now: [], next: [], blocked: [] }`.
- `RepoRollup.tier` is populated (non-`None`) in every branch.
- **Acceptance:**
  - Core-tier brain file scopes to exactly the `tier == "core"` config repos; HQ (`repo` not a tier) scopes to all.
  - A tier repo with no loadable `state.json` retains its existing `repos[]` entry (or gets a tier-tagged empty stub
    when none exists) — never dropped.
  - `RepoRollup.tier` is set from config in all three branches.
  - `src/brain/state.rs` carries ≥6 new unit tests: core scoping, HQ all-scope, derive-branch, preserve-branch,
    stub-branch, tier populated.

### 2. `derive_brain_focus` — repo-tagged union (`state.rs`)
- Add `pub fn derive_brain_focus(scope, children, graph, files) -> Focus` computing brain `focus.now/next/blocked` as
  the **union of the in-scope children's derived focus** (reusing `derive_focus`), each `Block` tagged with its source
  `repo`. Ordering: config repo order, then the child's within-focus order. Dedup by `(repo, id)`, keep first.
- **Acceptance:**
  - `focus.now/next/blocked` are the union of in-scope children's derived focus; each block carries `repo`.
  - Tier scope is respected (core brain excludes non-core children; HQ includes all).
  - Deterministic ordering (config-repo order then within-child) and `(repo, id)` dedup.
  - ≥4 new unit tests: two-child union, repo-tagging, tier-scope exclusion, dedup + ordering.

### 3. Thread `BrainConfig` into `plan_state_json` + wire the brain branch (`emit.rs`, `lib.rs`)
- Change `plan_state_json(files, graph)` → `plan_state_json(files, graph, config: &BrainConfig)`. In the
  `kind == "brain"` arm: compute `tier_scope_for(file, config)`, call the tier-scoped `derive_rollup` (passing
  `file.repos` for preservation) and set `derived.focus = derive_brain_focus(...)`. Leaf (`"project"`) arm unchanged.
- Update `src/lib.rs::emit_state` to pass the already-resolved `config` into `plan_state_json`. Update the fixed-point
  note comment (`emit.rs` L376-383) to reflect that brain focus is now regenerated.
- **Acceptance:**
  - `plan_state_json` takes and uses `&BrainConfig`; brain files get tier-scoped `repos[]` + derived `focus`.
  - Leaf-file behaviour is byte-identical to before (no regression in `focus` derivation for `kind:"project"`).
  - Fixed point holds: running `emit-state --write` twice on the same tree produces no second-pass action.

### 4. Integration tests (`tests/brain_emit.rs`)
- End-to-end `emit-state` over a temp brain fixture with a `brain.toml` declaring ≥5 core-tier repos where only 2
  have a `state.json`:
  - `repos[]` contains **all** in-scope repos (2 derived + 3 preserved/stub), each with `tier` populated, none dropped.
  - **Regression:** a child whose `state.json` is malformed (`E_STATE_MALFORMED_JSON`) does **not** truncate the
    brain `repos[]` — its existing entry is preserved (reproduces the live bastion-drop incident).
  - Brain `focus` is the repo-tagged union of the loadable children's focus.
  - An HQ-shaped fixture (`repo` not a tier) aggregates across all tiers.
  - Fixed-point: a second `emit-state --write` is a no-op.
- **Acceptance:** all new integration tests pass under `cargo test`.

### 5. Docs + schema + decision note
- `../planning/state-schema.md` (the `core` repo): extend the brain-rollup derivation rules (tier-scoping + preserve
  rule) and add the brain-focus derivation rule (repo-tagged union, dedup, ordering). Note HQ = full set.
- mev `docs/cli.md`: update the `emit-state` section to state that brain-kind files now get tier-scoped `repos[]`
  and derived `focus` (and that sourceless tier repos are preserved).
- mev `docs/architecture.md`: add `derive_brain_focus` + `tier_scope_for`, and the updated `derive_rollup` signature,
  to the module map + function table.
- Append a **new** `planning/decisions/` file recording the tier-scoping + preserve + brain-focus-union decisions
  (append-only; link back to MV.3B.T and this spec). Update `planning/decisions/index.md`.
- Update `planning/index.md` (Active Concept Folders row) and this block's `docs/*/index.md` rows as needed.
- **Acceptance:** schema, cli.md, architecture.md, and a decision file all reflect the new behaviour; indexes updated.

### 6. Validate + clear carryover
- Run the four harness gates: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, `cargo build --release`.
- **Dry-run** (no `--write`) `mev emit-state ..` against the live company brain from the mev dir; confirm the plan
  **preserves every existing `repos[]` entry** and populates `tier` with **zero drops** before anyone runs `--write`.
- Resolve the `mev-brain-rollup-tier-scoping` carryover in `core/planning/state.json` (brain-side edit) once verified.
- **Acceptance:** all four gates pass; a live dry-run shows a non-destructive, tier-scoped, tier-populated plan with
  no dropped repos.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes
- Do **not** run `mev emit-state --write` against any brain-kind `state.json` until tasks 1–4 land — it will
  destructively truncate the rollup (this is the carryover `mev-brain-rollup-tier-scoping`).
- Preserve rule chosen over empty-stub/abort-on-gap (user decision, this session). Brain-focus aggregation defined
  now (union) rather than punted (user decision, this session).
