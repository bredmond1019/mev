---
type: Decision
title: "D9: BA.15.12 (okf-core format convergence) — mev-side mirror"
description: bastion's D15/D16 scope a future block, BA.15.12, that deletes mev's brain/okf.rs, brain/state.rs, brain/graph.rs, and brain/graph_emit.rs and repoints mev at bastion's okf-core crate as the single implementation of each format. This doc is mev's own record of that pending cross-repo dependency, since mev's planning previously had zero awareness of it.
doc_id: D9-ba15-12-okf-core-convergence-mirror
layer: [factory, console]
project: mev
status: active
keywords: [okf-core, BA.15.12, bastion, format convergence, graph.rs]
related: [core:D15-mev-integration-cross-repo-path-dep, core:D16-ba15-12-scope-widened-graph-resolution]
---

# D9: BA.15.12 (okf-core format convergence) — mev-side mirror

## Context

bastion consumes mev as a cross-repo Cargo path dependency (bastion's
`D15-mev-integration-cross-repo-path-dep.md`, at `core/bastion/planning/decisions/`). D15 named a
follow-on block, **BA.15.12**, tracked in bastion's own `state.json`/`master-plan.md`: extract a
`state.json` serde schema + a reconciled `OkfFrontmatter` model into bastion's `okf-core` crate, then
repoint this repo's `brain/okf.rs` (899 lines) and `brain/state.rs` (5,383 lines) at `okf-core`,
deleting the duplicate struct definitions here.

This repo shipped `MV.3B.V` since D15 was written (a `resolve_edge`/`ExportedEdge` graph-resolution
module in `brain/graph.rs` + `graph_emit.rs`, 1,089 lines) with no awareness that BA.15.12 existed.
`MV.3B.V`'s own `master-plan.md` write-up (Phase 3B) explicitly listed the `BA.15.12 okf-core dedup` as
**out of scope**, citing "different files: `okf.rs`/`state.rs`, not `graph.rs`/`graph_emit.rs`" — that
statement was accurate against D15 at the time, but bastion has since superseded it with
`D16-ba15-12-scope-widened-graph-resolution.md`, which widens BA.15.12 to include exactly the
`graph.rs`/`graph_emit.rs` module `MV.3B.V` added. Left uncorrected, this repo's own planning docs
would actively mislead the next `/generate-tasks` pass here about BA.15.12's real scope.

**mev had zero mirror of this work anywhere** — no decision doc, no `state.json` block or carryover,
no `status.md` mention — despite BA.15.12 requiring a real SDLC run in this repo (D15: "executed
partly in mev's own repo"). This doc is that mirror.

## Decision

1. **This repo records BA.15.12 as a pending cross-repo dependency**, not yet a scheduled block here
   (bastion has not run `/generate-tasks` for it). When bastion does, the mev-side half of that work —
   deleting `brain/okf.rs`, `brain/state.rs`, `brain/graph.rs`, `brain/graph_emit.rs` and repointing at
   `okf-core = { path = "../bastion/crates/okf-core" }` — becomes its own task spec in **this** repo,
   under its own SDLC run, per D15/D16.
2. **Scope mirrors bastion's D16, not just D15**: all four files (`okf.rs`, `state.rs`, `graph.rs`,
   `graph_emit.rs`), not only the original two. `master-plan.md`'s stale "out of scope" note (Phase 3B,
   `MV.3B.V` write-up) is annotated to point here rather than rewritten — it was correct when written.
3. **No mev-side code changes are made by this decision.** `mev::graph_brain`, `mev::validate_brain*`,
   and friends keep their current internals until BA.15.12 actually executes here; this repo's public
   API contract with `bastion` (consumed via the unpinned path dependency, D15) is unaffected.

## Consequences

- Any future session working `master-plan.md` Phase 3B or planning this repo's next block should check
  `core/bastion/planning/master-plan.md`'s BA.15.12 write-up for the live spec before assuming
  `graph.rs`/`graph_emit.rs` is out of scope — it no longer is.
- `state.json` carries a matching `carryover` entry (`ba15-12-okf-core-convergence`) so
  `/generate-tasks` or `/prime` in this repo surfaces the dependency without reading bastion's repo
  first.
- If bastion ever abandons or re-splits BA.15.12 again, supersede this doc rather than editing it.
