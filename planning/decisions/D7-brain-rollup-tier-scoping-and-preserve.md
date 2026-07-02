---
type: Decision
title: "D7: Brain rollup tier-scoping, preserve rule, and brain-focus union"
description: Brain-kind state.json rollups scope by tier (not global), never silently drop a repo with no loadable child, and brain focus is derived as a repo-tagged union of children's focus.
doc_id: D7-brain-rollup-tier-scoping-and-preserve
layer: [factory]
project: mev
status: active
keywords: [emit-state, rollup, tier-scoping, preserve, brain focus, state.json, derive_rollup]
related: [core:state-json-schema, D6-scope-doc-id-namespacing]
---

# D7: Brain rollup tier-scoping, preserve rule, and brain-focus union

## Context

`MV.3B.T` shipped `mev emit-state`, the derivation engine that regenerates leaf `focus` and brain
`repos[]`/`cross_repo[]` from the authored `tracks[]` DAG. Its brain-rollup path (`derive_rollup`) built
`repos[]` from a **global** scan of every loadable `kind:"project"` file in the corpus — not scoped to the
brain file's own tier — and **silently dropped** any tier repo with no loadable child `state.json` (none
authored yet, or a parse failure) out of `repos[]`. This corrupted both `core/planning/state.json` and the
HQ root `planning/state.json` live in session MV.3B.T-adjacent work: the `bastion` entry vanished from
`core`'s rollup when its `state.json` hit `E_STATE_MALFORMED_JSON`. Both files were manually restored from
`git show HEAD` before this block (`MV.3B.U`) landed the fix.

## Decision

1. **Tier-scope the rollup.** A brain file self-identifies by its `repo` slug. If that slug matches a
   `tier` value declared in `brain.toml`'s `[[repos]]`, the brain scopes to only that tier's repos
   (`TierScope::Tier`). If it matches no declared tier (the HQ root), it scopes to every repo across every
   tier (`TierScope::All`). Encapsulated in `tier_scope_for(brain_file, config) -> TierScope`
   (`src/brain/state.rs`).
2. **Preserve over drop.** `derive_rollup` iterates the in-scope `config.repos[]` (config order). For each:
   derive from a loadable child if one exists; else **preserve the brain file's existing `repos[]` entry
   verbatim** (backfilling `tier` from config) if one exists; else emit a tier-tagged empty stub. This was
   chosen over the alternatives considered — emitting an empty stub unconditionally (loses the last-known
   headline for a temporarily-broken child) or aborting the whole emit on any gap (blocks progress on
   unrelated repos for one bad file). Preserve degrades gracefully: a malformed or not-yet-authored child
   can never again truncate the rollup.
3. **Populate `RepoRollup.tier`.** All three `derive_rollup` branches now set `tier` from config
   (previously hardcoded `None`).
4. **Derive brain `focus` as a repo-tagged union**, rather than leaving it hand-authored/untouched (which
   was the state left "not yet settled" by `MV.3B.T`). `derive_brain_focus(scope, config, graph, files)`
   unions the in-scope children's own `derive_focus` output, tagging each `Block` with its source `repo`;
   ordering is config-repo order then the child's within-focus order; dedup is by `(repo, id)`, first
   occurrence wins. This was chosen over punting brain focus to stay hand-authored, because a stale
   hand-authored brain focus is exactly the kind of drift the emit engine exists to eliminate, and the
   union is a natural closed-form definition once the tier-scoped children set is well-defined.

## Consequences

- `plan_state_json` now takes `&BrainConfig` (threaded from `emit_state` in `src/lib.rs`) so the brain arm
  can compute tier scope and call the tier-scoped `derive_rollup`/`derive_brain_focus`. Leaf (`"project"`)
  arm behaviour is unchanged.
- The fixed-point property (`emit-state --write` then re-emit is a no-op) is preserved — proven by
  integration tests in `tests/brain_emit.rs`, including a regression test reproducing the malformed-child
  incident and asserting the preserve branch holds.
- `core/planning/state-schema.md` ("Brain rollup derivation rules" section) is the schema-level record of
  these rules; this decision is the append-only rationale trail.

## Links

- Spec: `planning/3B.U-brain-rollup-tier-scoping/tasks.md`
- Prior block: `planning/3B.T-state-table-rollup-emit/tasks.md` (the emit engine this fixes)
- Schema: `../../../planning/state-schema.md` (core repo, relative to this file)
