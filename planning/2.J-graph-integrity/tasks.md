---
type: TaskSpec
title: Task Spec — Phase 3, Block J — graph integrity (global scope:doc_id graph)
description: Decomposed task spec for the global scope:doc_id node index, uniqueness, extensible related-edge resolution, and leaf-as-target lint, surfaced via `mev validate-brain --graph`.
doc_id: 2j-graph-integrity-tasks
layer: [factory, brain]
project: mev
status: active
keywords: [graph integrity, knowledge graph, scope namespacing, doc_id, related edges, validate-brain]
related: [master-plan, status, block-j-namespacing-decision, 2j-corpus-crawl-tasks, D4-corpus-engine-and-knowledge-graph, D5-heterogeneous-format-ingest, D29-mev-brain-validation-engine]
---

# Task Spec — Phase 3, Block J — Graph integrity (global `scope:doc_id` graph)

**Status:** Task 3 in progress · **Last run:** now

## Goal
Over the multi-root corpus, build a global **`scope:doc_id`** knowledge-graph node index and flag every
duplicate canonical id and every `related:` edge that fails to resolve, surfaced via
`mev validate-brain --graph` — with the edge model shaped so typed edges extend it later.

## Context Pointers

- **Authoritative design:** `planning/2.J-graph-integrity/namespacing-and-corpus-decision.md` — the id
  scheme, leaf/node split, edge-resolution rules, and the **2026-06-28 Update** (registry-driven stable
  slugs; extensible edge model; root files are OKF nodes; one global graph). Read it first.
- **Depends on `2.J-corpus-crawl`** (run that block first): it provides `crawl_corpus`, the scope
  registry, and `scope_for(rel, config)` in `src/brain/scope.rs`. This block consumes them — do **not**
  re-derive scope here.
- **Destination architecture:** `planning/decisions/D4-corpus-engine-and-knowledge-graph.md` — the graph
  is a **first-class emitted artifact**, not a build-and-discard structure. The same graph this block
  *validates* is the graph Phase 3B Block R *emits* (loaded into Postgres beside the embeddings). Honor
  the forward-compat constraint below (decision 6).
- **Plan:** `planning/master-plan.md` → Phase 3, **Block J** (+ Phase 3B Block R graph emit). Governed by
  brain **D29** (mev is the single validation engine; checks are read-only and `--json`-able).
- **Repo files that apply:**
  - `src/brain/scope.rs` — `scope_for` (from the crawl block); reuse for canonical ids.
  - `src/brain/okf.rs` — `OkfFrontmatter` exposes `doc_id` + `related`; reuse it + `extract_frontmatter`.
  - `src/brain/crawl.rs` — `MdFile { path, rel, stem }` from `crawl_corpus`.
  - `src/brain/mod.rs` — register the `graph` module; the check is a corpus-wide pass over the full list.
  - `src/lib.rs` — `validate_brain` / `validate_brain_sync` are the siblings to mirror for
    `validate_brain_graph`; `Diagnostic`/`Report`/`JsonReport` (the `--json` envelope serializes all
    diagnostics — graph findings flow through unchanged).
  - `src/main.rs` — the `--sync` flag on `ValidateBrain` is the sibling pattern for `--graph`.
- **CLAUDE.md standing rules:** every behaviour change ships with tests; all four harness gates green;
  existing brain + learn-ai tests stay green.

### Scoping decisions made at authoring time (do not relitigate)

1. **Canonical node id = `scope:doc_id`**, scope from `scope_for` (crawl block), `doc_id` the authored,
   location-independent frontmatter field. A file **with** an authored `doc_id` is a **node** (globally
   unique canonical id; legal `related:` target); a file **without** one is a **leaf** (tracked by
   `scope:stem` for the leaf lint; never a node). Root files (`CLAUDE.md`/`README.md`) follow the same
   rule: frontmatter is optional, so one without a `doc_id` is a leaf and one with a `doc_id` is a node —
   no special-casing needed in this block.
