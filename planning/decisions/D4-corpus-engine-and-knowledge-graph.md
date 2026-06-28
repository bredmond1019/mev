---
type: Decision
title: D4 — mev as the corpus engine; the brain knowledge graph as an emitted product
description: Settles the end-state architecture for the Bastion Brain — one Rust corpus engine (crawl→validate→emit), two retrieval modes (semantic + structural) over one Postgres store, and the division of labor between mev (deterministic/free) and the orchestrator (embedding/AI).
doc_id: D4-corpus-engine-and-knowledge-graph
layer: [factory, brain, engine]
project: mev
status: active
keywords: [knowledge graph, corpus engine, RAG, pgvector, scope:doc_id, retrieval, manifest]
related: [block-j-namespacing-decision, master-plan, D3-corpus-config-system, context]
---

# D4 — mev as the corpus engine; the brain knowledge graph as an emitted product

Settled 2026-06-28 in a design pass with Brandon, after reviewing the existing
`workflow-engine-rs/services/knowledge_graph` service and the orchestrator's
`index_brain.py` embedder. This decision states the **destination architecture** for the
Bastion Brain so that Phase 3 builds toward it rather than only toward the next block. It
refines and extends `block-j-namespacing-decision.md` (the `scope:doc_id` scheme there is
unchanged); it does not supersede it.

## Why this matters

The Brain is meant to scale, and to be **reused as the knowledge-base substrate for client
engagements**. Two retrieval needs coexist:

1. **Semantic** — "what is this about?" — fuzzy questions answered by vector search over
   embedded chunks (the `brain_documents` pgvector table). Costs tokens; needs the embedding
   API and (usually) an LLM to synthesize.
2. **Structural** — "where does this live? what is it connected to? what's the status of X?"
   — exact questions answered by walking the knowledge graph + frontmatter. **Free, instant,
   deterministic; no tokens.**

A foolproof, reusable Brain needs both, and needs them to not drift apart.

## The problem this fixes

Today the corpus is **crawled twice by two programs that re-implement the same rules**:

- `mev` (Rust) walks the corpus to **validate** (OKF schema, sync, soon graph integrity).
- `index_brain.py` (Python, orchestrator) walks the *same* corpus to **embed** it.

Both encode the same crawl rules independently (subtrees `docs/`+`planning/`, root
`README`/`CLAUDE`, `skip_dirs`, ephemeral skips, `doc_id`-from-stem fallback, OKF
normalization). They already drift and will drift further. "What's validated" and "what's
embedded" are kept in sync by vigilance, not by construction.

## Decision

### 1. mev is the single corpus engine — one crawl, three outputs

`mev` owns the canonical corpus crawl. One walk produces three artifacts:

1. **Diagnostics** — the validation gate (Phases 2–3: OKF schema, graph integrity, links,
   structure, sync).
2. **Manifest** — the canonical file-list + per-file OKF metadata. The embedder consumes
   this instead of re-crawling, so "what's validated == what's embedded" holds **by
   construction**.
3. **Graph** — the global `scope:doc_id` node index + edges. Emitted, not discarded.

### 2. mev is a pure, side-effect-free compiler

Files in → `{diagnostics, manifest, graph}` JSON out. **mev never touches a database or the
network.** This keeps it fast, trivially testable, runnable in CI/pre-commit with no
credentials, and **droppable into a client repo** (validate a client KB with zero infra).
All persistence is the orchestrator's job, fed by mev's stdout.

Rust/Python boundary: **Rust owns everything deterministic and free** (crawl, frontmatter
parse, validation, graph construction, structural queries); **Python owns everything that
needs the embedding API or an LLM** (chunking-for-embedding, Voyage calls, RAG retrieval/
rerank, the AI answer layer). They share `brain.toml` and the `brain_documents` schema.

### 3. The knowledge graph is a first-class emitted artifact — not a throwaway

Block J's graph stops being an in-memory structure that is built, checked, and discarded.
The graph-construction is a **reusable module** and its node/edge structs are
**`Serialize`-able**, so the same graph mev *validates* is the graph mev *emits*. Cheap to
honor while building Block J; expensive to retrofit later.

**Edge model built to grow.** Edges are represented generically as
`Edge { from, to_ref, kind }`. Block J validates `related:` only (kind `Related`); typed
edges — `supersedes`, `depends-on`, `parent` — drop in as later blocks with **no refactor**
as authored frontmatter grows. The `kind` discriminant is what keeps the same node/edge
structs (and the emitted graph schema) stable across that growth. This is already the stance
in `block-j-namespacing-decision.md` (point 3); D4 binds it to the *serializable, emitted*
graph so the on-disk/Postgres edge shape carries `kind` from day one.

### 4. The brain graph lives in Postgres, beside the embeddings — not in a graph DB

Edges land in a table in the same pgvector database, loaded by the orchestrator from mev's
emitted graph JSON. Rationale:

- **One joinable store.** Semantic + structural can be combined in a single query (vector
  search expanded/reranked by graph neighbors). The `related` column already exists per-row
  in `brain_documents`; nothing traverses it yet — this is the bridge already in the DB.
- **Right-sized.** The doc graph is hundreds of nodes now, low thousands at client scale.
  Postgres recursive CTEs traverse that instantly.
- **No second store to sync.** A dedicated graph DB (Dgraph) would split the brain across
  two stores for no benefit at this scale.

We **reject adopting the `workflow-engine-rs/services/knowledge_graph` service** for the
Brain: it is UUID-keyed, Dgraph-backed, and its edges are *inferred from concept
properties* — the wrong model for an **authored** document graph keyed by `scope:doc_id`
with edges authored in frontmatter. Its **algorithms** (Dijkstra / topological sort /
PageRank, in Rust) are harvestable *ideas* if we later want ranking or path queries over the
brain graph ("most central doc", "shortest path between two docs") — borrow the ideas, not
the service.

### 5. Two retrieval modes, fused at the retrieval layer

The endgame is a **router** at retrieval time: structural queries → graph/SQL (free, exact);
semantic queries → vector + LLM; hybrid → **graph-aware RAG** (traverse `related`/
`supersedes`/`parent` edges from the top semantic hits to feed the LLM connected context,
not isolated chunks). This fuses the cheap/deterministic and the fuzzy/AI modes into one
answer surface.

## Consequences for the roadmap

- Phase 3's two queued specs (`2.J-corpus-crawl`, then `2.J-graph-integrity`) are correct as
  written; they gain two forward-compat constraints: (a) the crawl produces a clean owned
  data structure (it will feed manifest + embedder), and (b) the graph module is reusable and
  serializable (it will be emitted).
- Two **additive** blocks follow: **manifest emit** (`mev` emits the file-list; orchestrator's
  `index_brain.py` consumes it — kills the duplication) and **graph emit + structural query
  surface** (orchestrator loads edges into Postgres; a thin surface — `bastion` / MCP —
  answers structural questions free).
- A later block delivers **graph-aware RAG** in the orchestrator's retrieval path.
- Companion work outside this repo: register tier sub-brains as scope units in `brain.toml`;
  refactor `index_brain.py` to consume the manifest; add the Postgres edges table.

## What this does not change

- The `scope:doc_id` node-id scheme and corpus rules in `block-j-namespacing-decision.md`.
- The `brain.toml`-as-single-config decision (supersedes D3) — D4 builds on it.
- mev's existing Phase 2 validators and public API.
