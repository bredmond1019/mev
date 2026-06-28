---
type: TaskSpec
title: Task Spec — Phase 3, Block J — graph integrity (global scope:doc_id graph)
description: Decomposed task spec for the global scope:doc_id node index, uniqueness, extensible related-edge resolution, and leaf-as-target lint, surfaced via `mev validate-brain --graph`.
doc_id: 2j-graph-integrity-tasks
layer: [factory, brain]
project: mev
status: active
keywords: [graph integrity, knowledge graph, scope namespacing, doc_id, related edges, validate-brain]
related: [master-plan, status, block-j-namespacing-decision, 2j-corpus-crawl-tasks, D29-mev-brain-validation-engine]
---

# Task Spec — Phase 3, Block J — Graph integrity (global `scope:doc_id` graph)

**Status:** Not started · **Last run:** never

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
- **Plan:** `planning/master-plan.md` → Phase 3, **Block J**. Governed by brain **D29** (mev is the
  single validation engine; checks are read-only and `--json`-able).
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

### Locator codes (the `Graph` diagnostic vocabulary)

- `E_GRAPH_DUPLICATE_DOC_ID` (error) — two or more nodes share one canonical `scope:doc_id`.
- `E_GRAPH_DANGLING_RELATED` (error) — a `related:` entry resolves to no node and no leaf.
- `W_GRAPH_LEAF_TARGET` (warning) — a `related:` entry resolves to a real file that has no `doc_id`.

## Step-by-Step Tasks

### 1. Node/leaf index + extensible edge model
- Create `src/brain/graph.rs` and register `pub mod graph;` in `src/brain/mod.rs`.
- Define the edge representation: `enum EdgeKind { Related }` and `struct Edge { from: String, to_ref: String, kind: EdgeKind }` (room for future kinds).
- Add a per-file parse helper (reuse `extract_frontmatter` + `OkfFrontmatter`) returning
  `(scope, doc_id: Option<String>, related: Vec<String>)`; scope via `scope_for`.
- Add `build_node_index(items, config)` → (a) `node_map: canonical_id -> Vec<rel>` over files with a
  non-empty authored `doc_id` (canonical id = `format!("{scope}:{doc_id}")`); (b) `leaf_keys:
  Set<scope:stem>` for files without a `doc_id`; (c) `edges: Vec<Edge>` collected from each node's
  `related:` entries (`from` = the node's canonical id).
- Unit tests: authored doc becomes `scope:doc_id`; a no-`doc_id` file is a leaf (not in `node_map`);
  `related:` entries are captured as `Related` edges; same `doc_id` under two scopes yields two distinct
  canonical ids.
- Files: `src/brain/graph.rs`, `src/brain/mod.rs`.

### 2. Graph checks — uniqueness + edge resolution + leaf lint
- In `src/brain/graph.rs`, add `pub fn check_graph(items: &[MdFile], config: &BrainConfig) -> Vec<Diagnostic>`
  built on the Task 1 index:
  - **Uniqueness:** any canonical id with ≥2 nodes → one `E_GRAPH_DUPLICATE_DOC_ID` (lists the rel paths).
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
  `crawl_corpus`, run the per-item OKF schema validation, then append
  `brain::graph::check_graph(&items, &config)` into the same `Report`. Re-export `check_graph` (or expose
  only `validate_brain_graph`) consistent with the module's `pub use` style.
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

## Amendment Log
<!-- Append-only. Pipeline stages append one dated line here when they deviate from the spec. -->
_No amendments yet._
