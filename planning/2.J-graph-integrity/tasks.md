---
type: TaskSpec
title: Task Spec — Phase 3, Block J — graph integrity (scope:doc_id edges)
description: Decomposed task spec for the scope-namespaced doc_id node index, uniqueness + dangling related-edge detection, and leaf-as-target lint, surfaced via `mev validate-brain --graph`.
doc_id: 2j-graph-integrity-tasks
layer: [factory, brain]
project: mev
status: active
keywords: [graph integrity, related edges, scope namespacing, doc_id index, duplicate doc_id, validate-brain]
related: [master-plan, status, block-j-namespacing-decision, D29-mev-brain-validation-engine]
---

# Task Spec — Phase 3, Block J — Graph integrity (`scope:doc_id` `related:` edges)

**Status:** Not started · **Last run:** never

## Goal
Build a corpus-wide **`scope:doc_id`** node index over authored docs and flag every `related:` edge
that fails to resolve and every duplicate canonical id, surfaced via `mev validate-brain --graph`.

## Context Pointers

- **Authoritative design:** `planning/2.J-graph-integrity/namespacing-and-corpus-decision.md`
  (settled 2026-06-28 from the live widened corpus). It is the source of truth for the id scheme this
  block validates — read it first. Key points carried into the tasks below:
  - Canonical graph-node id = **`scope:doc_id`**. `scope` is derived from `brain.toml` (repo `slug`
    for sub-repo files, tier name for tier files, `brain` for HQ-root files); `doc_id` stays the
    authored, location-independent frontmatter field.
  - A file **with** an authored `doc_id` is a **node** (`scope:doc_id` must be globally unique; a legal
    `related:` target). A file **without** `doc_id` is a **leaf** (not a node, not a legal target);
    stem clashes among leaves are harmless.
  - `related:` resolution: a **bare `doc_id`** resolves within the *referrer's own scope*; a qualified
    **`scope:doc_id`** resolves across scopes. Existing edges are all intra-scope bare; cross-scope
    edges are new and opt-in.
- **Plan:** mev `planning/master-plan.md` → Phase 3, **Block J**. Governed by brain **D29** (mev is
  the single validation engine; checks are read-only and `--json`-able).
- **Repo files that apply:**
  - `src/brain/config.rs` — `BrainConfig` / `RepoEntry` carry `slug`, `tier`, `repo_path` (the inputs
    for scope derivation). No config struct change needed.
  - `src/brain/okf.rs` — `OkfFrontmatter` exposes `doc_id` + `related`; reuse it (plus
    `extract_frontmatter`) to read each file's id and edges.
  - `src/brain/crawl.rs` — `MdFile { path, rel, stem }`; `rel` (relative to the HQ root) is the input
    to scope derivation. `CLAUDE.md` / `handoff.md` are already file-blocklisted here.
  - `src/brain/mod.rs` — register the new `graph` module; the graph check is a corpus-wide pass over
    the full crawled `MdFile` list (not a per-item check).
  - `src/lib.rs` — `validate_brain()` / `validate_brain_sync()` are the siblings to mirror for
    `validate_brain_graph()`; `Diagnostic` / `Report` / `JsonReport` (the `--json` envelope serializes
    every diagnostic, so graph findings flow through unchanged).
  - `src/main.rs` — `ValidateBrain` subcommand with the `--sync` flag from Block N is the sibling
    pattern to mirror for `--graph`.
- **CLAUDE.md standing rules:** every behaviour change ships with tests; all four harness gates stay
  green; existing learn-ai + brain (sync) tests must keep passing.

### Scoping decisions made at authoring time (do not relitigate)

1. **CLI surface = a `--graph` flag** on `validate-brain`, parallel to Block N's `--sync`. Default
   `validate-brain` stays the fast per-file schema check (pre-commit tier); `--graph` is the fuller
   pre-push tier and runs the schema pass **plus** the graph check.
2. **The widened multi-root crawl is OUT of scope.** Pulling sub-repo docs into one corpus is the
   Bastion program's corpus-widening block (HQ master-plan: "Widening to per-repo planning/code
   corpora (Bastion Blocks O/P)" — explicitly out of scope here). Block J's job is to be *correct
   under* widening: scope derivation is a pure function of a file's rel-path + `brain.toml`, exercised
   with fixtures that simulate sub-repo / tier paths (plain dirs, no `.git`, so nested-git pruning is
   irrelevant in fixtures). Today's live single-root crawl yields only `brain`- and tier-scoped files;
   that is fine.
