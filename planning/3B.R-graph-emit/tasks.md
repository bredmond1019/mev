---
type: TaskSpec
title: MV.3B.R — Graph emit (knowledge-graph JSON artifact)
description: Task spec for emitting the scope:doc_id knowledge graph as a JSON artifact (nodes + related edges + leaves) via a mev emit-graph subcommand, for the orchestrator's Postgres edges table + structural query surface (D4).
doc_id: 3BR-graph-emit-tasks
layer: [factory]
project: mev
status: active
keywords: [graph emit, knowledge graph, emit-graph, nodes, edges, structural query, D4]
related: [master-plan, status, D4-corpus-engine-and-knowledge-graph]
---

# Task Spec — Phase 3B, Block R (MV.3B.R)

**Status:** In progress · **Last run:** never

## Goal
Emit the `scope:doc_id` knowledge graph — authored nodes, `related:` edges, and marked leaves — as a canonical JSON artifact via a `mev emit-graph` subcommand, so the orchestrator can load it into a Postgres edges table beside `brain_documents`.

## Context Pointers
- **Plan section:** `planning/master-plan.md` → Phase 3B → `MV.3B.R — Graph emit + structural query surface` (D4). mev's in-repo scope is the **graph-JSON emit only**; the Postgres load + structural-query surface (bastion/MCP) are the out-of-repo orchestrator companion and are **out of scope** here.
- **Reuse (do not rebuild):** `src/brain/graph.rs` already produces the fully serializable graph — `build_graph(&Corpus, &BrainConfig) -> GraphArtifact { graph: Graph { nodes, edges }, node_map, leaf_keys }`. `Node`, `Edge`, `EdgeKind`, `Graph` all already derive `serde::Serialize` (the D4 forward-compat work landed in `MV.3.J`). This block adds only the **emit envelope + driver + CLI + tests + docs** on top of that.
- **Direct template:** the `MV.3B.Q` manifest-emit block is the exact parallel to follow — `src/brain/manifest.rs::build_manifest` (envelope with `version`/`root` header), `lib.rs::manifest_brain` (driver: `find_brain_config` → `crawl_corpus` → build), `main.rs::Command::Manifest` (`--pretty`, compact-by-default, JSON to stdout, exit 0/1), and `tests/brain_manifest.rs` (temp-dir + `brain.toml` fixtures).
- **Naming:** the subcommand is `emit-graph` (parallels the sibling `emit-state`; deliberately distinct from the existing `generate-graph`, which emits an interactive HTML visual, not JSON).
- **CLAUDE.md:** Standing Rule 1 (tests ship with every behaviour change); Standing Rule 2 (OKF frontmatter + `index.md` update for any new doc). `planning/harness.json` supplies the four gated checks.

## Design notes (scope of the artifact)
- **Envelope:** a new `GraphExport { version: String, root: String, nodes: Vec<Node>, edges: Vec<Edge>, leaves: Vec<String> }` — mirrors `Manifest`'s `version`/`root` header. `version = "1"`.
- **Leaves marked (AC):** the emitted artifact must include the corpus's leaf files (files with no authored `doc_id`) so a consumer can distinguish a `related:` target that is a real-but-leaf file from a dangling one. Source them from `GraphArtifact::leaf_keys` (the `scope:stem` set), emitted as a **sorted** `Vec<String>` for deterministic output.
- **Nodes/edges** come straight from `artifact.graph` in walk order (already deterministic). Do not re-derive or re-walk.
- **Pure compiler (D4):** the builder returns a value; nothing is written to disk or a DB. The CLI serialises to stdout.

## Step-by-Step Tasks

### 1. Graph-export module (`src/brain/graph_emit.rs`)
- Create `src/brain/graph_emit.rs`. Define `GraphExport { version, root, nodes, edges, leaves }` deriving `serde::Serialize` (reuse `Node`/`Edge` from `crate::brain::graph`).
- Add `build_graph_export(root: &Path, artifact: &GraphArtifact) -> GraphExport`: `version = "1"`; `root = root.display().to_string()`; clone `artifact.graph.nodes` / `artifact.graph.edges`; `leaves` = `artifact.leaf_keys` collected into a `Vec<String>` and **sorted** for determinism.
- Register `pub mod graph_emit;` in `src/brain/mod.rs` (append to the module list).
- Unit tests in the module: node/edge/leaf mapping from a small hand-built `GraphArtifact` (or via `build_graph` over a temp-dir corpus, mirroring the `graph.rs` test helper), leaves sorted, empty-corpus → empty vecs, and a `serde_json::to_string` round-trip asserting `version`/`root`/`nodes`/`edges`/`leaves` keys are present.
- **Files:** `src/brain/graph_emit.rs` (new), `src/brain/mod.rs` (append one `pub mod` line).

