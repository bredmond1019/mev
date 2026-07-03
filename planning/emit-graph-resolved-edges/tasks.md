---
type: LocalContext
title: Task Spec — Phase 3B, Block V (emit-graph resolved edges)
description: Decomposed task spec for MV.3B.V — export check_graph's edge resolution as a shared pure function so emit-graph ships resolved target_node_id/target_doc_id and lint/export agree by construction.
doc_id: emit-graph-resolved-edges-tasks
layer: [brain, engine]
project: mev
status: active
keywords: [emit-graph, resolve_edge, check_graph, target_node_id, brain_edges, parity]
related: [master-plan, emit-graph-resolved-edges]
---

# Task Spec — Phase 3B, Block V (emit-graph resolved edges)

**Status:** Not started · **Last run:** never

## Goal
Extract `check_graph()`'s per-edge resolution into one pure function both the lint pass and
`emit-graph` share, so the exported graph ships resolved `target_node_id`/`target_doc_id` and
Rust/Python edge semantics can no longer diverge.

## Context Pointers
- **Master-plan:** `planning/master-plan.md` §MV.3B.V (lines 389–424) — the block definition,
  acceptance, out-of-scope boundary.
- **Context seed:** `planning/emit-graph-resolved-edges/notes.md` — the full seam contract, the
  Rust/Python divergence it kills, exact code pointers, cross-repo sequencing.
- **Resolution logic to extract:** `src/brain/graph.rs:232–281` (the `--- Edge resolution ---`
  loop inside `check_graph`): qualify a bare `to_ref` to the **referrer's own scope**, look up
  `node_map`, else classify leaf (`W_GRAPH_LEAF_TARGET`) / dangling (`E_GRAPH_DANGLING_RELATED`).
- **Export builder to extend:** `src/brain/graph_emit.rs` — `GraphExport` struct (lines 29–42)
  and `build_graph_export` (lines 55–66), which today clones raw `Edge`s verbatim and sets
  `version: "1"`.
- **Contract doc:** `docs/cli.md` §`emit-graph` "Output shape" (lines 407–448) — the section the
  orchestrator's `load_brain_edges.py` tests pin.
- **Standing rules (`CLAUDE.md`):** every behaviour change ships with tests (rule 1); decisions
  append-only; keep all existing learn-ai + brain tests green.
- **Out of scope (hard boundary, from the block):** the orchestrator-side loader change
  (separate repo/spec); `leaves[]` semantics (doc-id-less files — unchanged); any change to
  `validate-brain` diagnostics or other subcommands; the BA.15.12 okf-core dedup
  (`okf.rs`/`state.rs`, not touched here).

## Step-by-Step Tasks
See `tasks.json` in this directory — the task list is defined there, not here.

## Acceptance Criteria
- A single pure function (e.g. `resolve_edge`) computes edge resolution; both `check_graph()`
  and `build_graph_export()` call it — resolution exists in exactly one place.
- `check_graph()`'s diagnostics are **byte-identical** to before the refactor (same codes, same
  order, same messages) — asserted by the existing graph tests staying green unchanged.
- Each exported edge carries nullable `target_node_id` (qualified `scope:doc_id`) and
  `target_doc_id`; both non-null ⇔ the edge resolves to a node, both null ⇔ dangling or
  leaf-target. `to_ref` stays raw as-authored in every case.
- `emit-graph` output `version` is bumped `"1"` → `"2"`.
- A parity test over a synthetic corpus (covering resolved, leaf-target, and dangling edges)
  asserts, edge by edge, that a non-null `target_node_id` ⇔ no `check_graph` diagnostic for that
  edge, and a null one ⇔ `E_GRAPH_DANGLING_RELATED` or `W_GRAPH_LEAF_TARGET`.
- `docs/cli.md` §`emit-graph` "Output shape" documents the two new fields and shows `version` `"2"`.
- All gated checks pass: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`,
  `cargo build --release`.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes
- **Confine the schema change to `graph_emit.rs`.** Do **not** add the two fields to
  `graph.rs`'s shared `Edge` struct (it also backs `generate-graph` HTML and `check_graph`).
  Introduce an export-local edge type (e.g. `ExportedEdge`) so `GraphExport.edges` carries the
  resolved fields while the graph model stays unchanged — this keeps the graph.rs edit
  (task 1) and the graph_emit.rs edit (task 2) on disjoint structs.
- The "full live brain corpus" parity in the block's acceptance is a manual operator check
  (`mev emit-graph ~/Dev/agentic-portfolio` vs `mev validate-brain --graph`); the automated
  guarantee is the synthetic-corpus parity test (task 3), which is deterministic and CI-safe.
- Cross-repo follow-up (do NOT scope here): orchestrator's `load_brain_edges.py` deletes its own
  `build_node_maps()`/`resolve_ref()` and reads the exported fields — its own small spec, run there.

## Amendment Log
<!-- Append-only. Pipeline stages append one dated line here when they deviate from the spec. -->
_No amendments yet._
