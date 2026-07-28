---
type: Reference
title: mev Architecture
description: Module map, ContentValidator trait, and core types — how mev's pluggable validation pipeline is structured
doc_id: architecture
layer: [factory]
project: mev
status: active
keywords: [architecture, ContentValidator, Diagnostic, Report, modules, trait, mev]
related: [cli-reference, brain-toml-config, okf-schema]
---

# mev Architecture

## Module map

```
src/
├── lib.rs          ← crate root: core types (Diagnostic, Report, JsonReport) + public re-exports
├── main.rs         ← clap CLI — thin wrapper: parse → dispatch → exit code
├── shared.rs       ← internal helpers: extract_frontmatter, is_kebab_case, non_empty
├── validator.rs    ← ContentValidator trait (the extension point)
├── learn_ai/
│   ├── mod.rs      ← LearnAiValidator (implements ContentValidator)
│   ├── crawl.rs    ← crawl() → (Vec<ContentFile>, Vec<Diagnostic>); ContentFile, Corpus, FileKind, Locale
│   └── meta.rs     ← validate_file() — per-file frontmatter + JSON struct checks
└── brain/
    ├── mod.rs      ← BrainValidator (implements ContentValidator); wires crawl_corpus
    ├── config.rs   ← BrainConfig, CrawlConfig, VocabConfig, RepoEntry; find_brain_config(), load_brain_config(); BrainConfig::scope_dependencies(), ScopeDependencySet, ScopeError — `emit-state --scope` dependency resolution (ticket-emit-state-scope-and-lock)
    ├── crawl.rs    ← crawl_corpus() → (Corpus, Vec<Diagnostic>); Corpus, CorpusEntry, MdFile; crawl_brain() (legacy)
    ├── okf.rs      ← OkfFrontmatter (re-exported from bastion's okf-core crate, BA.15.12/D15/D16), validate_md_file() — OKF field checks; root-instruction-file exemption
    ├── scope.rs    ← scope_units(), scope_for(), owning_unit() — registry-driven scope resolver
    ├── sync.rs     ← (internal) sync helpers
    ├── graph.rs    ← EdgeKind, Edge, Node, Graph, GraphArtifact, EdgeResolution, resolve_edge (re-exported from okf-core, BA.15.12/D16), DocMeta; build_graph(), check_graph() — Phase 3 Block J (read_doc_metadata removed by D5 extract-once refactor in Block Q)
    ├── state.rs    ← serde schema, StateGraph, StateNode, StateEdge, StateSource, TierEntry, load_state(), build_state_graph() (re-exported from okf-core, BA.15.12/D15/D16); mev-local: discover_state_files(), check_schema(), check_state_graph(), check_rollup(), detect_cycles(), ready_order(), check_focus_drift(), derive_focus(), derive_rollup(), derive_cross_repo(), tier_scope_for(), derive_brain_focus(), check_epics(), derive_epic_focus(), derive_epic_edges() — Phase 3 Block P / P2 / T / MV.3B.U (v2: depends_on DAG, cycle detection, derived-blocked enforcement, backlog nodes, focus-drift warnings, single-source derivation helpers; MV.3B.U: tier-scoped non-destructive rollup + brain-focus union; epics: cross-repo initiative registry + membership integrity + derived cross-epic relationships)
    ├── emit.rs     ← EmitError, EmitAction, EmitPlan, markers (WAVE_TABLE, PROJECT_CACHE, TIER_ROLLUP, HQ_BOARD, UNIFIED_BOARD); wave_order(), render_wave_table(), global_status_map(), splice_generated(), plan_state_json(), plan_master_plan_tables(), plan_project_caches(), plan_tier_rollups(), render_hq_board(), plan_hq_board(), render_unified_board(), plan_unified_board(), apply_plan(), filter_plan_by_scope() — Phase 3 Block T (derived-view generation: wave tables, focus regen, brain rollup; MV.4.A: cross-repo depends_on resolution via global_status_map; MV.4.B: project-cache + tier-rollup splice; MV.4.C: HQ root Operating Board splice; MV.6.B: priority-ranked unified NOW/NEXT/BLOCKED/DUE-SOON board splice; ticket-emit-state-scope-and-lock: filter_plan_by_scope() narrows an already-built EmitPlan down to one repo's ScopeDependencySet targets, applied after planning so scoping cannot change which actions the unscoped planners themselves would compute)
    ├── lock.rs     ← LockError, LockGuard, acquire_lock(), DEFAULT_LOCK_TIMEOUT — advisory lockfile (`<root>/.mev-emit.lock`) guarding `emit-state --write` against concurrent writers; stale (dead-pid) lockfiles are reclaimed automatically (ticket-emit-state-scope-and-lock)
    ├── links.rs    ← LinkKind, LinkRef; extract_links(), check_links(), collect_doc_ids(), read_moves_pending(), check_moved_references() — Phase 3 Block K
    ├── structure.rs ← check_structure() — Phase 3 Block L (bidirectional index.md <-> directory structural coverage: orphan files, dangling rows)
    ├── manifest.rs ← ManifestEntry, Manifest, build_manifest() — Phase 3 Block Q (canonical corpus manifest for RAG indexer)
    └── graph_emit.rs ← GraphExport, ExportedEdge, build_graph_export() (re-exported from okf-core, BA.15.12/D16) — Phase 3B Block R (graph-export envelope for the orchestrator's Postgres edges table, D4)
└── doc/
    ├── mod.rs             ← module docs + re-exports — Phase 9 Block MV.9.A
    ├── materialize.rs     ← plan_document() — generic per-model doc planner
    ├── index_reconcile.rs ← plan_index_reconcile() — index.md row upsert
    └── opportunity.rs     ← OpportunityKind, plan_ingest/plan_set_stage/plan_add_action/plan_merge_contacts

tests/
├── brain_config.rs    ← integration tests for brain.toml loading + BrainConfig
├── brain_corpus.rs    ← integration tests for crawl_corpus() multi-root walk + scope resolution
├── brain_crawl.rs     ← integration tests for crawl_brain()
├── brain_graph.rs     ← integration tests for validate_brain_graph() end-to-end — Phase 3 Block J
├── brain_graph_emit.rs ← integration tests for graph_brain() end-to-end — Phase 3B Block R
├── brain_links.rs     ← integration tests for validate_brain_links() end-to-end — Phase 3 Block K
├── brain_manifest.rs  ← integration tests for manifest_brain() end-to-end — Phase 3 Block Q
├── brain_okf.rs       ← integration tests for validate_md_file()
├── brain_state.rs     ← integration tests for validate_brain_state() end-to-end — Phase 3 Block P
├── brain_structure.rs ← integration tests for validate_brain_structure() end-to-end — Phase 3 Block L
├── brain_validate.rs  ← integration tests for BrainValidator end-to-end
├── doc_materialize.rs ← integration tests for plan_document() — Phase 9 Block MV.9.A
├── doc_index_reconcile.rs ← integration tests for plan_index_reconcile()
├── doc_opportunity.rs ← integration tests for the Opportunity command family
├── doc_cli.rs         ← integration tests for the `mev doc ...` CLI surface
├── emit_state_scope.rs ← integration tests for `emit-state --scope` (byte-identity of unvisited repos, unknown-slug diagnostic, unscoped-unchanged) — ticket-emit-state-scope-and-lock
├── emit_state_lock.rs ← integration tests for the advisory lock (contention, stale-lock reclaim) — ticket-emit-state-scope-and-lock
├── smoke.rs           ← integration tests for the learn-ai validate() public API
└── fixtures/
    └── brain.toml     ← minimal fixture — NOT the live brain.toml
```

---

## The `ContentValidator` trait

`ContentValidator` (in `src/validator.rs`) is the single extension point. Every consumer implements it; the default `run` driver stitches crawl + validate together.

```rust
pub trait ContentValidator {
    type Item;

    fn crawl(&self, root: &Path) -> (Vec<Self::Item>, Vec<Diagnostic>);
    fn validate_item(&self, item: &Self::Item) -> Vec<Diagnostic>;

    // Default driver — override only for non-standard collect strategies.
    fn run(&self, root: &Path) -> Report { ... }
}
```

**To add a new consumer:**
1. Define an `Item` type (the unit your crawl produces — a path-like struct, a parsed record, etc.)
2. Implement `crawl` — walk `root`, return items + any crawl-time diagnostics
3. Implement `validate_item` — check a single item, return diagnostics
4. Wire into `main.rs` as a new `Subcommand` variant

The two current consumers:

| Struct | Item type | Source module |
|---|---|---|
| `LearnAiValidator` | `ContentFile` | `src/learn_ai/` |
| `BrainValidator` | `MdFile` | `src/brain/` |

---

## Core types

### `Diagnostic`

A single validation finding. Every check emits `Diagnostic`s; the reporter prints them.

```rust
pub struct Diagnostic {
    pub severity: Severity,   // Error | Warning
    pub file: PathBuf,        // file the finding concerns
    pub locator: String,      // in-file locator, e.g. "type", "layer[0]", "" for whole-file
    pub message: String,
}
```

**Severity drives the exit code:** any `Error` → exit 1. `Warning` → reported, exit 0.

