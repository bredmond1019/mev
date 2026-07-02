---
type: Decision
title: "D8: portfolio kind — terminal repos with no planning state"
description: New state.json kind:"portfolio" for repos published to GitHub with no further planning state; exempts them from the tracks[]-required and master-plan.md sentinel warnings.
doc_id: D8-portfolio-kind-terminal-repos
layer: [factory]
project: mev
status: active
keywords: [portfolio, state.json, kind, terminal repo, emit-state, validate-brain, tracks]
related: [D7-brain-rollup-tier-scoping-and-preserve, core:state-json-schema]
---

# D8: portfolio kind — terminal repos with no planning state

## Context

Running `mev validate-brain --state` and `mev emit-state` against the live brain surfaced two
warnings on every repo in the `portfolio/` tier (`rag-engine-rs`, `workflow-engine-rs`,
`claude-sdk-rs`): `E_STATE_SCHEMA_MISSING_FIELD` (empty `tracks[]`) and `W_EMIT_NO_SENTINEL`
(no `master-plan.md`). These repos are the **final destination** for projects once they go public
on GitHub — they intentionally carry no roadmap, no `tracks[]`, and no `master-plan.md`. Treating
them as `kind:"project"` (which expects both) produced permanent, unresolvable noise.

## Decision

Add a third `state.json` `kind`: `"portfolio"`.

- `discover_state_files` assigns `expected_kind: "portfolio"` (instead of `"project"`) to any
  `[[repos]]` entry in `brain.toml` whose `tier == "portfolio"`.
- `check_schema` accepts `"portfolio"` as a valid `kind` and, in place of the `tracks[]`-required
  warning, requires a non-empty `note` field (e.g. `"Completed — live on GitHub"`) — the minimal
  structured signal that the repo is a terminal state, not a missing roadmap.
- `plan_master_plan_tables` (the `emit-state` wave-table splice pass) skips `kind:"portfolio"`
  files entirely — no `master-plan.md` is expected, so no `W_EMIT_NO_SENTINEL` warning fires.
- `plan_state_json` already skips unrecognized `kind` values via its `_ => continue` arm, so
  `"portfolio"` files are never rewritten (no `focus` to derive).
- Brain-level rollup (`derive_rollup`) still filters children by `kind == "project"`; a
  `kind:"portfolio"` child therefore falls into the existing preserve/stub branch (no dedicated
  portfolio-aware rollup rendering was in scope for this change).

Applies only to leaf repos; brain-kind and project-kind dispatch are unchanged.

## Consequences

- The three live `portfolio/` repos' `state.json` files were rewritten to
  `kind:"portfolio"` + a `note`, clearing both warnings against the live brain.
- Any future repo added to the `portfolio` tier in `brain.toml` must ship a `kind:"portfolio"`
  `state.json` with a `note`, not a `kind:"project"` stub.
- `derive_rollup`/`derive_brain_focus` do not yet render the portfolio `note` into the brain
  rollup — flagged as a possible follow-on, not required by this change.