2. **One global graph.** Uniqueness is checked across the whole corpus; edges resolve across scopes.
3. **Edge model is extensible.** Represent edges as `{ from: canonical_id, to_ref: String, kind }` with
   a `kind` enum that today has a single `Related` variant. Block J extracts edges from `related:` only;
   typed edges (`supersedes`/`depends-on`/`parent`) are a later block and must not require reshaping.
4. **CLI surface = a `--graph` flag** on `validate-brain`, parallel to `--sync`. `--graph` runs the OKF
   schema pass **plus** the graph check over `crawl_corpus`.
5. **`Graph` findings reuse the `Diagnostic` currency:** `E_GRAPH_*` errors and a `W_GRAPH_LEAF_TARGET`
   warning. No new `Severity` variant; `--json` needs no envelope change.
6. **D4 forward-compat — graph construction is a reusable, serializable, emittable module.** Separate
   **building** the graph from **checking** it: a `build_graph(corpus, config) -> Graph` produces an
   owned `Graph { nodes, edges }`, and `check_graph` takes that built `Graph` and returns diagnostics.
   `Graph`, the node struct, `Edge`, and `EdgeKind` all derive `serde::Serialize` so the *validated*
   graph is byte-for-byte the *emittable* graph (Phase 3B Block R loads it into Postgres). The `kind`
   discriminant is on `Edge` from day one so typed edges extend the same emitted schema with no reshape.
   Do *not* build the emitter/persistence here — only ensure the in-memory graph is the serializable
   artifact a later block emits.
7. **D5 forward-compat — metadata through a single extractor seam.** `2.J-corpus-crawl` shipped a
   `CorpusEntry` that carries only `path`/`rel`/`stem`/`scope` (no parsed metadata), so this block reads
   each entry's `doc_id`/`related` itself. Route that reading through **one** helper
   (e.g. `read_doc_metadata(entry) -> DocMeta { doc_id, related, .. }`) — the single place that knows
   metadata comes from inline Markdown frontmatter (`extract_frontmatter` + `OkfFrontmatter`). Do **not**
   scatter `extract_frontmatter`/inline-frontmatter assumptions through `build_graph`. This keeps the
   future foreign-format extractor (`.docx`/`.txt`/sidecars — `D5 — heterogeneous-format ingest`) a
   single-point swap rather than a refactor. *(The seam itself, and the corpus-model refactor, are D5
   backlog — corpus-crawl already shipped; this block only avoids adding new hardwiring.)*
8. **The graph is authored-only — never inferred.** Nodes and edges are built **solely** from
   authored/confirmed metadata (frontmatter today; reviewed sidecars later). mev does **not** infer,
   propose, or auto-apply nodes or edges — no similarity-derived `related`, no AI-suggested edges enter
   the graph here. Proposed metadata (a future `mev discover` / orchestrator AI enrichment, per D5) lands
   as **reviewable artifacts** and only becomes graph input once a human confirms it into authored
   frontmatter/sidecars. This preserves the "authored, not inferred" property that made us reject the
   Dgraph `knowledge_graph` service (D4).

### Locator codes (the `Graph` diagnostic vocabulary)

- `E_GRAPH_DUPLICATE_DOC_ID` (error) — two or more nodes share one canonical `scope:doc_id`.
- `E_GRAPH_DANGLING_RELATED` (error) — a `related:` entry resolves to no node and no leaf.
- `W_GRAPH_LEAF_TARGET` (warning) — a `related:` entry resolves to a real file that has no `doc_id`.

## Step-by-Step Tasks

### 1. Serializable graph model + `build_graph`
- Create `src/brain/graph.rs` and register `pub mod graph;` in `src/brain/mod.rs`.
- Define the **serializable, emittable** graph model (all `#[derive(serde::Serialize)]`, per decision 6):
  `enum EdgeKind { Related }`; `struct Edge { from: String, to_ref: String, kind: EdgeKind }`;
  a node struct (e.g. `Node { id: canonical_id, scope, doc_id, rel }`); and
  `struct Graph { nodes: Vec<Node>, edges: Vec<Edge> }` — the artifact Phase 3B Block R emits.