3. **No stem fallback for nodes.** Only authored `doc_id` files are nodes (the prior draft's
   "effective doc_id defaulting to stem" is replaced by the decision's node/leaf split). Leaves are
   tracked (by scope + stem) only to power the leaf-as-target lint.
4. **`Graph` findings reuse the existing `Diagnostic` currency.** Errors use `E_GRAPH_*` locator
   codes; the leaf-as-target lint is a `warning` with `W_GRAPH_LEAF_TARGET`. No new `Severity`
   variant; `JsonReport` serializes them automatically, so `--json` needs no envelope change.
5. **Out of scope (brain-side / content, not mev code):** editing `brain.toml` `skip_dirs` to the new
   bare-component list (`archive`, `archived`, `trees`, `sdlc`, …) — the crawl already matches bare
   names at any depth, so this is a brain-repo config commit; and authoring `doc_id`s onto the 5 bare
   `index.md` files (decision item 4, "optional polish"). The test fixtures may mirror the new
   `skip_dirs` for realism, but no production crawl change is required.

### Locator codes (the `Graph` diagnostic vocabulary)

- `E_GRAPH_DUPLICATE_DOC_ID` (error) — two or more nodes share one canonical `scope:doc_id`.
- `E_GRAPH_DANGLING_RELATED` (error) — a `related:` entry resolves to no node and no leaf (typo / deleted).
- `W_GRAPH_LEAF_TARGET` (warning) — a `related:` entry resolves to a real file that has no `doc_id`
  ("referenced but not addressable; author a `doc_id`").

## Step-by-Step Tasks

### 1. Scope derivation + node/leaf index
- Create `src/brain/graph.rs` and register it with `pub mod graph;` in `src/brain/mod.rs`.
- Implement `scope_for(rel: &Path, config: &BrainConfig) -> String`:
  - Longest-prefix match of `rel` against each `RepoEntry.repo_path` that is **not** `"."` → that
    repo's `slug`.
  - Else, if `rel`'s first component is a known tier dir (the set of first components of the non-`"."`
    `repo_path`s, e.g. `core`, `portfolio`, `side`, `client`) → that tier name.
  - Else → the root repo's slug (the entry whose `repo_path == "."`, i.e. `brain`).
- Add an internal helper that, for an `MdFile`, parses frontmatter via `extract_frontmatter` +
  `OkfFrontmatter` and returns `(scope, doc_id: Option<String>, related: Vec<String>)` (read/parse
  failure ⇒ `doc_id = None`, `related = []`).
- Add `build_node_index(items, config)` producing: (a) `node_map: canonical_id -> Vec<rel>` over
  files with a non-empty authored `doc_id` (canonical id = `format!("{scope}:{doc_id}")`); and
  (b) `leaf_keys: Set<scope:stem>` for files without a `doc_id`.
- Unit tests: `core/mev/planning/x.md` → scope `mev`; `core/docs/projects/x.md` → scope `core`;
  `planning/x.md` and `README.md` → scope `brain`; an authored doc becomes `scope:doc_id`; a
  no-`doc_id` file is recorded as a leaf, not a node.
- Files: `src/brain/graph.rs`, `src/brain/mod.rs`.

### 2. Graph checks — uniqueness + edge integrity + leaf-as-target
- In `src/brain/graph.rs`, add `pub fn check_graph(items: &[MdFile], config: &BrainConfig) -> Vec<Diagnostic>`
  built on the Task 1 index:
  - **Uniqueness:** any canonical id claimed by ≥2 nodes → one `E_GRAPH_DUPLICATE_DOC_ID` (message
    lists the colliding rel paths).
  - **Edge resolution** for each node's `related:` entries — resolve in this order:
    1. Normalise the target id: if the entry contains `:` it is qualified (`scope:doc_id`); otherwise
       it is bare and qualifies to `<referrer scope>:<entry>`.
    2. If the qualified id is in `node_map` → resolved (no diagnostic).
    3. Else if it matches a `leaf_key` → `W_GRAPH_LEAF_TARGET` (warning).
    4. Else → `E_GRAPH_DANGLING_RELATED` (error). Both diagnostics use locator `related`, `file` =
       the referring file's rel path, and name the unresolved/leaf target.
- Unit tests with temp-dir fixtures: duplicate canonical id → one error; same `doc_id` in two
  different scopes → **0** (disambiguated); bare edge resolving within scope → 0; qualified
  cross-scope edge resolving → 0; bare edge naming another scope's id → dangling; typo'd edge →
  dangling; edge resolving to a leaf (file without `doc_id`) → one `W_GRAPH_LEAF_TARGET` warning.
