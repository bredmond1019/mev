---
type: Note
title: MV.3B.V — emit-graph Resolved Edges (interface contract + context seed)
description: Context seed for /generate-tasks MV.3B.V — the exported-resolution contract, the Rust/Python divergence it kills, exact code pointers, and the cross-repo sequencing with orchestrator's loader and the embed pass.
doc_id: emit-graph-resolved-edges
layer: [brain, engine]
project: mev
status: draft
keywords: [emit-graph, check_graph, edge resolution, target_node_id, brain_edges, OR.G]
related: [master-plan, planning-index]
---

# MV.3B.V — emit-graph Resolved Edges

> **Status:** context seed for `/generate-tasks MV.3B.V`. The block is registered in
> `master-plan.md` §MV.3B.V and `state.json` (Phase 3B track, wave 7). This file carries the
> seam contract and code pointers so task generation runs cold, with no brain-repo reading
> required. Source audit: brain `core/planning/brain-graph-overlap/notes.md` (2026-07-03).

## The problem this block solves

`mev emit-graph` exports edges with a **raw, unresolved** `to_ref` (as-authored; documented in
`docs/cli.md` "Output shape" and the `Edge` doc-comment, `src/brain/graph.rs:52-54`). Resolution —
turning `to_ref` into a target node — exists only inside `check_graph()`'s lint pass and is
**discarded** after diagnostics.

Because the export carries no resolution, the orchestrator's `OR.G` loader
(`core/orchestrator/scripts/load_brain_edges.py`) re-implemented it in Python — **with different
semantics**:

| | mev `check_graph` (`graph.rs:251-255`) | Python `resolve_ref`/`doc_id_map` |
|---|---|---|
| Bare `to_ref` (no `:`) | Qualified to the **referrer's own scope** only | Looked up **globally across all scopes**, last-write-wins on collision |

Consequence: the same edge can be "resolved" in Postgres but `E_GRAPH_DANGLING_RELATED` in
`validate-brain` — or resolve to the **wrong repo's document** (every repo's
`planning/master-plan.md` shares the bare doc_id `master-plan`, so cross-scope collisions are
guaranteed in this corpus, not hypothetical). **mev's scope-qualified semantics are the decided
winner** — they are the validated ones (decision recorded in the brain's consolidated program
plan, Program Phase 1).

## The contract change (what `emit-graph` v2 exports)

Each entry in `edges[]` gains two **nullable** fields:

```json
{ "from": "scope:doc_id", "to_ref": "<raw as-authored>", "kind": "related",
  "target_node_id": "scope:doc_id" | null, "target_doc_id": "doc_id" | null }
```

- Non-null ⇔ the edge resolves under `check_graph()`'s algorithm (bare refs qualified to the
  referrer's scope; qualified refs looked up as-is in `node_map`).
- Null ⇔ dangling or leaf-target (the same cases that produce `E_GRAPH_DANGLING_RELATED` /
  `W_GRAPH_LEAF_TARGET` diagnostics today). `to_ref` stays raw in both cases — additive change.
- Top-level `"version"` bumps `"1"` → `"2"`. `leaves[]` (doc-id-less files) is **unchanged** —
  orthogonal concept, do not conflate with dangling edges.
- Update `docs/cli.md` §emit-graph "Output shape" — that section is the contract orchestrator's
  tests pin.

## Implementation shape

1. Extract the per-edge resolution block from `check_graph()` (`src/brain/graph.rs:232-281`) into
   a pure function, e.g. `resolve_edge(edge, node_map, leaf_keys) -> EdgeResolution`
   (`Resolved{node_id, doc_id} | LeafTarget | Dangling`).
2. `check_graph()` consumes it for diagnostics — **byte-identical diagnostic output** to before
   the refactor (regression-asserted).
3. `build_graph_export()` (`src/brain/graph_emit.rs:55-66` — currently clones raw edges verbatim,
   never resolves) consumes it to populate the new fields; extend the export structs
   (`graph_emit.rs:29-42`).
4. Parity test over the full live corpus: exported resolution ⇔ `check_graph` diagnostic, edge by
   edge.

## Cross-repo sequencing (do not scope these into mev's spec)

- **After this ships → orchestrator follow-up (its own small spec, run there):**
  `load_brain_edges.py` deletes `build_node_maps()` + `resolve_ref()`, reads the exported fields;
  fixture payloads in `tests/test_load_brain_edges.py` + `tests/workflows/test_brain_graph_retrieval.py`
  update to v2. The loader ends up with **no resolution logic, ever** (its standing seam rule).
- **Then the embed pass (OR.H, Mac Mini):** first-ever live `mev emit-graph | load_brain_edges.py`
  run (the `brain_edges` table is empty today) happens against the corrected contract, then
  `index_brain.py --rebuild` off `mev manifest`. **This block gates the embed pass** — the operator
  is holding embedding until it lands.
- **Not this block:** BA.15.12 (okf-core dedup) touches `brain/okf.rs`/`brain/state.rs` — disjoint
  files; the two can proceed independently, MV.3B.V first. *(Update 2026-07-03: accurate against
  bastion's D15 at the time this shipped. Bastion's D16 has since widened BA.15.12 to also cover
  `brain/graph.rs`/`graph_emit.rs` — the files this block touches. See this repo's own
  `planning/decisions/D9-ba15-12-okf-core-convergence-mirror.md`.)*

## Acceptance (mirror of master-plan §MV.3B.V)

Parity on the full corpus (export ⇔ diagnostics); `check_graph` diagnostics unchanged; version
bumped + `docs/cli.md` updated; `cargo fmt --check` / `clippy -D warnings` / `test` / release
build green.