### 2. Library driver + `emit-graph` CLI subcommand
- Add `graph_brain(root: &Path) -> anyhow::Result<GraphExport>` to `src/lib.rs`, mirroring `manifest_brain`: `find_brain_config(root)` (map error to `anyhow` mentioning `brain.toml`) → `crawl_corpus(root, &config)` → `build_graph(&corpus, &config)` → `build_graph_export(root, &artifact)`. Re-export `GraphExport` from the crate root alongside `Manifest`.
- Add `Command::EmitGraph { path: PathBuf (default "."), pretty: bool }` to `src/main.rs` with a doc-comment block (purpose, `--pretty`, exit codes 0/1) matching the `Manifest`/`EmitState` style, and a handler mirroring `Command::Manifest`: resolve root via `find_brain_root`, call `mev::graph_brain`, serialise compact (or `to_string_pretty` with `--pretty`) to stdout, exit `SUCCESS`; config/serialisation errors → `eprintln!` + `FAILURE`.
- **Files:** `src/lib.rs`, `src/main.rs`.

### 3. Integration tests (`tests/brain_graph_emit.rs`)
- New `tests/brain_graph_emit.rs` mirroring `tests/brain_manifest.rs` (temp-dir helper + `write_brain_toml` + OKF-doc helper). Cover: (a) a small corpus with two linked nodes + one leaf → `graph_brain` returns the expected node count, an edge from → to_ref, and the leaf present in `leaves`; (b) round-trip — `serde_json::to_string` parses back as a `serde_json::Value` with `version == "1"` and `nodes`/`edges`/`leaves` arrays; (c) a `related:` edge to a `doc_id` node is present in `edges`; (d) `graph_brain` errors (mentioning `brain.toml`) when no `brain.toml` is present.
- **Files:** `tests/brain_graph_emit.rs` (new).

### 4. Documentation (`docs/cli.md`, `docs/architecture.md`)
- `docs/cli.md`: add an `### emit-graph [--pretty] [path]` subcommand section (synopsis, output shape with a sample JSON envelope, exit codes, examples) modelled on the existing `manifest` section; note the distinction from `generate-graph`.
- `docs/architecture.md`: add `graph_emit.rs` to the module map, an entry for `build_graph_export` / `GraphExport` under the knowledge-graph section, and `brain_graph_emit.rs` to the integration-tests list.
- Update `docs/index.md` only if a new file row is warranted (no new top-level doc is added here, so likely unchanged — confirm).
- **Files:** `docs/cli.md`, `docs/architecture.md` (and `docs/index.md` if needed).

### 5. Validate
- Run the Validation Commands listed below and confirm all pass.
- Sanity-run `mev emit-graph ~/Dev/agentic-portfolio | jq '{nodes: (.nodes|length), edges: (.edges|length), leaves: (.leaves|length)}'` against the live brain and record the counts in Notes (no false-positive triage expected — this is a pure emit, not a check).

## Acceptance Criteria
- `mev emit-graph [path]` prints a JSON envelope `{ version, root, nodes[], edges[], leaves[] }` to stdout; compact by default, indented with `--pretty`; exit 0 on success, 1 on config/serialisation error.
- Every authored `doc_id` file in the corpus appears exactly once in `nodes` (as `scope:doc_id`); every `related:` entry appears in `edges` as `{from, to_ref, kind}`; every no-`doc_id` corpus file appears in `leaves` (as `scope:stem`).
- The emitted JSON round-trips: `serde_json::from_str::<serde_json::Value>` succeeds and `version == "1"`.
- `leaves` is deterministically ordered (sorted) so repeated runs over an unchanged corpus emit byte-identical output.
- mev writes nothing to any DB or file (pure stdout emit).
- A missing `brain.toml` produces an error mentioning `brain.toml` and exit 1.
- All four harness gates pass; the existing test suite stays green (no signature changes to `build_graph`/`check_graph`).

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes
<filled in as work happens>

## Amendment Log
<!-- Append-only. Pipeline stages append one dated line here when they deviate from the spec. -->
_No amendments yet._
