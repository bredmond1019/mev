---
type: Handoff
created: 2026-06-28
---

# Handoff — Destination architecture settled (D4); ready to build Block J

> **For the next agent:** Read this immediately after `/prime`. Delete this file once consumed.

## What we're doing and why

`mev` is the validation engine for the Bastion Brain. Phase 2 (OKF schema) + sync watermark are
done/merged. The active work is **Phase 3 graph integrity** — building a global, cross-repo
`scope:doc_id` knowledge graph. This session did **not** write mev code; it resolved the *destination
architecture* so the upcoming blocks build toward the right end-state instead of just the next block.

The headline, now captured in **`planning/decisions/D4-corpus-engine-and-knowledge-graph.md`**: mev is
the **single corpus engine** — one Rust crawl produces three outputs (diagnostics + manifest + graph) —
and is a **pure, side-effect-free compiler** (JSON out; no DB, no network). The knowledge graph is a
**first-class emitted artifact**, stored in **Postgres beside the embeddings** (one joinable store), which
enables **two retrieval modes**: *semantic* (vector/RAG, fuzzy, costs tokens) and *structural* (graph/SQL,
exact, free), fusing into graph-aware RAG. Division of labor: **Rust owns the deterministic and free;
Python (orchestrator) owns the embedding/AI layer.** Read D4 first — it is the source of truth for the
end-state. It refines (does not supersede) `2.J-graph-integrity/namespacing-and-corpus-decision.md`.

## Completed this session

- **Reviewed `workflow-engine-rs/services/knowledge_graph`** (the priority review item from the prior
  handoff): a UUID-keyed, Dgraph-backed service with edges *inferred* from concept properties. **Verdict:
  do NOT adopt it for the brain** — wrong model for an *authored* `scope:doc_id` doc graph. Borrow its
  algorithms (Dijkstra/topo-sort/PageRank) as ideas later if needed; not the service or its Dgraph backend.
- **Read the embedder** (`core/orchestrator/scripts/index_brain.py`) and confirmed the **double-crawl
  problem**: mev and `index_brain.py` independently re-implement the same corpus rules and will drift.
- **Wrote `planning/decisions/D4-corpus-engine-and-knowledge-graph.md`** (new) — the 5 settled decisions
  (Brandon confirmed all via the question prompt): (1) mev = single corpus engine; (2) mev = pure compiler,
  no DB/network; (3) graph = emitted, serializable, reusable module, **edge model `Edge { from, to_ref,
  kind }` built to grow** (related → supersedes/depends-on/parent, no refactor); (4) graph in Postgres
  beside embeddings (not Dgraph); (5) two retrieval modes fuse at retrieval.
- **Refreshed `planning/master-plan.md`** — "bigger destination (D4)" framing; D4 note on Phase 3; two
  **forward-compat constraints** stamped on the queued blocks (J-crawl returns an owned crawl result; J's
  graph module is reusable + `Serialize`-able); new **Phase 3B** with Blocks **Q** (manifest emit), **R**
  (graph emit + structural query surface), **S** (graph-aware RAG, orchestrator); sequence table updated.
- **Updated `status.md`** (frontmatter scalars, momentum board, Phase 3B progress rows) and
  **`decisions/index.md`** (D4 entry; marked D3 superseded).
- Deleted the prior `planning/handoff.md` (consumed).

## Remaining work

In order:

1. **`/sdlc-flow 2.J-corpus-crawl`** — the foundation: scope-unit registry (`brain.toml`) + longest-prefix
   `scope_for` resolver + multi-root `crawl_corpus`. **Honor D4 forward-compat:** return a clean *owned
   crawl result* (not state buried in a validation pass) — it will feed the manifest emit + the embedder.
2. **`/sdlc-flow 2.J-graph-integrity`** — global `scope:doc_id` node index + edge integrity. **Honor D4
   forward-compat:** graph construction is a reusable module with `Serialize`-able node/edge structs, and
   the edge carries `kind` from day one.
3. **Phase 3B (additive, after J):** Block Q (manifest emit → `index_brain.py` consumes it), Block R
   (graph emit → Postgres edges table + bastion/MCP structural query), Block S (graph-aware RAG,
   orchestrator-side).
4. **Companion work (not mev code — flag to Brandon):** register tier sub-brains as scope units in
   `brain.toml`; switch `skip_dirs` to bare-component bloat list; refactor `index_brain.py` to consume
   mev's manifest; add the Postgres edges table.

## Open questions / choices

None — the architecture is settled per **D4** (all 5 decisions confirmed by Brandon this session).
Clear to proceed to `2.J-corpus-crawl`.

## Context the next agent needs

- **Branch:** `main`. This session's planning changes are committed (see the commit from `/commit`).
- **Tests:** ~196 green pre-Block-J. Harness: `cargo fmt --check && cargo clippy -- -D warnings &&
  cargo test && cargo build --release`.
- **Key source:** `src/brain/{config.rs (registry: slug + repo_path), okf.rs, crawl.rs, sync.rs, mod.rs}`,
  `src/lib.rs` (`validate_brain`, `validate_brain_sync`), `src/main.rs` (`--sync` is the sibling pattern
  for the eventual `--graph`/`--emit-manifest` flags).
- **brain.toml** at HQ root (`~/Dev/agentic-portfolio/brain.toml`): 9 repos, no tier units yet (companion
  work). mev reads whatever the registry declares.
- **Both specs already exist and are correct as written** — D4 only adds the two forward-compat
  constraints above; no spec rewrite needed.

## First command after `/prime`

```
/sdlc-flow 2.J-corpus-crawl
```