Constructors: `Diagnostic::error(file, locator, message)` and `Diagnostic::warning(...)`.

### `Report`

The outcome of a `run()` call — a flat list of diagnostics with summary counts.

```rust
pub struct Report {
    pub diagnostics: Vec<Diagnostic>,
}
impl Report {
    pub fn error_count(&self) -> usize { ... }
    pub fn warning_count(&self) -> usize { ... }
    pub fn is_failure(&self) -> bool { self.error_count() > 0 }
}
```

### `JsonReport`

The machine-readable envelope emitted by `--json`. Consumed by the Brain RAG indexer as a pre-rebuild gate.

```rust
pub struct JsonReport {
    pub validator: String,       // "brain" | "learn-ai"
    pub root: String,
    pub errors: usize,
    pub warnings: usize,
    pub diagnostics: Vec<Diagnostic>,
}
```

See the [CLI reference](cli.md) for the serialized JSON shape.

---

## Data flow

```
mev validate-brain <root>
        │
        ▼
find_brain_config(root)          ← walks up from root, parses brain.toml
        │
        ▼
BrainValidator::new(config)
        │
        ▼
.run(root)
  ├── crawl_corpus(root, config)      ← registry-driven multi-root walk → (Corpus, Vec<Diagnostic>)
  │        per file:
  │          scope: owning_unit(rel, config) → (slug, repo_path)  [scope.rs]
  │          membership: is_corpus_member(rel_to_unit) — planning/, docs/, README.md, CLAUDE.md
  │          ephemeral: is_ephemeral(name) — drops handoff.md, _-prefixed files
  │          CorpusEntry { path, rel, stem, scope } mapped → MdFile
  │
  └── for each MdFile:
        validate_md_file(item, config)
            ├── root instruction file? (README.md / CLAUDE.md at unit root)
            │       └── no-frontmatter exempt → skip OKF checks
            ├── read file
            ├── extract YAML frontmatter
            ├── deserialize OkfFrontmatter
            └── check each field → Vec<Diagnostic>
        │
        ▼
Report { diagnostics }
        │
        ▼
exit 0 (clean) | exit 1 (any Error)
```

### Key types in `src/brain/crawl.rs`

| Type | Description |
|---|---|
| `MdFile` | Single validated file: `path`, `rel`, `stem` |
| `CorpusEntry` | Multi-root corpus entry: `path`, `rel`, `stem`, `scope` (owning-unit slug), `metadata` (D5 extract-once OKF parse result) |
| `Corpus` | Complete Brain corpus: `entries: Vec<CorpusEntry>`; serde-serializable for manifest emission |

**D5 extract-once refactor (Phase 3B, Block Q):** `CorpusEntry` now carries an
`Option<OkfFrontmatter>` field (`metadata`). `crawl_corpus()` reads and parses each file's
frontmatter exactly once during the crawl; the result is stored on the entry and shared by the
OKF validator, graph builder, link checker, and manifest emitter — no double-parse. A parse
failure stores `None` (graceful degradation). The `read_doc_metadata()` function that
previously re-read frontmatter from disk in `graph.rs` has been removed; `build_graph()` now
derives `doc_id` and `related` directly from `entry.metadata`.

### Scope resolution (`src/brain/scope.rs`)

| Function | Signature | Description |
|---|---|---|
| `scope_units` | `(&BrainConfig) -> Vec<(String, String)>` | All `(slug, repo_path)` pairs from the registry |
| `scope_for` | `(&Path, &BrainConfig) -> String` | Resolve a HQ-relative path to its owning unit's slug |
| `owning_unit` | `(&Path, &BrainConfig) -> (String, String)` | Resolve a HQ-relative path to its `(slug, repo_path)` pair |

Resolution algorithm: longest-prefix match using `Path::strip_prefix` (prevents `core/mev-extra` from matching `core/mev`). Root unit (`repo_path = "."`) is the fallback when no prefix matches.

### Knowledge graph (`src/brain/graph.rs`) — Phase 3 Block J

The graph module builds and validates the serializable `scope:doc_id` knowledge graph over the Brain corpus.

> **BA.15.12/D16 convergence:** `EdgeKind`, `Edge`, `Node`, `Graph`, `GraphArtifact`,
> `EdgeResolution`, and `resolve_edge` are re-exported from bastion's `okf-core` crate
> (single source of truth for the shared model); `build_graph()`/`check_graph()` (mev-specific
> corpus-walk and diagnostic logic) stay local and consume the shared types. Behavior is
> unchanged — verified byte-identical output against the pre-convergence build.

#### Public functions

| Function | Signature | Description |
|---|---|---|
| `build_graph` | `(&Corpus, &BrainConfig) -> GraphArtifact` | Walks the corpus once; files with a `doc_id` become nodes, others become leaves. Derives `doc_id` and `related` from `entry.metadata` (D5 extract-once). Returns the serializable graph plus lookup structures. |
| `check_graph` | `(&GraphArtifact) -> Vec<Diagnostic>` | Checks the built graph for integrity violations without re-walking the corpus. Internally matches on `resolve_edge`'s result per edge. |
| `resolve_edge` | `(&GraphArtifact, &Edge) -> EdgeResolution` | `pub(crate)`. Resolves a single edge's `to_ref` against the graph's nodes/leaves without re-walking the corpus. Shared by `check_graph` and `graph_emit::build_graph_export` so diagnostics and the export's resolved target fields stay in lockstep. |

#### Graph types

| Type | Description |
|---|---|
| `EdgeKind` | Discriminant enum — `Related` is the only variant today (from `related:` frontmatter). |
| `Edge` | Directed edge: `from` (canonical `scope:doc_id`), `to_ref` (as-authored), `kind`. |
| `Node` | Graph node: `id` (canonical `scope:doc_id`), `scope`, `doc_id`, `rel` path. |
| `Graph` | Serializable D4 artifact: `nodes: Vec<Node>`, `edges: Vec<Edge>`. |
| `GraphArtifact` | Build output: `graph`, `node_map` (canonical id → node index), `leaf_keys` (files with no `doc_id`). |
| `DocMeta` | Metadata extracted by `read_doc_metadata`: `doc_id: Option<String>`, `related: Vec<String>`. |
| `EdgeResolution` | `pub(crate)` enum returned by `resolve_edge`: `Resolved { node_id, doc_id }` (qualified target found), `LeafTarget` (target exists but has no `doc_id`), `Dangling` (target not found). Derives `Debug`, `Clone`, `PartialEq`, `Eq`. |

#### Diagnostic locators emitted by `check_graph`

| Locator | Severity | Condition |
|---|---|---|
| `E_GRAPH_DUPLICATE_DOC_ID` | Error | Two or more corpus files share the same `scope:doc_id`. |
| `related` | Error | A `related:` entry resolves to no node and no leaf (`E_GRAPH_DANGLING_RELATED`). |
| `related` | Warning | A `related:` entry resolves to a real corpus file that has no `doc_id` (`W_GRAPH_LEAF_TARGET`). |

#### Public library entry point

`validate_brain_graph(root: &Path) -> anyhow::Result<Report>` (in `src/lib.rs`) runs the full OKF schema pass followed by the graph pass — crawls the corpus once, builds the graph, and appends graph diagnostics to the same `Report`. Invoked by `mev validate-brain --graph`.

---

### State integrity (`src/brain/state.rs`) — Phase 3 Block P / P2

The state module discovers, loads, and validates all `planning/state.json` files across the registered repos, then builds and checks the cross-repo block-dependency graph. Block P2 extends the v1 model to the v2 schema: `depends_on[]` DAG edges on track blocks, cycle detection, derived-blocked enforcement, backlog-node integrity, and focus-drift warnings.

**Epics** add a cross-repo *initiative* axis on top of that graph. `tracks[]` groups work within one repo and `tier` groups repos organizationally; neither expresses a program like "Bastion Web + UI" that spans several repos at once. A block carries `epics: ["<slug>", ...]` (multi-valued — a block can serve two initiatives), validated against an HQ-only `epics[]` registry by `check_epics`. Epic-to-epic *relationships* are never authored: `derive_epic_edges` computes them from the same `depends_on` edges that already drive blocked-ness, so the relationship map cannot drift from the work graph. The canonical schema lives in the brain's [`docs/state/state-schema.md`](../../../docs/state/state-schema.md).

> **BA.15.12/D15/D16 convergence:** the `state.json` serde schema, `load_state`, and the
> `StateGraph`/`build_state_graph` model are re-exported from bastion's `okf-core` crate
> (single source of truth); the mev-specific validation/derivation logic below
> (`check_*`/`derive_*`, `discover_state_files`, and friends) stays local and consumes the
> shared types. Behavior is unchanged — verified byte-identical output against the
> pre-convergence build.

#### Public functions

