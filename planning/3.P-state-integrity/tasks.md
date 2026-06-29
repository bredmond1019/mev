---
type: TaskSpec
title: Task Spec — Phase 3, Block P — state.json integrity (schema + cross-repo block graph)
description: Decomposed task spec for `mev validate-brain --state` — schema validation of each repo's planning/state.json plus the cross-repo block-dependency graph integrity check (blocked_by / cross_repo edges resolve).
doc_id: 3p-state-integrity-tasks
layer: [factory, brain]
project: mev
status: draft
keywords: [state.json, validate-brain, cross-repo, block graph, blocked_by, dependency graph, D29]
related: [master-plan, status, state-json-schema, D29-mev-brain-validation-engine, block-n-sync-watermark-tasks]
---

# Task Spec — Phase 3, Block P — `state.json` integrity

**Status:** Draft · **Authored:** 2026-06-29

## Goal

Implement `mev validate-brain --state`: discover and validate every repo's `planning/state.json`
against the canonical schema, and check the **cross-repo block-dependency graph** for referential
integrity — so a `blocked_by` edge pointing at a block that doesn't exist (or a brain rollup that has
drifted from its child) becomes a deterministic, machine-caught failure instead of silent rot.

This is the work-block analogue of `MV.3.J` (graph integrity over `related:` doc edges). Where
`MV.3.J` validates the **document graph** (`scope:doc_id` nodes, `related:` edges), this block
validates the **work-block graph** (block-ID nodes, `blocked_by` / `cross_repo` edges) — the second
graph over the same corpus. The marquee check (`E_STATE_DANGLING_BLOCKED_BY`) is the direct port of
`E_GRAPH_DANGLING_RELATED` from docs to blocks.

## Context Pointers

- **Plan:** mev `planning/master-plan.md` → Phase 3, **Block P — State integrity**. Governed by brain
  **D29** (mev is the single validation engine; `bastion validate` is the front door). Read-only
  diagnostics, never mutates the corpus (upholds D25), `--json`-able so an agent can act on findings.
- **Schema (source of truth):** `core/planning/state-schema.md` (`doc_id: state-json-schema`) — the
  canonical schema + leaf/brain templates. **Validate against this; do not relitigate the format.**
  Key shape recap:
  - Every file: `{ repo, kind, updated, focus }`. `kind` ∈ `"project"` | `"brain"`.
  - `focus = { now[], next[], blocked[] }`. `now` entries: `{ block, title, status, note? }`;
    `next` entries: `{ block, title }`; `blocked` entries: `{ block, title, blocked_by[] }`.
  - `status` ∈ `open` · `in_progress` · `blocked` · `closed`.
  - `blocked_by[]` entries are tagged by `type`: `{ type:"block", repo, id, what? }` or
    `{ type:"external", what }`.
  - **Leaf** (`kind:"project"`) adds `tracks[]` — the roadmap catalog `[{ title, blocks:[{id,title,status}] }]`.
  - **Brain** (`kind:"brain"`) adds `repos[]` (denormalized child rollup) + `cross_repo[]`
    (`[{ from:{repo,block}, to:{repo,block}, note }]`); HQ also adds `tiers[]`.
- **The state.json files that exist today** (live tree, 2026-06-29):
  - Brain: `agentic-portfolio/planning/state.json` (HQ, `repo:"hq"`), `core/planning/state.json`
    (core sub-brain).
  - Leaf: `core/{bastion,orchestrator,mev}/planning/state.json`. (`bastion-ui`, `bella` not yet seeded
    — see scoping decision 5.)
- **Closest prior block to mirror:** `MV.3.M` (sync watermark) — `archive/block-n-sync-watermark/tasks.md`.
  It established the **cross-repo read mode** (read files from gitignored sub-repos by absolute path
  off the HQ root, *not* via the corpus crawl) and the `E_*` locator-code + `Diagnostic`-currency
  pattern this block reuses wholesale.
- **Repo files that apply:**
  - `src/brain/config.rs` — `BrainConfig` / `RepoEntry`; `[[repos]]` carries each leaf repo's `path`.
    `BrainConfig::projects()` is the access pattern for discovering leaf repos.
  - `src/brain/sync.rs` — the cross-repo read template (`check_sync` reads files by
    `root.join(rel)` and tolerates missing/malformed inputs with distinct codes). Mirror its shape.
  - `src/brain/graph.rs` — the `Graph`/`Node`/`Edge`/`EdgeKind` model and the `build_graph` /
    `check_graph` split (build a serializable artifact, then check it without re-reading). Mirror this
    structure for the state graph (forward-compat note below).
  - `src/lib.rs` — `validate_brain`, `validate_brain_sync`, `validate_brain_graph` (add
    `validate_brain_state` beside them); `Diagnostic` / `Report` / `JsonReport`.
  - `src/main.rs` — `ValidateBrain` subcommand + `--sync` / `--graph` / `--json` flags (add `--state`).
  - `src/shared.rs` — reuse helpers; note state files are **JSON**, parsed with `serde_json`, not
    `extract_frontmatter`.