- Add the **single metadata-extractor seam** (decision 7): one helper
  `read_doc_metadata(entry) -> DocMeta { doc_id: Option<String>, related: Vec<String> }` that is the
  *only* site reusing `extract_frontmatter` + `OkfFrontmatter` to read inline Markdown frontmatter
  (`scope` comes from the `CorpusEntry`). `build_graph` calls this helper — it must not parse frontmatter
  inline anywhere else, so a future foreign-format/sidecar extractor (D5) is a one-function swap.
- Add `build_graph(corpus, config) -> Graph` (consuming the owned `Corpus`/entries from `2.J-corpus-crawl`)
  that populates nodes (files with a non-empty authored `doc_id`; canonical id = `format!("{scope}:{doc_id}")`)
  and edges (each node's `related:` entries → `Edge { from: canonical_id, to_ref, kind: Related }`). Also
  expose the lookup structures the checks need — `node ids` and `leaf_keys: Set<scope:stem>` for
  no-`doc_id` files — derived from / alongside the `Graph` (keep `build_graph` the single construction site).
- Unit tests: authored doc becomes a `Node` with `scope:doc_id`; a no-`doc_id` file is a leaf (no `Node`);
  `related:` entries become `Related` edges; same `doc_id` under two scopes → two distinct node ids;
  `serde_json::to_string(&graph)` round-trips (the graph is emittable).
- Files: `src/brain/graph.rs`, `src/brain/mod.rs`.

### 2. Graph checks — uniqueness + edge resolution + leaf lint
- In `src/brain/graph.rs`, add `pub fn check_graph(graph: &Graph, leaf_keys: &Set, ...) -> Vec<Diagnostic>`
  that **consumes the built `Graph`** from Task 1 (build once, check separately — do not re-walk the
  corpus here):
  - **Uniqueness:** any canonical id held by ≥2 nodes → one `E_GRAPH_DUPLICATE_DOC_ID` (lists the rel paths).
  - **Edge resolution** for each `Edge`: normalise `to_ref` — if it contains `:` it is qualified
    (`scope:doc_id`); otherwise bare, qualifying to `<from-scope>:<to_ref>`. Then:
    1. in `node_map` → resolved (no diagnostic);
    2. else matches a `leaf_key` → `W_GRAPH_LEAF_TARGET` (warning);
    3. else → `E_GRAPH_DANGLING_RELATED` (error). Both use locator `related`, `file` = the referrer's
       rel path, and name the target.
- Unit tests (temp-dir fixtures): duplicate canonical id → one error; same `doc_id` different scopes →
  0; bare edge resolving within scope → 0; qualified cross-scope edge resolving → 0; bare edge naming
  another scope's id → dangling; typo → dangling; edge to a leaf → one `W_GRAPH_LEAF_TARGET`.
- Files: `src/brain/graph.rs`. (Depends on Task 1.)

### 3. Public API + `--graph` CLI flag
- In `src/lib.rs`, add `pub fn validate_brain_graph(root: &Path) -> anyhow::Result<Report>` beside
  `validate_brain_sync`: resolve config (reuse the `E_CONFIG_NOT_FOUND` fallback), crawl once via
  `crawl_corpus`, run the per-item OKF schema validation, then `let graph = brain::graph::build_graph(&corpus, &config)`
  and append `brain::graph::check_graph(&graph, …)` into the same `Report`. Re-export `build_graph` +
  `Graph` (so Phase 3B Block R can emit) and `check_graph`, consistent with the module's `pub use` style.
- In `src/main.rs`, add a `--graph` flag to `ValidateBrain`; when set, dispatch to
  `mev::validate_brain_graph`; `--json` / human / exit-code branches unchanged (a graph error → exit 1;
  the leaf warning does not). Update the subcommand help text.
- Files: `src/lib.rs`, `src/main.rs`. (Depends on Task 2.)

### 4. Integration tests — end-to-end `--graph` over a multi-unit fixture
- Add `tests/brain_graph.rs` building a temp HQ-root fixture: `brain.toml` registering `brain` (`.`),
  a tier (`core`), and a repo (`mev` → `core/mev`); OKF-clean docs placed to exercise each scope.
- Tests:
  - Clean corpus (edges resolve, canonical ids unique) → report with **0 errors**.
  - Same `doc_id` under two scopes (`mev:knowledge` vs `brain:knowledge`) → **0 errors**.
  - Two nodes in one scope claiming one `doc_id` → exactly one `E_GRAPH_DUPLICATE_DOC_ID`.
  - A `brain`-scope file with `related: [mev:<id>]` resolving cross-scope → 0; rename/delete that
    target → exactly one `E_GRAPH_DANGLING_RELATED`.
  - A `related:` entry pointing at a file without a `doc_id` → exactly one `W_GRAPH_LEAF_TARGET`, 0 errors.
  - (Optional, if cheap) a `--json` round-trip asserting a graph diagnostic appears in the envelope.
- Files: `tests/brain_graph.rs`. (Depends on Task 3.)

### 5. Validate
- Run the Validation Commands listed below and confirm all pass.

## Acceptance Criteria
- A global canonical-id index keyed on `scope:doc_id` is built over the corpus crawl; scope comes from
  the registry resolver (no path encoded into `doc_id`).
- Only files with an authored `doc_id` are nodes; files without one are leaves (never uniqueness-checked).
- `mev validate-brain --graph` exists, runs the schema pass plus the graph check over `crawl_corpus`, and
  exits 1 on any graph **error** (the leaf warning alone does not fail the run).
- Bare `related:` resolves within the referrer's scope; qualified `scope:doc_id` resolves cross-scope;
  an entry resolving to neither a node nor a leaf is flagged `E_GRAPH_DANGLING_RELATED`.
- Two nodes sharing one canonical `scope:doc_id` are flagged `E_GRAPH_DUPLICATE_DOC_ID`; the same
  `doc_id` under different scopes is **not** flagged.
- A `related:` entry pointing at a real file lacking a `doc_id` is flagged `W_GRAPH_LEAF_TARGET`.
- The edge model carries a `kind` so typed edges extend it without reshaping `Edge`/`check_graph`.
- Graph construction is a reusable module: `build_graph` returns an owned `Graph { nodes, edges }`;
  `Graph`/`Node`/`Edge`/`EdgeKind` derive `Serialize` and `serde_json` round-trips (D4 emittable artifact);
  `check_graph` consumes the built `Graph` rather than re-walking the corpus.
- Inline-frontmatter parsing is confined to one `read_doc_metadata` seam (D5 forward-compat) — no
  `extract_frontmatter` calls scattered through `build_graph`/`check_graph`.
- The graph is built only from authored metadata; no node or edge is inferred, proposed, or auto-applied
  (D5 authored-only guarantee).
- All four harness gates pass; existing tests stay green.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes
- Reworked 2026-06-28: split the corpus crawl into `2.J-corpus-crawl` (run first); this block is now the
  global `scope:doc_id` graph on top of it, per `namespacing-and-corpus-decision.md`.
- Amended 2026-06-28 for **D4**: graph build is a reusable, `Serialize`-able, emittable module
  (`build_graph` → `Graph`), separate from `check_graph` — the validated graph == the graph Phase 3B
  Block R emits to Postgres.
- Amended 2026-06-29 for **D5** (decisions 7–8): metadata reading confined to one `read_doc_metadata`
  seam (future foreign-format swap), and the graph is authored-only (never inferred). These two
  forward-compat guardrails moved here because `2.J-corpus-crawl` had already shipped to review.

## Amendment Log
<!-- Append-only. Pipeline stages append one dated line here when they deviate from the spec. -->
_No amendments yet._