- Files: `src/brain/graph.rs`. (Depends on Task 1.)

### 3. Public API + `--graph` CLI flag
- In `src/lib.rs`, add `pub fn validate_brain_graph(root: &Path) -> anyhow::Result<Report>` beside
  `validate_brain_sync`: resolve `brain.toml` via the existing root/config resolver (reuse the
  `E_CONFIG_NOT_FOUND` fallback), crawl once via `BrainValidator`, run per-item OKF schema
  validation, then append `brain::graph::check_graph(&items, &config)` into the same `Report`.
  Re-export `check_graph` (or expose only `validate_brain_graph`) consistent with the module's
  `pub use` style.
- In `src/main.rs`, add a `--graph` flag to the `ValidateBrain` subcommand. When set, dispatch to
  `mev::validate_brain_graph`; the existing `--json` / human and exit-code branches stay unchanged
  (a graph error makes `report.is_failure()` true → exit 1; the leaf warning does not). Update the
  subcommand help text to mention the `--graph` corpus integrity check.
- Files: `src/lib.rs`, `src/main.rs`. (Depends on Task 2.)

### 4. Integration tests — end-to-end `--graph` over a fixture tree
- Add `tests/brain_graph.rs` that builds a temp HQ-root fixture: a `brain.toml` with `[[repos]]` for
  `brain` (`repo_path = "."`) and at least one sub-repo (e.g. `mev`, `repo_path = "core/mev"`) plus a
  tier dir (`core/docs/...`), full `[vocab]` so the schema pass is clean, and OKF-clean docs placed to
  exercise each scope. (Plain dirs, no `.git`.)
- Tests:
  - Clean corpus (all edges resolve, canonical ids unique) → report with **0 errors**.
  - Same `doc_id` under two scopes (`mev:knowledge` vs `brain:knowledge`) → **0 errors** (scope
    disambiguates).
  - Two files in one scope claiming one `doc_id` → exactly one `E_GRAPH_DUPLICATE_DOC_ID`.
  - A brain-scope file with `related: [mev:<id>]` resolving to a node in the `mev` scope → 0 errors
    (cross-scope qualified edge works); rename/delete that target → exactly one
    `E_GRAPH_DANGLING_RELATED`.
  - A `related:` entry pointing at a file that has no `doc_id` → exactly one `W_GRAPH_LEAF_TARGET`
    warning, 0 errors.
  - (Optional, if cheap) a `--json` round-trip asserting a graph diagnostic appears in the envelope.
- Files: `tests/brain_graph.rs`. (Depends on Task 3.)

### 5. Validate
- Run the Validation Commands listed below and confirm all pass.

## Acceptance Criteria
- A corpus-wide canonical-id index keyed on `scope:doc_id` is built; `scope` is derived purely from a
  file's rel-path + `brain.toml` (repo slug / tier / `brain`), with no path encoded into `doc_id`.
- Only files with an authored `doc_id` are nodes; files without one are leaves (never nodes, never
  uniqueness-checked).
- `mev validate-brain --graph` exists, runs the schema pass plus the graph check, and exits 1 when any
  graph **error** is present (the leaf warning alone does not fail the run).
- A bare `related:` entry resolves within the referrer's scope; a qualified `scope:doc_id` entry
  resolves across scopes; an entry resolving to neither a node nor a leaf is flagged
  `E_GRAPH_DANGLING_RELATED`.
- Two nodes sharing one canonical `scope:doc_id` are flagged `E_GRAPH_DUPLICATE_DOC_ID`; the same
  `doc_id` under different scopes is **not** flagged.
- A `related:` entry pointing at a real file that lacks a `doc_id` is flagged `W_GRAPH_LEAF_TARGET`.
- A clean corpus passes with zero graph errors.
- All four harness gates pass (`fmt`, `clippy -D warnings`, `test`, `build`); existing tests stay green.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes
- Reworked 2026-06-28 from the original bare-`doc_id`/stem-fallback draft to the `scope:doc_id` scheme
  per `namespacing-and-corpus-decision.md`. The in-flight `/sdlc-flow` worktree had only the init
  commit (no `graph.rs`), so no implementation work was discarded — restart the flow against this spec.

## Amendment Log
<!-- Append-only. Pipeline stages append one dated line here when they deviate from the spec. -->
_No amendments yet._