- **CLAUDE.md standing rules:** every behaviour change ships with tests (rule 1); all four harness
  gates stay green; decisions append-only.

### Scoping decisions made at authoring time (do not relitigate)

1. **Cross-repo read mode, not the corpus crawl.** State files live at `<repo>/planning/state.json`,
   including in the **gitignored, nested-git sub-repos** that `MV.3.J-crawl`'s nested-git pruning makes
   invisible. So discovery follows `MV.3.M` (read by absolute path off the HQ root), **not** the
   `ContentValidator` corpus pass. Discover via: each `brain.toml` `[[repos]]` entry's `path` +
   `/planning/state.json` (leaf files), plus the HQ-root `planning/state.json` and each tier
   sub-brain's `planning/state.json` (brain files). This is **not** a new `ContentValidator` impl.
2. **Struct validation, not an external JSON Schema file.** Mirror how OKF is validated — serde structs
   (`StateFile`, `Focus`, `Block`, `BlockedBy` enum tagged by `type`, `Track`, `RepoRollup`,
   `CrossRepoEdge`) with `serde_json`, then explicit checks after deserialization. **Do not** add the
   `jsonschema`/`valico` crate — out of scope, and inconsistent with the rest of the engine.
3. **The state graph build is forward-compat for emit (D4).** Build a **`Serialize`-able** state graph
   (`StateNode` = a block; `StateEdge { from, to_ref, kind }` with `kind` ∈ `BlockedBy` | `CrossRepo`)
   in a reusable `build_state_graph` step, then `check_state_graph` over it — same build/check split as
   `MV.3.J`. The graph mev *validates* here is the graph a future block *emits* (the state-graph
   parallel to `MV.3B.R`, and the input to mev *generating* the brain `repos[]`/`cross_repo[]` rollup —
   "Direction 2", a separate additive block). Do not bury it in a build-check-discard function.
4. **Rollup drift is a WARNING, not an error.** The brain `repos[]` rollup legitimately lags between
   `/log-work` runs — same rationale the brain documents for why program trackers are not auto-synced
   ("Status columns are hand-refreshed snapshots that lag"). `W_STATE_ROLLUP_DRIFT` surfaces the drift
   without failing the gate. *Referential* errors (dangling `blocked_by`, unknown repo) stay
   `Error`-severity → exit 1.
5. **A missing state.json for a registered repo is a WARNING during rollout.**
   `W_STATE_FILE_MISSING` — `bastion-ui` and `bella` are registered but not yet seeded. Promote to
   `Error` later, once all core repos carry the file (track as a follow-on, not in this block).
6. **`updated` is not date-validated here.** Watermark/freshness comparison is `MV.3.M`'s job; this
   block only checks `updated` is present and a non-empty string.

### Locator codes (the `State` diagnostic vocabulary)

Schema ring:
- `E_STATE_MALFORMED_JSON` — file is not parseable JSON.
- `E_STATE_SCHEMA_MISSING_FIELD` — a required key is absent (`repo` / `kind` / `updated` / `focus`, or
  a required sub-field per the template).
- `E_STATE_SCHEMA_BAD_KIND` — `kind` ∉ `{project, brain}`.
- `E_STATE_SCHEMA_BAD_STATUS` — a `status` value ∉ the enum.
- `E_STATE_SCHEMA_BAD_BLOCKED_BY` — a `blocked_by[]` entry has an unknown `type`, or is missing the
  fields its `type` requires (`block` needs `repo`+`id`; `external` needs `what`).

Integrity ring (intra-repo):
- `E_STATE_DUPLICATE_BLOCK_ID` — two `tracks[]` blocks in one repo share an `id`.
- `E_STATE_DANGLING_FOCUS` — a `focus.now/next/blocked` entry's `block` is absent from that repo's
  `tracks[]`. *(Leaf files only; brain `focus` entries are cross-repo — see below.)*

Integrity ring (cross-repo) — the marquee:
- `E_STATE_DANGLING_BLOCKED_BY` — a `{type:"block", repo, id}` dependency names a block that does not
  exist in the target repo's `tracks[]`. *(The port of `E_GRAPH_DANGLING_RELATED`.)*