| Function | Signature | Description |
|---|---|---|
| `discover_state_files` | `(&Path, &BrainConfig) -> (Vec<StateSource>, Vec<Diagnostic>)` | Discovers state.json paths for the HQ brain, tier sub-brains (via `tiers[].rollup`), and leaf repos (via `[[repos]]` in `brain.toml`). Missing files emit `W_STATE_FILE_MISSING`. |
| `load_state` | `(&Path) -> Result<StateFile, StateLoadError>` | Deserializes a `planning/state.json` file into a `StateFile`. Returns `StateLoadError::Malformed` on schema mismatch or `StateLoadError::Io` on I/O failure. |
| `check_schema` | `(&StateSource, &StateFile) -> Vec<Diagnostic>` | Schema-ring validation: kind membership, `updated` non-empty, status enum values, `blocked_by` well-formedness, kind-appropriate section presence. In v2 files: validates `tracks[].blocks[].depends_on[]` entry well-formedness, rejects authored `status:"blocked"` (`E_STATE_AUTHORED_BLOCKED`) while accepting authored `status:"deferred"`, and validates `backlog[].status` membership. |
| `build_state_graph` | `(&[(StateSource, StateFile)]) -> StateGraph` | Builds the cross-repo block-dependency graph: nodes from `tracks[]` blocks, edges from `depends_on[]` (v2) and `cross_repo[]`. `External`-type `depends_on` entries are leaves, not graph edges. |
| `check_state_graph` | `(&StateGraph, &[(StateSource, StateFile)]) -> Vec<Diagnostic>` | Graph integrity checks: duplicate block IDs, dangling focus references, unknown repos, dangling blocked_by, dangling cross_repo edges. |
| `cycle_paths` | `(&StateGraph) -> Vec<CyclePath>` | MV.10.A: DFS cycle finder over `depends_on` edges, deduplicated by canonical rotation (each cycle's keys rotated so the lexicographically smallest key is first; only the first cycle to produce a given rotated form is kept). Returns raw cycle data (`keys` in DFS discovery order, plus the back-edge's `source_path`) in discovery order — the reusable core `detect_cycles` formats into diagnostics. |
| `detect_cycles` | `(&StateGraph) -> Vec<Diagnostic>` | DFS cycle detection over `depends_on` edges; emits `E_STATE_CYCLE` naming the cycle path (e.g. `A → B → A`). MV.10.A: now a thin formatter over `cycle_paths`, preserving byte-identical diagnostic messages. |
| `check_status_consistency` | `(&[(StateSource, StateFile)]) -> Vec<Diagnostic>` | Emits `E_STATE_STATUS_INCONSISTENT` when a `closed` block has a `type:block` `depends_on` target that is not `closed`. Dangling targets are skipped (reported by `check_state_graph`). |
| `check_backlog_integrity` | `(&[(StateSource, StateFile)], &StateGraph) -> Vec<Diagnostic>` | Backlog-node integrity: dangling `depends_on` targets (`E_STATE_DANGLING_BLOCKED_BY`) and orphan/unresolved promoted nodes (`E_STATE_DANGLING_PROMOTION`). |
| `ready_order` | `(&StateGraph, &[(StateSource, StateFile)]) -> Vec<String>` | Reusable standalone function: returns open blocks ordered by `wave` (tiebreak: track order, array order) whose every `type:block` dep is `closed` and which have no `type:external` deps. Forward-compat input for `MV.3B.T` topo-emit. |
| `check_focus_drift` | `(&StateSource, &StateFile, &StateGraph, &[(StateSource, StateFile)]) -> Vec<Diagnostic>` | Recomputes the expected `focus` from authored `tracks[]` by calling `derive_focus` and emits `W_STATE_FOCUS_DRIFT` (warning, exit 0) on block-id set mismatch. Shares the same derivation logic as `mev emit-state` — they cannot disagree. |
| `check_rollup` | `(&Path, &StateFile, &HashMap<String, StateFile>) -> Vec<Diagnostic>` | Rollup drift check: compares brain `repos[]` now/next headline entries against each child's actual `focus` values. Emits `W_STATE_ROLLUP_DRIFT` on mismatch. |
| `derive_focus` | `(&StateSource, &StateFile, &StateGraph, &[(StateSource, StateFile)]) -> DerivedFocus` | **Single-source derivation** — computes the expected `focus` from authored `tracks[]`: `now` = `in_progress` blocks; `next` = ready open blocks in `wave` order; `blocked` = open blocks with an unmet `depends_on`, each carrying the unmet subset as `blocked_by[]`. Called by both `check_focus_drift` and the emit planners. |
| `derive_cross_repo` | `(&[(StateSource, StateFile)]) -> Vec<CrossRepoEdge>` | For every `tracks[].blocks[].depends_on` entry of `{type:"block"}` whose `repo` differs from the owning repo, produces a `CrossRepoEdge`. Same-repo deps are excluded. |
| `tier_scope_for` | `(&StateFile, &BrainConfig) -> TierScope` | Determines the `TierScope` a brain file's `repos[]`/`focus` should be scoped to: `TierScope::Tier(t)` when `brain_file.repo` matches a `tier` value declared in `brain.toml`'s `[[repos]]`; `TierScope::All` (every repo) otherwise (the HQ root). MV.3B.U. |
| `derive_rollup` | `(&TierScope, &BrainConfig, &[RepoRollup], &StateGraph, &[(StateSource, StateFile)]) -> Vec<RepoRollup>` | Tier-scoped, non-destructive rollup (MV.3B.U). Iterates the in-scope `config.repos[]` (config order): if a loadable `kind == "project"` child exists, derives its headline via `derive_focus` and sets `tier` from config; else if `existing` (the brain file's current `repos[]`) already has an entry for that slug, preserves it verbatim (backfilling `tier`); else emits a tier-tagged empty stub. `RepoRollup.tier` is non-`None` in every branch, and a malformed/missing child can never silently drop a repo. |
| `derive_brain_focus` | `(&TierScope, &BrainConfig, &StateGraph, &[(StateSource, StateFile)]) -> Focus` | Derives a brain file's `focus.now/next/blocked` as the repo-tagged union of its in-scope children's `derive_focus` output (MV.3B.U). Iterates in-scope `config.repos[]` in config order; for each repo with a loadable child of `kind == "project"` **or `kind == "brain"`** (a tier sub-brain root — e.g. `business`, `tier = "_root"` — that carries its own authored `tracks[]`, fixed 2026-07-17: previously `"project"`-only, so a container tier's own tracks were dropped from `TierScope::All` unions even though `Facet A` folds the literal `self_file`), appends its `now`/`next`/`blocked` blocks (each tagged `repo: Some(<slug>)`) in the child's own within-focus order, deduplicated by `(repo, id)` per list. A `"brain"`-kind child with empty `tracks[]` is a no-op via `derive_focus`'s short-circuit — byte-identical for pure container tiers (`core`, `side`, `client`, `portfolio`). Repos with no loadable child contribute nothing. MV.6.B: each constructed `Block` also carries the source `TrackBlock`'s `priority`/`due` (previously hardcoded to `None`). |
| `check_epics` | `(&BrainConfig, &[(StateSource, StateFile)]) -> Vec<Diagnostic>` | Epic registry + membership integrity. A **corpus-level** check (not per-file like `check_field_policy`) because a block's membership is validated against a registry living in one other file — same shape as `check_backlog_integrity`. Locates the registry via the single `kind == "brain"` file whose `tier_scope_for` is `TierScope::All` (HQ-only, D2 `backlog[]` precedent). Emits `E_STATE_DUPLICATE_EPIC_SLUG`, `E_STATE_EPIC_BAD_STATUS` (∉ `{active, paused, complete}`), `E_STATE_UNKNOWN_EPIC` (a block claims a slug absent from the registry — the typo guard), `W_STATE_EPIC_REGISTRY_IGNORED` (a non-HQ file declares its own `epics[]`, which is silently unused), `W_STATE_EPIC_EMPTY` (registered epic with no members), and `W_STATE_EPIC_UNREACHABLE_DEP` (an unclosed epic block depends on an unclosed block belonging to no epic — a gate invisible on that epic's board). Dangling dep targets are skipped, not double-reported (`check_state_graph` owns those). With no registry authored, every check is a silent no-op. |
| `derive_epic_focus` | `(&Focus, &str) -> Focus` | Filters an already-derived `Focus` down to one epic's members, reading `Block::epics` (which `derive_brain_focus` carries through from the authoring `TrackBlock`). Takes `derive_brain_focus`'s **output** rather than re-deriving, so an epic board cannot disagree with the unified board about now/next/blocked; surviving order (and therefore the effective-priority sort) is preserved. |
| `derive_epic_edges` | `(&[(StateSource, StateFile)], &str) -> EpicEdges` | Derives one epic's relationships to the rest of the corpus from the block `depends_on` graph — **nothing authored**. Returns `outbound` (member depends on a non-member: what the initiative waits on) and `inbound` (non-member depends on a member: what it holds up). A block in *both* endpoints' epics counts as inside, so a shared block never self-edges; `external` deps and unresolvable targets are skipped. Both lists are complete, including satisfied edges — `EpicEdge::blocking` (dependency not `closed` **and** dependent not `closed`) distinguishes live gates from history. |
| `effective_priorities` | `(&StateGraph, &[(StateSource, StateFile)]) -> HashMap<String, u8>` | MV.7.A: computes each block's effective priority via reverse-topological `min`-propagation over the `depends_on` DAG, keyed by `"repo:id"` — `effective(n) = min(own(n), min{ effective(m) : m depends_on n })`. Own priority comes from `TrackBlock.priority` (absent → `u8::MAX`, never wins a `min`); propagation walks the reverse `BlockedBy` adjacency (dependency → dependents), so a gate that blocks a hotter block inherits that block's priority. Memoized DFS with a recursion-stack guard: a `depends_on` cycle short-circuits to the node's own priority instead of re-recursing, so it terminates deterministically without hanging or panicking. Only nodes whose effective value lands in `0..=3` get a map entry — nodes that stay at `u8::MAX` are omitted so callers naturally treat them as absent. Feeds the unified board's `NEXT` sort (see `render_unified_board`, below). |

#### State types

| Type | Description |
|---|---|
| `StateFile` | Top-level deserialized `state.json`: `kind`, `updated`, `focus`, `tracks`, `repos`, `tiers`, `cross_repo`, `backlog`, `note`. |
| `Focus` | Current focus entry: `now`, `next`, `blocked_by`. |
| `TrackBlock` | A single `tracks[].blocks[]` entry: `id`, `title`, `status`, `blocked_by` (v1), `depends_on` (v2, `#[serde(default)]`), `wave`, `origin`. |
| `BlockedBy` | Internally-tagged enum for block dependencies (shared by both `blocked_by` and `depends_on`): `Block { repo, id, what }` or `External { what }`. |
| `Origin` | Block provenance for promoted backlog nodes: `kind` (serde: `"type"`), `slug`. |
| `Backlog` | HQ-only backlog node: `slug`, `title`, `repo`, `kind` (serde: `"type"`), `status` (`idea`/`ready`/`promoted`), `depends_on`, `block` (pointer on promote), `notes`. |
| `StateSource` | Discovery result: `abs_path`, `repo_slug`, `kind` (`hq`, `tier`, `leaf`). |
| `StateGraph` | Cross-repo block-dependency graph: `nodes: Vec<StateNode>`, `edges: Vec<StateEdge>`. Serde-serializable. |
| `StateNode` | Graph node: `repo`, `id`, `title`, `status`, `source_path` (skipped in serialization). |
| `StateEdge` | Graph edge: `from_repo`, `from_id`, `to_repo`, `to_id`, `kind` (`blocked_by` or `cross_repo`), `source_path` (skipped in serialization). |
| `DerivedFocus` | Output of `derive_focus`: `now: Vec<String>`, `next: Vec<String>`, `blocked: Vec<(String, Vec<BlockedBy>)>` — block ids (and, for `blocked`, the unmet subset of each block's `depends_on`). |
| `Epic` | HQ `epics[]` registry entry (re-exported from `okf-core`): `slug`, `title`, `description?`, `status?` (`active`/`paused`/`complete`), `plan?` (path to the owning plan doc), `repos[]` (a reader's hint, not the source of truth). Membership itself is authored on blocks (`TrackBlock::epics`), so the registry is only the closed vocabulary those declarations are validated against. |
| `EpicEdge` | One `depends_on` edge crossing an epic boundary: `from`/`to` (`"repo:id"` of dependent/dependency), `other_epics` (the epics of whichever endpoint is outside), `blocking` (whether the edge still gates). |
| `EpicEdges` | One epic's boundary edges split by direction: `outbound` (waiting on) and `inbound` (holding up). |
| `TierScope` | MV.3B.U: the repo set a brain file's `repos[]`/`focus` should be scoped to — `Tier(String)` (one tier's `[[repos]]`) or `All` (every repo, the HQ root). Produced by `tier_scope_for`, consumed by `derive_rollup` and `derive_brain_focus`. |
| `CyclePath` | MV.10.A: one cycle found by `cycle_paths` — `keys: Vec<String>` (the cycle's `"repo:id"` nodes in DFS discovery order, closing node **not** repeated) and `source_path: PathBuf` (the back-edge's source, where `E_STATE_CYCLE` anchors). |

#### Public library entry point

`validate_brain_state(root: &Path) -> anyhow::Result<Report>` (in `src/lib.rs`) runs the full OKF schema pass followed by the multi-step state pipeline (discovery → load → schema → graph build + check → cycle detection → status consistency → backlog integrity → rollup → focus drift) and appends all state diagnostics to the same `Report`. Invoked by `mev validate-brain --state`.

---

### Emit module (`src/brain/emit.rs`) — Phase 3 Block T

The emit module is the **single derivation engine** for all generated views declared by the v2 state schema. It is a pure compiler (files in → files out; no DB, no network), and its planners share the same `derive_focus` / `derive_rollup` / `derive_cross_repo` / `derive_brain_focus` helpers used by the validator's drift checks — so the emit is, by construction, the fixed point of `check_focus_drift` and `check_rollup`.

#### Public functions

| Function | Signature | Description |
|---|---|---|
| `wave_order` | `(&StateGraph, &[(StateSource, StateFile)]) -> Vec<String>` | All block keys (`"repo:id"`) sorted by `wave` ascending (`None` last), tiebreak by track iteration order then block array index. Full-roadmap sibling of `ready_order` — includes every block regardless of status. |
| `render_wave_table` | `(&str, &StateFile, &StateGraph, &HashMap<String, Option<String>>) -> String` | Renders a Markdown table of one repo's blocks in wave order. Columns: `Wave \| Block \| Title \| Status \| Depends on`. Open blocks with an unmet `depends_on` render as `blocked` in the Status column. Same-repo deps resolve against the file's own blocks; cross-repo deps resolve against the `global_status` map (from `global_status_map`, below) — met only when the target block's authored status is `closed`, unmet when open or absent. `graph` is accepted for API symmetry but not consulted for this derivation. |
| `global_status_map` | `(&[(StateSource, StateFile)]) -> HashMap<String, Option<String>>` | Builds a `"{repo_slug}:{block_id}" → authored status` map across **every** loaded state file (not just one repo), namespaced so same-`id` blocks in different repos never collide. The cross-file status lookup `render_wave_table` needs to resolve cross-repo `depends_on` edges. |
| `splice_generated` | `(&str, &str, &str) -> Result<String, EmitError>` | Idempotent sentinel-splice: replaces the text between `<!-- BEGIN generated:{marker} -->` and `<!-- END generated:{marker} -->` with `generated`, preserving every line outside the sentinels verbatim. Returns `EmitError` when a sentinel is missing or unbalanced. |
| `plan_state_json` | `(&[(StateSource, StateFile)], &StateGraph, &BrainConfig) -> EmitPlan` | For each loaded state file, clones it, regenerates derived sections (leaf: `focus`; brain: tier-scoped non-destructive `repos[]` via `tier_scope_for`/`derive_rollup`, `cross_repo[]`, and `focus` via `derive_brain_focus`), re-serializes, and adds an `EmitAction` only when the content actually changed. Authored fields survive the round-trip unchanged. MV.3B.U threaded `&BrainConfig` in to drive tier scoping. |
| `plan_master_plan_tables` | `(&[(StateSource, StateFile)], &StateGraph) -> EmitPlan` | For each state file, resolves the sibling `master-plan.md`; if it exists and carries the `wave-table` sentinels, splices the rendered table (using a `global_status_map(files)` built once up front to resolve cross-repo deps) and adds an `EmitAction`. A missing file or sentinels yields `W_EMIT_NO_SENTINEL` (never invents sentinels). |
| `plan_project_caches` | `(&Path, &[(StateSource, StateFile)], &StateGraph, &BrainConfig) -> EmitPlan` | MV.4.B: for each loaded `kind == "project"` file, resolves its `brain.toml` `[[repos]]` entry and the target doc at `root.join(&entry.cache_doc)` (same resolution `check_sync` uses); if the doc exists and carries the `PROJECT_CACHE` sentinels, splices in a one-line derived focus headline (`render_focus_line`, private) and reconciles the doc's OKF `synced_from` frontmatter field to the child file's `updated` watermark (`reconcile_synced_from`, private). A missing doc, missing sentinels, or no matching `[[repos]]` entry / blank `cache_doc` yields `W_EMIT_NO_SENTINEL` or a silent skip; never invents sentinels or a frontmatter block. Wired into `emit_state` by MV.4.E. |
| `plan_tier_rollups` | `(&[(StateSource, StateFile)], &StateGraph, &BrainConfig) -> EmitPlan` | MV.4.B: for each loaded `kind == "brain"` file whose `tier_scope_for` resolves to `TierScope::Tier` (the HQ root, which resolves to `TierScope::All`, is out of scope — that's MV.4.C's `plan_hq_board`), derives tier-scoped rollup rows via `derive_rollup` and renders them (`render_tier_rollup_table`, private: `Repo \| Now \| Next \| Blocked`) into the sibling `status.md`'s `TIER_ROLLUP` sentinel. A missing `status.md` or missing sentinels yields `W_EMIT_NO_SENTINEL` (never invents sentinels). Wired into `emit_state` by MV.4.E. |
| `render_hq_board` | `(&Focus, &[CrossRepoEdge]) -> String` | MV.4.C: pure renderer for the HQ root's Operating Board — three always-present `## NOW` / `## NEXT` / `## BLOCKED` sections (each `_none_` when empty), one `- {repo}:{id} — {title}` line per block in the input `Focus`'s own order. A block with `blocked_by` entries appends a trailing `(blocked by ...)` parenthetical, comma-joining multiple blockers; per-blocker text prefers a matching `cross_repo[]` edge's note, falling back to the dependency's own `what` gloss, then the bare `repo:id` target (private helpers: `render_hq_board_section`, `render_hq_board_line`, `render_hq_board_blocker`). |
| `plan_hq_board` | `(&[(StateSource, StateFile)], &StateGraph, &BrainConfig) -> EmitPlan` | MV.4.C: for the loaded `kind == "brain"` file whose `tier_scope_for` resolves to `TierScope::All` (the HQ root; tier sub-brains are skipped — that's `plan_tier_rollups`'s job), resolves the sibling `status.md` (state.json's parent dir) and splices `render_hq_board(derive_brain_focus(...), derive_cross_repo(...))` into its `HQ_BOARD` sentinel. A missing `status.md` or missing sentinels yields `W_EMIT_NO_SENTINEL` (never invents sentinels). Wired into `emit_state` by MV.4.E. |
| `render_unified_board` | `(&Focus, &[CrossRepoEdge], &BrainConfig, chrono::NaiveDate, &HashMap<String, u8>) -> String` | MV.6.B: pure renderer for the HQ root's priority-ranked unified board — `## NOW` / `## NEXT` / `## BLOCKED` / `## DUE-SOON`, unioning every registered repo (including the business tier) and tagging each row `[BIZ]`/`[ENG]` by the block's `repo` slug looked up in `config.repos` (`tier == "business"` renders `[BIZ]`; the `business` tier ROOT itself — `tier = "_root"`, slug `business` — also renders `[BIZ]` by slug match, fixed 2026-07-17 alongside `derive_brain_focus`'s tier-root fold; everything else, including an unrecognised slug, renders `[ENG]`). MV.7.A: `NEXT` is stably re-sorted by `(effective_priority asc, due asc)`, where `effective_priority_for(block, effective)` looks up `"repo:id"` in the new `effective` map parameter first, falling back to the block's own raw `priority`, then `u8::MAX` when neither is present — so a block that gates a hotter dependent floats to the top even with no own priority. Absent values sort last, keeping wave as the implicit tertiary key since `Focus::next` is already wave-ordered; `NOW`/`BLOCKED` preserve caller order, matching `render_hq_board`. `DUE-SOON` lists every block from the now+next+blocked union whose `due` parses (`%Y-%m-%d`) and is `<= today + 14 days` (`DUE_SOON_WINDOW_DAYS`), sorted by due date ascending, annotating `(overdue)` when `due < today`; blocks with an absent/unparseable `due` are excluded (private helpers: `render_unified_board_section`, `sort_unified_board_next`, `effective_priority_for`, `parse_due`, `render_due_soon_section`). |
| `plan_unified_board` | `(&[(StateSource, StateFile)], &StateGraph, &BrainConfig, chrono::NaiveDate) -> EmitPlan` | MV.6.B: mirrors `plan_hq_board` exactly (same HQ-root `TierScope::All` gating, same `status.md` target resolution, same `W_EMIT_NO_SENTINEL` diagnostics) but targets the independent `markers::UNIFIED_BOARD` sentinel in the same document and splices `render_unified_board(derive_brain_focus(...), derive_cross_repo(...), config, today, effective)`. MV.7.A: `effective` is `effective_priorities(graph, files)`, computed once up front and threaded into the renderer so `NEXT` sorts by effective (inherited) priority rather than each block's raw own priority. Wired into `emit_state` by MV.6.B, run after `plan_hq_board`. |
| `topo_order` | `(&StateGraph, &[(StateSource, StateFile)]) -> Vec<String>` | MV.10.A: cycle-safe DFS topological order over the full `depends_on` graph (every block, every repo), seeded in `wave_order` so an unconstrained pair still reads in stable wave-then-iteration order. Only `{type:"block"}` deps resolving to a real node participate; a node already on the DFS stack short-circuits instead of recursing again (mirrors the guard in `effective_priorities`). Extracted out of `epic_members`, which is now a thin filter over this. |
| `epic_members` | `(&StateGraph, &[(StateSource, StateFile)], &str) -> Vec<(String, &TrackBlock)>` | Every block claiming a slug, across all repos, in dependency-respecting order — the cross-repo sequence for one initiative. MV.10.A: filters `topo_order`'s full-corpus topological order down to the epic's members. |
| `render_epic_board` | `(&[EpicBoardInput], &Focus, &HashMap<String, u8>, &BrainConfig) -> String` | Pure renderer for the per-epic board: one `### {title}` section per **active** epic (`RENDERED_EPIC_STATUSES`; an absent `status` defaults to active, so a `complete`/`paused` epic keeps its entry and membership but stops competing for attention), each with a progress line (`**7/23 closed** · 2 in progress · 14 open`), `NOW`/`NEXT`/`BLOCKED` lanes (via `derive_epic_focus` over the passed brain-level union, reusing `render_unified_board_section` and `sort_unified_board_next` so ordering matches the unified board exactly), and the derived relationship lines. Renders `_no active epics_` when everything is filtered out. Private helpers: `epic_progress`, `render_epic_progress_line`, `render_epic_relationships` (lists only `EpicEdge::blocking` edges, deduped by counterpart; a counterpart in no epic renders `(no epic)` — the readable face of `W_STATE_EPIC_UNREACHABLE_DEP`). |
| `render_epic_sequence_table` | `(&[(String, &TrackBlock)], &HashMap<String, Option<String>>) -> String` | `render_wave_table`'s cross-repo sibling: columns `Wave \| Repo \| Block \| Title \| Status \| Depends on`, one epic across every repo instead of one repo across every epic. Same derived-status rule (an open block with an unmet `depends_on` renders `blocked`, via the private `has_unmet_dep`) resolved against the same `global_status_map`. An empty member set renders a `_no member blocks_` placeholder row rather than a bare header. |
| `plan_epic_boards` | `(&[(StateSource, StateFile)], &StateGraph, &BrainConfig) -> EmitPlan` | Splices `render_epic_board` into the `markers::EPIC_BOARD` sentinel of every `kind == "brain"` file's sibling `status.md`. **Epic lanes are always global** — derived once from the HQ file at `TierScope::All`, even on a tier sub-brain's board, because a tier-truncated slice of a cross-repo initiative hides the very sequence the board exists to show. What *is* tier-scoped is **which epics appear**: HQ shows all, a tier shows those with a member block in its tier (a tier owning no epic work is skipped entirely, leaving its sentinel untouched). This is the one board that deliberately departs from `plan_attention_board`'s tier-scoped-*content* rule. Returns an empty plan when there is no HQ file or no registry, so the whole feature is inert until adopted. `W_EMIT_NO_SENTINEL` on a missing `status.md` or sentinel pair; never invents sentinels. |
| `plan_epic_sequences` | `(&Path, &[(StateSource, StateFile)], &StateGraph, &BrainConfig) -> EmitPlan` | For each registry entry carrying a `plan` path, resolves it against `root` and splices `render_epic_sequence_table` into its `markers::EPIC_SEQUENCE` sentinel. An entry with no `plan` is skipped silently (not every epic has a doc); a path that does not resolve, or a doc without the sentinels, yields `W_EMIT_NO_SENTINEL`. Never creates the document. |
| `apply_plan` | `(&EmitPlan, bool) -> Vec<Diagnostic>` | When `write` is `true`, writes each action's `new_content` to its `path` and emits `I_EMIT_WROTE` per file. When `false` (dry-run), writes nothing and emits `W_EMIT_DRY_RUN` per planned action. Always surfaces the plan's own diagnostics. |

#### Emit types

The `markers` submodule (`crate::brain::emit::markers`) exposes the sentinel marker name
constants as `pub const`s — `WAVE_TABLE`, `PROJECT_CACHE`, `TIER_ROLLUP`, `HQ_BOARD`,
`UNIFIED_BOARD`, `ATTENTION`, `EPIC_BOARD`, `EPIC_SEQUENCE` — so callers reference
`markers::WAVE_TABLE` instead of the bare string literal `"wave-table"`.

| Type | Description |
|---|---|
| `EmitError` | thiserror error for sentinel failures: `MissingSentinel { marker, which }`. |
| `EmitAction` | A single proposed write: `path: PathBuf`, `new_content: String`, `note: String`. |
| `EmitPlan` | A collection of proposed writes and accompanying diagnostics: `actions: Vec<EmitAction>`, `diagnostics: Vec<Diagnostic>`. |
| `EpicBoardInput` | One epic's already-derived inputs to `render_epic_board`: `epic: &Epic`, `members: Vec<(String, &TrackBlock)>`, `edges: EpicEdges`. Assembling these is the planner's job (it owns the corpus), keeping the renderer a pure data-in/string-out function. |

#### Diagnostic locators emitted by the emit module

| Locator | Severity | Condition |
|---|---|---|
| `W_EMIT_DRY_RUN` | Warning | Planned action in dry-run mode; no file written. |
| `I_EMIT_WROTE` | Warning | File written in `--write` mode. |
| `W_EMIT_NO_SENTINEL` | Warning | A target document is missing its marker's sentinel pair (`wave-table`, `project-cache`, `tier-rollup`, `hq-board`, `unified-board`, `attention`, `epic-board`, or `epic-sequence`) — or, for `epic-sequence`, the registry's `plan` path does not resolve at all; file skipped. |
| `E_EMIT_WRITE_FAILED` | Error | IO error writing a file. |
| `E_EMIT_INCOMPLETE_CORPUS` | Error | `--write` refused because one or more discovered `state.json` files failed to load (`loaded.len() < sources.len()`); derived views are cross-repo unions, so regenerating them from a partial corpus would silently erase the missing repo(s). Checked immediately after the load loop, before `build_state_graph` and the first planner; dry-run is exempt (the `write &&` conjunct is load-bearing). |

#### Public library entry point

`emit_state(root: &Path, write: bool) -> anyhow::Result<Report>` (in `src/lib.rs`) resolves `brain.toml`, discovers and loads all state files, builds the graph, then runs the planners in a stable order, applies each plan with `apply_plan(write)`, and merges all diagnostics into a single `Report`. Invoked by `mev emit-state`. MV.4.E wired the `plan_project_caches`/`plan_tier_rollups`/`plan_hq_board` planners in; MV.6.B added `plan_unified_board`.

**Ordering matters.** `plan_state_json`, `plan_master_plan_tables`, `plan_project_caches`, `plan_tier_rollups`, `plan_hq_board`, `plan_unified_board`, `plan_attention_board`, and `plan_brain_cache_watermarks` are each planned and applied immediately, one at a time — not planned as a batch and applied afterward. Every planner reads its target file fresh at call time, and several targets are shared (`status.md` carries `hq-board` + `unified-board` + `attention` for the HQ root, and `tier-rollup` + `attention` for a tier sub-brain); if all eight were planned before any of them wrote, a later planner would read the same pre-batch original as an earlier one and its write would silently drop the earlier planner's just-applied sentinel edit for that file (the `emit-state-same-file-batching` bug, fixed — see `same_file_batching_regression` in `tests/brain_emit.rs`). Interleaving plan+apply per planner means each one always reads whatever the previous ones already wrote. `loaded`/`graph` stay a single fixed in-memory snapshot for the whole run — only the on-disk *rendered documents* progress. `plan_epic_boards` / `plan_epic_sequences` run **after** all eight (they share `status.md` with the HQ/unified/attention boards, and `master-plan.md` with `plan_master_plan_tables`) — planned and applied together since they target disjoint files, so no ordering hazard between the two — and `plan_status_frontmatter` runs last of all for the same reason: each reads the already-updated text in write mode.

---

### Link integrity (`src/brain/links.rs`) — Phase 3 Block K

The links module extracts and validates all local references (markdown links, `file://` URIs, and `[[wikilinks]]`) across the Brain corpus, and re-checks references against `.brain-moves-pending` to surface stale targets after moves or deletions.

#### Public functions

| Function | Signature | Description |
|---|---|---|
| `extract_links` | `(&str) -> Vec<LinkRef>` | Parses a file's body and returns all local link references. External links and pure anchors are skipped. |
| `collect_doc_ids` | `(&Corpus) -> HashSet<String>` | Builds the set of authored bare `doc_id`s from corpus frontmatter — reads `entry.metadata` (the D5 extract-once field on `CorpusEntry`). |
| `check_links` | `(&Corpus, &Path, &HashSet<String>) -> Vec<Diagnostic>` | For each corpus entry, extracts links and resolves each against the filesystem or the `doc_id` set. Emits `E_LINK_*` diagnostics on resolution failures. |
| `read_moves_pending` | `(&Path) -> Vec<String>` | Reads `<root>/.brain-moves-pending`; returns the set of moved/deleted repo-relative paths. Missing file returns empty set. |
| `check_moved_references` | `(&Corpus, &Path, &[String]) -> Vec<Diagnostic>` | Scans the corpus for references that still resolve to a path listed in the moved set. Emits `E_LINK_MOVED_REFERENCE` for each hit. |

#### Link types

| Type | Description |
|---|---|
| `LinkKind` | Discriminant enum: `Markdown`, `FileUri`, `WikiLink`. |
| `LinkRef` | A single extracted reference: `kind`, `raw` (as-authored), `target` (path/slug with any `#anchor` suffix stripped). |

#### Diagnostic locators emitted by `check_links` and `check_moved_references`

| Locator | Severity | Condition |
|---|---|---|
| `E_LINK_DEAD_MARKDOWN` | Error | A relative markdown link's resolved path does not exist on disk. |
| `E_LINK_DEAD_FILE_URI` | Error | A `file://` URI's resolved path does not exist on disk. |
| `E_LINK_DANGLING_WIKILINK` | Error | A `[[wikilink]]` target slug is absent from the corpus `doc_id` set. |
| `E_LINK_MOVED_REFERENCE` | Error | A markdown or `file://` reference still points at a path listed in `.brain-moves-pending`. |

#### Public library entry point

`validate_brain_links(root: &Path) -> anyhow::Result<Report>` (in `src/lib.rs`) runs the full OKF schema pass, then crawls the corpus once, collects `doc_id`s, runs `check_links`, reads `.brain-moves-pending`, runs `check_moved_references`, and appends all link diagnostics to the same `Report`. Invoked by `mev validate-brain --links`.

---

### Structural coverage (`src/brain/structure.rs`) — Phase 3 Block L

The structure module enforces D17 / CLAUDE.md Standing Rule 7: every corpus file in a directory must appear in that directory's `index.md`, and every `index.md` row must point at a file that exists on disk. Both directions are per-directory, direct children only — subdirectories are covered by their own `index.md`, so this check does not recurse into them.

Reuses `links::extract_links` for parsing `index.md` rows (index rows are ordinary markdown `[text](path)` links / `file://` URIs — no separate parser) and `crawl::Corpus` / `CorpusEntry` as the authoritative, already skip-pruned and ephemeral-filtered set of "files in scope".

#### Public functions

| Function | Signature | Description |
|---|---|---|
| `check_structure` | `(&Corpus, &Path) -> Vec<Diagnostic>` | Groups corpus entries by parent directory, matches each directory's `index.md` member, extracts and resolves its markdown/`file://` links, and emits `E_STRUCT_*` diagnostics for orphaned direct-child files and dangling `index.md` rows. `root` bounds the "in corpus" test for dangling-row detection — a resolved target outside `root` is ignored. |

A private path-normalization helper lexically resolves `.` / `..` components (no `canonicalize`, since dangling targets may not exist on disk) so `./foo.md`, `foo.md`, and mixed-separator variants of the same target compare equal via `PathBuf` component comparison rather than raw string equality.

#### Diagnostic locators emitted by `check_structure`

| Locator | Severity | Condition |
|---|---|---|
| `E_STRUCT_ORPHAN_FILE` | Error | A corpus file is a direct child of a directory that has an `index.md`, but no markdown/`file://` link in that `index.md` resolves to it. Located at the orphan file. |
| `E_STRUCT_DANGLING_ROW` | Error | A markdown/`file://` link in an `index.md`, resolved against the `index.md`'s directory, lands inside `root` but does not exist on disk. Located at the `index.md`. |

Directories with no `index.md` corpus member are skipped entirely (no coverage obligation, so no orphan diagnostics). `[[wikilink]]` targets, external links, and out-of-corpus-root resolved targets are ignored (owned elsewhere / out of scope for this check).

#### Public library entry point

`validate_brain_structure(root: &Path) -> anyhow::Result<Report>` (in `src/lib.rs`) resolves `brain.toml` (same `E_CONFIG_NOT_FOUND` fallback as the other `validate_brain_*` drivers), runs the full OKF schema pass via `BrainValidator`, crawls the corpus once, calls `structure::check_structure`, and appends the resulting diagnostics to the same `Report`. Invoked by `mev validate-brain --structure`.

---

### Manifest emitter (`src/brain/manifest.rs`) — Phase 3 Block Q

The manifest module converts a pre-crawled `Corpus` into a canonical, JSON-serializable file
list (`Manifest`) with per-file OKF metadata. Its output is the single source consumed by
`index_brain.py` — "what's validated == what's embedded" holds by construction.

Design principles:
- **Pure output** — `build_manifest` does not write to disk; it returns a value the caller
  serializes to stdout. Consistent with the D4 pure-compiler model.
- **No re-crawl** — the function consumes a `&Corpus` already built by `crawl_corpus`; callers
  that also run the OKF validator share the same crawl result.
- **Graceful degradation** — entries without parseable frontmatter appear in the manifest with
  all metadata fields set to `null`; the OKF validator reports the error separately.

#### Public function

| Function | Signature | Description |
|---|---|---|
| `build_manifest` | `(&Path, &Corpus) -> Manifest` | Maps each `CorpusEntry` to a `ManifestEntry` by extracting OKF fields from `entry.metadata`. The `root` path is stored as a display string in the manifest header. |

#### Manifest types

| Type | Description |
|---|---|
| `ManifestEntry` | A single file entry: `rel` (repo-relative path), `scope`, and OKF metadata fields (`doc_id`, `doc_type`, `title`, `description`, `layer`, `project`, `status`, `keywords`) — all metadata fields are `Option`. |
| `Manifest` | The complete manifest: `version` (`"1"`), `root` (display path of HQ root), `entries: Vec<ManifestEntry>`. Derives `Serialize`. |

`ManifestEntry.doc_type` is the serialized form of the OKF `type` field (renamed to avoid the
Rust keyword).

#### Public library entry point

`manifest_brain(root: &Path) -> anyhow::Result<Manifest>` (in `src/lib.rs`) resolves
`brain.toml`, crawls the corpus with `crawl_corpus`, and calls `build_manifest`. The returned
`Manifest` is a pure value — nothing is written to disk. Invoked by `mev manifest`.

---

### Graph exporter (`src/brain/graph_emit.rs`) — Phase 3B Block R

The graph-export module converts a pre-built `GraphArtifact` (from `graph::build_graph`) into
a canonical, JSON-serializable envelope (`GraphExport`) with a `version`/`root` header,
mirroring `manifest.rs`'s `Manifest`. Consumed by the orchestrator to load nodes and edges into
a Postgres edges table beside `brain_documents` (D4).

Design principles (shared with `manifest.rs`):
- **Pure output** — `build_graph_export` does not write to disk or a DB; it returns a value the
  caller serialises to stdout.
- **No re-derivation of nodes** — nodes are cloned straight from `artifact.graph` in walk order;
  nothing is re-walked here. Edges are resolved per-edge via `graph::resolve_edge` (the same
  resolution `check_graph` uses) so the export's `target_node_id`/`target_doc_id` stay in
  lockstep with the validator's dangling/leaf-target diagnostics.
- **Deterministic leaves** — `leaves` is a sorted `Vec<String>` (from `artifact.leaf_keys`, a
  `HashSet`) so repeated runs over an unchanged corpus emit byte-identical output.

> **BA.15.12/D16 convergence:** `GraphExport`, `ExportedEdge`, and `build_graph_export` are
> re-exported from bastion's `okf-core` crate — this module has zero mev-specific logic
> layered on top of the shared port. Behavior is unchanged — verified byte-identical output
> against the pre-convergence build.

#### Public function

| Function | Signature | Description |
|---|---|---|
| `build_graph_export` | `(&Path, &GraphArtifact) -> GraphExport` | `root` is stored as a display string in the envelope header. Clones `nodes` from `artifact.graph`; builds `edges: Vec<ExportedEdge>` by resolving each edge with `graph::resolve_edge` and mapping the result to nullable `target_node_id`/`target_doc_id`; collects `artifact.leaf_keys` into a sorted `Vec<String>`. |

#### Graph-export types

| Type | Description |
|---|---|
| `GraphExport` | The complete graph-export envelope: `version` (`"2"`), `root` (display path of HQ root), `nodes: Vec<Node>`, `edges: Vec<ExportedEdge>`, `leaves: Vec<String>` (sorted `scope:stem` for files with no `doc_id`). Derives `Serialize`; reuses `Node` from `graph.rs`. |
| `ExportedEdge` | Export-local edge shape: `from`, `to_ref`, `kind` (mirrors `graph::Edge`), plus `target_node_id: Option<String>` and `target_doc_id: Option<String>` — both `Some` when the edge resolves to a real node, both `None` when dangling or targeting a leaf. Derives `Serialize`. |

#### Public library entry point

`graph_brain(root: &Path) -> anyhow::Result<GraphExport>` (in `src/lib.rs`) resolves
`brain.toml`, crawls the corpus with `crawl_corpus`, builds the graph with `build_graph`, and
calls `build_graph_export`. The returned `GraphExport` is a pure value — nothing is written to
disk or a DB. Invoked by `mev emit-graph`.

---

### Block-graph exporter (`src/brain/block_graph.rs`) — Phase 10 Block MV.10.B

`build_block_graph_export` is *the* single enriched block-graph derivation shared by
`MV.10.C`'s CLI and bastion's `BA.17.A` endpoint — neither of those consumers ever
re-derives a field; both project this module's output. Mirrors `graph_emit.rs`'s envelope
style: a `version`/`root` header, a `nodes`/`edges` body, and a resolved-target field on
edges — but where the graph exporter walks the OKF `scope:doc_id` corpus, this module
enriches the `state.json` corpus's `tracks[].blocks[]` (`okf_core::StateGraph`).

Design principles (shared with `graph_emit.rs`):
- **Pure output** — `build_block_graph_export` does not write to disk; it returns a value
  the caller (`MV.10.C`'s CLI subcommand or bastion's `BA.17.A` endpoint) serialises.
- **No re-derivation** — every enrichment field is consumed from an existing primitive
  (`topo_order`, `cycle_paths`, `effective_priorities`, `ready_order`, `derive_focus`) —
  never recomputed independently. `external_deps` becomes node data (the `what` strings
  from `BlockedBy::External` entries); no synthetic node is ever created for an external
  dependency, so node count always equals the in-scope block count.
- **Full corpus before scope** — every derivation (`lane`, `effective_priority`, `layer`,
  `topo_index`, `in_cycle`) runs over the **full corpus** first. The seven-stage scope
  pipeline is layered strictly *after* enrichment, which is the invariant that guarantees
  a scoped export can never report a different value for one of those fields than an
  unscoped export does for the same node.
- **okf-core untouched** — the enrichment sits strictly above `okf_core::StateGraph`;
  `build_state_graph`'s node/edge semantics are unchanged, and no `okf-core` file is
  modified by this module.

#### The seven-stage scope pipeline

Applied in order, strictly after full-corpus enrichment:

1. **Tier** — resolve in-scope repo slugs via `derive_rollup(&scope.tier, …)`, the same
   way bastion's `assemble_board` does, so the graph and the board cannot disagree about
   tier membership.
2. **Repo** — if `scope.repo` is `Some`, intersect down to that slug.
3. **Epic** — if `scope.epic` is `Some`, membership is `epic_members`'s key set. Epic
   **overrides** tier rather than intersecting with it (epic is a cross-repo projection,
   matching bastion's `BoardScope::Epic` arm).
4. **Closed** — when `include_closed` is `false`, drop `BlockLane::Closed` nodes.
5. **Boundary** — when `include_boundary` is `true`, re-add any direct dependency or
   dependent of a surviving node, flagged `in_scope: false`; survivors keep
   `in_scope: true`.
6. **Edges** — keep an edge when its `from` survives and its `to_ref` either survives or
   is dangling; drop edges pointing at a filtered-out node unless boundary re-added it.
7. **Truncate** — record the pre-truncation count in `total_nodes`, then keep the first
   `max_nodes` entries in `topo_index` order and set `truncated` accordingly.

#### Public function

| Function | Signature | Description |
|---|---|---|
| `build_block_graph_export` | `(&Path, &BrainConfig, &StateGraph, &[(StateSource, StateFile)], &BlockGraphScope) -> BlockGraphExport` | `root` is stored as a display string in the envelope header. Computes full-corpus enrichment (`lane`, `effective_priority`, `layer`, `topo_index`, `ready`, `in_cycle`, `external_deps`, `unmet_count`) for every block, then applies the seven-stage scope pipeline. Nodes are emitted in `topo_index` order. |

#### Block-graph types

| Type | Description |
|---|---|
| `BlockGraphExport` | The complete envelope: `version` (`"1"`), `root` (display path), `scope: BlockGraphScopeEcho`, `nodes: Vec<BlockGraphNode>`, `edges: Vec<BlockGraphEdge>`, `cycles: Vec<Vec<String>>` (over the **full corpus**, from `cycle_paths` — never the scoped subgraph), `total_nodes` (pre-truncation count), `truncated`. Derives `Serialize`. |
| `BlockGraphNode` | One enriched block: `key`/`repo`/`id`/`title`/`status`, `lane: BlockLane`, `track`/`wave`/`priority`/`effective_priority`/`due`, `epics`, `layer` (longest path over resolved `depends_on` edges, `0` = no resolved prerequisites, terminates on a cycle via an on-stack recursion guard), `topo_index`, `ready`, `in_cycle`, `in_scope`, `external_deps`, `unmet_count`. Derives `Serialize`. |
| `BlockGraphEdge` | One directed edge: `from`, `to_ref` (raw, as-authored), `kind: StateEdgeKind`, `target_node_id: Option<String>` (`Some` when resolved, `None` when dangling — a dangling edge is retained, never dropped), `blocking` (`false` when either endpoint is `closed`). Derives `Serialize`. |
| `BlockLane` | `#[serde(rename_all = "snake_case")]` enum: `Now`/`Next`/`Blocked`/`Deferred` mirror `derive_focus`'s four lanes (with the owning file's repo slug prefixed onto each bare block ID before joining against node keys); `Closed` comes from the authored `TrackBlock.status == "closed"`; `Other` is the fallback for an unrecognised authored status. |
| `BlockGraphScope` | The scope request: `tier: TierScope`, `epic: Option<String>`, `repo: Option<String>`, `include_closed`, `include_boundary`, `max_nodes`. |
| `BlockGraphScopeEcho` | The request echoed back on `BlockGraphExport`: `tier: Option<String>` (`None` for `TierScope::All`), `epic`, `repo`, `include_closed`, `include_boundary`. Derives `Serialize`. |

#### Public library entry point

`block_graph_brain(root: &Path, scope: &BlockGraphScope) -> anyhow::Result<BlockGraphExport>`
(in `src/lib.rs`) resolves `brain.toml`, discovers and loads every `planning/state.json`
(`discover_state_files` → `load_state`, mirroring `emit_state`'s corpus-load pipeline),
builds the `StateGraph`, and calls `build_block_graph_export`. An individual malformed
`state.json` is skipped rather than failing the whole call, matching bastion's
`assemble_board` posture; only an unresolvable brain root is a hard `Err`. The returned
`BlockGraphExport` is a pure value — nothing is written to disk.

---

### Doc materializer (`src/doc/`) — Phase 9, Block MV.9.A

The doc materializer is the generic brain-document **writer**, sitting on the same
`EmitPlan`/`apply_plan` seam as `src/brain/emit.rs` (above) rather than replacing it: every
`src/doc/*` planner returns an `EmitPlan`, and `apply_plan` remains the single place any bytes
actually land on disk. Where `emit.rs` derives content from the state-graph corpus, `src/doc/`
derives content from an okf-core `BrainDocModel` — a typed document (`Opportunity`,
`LearningArtifact`, `Proposal`) built from an external JSON payload (a company research brief, a
lesson payload, an automation roadmap) rather than from `state.json`.

```
src/doc/
├── mod.rs             ← module docs + re-exports (plan_document, plan_index_reconcile,
│                         plan_ingest, plan_set_stage, plan_add_action, plan_merge_contacts,
│                         OpportunityKind)
├── materialize.rs      ← plan_document() — the generic per-model doc planner
├── index_reconcile.rs  ← plan_index_reconcile() — the index.md row upsert
└── opportunity.rs      ← the four Opportunity command-family planners
```

**D53 boundary:** mev plans and writes the source `.md` (this module); engine-rs executes the
workflow nodes that call it (`EN.7.A`/`EN.7.B`, out of scope here); `bastion serve` only ever
reads the result. See `docs/decisions/D53-engine-executes-mev-writes-brain-docs.md` in the brain
repo.

#### `materialize.rs` — the generic per-model planner

| Function | Signature | Description |
|---|---|---|
| `plan_document` | `(&impl BrainDocModel, &Path) -> EmitPlan` | The target path is derived from the model's `IndexIntent` — `root.join(dirname(index_intent.index_path)).join(index_intent.link_target)` — never from a per-model constant, so the same function plans successfully for all three okf-core models. **Create** (target absent): content is `okf_core::render_document(model)`. **Update** (target present): every `BodySection::Generated` in `model.body()` is re-spliced over the existing bytes via `crate::brain::emit::splice_generated` (a missing sentinel pair pushes `W_DOC_MISSING_SENTINEL` and leaves that section untouched), and the leading frontmatter fence is replaced with `serialize_nested_frontmatter(&model.frontmatter())` — every byte outside the sentinel pairs and the frontmatter fence survives verbatim. **Idempotency:** when computed content equals the existing bytes, no `EmitAction` is planned and `W_DOC_UNCHANGED` is pushed instead. Internally calls `plan_index_reconcile` and merges the result via `EmitPlan::extend`, so one call plans both the doc write and its index row. This function performs the one read needed to compute an update; it performs no writes — `apply_plan` stays the single write point. |

A bad `index_path` (no parent directory component, e.g. a bare `"index.md"`) raises
`E_DOC_BAD_INDEX_PATH` rather than writing to `root` itself.

#### `index_reconcile.rs` — index.md row upsert

| Function | Signature | Description |
|---|---|---|
| `plan_index_reconcile` | `(&IndexIntent, &Path) -> EmitPlan` | Locates `root.join(&intent.index_path)`, parses its first Markdown table, and upserts one row keyed on `link_target`: a body row whose first cell links to `intent.link_target` is replaced in place with the row rendered from `intent.row_cells` (cell 1 is `[<row_cells[0]>](<link_target>)`); no such row means one new row is appended. Never duplicates, reorders, or alters any other row. A `row_cells` length that doesn't match the table's column count pushes `W_DOC_INDEX_COLUMN_MISMATCH` and plans no action. A missing `index.md` pushes `W_DOC_INDEX_MISSING`; a table-less `index.md` pushes `W_DOC_INDEX_NO_TABLE` — in both cases no index is ever created. Unchanged after upsert pushes `W_DOC_UNCHANGED`. |

#### `opportunity.rs` — the Opportunity command family

All four planners resolve their target file through the same `IndexIntent`-derived path
`plan_document` uses, and mutate a parsed `Opportunity` model rather than raw text.

| Function | Signature | Description |
|---|---|---|
| `plan_ingest` | `(&serde_json::Value, Option<OpportunityKind>, &Path) -> EmitPlan` | Builds an `Opportunity` from a raw payload and plans it via `plan_document`. `kind: None` auto-detects: `company_name` present → `Company`; `prospects`/`vertical` present → `ProspectingSweep`; neither → `E_DOC_UNKNOWN_INPUT_SHAPE`, no plan. `Company` dispatches to `Opportunity::from_company_brief`; `ProspectingSweep` to `Opportunity::from_prospecting_result`; `JobPosting` builds from the same brief shape with `kind: "job-posting"`. |
| `plan_set_stage` | `(&str, &str, &Path) -> EmitPlan` | Loads the existing file, `parse_nested_frontmatter` + `Opportunity::from_frontmatter`, sets `stage` (validated against the seven documented values — an unknown value pushes `E_DOC_BAD_STAGE` and plans nothing), and re-plans via `plan_document`. |
| `plan_add_action` | `(&str, &str, &str, &str, &Path) -> EmitPlan` | Appends one `Action { at, kind, note }` to `actions[]`. An identical triple already present is not re-appended (the re-plan then becomes a `W_DOC_UNCHANGED` no-op via `plan_document`'s idempotency guard). |
| `plan_merge_contacts` | `(&str, &serde_json::Value, &Path) -> EmitPlan` | Merges `Contact` entries into `contacts[]` matched on `name`: unions `emails`/`whatsapp`/`phones`/`links` (deduped, order-stable); fills `role`/`note` only when the existing value is empty, so an enriched field is never overwritten by a blank one. |

All three mutators (`plan_set_stage`, `plan_add_action`, `plan_merge_contacts`) push
`E_DOC_NOT_FOUND` and plan nothing when the target file is absent or unparsable.

| Type | Description |
|---|---|
| `OpportunityKind` | `Company \| ProspectingSweep \| JobPosting` — parses from the CLI's `--kind` string (`company \| prospecting-sweep \| job-posting`). |

#### Diagnostic locators emitted by `src/doc/`

| Locator | Severity | Condition |
|---|---|---|
| `W_DOC_UNCHANGED` | Warning | Computed content already matches the existing file/row; no action planned. |
| `W_DOC_MISSING_SENTINEL` | Warning | A `BodySection::Generated` section's sentinel pair is absent; section left untouched. |
| `W_DOC_INDEX_MISSING` | Warning | The target `index.md` is absent; no index action planned. |
| `W_DOC_INDEX_NO_TABLE` | Warning | `index.md` has no parsable table; no index action planned. |
| `W_DOC_INDEX_COLUMN_MISMATCH` | Warning | `row_cells` count doesn't match the table's column count; no index action planned. |
| `E_DOC_BAD_INDEX_PATH` | Error | The model's `IndexIntent.index_path` has no parent directory component. |
| `E_DOC_UNKNOWN_INPUT_SHAPE` | Error | `ingest` input matches neither the company nor the prospecting-sweep shape, and no `--kind` was given. |
| `E_DOC_UNKNOWN_MODEL` | Error | `doc materialize --model` is not one of `opportunity`\|`learning-artifact`\|`proposal`. |
| `E_DOC_BAD_STAGE` | Error | `set-stage`'s stage argument is not one of the seven documented values. |
| `E_DOC_NOT_FOUND` | Error | A mutator's target file is absent or unparsable. |

`apply_plan`'s existing `W_EMIT_DRY_RUN` / `I_EMIT_WROTE` codes are reused unchanged for the
write half — no new codes were needed there.

#### Public library entry points

`doc_materialize`, `doc_opportunity_ingest`, `doc_opportunity_set_stage`,
`doc_opportunity_add_action`, and `doc_opportunity_merge_contacts` (all in `src/lib.rs`) mirror
`emit_state`'s shape: the caller resolves `root` (via `find_brain_root`), the function plans via
the appropriate `src/doc/` planner, applies with `apply_plan(&plan, write)`, and folds the
diagnostics into a `Report`. Invoked by `mev doc materialize` and the `mev doc opportunity ...`
subcommands respectively.