- `E_STATE_UNKNOWN_REPO` — a `blocked_by` / `cross_repo` / brain-`focus` entry names a `repo` not in
  the registry (no discoverable state.json for it).
- `E_STATE_DANGLING_CROSS_REPO` — a brain `cross_repo[]` edge's `from` or `to` endpoint (`{repo,block}`)
  does not resolve to a real block.

Rollup ring (brain files):
- `W_STATE_ROLLUP_DRIFT` — a brain `repos[]` headline (a child's `now`/`next`/`blocked` as cached) does
  not match that child's *actual* `state.json` `focus`.
- `W_STATE_FILE_MISSING` — a registered repo has no `planning/state.json`.

## Step-by-Step Tasks

### 1. [~] Foundation — serde model + JSON loader
- Add `serde_json` is already a dependency (confirm). Create `src/brain/state.rs` and register it with
  `pub mod state;` in `src/brain/mod.rs`.
- Define the serde model mirroring `state-schema.md` (all collections default-empty, extras tolerated):
  `StateFile { repo, kind, updated, focus, tracks?, repos?, cross_repo?, tiers? }`;
  `Focus { now: Vec<Block>, next: Vec<Block>, blocked: Vec<Block> }`;
  `Block { block, title, status?, note?, repo?, blocked_by: Vec<BlockedBy> }` (lenient superset across
  now/next/blocked variants); `BlockedBy` as an **internally-tagged enum** on `type`
  (`Block { repo, id, what? }` | `External { what }`); `Track { title, blocks: Vec<TrackBlock> }`,
  `TrackBlock { id, title, status }`; `RepoRollup { repo, tier?, now, next, blocked }`;
  `CrossRepoEdge { from: Endpoint, to: Endpoint, note? }`, `Endpoint { repo, block }`.
- Add `fn load_state(path: &Path) -> Result<StateFile, StateLoadError>` — read + `serde_json::from_str`,
  surfacing a parse failure distinctly so the caller can emit `E_STATE_MALFORMED_JSON`.
- Unit tests: the three live leaf files and two brain files deserialize clean (copy minimal fixtures);
  a malformed-JSON fixture surfaces the parse error; an unknown `blocked_by` `type` is rejected.
- Files: `src/brain/state.rs`, `src/brain/mod.rs`, `Cargo.toml` (only if `serde_json` needs a feature).

### 2. [~] Registry discovery + schema-ring checks
- Add `fn discover_state_files(root: &Path, config: &BrainConfig) -> Vec<StateSource>` where
  `StateSource { repo_slug, abs_path, expected_kind }`: HQ-root `planning/state.json` (brain) + each
  tier sub-brain root (brain) + each `[[repos]]` `path`/`planning/state.json` (project). Missing file →
  `W_STATE_FILE_MISSING` (decision 5).
- Add `fn check_schema(src: &StateSource, file: &StateFile) -> Vec<Diagnostic>`: required-field
  presence, `kind` membership, `status` enum on every `focus` block, `blocked_by` well-formedness, and
  `kind`-appropriate sections (`project` should carry `tracks`; `brain` should carry `repos`). Emit the
  schema-ring codes.
- Unit tests (temp-dir fixtures, mirror `sync.rs` style): clean file → 0; bad `status` → one
  `E_STATE_BAD_STATUS`; `blocked_by` missing `id` → one `E_STATE_SCHEMA_BAD_BLOCKED_BY`; bad `kind` →
  one `E_STATE_SCHEMA_BAD_KIND`.
- Files: `src/brain/state.rs`. (Depends on Task 1.)

### 3. [~] State graph build + integrity checks
- Add `build_state_graph(files: &[(StateSource, StateFile)]) -> StateGraph` (Serialize-able, decision 3):
  nodes = every `tracks[]` block keyed `repo:id`; edges = `blocked_by` block deps
  (`kind: BlockedBy`) and brain `cross_repo[]` (`kind: CrossRepo`). Build the `repo:id → node` lookup
  here.
- Add `check_state_graph(graph: &StateGraph, files: &[...]) -> Vec<Diagnostic>`:
  - duplicate `tracks` id within a repo → `E_STATE_DUPLICATE_BLOCK_ID`;
  - leaf `focus` block not in its own `tracks` → `E_STATE_DANGLING_FOCUS`;
  - `blocked_by {type:block,repo,id}` whose `repo` is unknown → `E_STATE_UNKNOWN_REPO`; whose `id`
    doesn't resolve in a known repo → `E_STATE_DANGLING_BLOCKED_BY`;
  - brain `cross_repo` endpoint unresolved → `E_STATE_DANGLING_CROSS_REPO` (unknown repo →
    `E_STATE_UNKNOWN_REPO`).
- Unit tests: a clean two-repo fixture passes; a `blocked_by` pointing at a nonexistent target id in a
  real repo yields exactly one `E_STATE_DANGLING_BLOCKED_BY`; an unknown repo yields one
  `E_STATE_UNKNOWN_REPO`; a duplicate id yields one `E_STATE_DUPLICATE_BLOCK_ID`.
- Files: `src/brain/state.rs`. (Depends on Task 2.)

### 4. [~] Rollup-drift check (brain files)
- Add `check_rollup(brain: &StateFile, children: &HashMap<String, StateFile>) -> Vec<Diagnostic>`: for
  each `repos[]` entry, compare its cached `now`/`next`/`blocked` headline against the child's actual
  `focus` (compare on block-id sets; ignore `title`/`note` cosmetic differences). Mismatch →
  `W_STATE_ROLLUP_DRIFT` (warning, decision 4).
- Unit tests: in-sync rollup → 0; a child advanced past its cached `repos[]` entry → one
  `W_STATE_ROLLUP_DRIFT`.
- Files: `src/brain/state.rs`. (Depends on Task 1.)

### 5. [~] Public API + CLI `--state` flag
- In `src/lib.rs`, add `pub fn validate_brain_state(root: &Path) -> anyhow::Result<Report>` beside the
  siblings: resolve `brain.toml` via `find_brain_config`, run the normal `BrainValidator` schema pass,
  then append `discover → load → check_schema → build_state_graph → check_state_graph → check_rollup`
  diagnostics into the same `Report`. Re-export consistent with the module's `pub use` style.
- In `src/main.rs`, add a `--state` flag to `ValidateBrain`; when set, dispatch to
  `validate_brain_state`. `--json` / human / exit-code branches unchanged (a `State` error makes
  `report.is_failure()` true → exit 1; a lone drift warning exits 0). Update the subcommand `about`/help.
- Files: `src/lib.rs`, `src/main.rs`. (Depends on Tasks 2–4.)

### 6. [~] Integration tests — end-to-end `--state` over a fixture tree
- Add `tests/brain_state.rs` building a temp HQ-root fixture: a `brain.toml` with two `[[repos]]`, each
  with a leaf `planning/state.json`, plus a brain `planning/state.json` whose `repos[]` rolls them up,
  plus a `cross_repo` edge between them.
- Tests:
  - clean fixture → `validate_brain_state` returns **0 errors**;
  - point one repo's `blocked_by` at a nonexistent target id → **exactly one**
    `E_STATE_DANGLING_BLOCKED_BY`;
  - advance a child past its brain `repos[]` headline → **exactly one** `W_STATE_ROLLUP_DRIFT`, and the
    report still has 0 *errors* (exits 0);
  - a `--json` round-trip asserting a `State` diagnostic appears in the serialized envelope.
- Files: `tests/brain_state.rs`. (Depends on Task 5.)

### 7. [x] Validate
- Run the Validation Commands below and confirm all pass. Then run the real thing:
  `cargo run -- validate-brain --state ~/Dev/agentic-portfolio` and confirm it parses all five live
  state.json files and reports clean (or surfaces only intended findings).

## Acceptance Criteria
- `mev validate-brain --state` exists, runs the OKF schema pass plus the state checks, and exits 1 when
  any `State` **error** is present (drift-only runs exit 0).
- Every discoverable `planning/state.json` (HQ + each tier brain + each `[[repos]]` leaf) is parsed and
  schema-checked against `state-schema.md`; a malformed-JSON or bad-enum file is flagged with the
  documented locator code.
- A `blocked_by` `{type:"block",repo,id}` whose target block does not exist in the named repo's
  `tracks[]` produces **exactly one** `E_STATE_DANGLING_BLOCKED_BY` for that edge; a clean cross-repo
  graph passes. An unknown `repo` produces `E_STATE_UNKNOWN_REPO`.
- A brain `repos[]` rollup that has drifted from its child's actual `focus` produces a
  `W_STATE_ROLLUP_DRIFT` **warning** (not an error).
- A registered repo with no `planning/state.json` produces `W_STATE_FILE_MISSING` (warning).
- The state graph build (`StateGraph`/`StateNode`/`StateEdge`) is `Serialize`-able and separated from
  the check pass (forward-compat for a future emit block).
- `cargo run -- validate-brain --state ~/Dev/agentic-portfolio` parses all five live files clean.
- All four harness gates pass (`fmt`, `clippy -D warnings`, `test`, `build`); existing tests stay green.

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
