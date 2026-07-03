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
    ├── config.rs   ← BrainConfig, CrawlConfig, VocabConfig, RepoEntry; find_brain_config(), load_brain_config()
    ├── crawl.rs    ← crawl_corpus() → (Corpus, Vec<Diagnostic>); Corpus, CorpusEntry, MdFile; crawl_brain() (legacy)
    ├── okf.rs      ← OkfFrontmatter (re-exported from bastion's okf-core crate, BA.15.12/D15/D16), validate_md_file() — OKF field checks; root-instruction-file exemption
    ├── scope.rs    ← scope_units(), scope_for(), owning_unit() — registry-driven scope resolver
    ├── sync.rs     ← (internal) sync helpers
    ├── graph.rs    ← EdgeKind, Edge, Node, Graph, GraphArtifact, EdgeResolution, resolve_edge (re-exported from okf-core, BA.15.12/D16), DocMeta; build_graph(), check_graph() — Phase 3 Block J (read_doc_metadata removed by D5 extract-once refactor in Block Q)
    ├── state.rs    ← serde schema, StateGraph, StateNode, StateEdge, StateSource, TierEntry, load_state(), build_state_graph() (re-exported from okf-core, BA.15.12/D15/D16); mev-local: discover_state_files(), check_schema(), check_state_graph(), check_rollup(), detect_cycles(), ready_order(), check_focus_drift(), derive_focus(), derive_rollup(), derive_cross_repo(), tier_scope_for(), derive_brain_focus() — Phase 3 Block P / P2 / T / MV.3B.U (v2: depends_on DAG, cycle detection, derived-blocked enforcement, backlog nodes, focus-drift warnings, single-source derivation helpers; MV.3B.U: tier-scoped non-destructive rollup + brain-focus union)
    ├── emit.rs     ← EmitError, EmitAction, EmitPlan; wave_order(), render_wave_table(), splice_generated(), plan_state_json(), plan_master_plan_tables(), apply_plan() — Phase 3 Block T (derived-view generation: wave tables, focus regen, brain rollup)
    ├── links.rs    ← LinkKind, LinkRef; extract_links(), check_links(), collect_doc_ids(), read_moves_pending(), check_moved_references() — Phase 3 Block K
    ├── structure.rs ← check_structure() — Phase 3 Block L (bidirectional index.md <-> directory structural coverage: orphan files, dangling rows)
    ├── manifest.rs ← ManifestEntry, Manifest, build_manifest() — Phase 3 Block Q (canonical corpus manifest for RAG indexer)
    └── graph_emit.rs ← GraphExport, ExportedEdge, build_graph_export() (re-exported from okf-core, BA.15.12/D16) — Phase 3B Block R (graph-export envelope for the orchestrator's Postgres edges table, D4)

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
| `check_schema` | `(&StateSource, &StateFile) -> Vec<Diagnostic>` | Schema-ring validation: kind membership, `updated` non-empty, status enum values, `blocked_by` well-formedness, kind-appropriate section presence. In v2 files: validates `tracks[].blocks[].depends_on[]` entry well-formedness, rejects authored `status:"blocked"` (`E_STATE_AUTHORED_BLOCKED`), and validates `backlog[].status` membership. |
| `build_state_graph` | `(&[(StateSource, StateFile)]) -> StateGraph` | Builds the cross-repo block-dependency graph: nodes from `tracks[]` blocks, edges from `depends_on[]` (v2) and `cross_repo[]`. `External`-type `depends_on` entries are leaves, not graph edges. |
| `check_state_graph` | `(&StateGraph, &[(StateSource, StateFile)]) -> Vec<Diagnostic>` | Graph integrity checks: duplicate block IDs, dangling focus references, unknown repos, dangling blocked_by, dangling cross_repo edges. |
| `detect_cycles` | `(&StateGraph) -> Vec<Diagnostic>` | DFS cycle detection over `depends_on` edges; emits `E_STATE_CYCLE` naming the cycle path (e.g. `A → B → A`). |
| `check_status_consistency` | `(&[(StateSource, StateFile)]) -> Vec<Diagnostic>` | Emits `E_STATE_STATUS_INCONSISTENT` when a `closed` block has a `type:block` `depends_on` target that is not `closed`. Dangling targets are skipped (reported by `check_state_graph`). |
| `check_backlog_integrity` | `(&[(StateSource, StateFile)], &StateGraph) -> Vec<Diagnostic>` | Backlog-node integrity: dangling `depends_on` targets (`E_STATE_DANGLING_BLOCKED_BY`) and orphan/unresolved promoted nodes (`E_STATE_DANGLING_PROMOTION`). |
| `ready_order` | `(&StateGraph, &[(StateSource, StateFile)]) -> Vec<String>` | Reusable standalone function: returns open blocks ordered by `wave` (tiebreak: track order, array order) whose every `type:block` dep is `closed` and which have no `type:external` deps. Forward-compat input for `MV.3B.T` topo-emit. |
| `check_focus_drift` | `(&StateSource, &StateFile, &StateGraph, &[(StateSource, StateFile)]) -> Vec<Diagnostic>` | Recomputes the expected `focus` from authored `tracks[]` by calling `derive_focus` and emits `W_STATE_FOCUS_DRIFT` (warning, exit 0) on block-id set mismatch. Shares the same derivation logic as `mev emit-state` — they cannot disagree. |
| `check_rollup` | `(&Path, &StateFile, &HashMap<String, StateFile>) -> Vec<Diagnostic>` | Rollup drift check: compares brain `repos[]` now/next headline entries against each child's actual `focus` values. Emits `W_STATE_ROLLUP_DRIFT` on mismatch. |
| `derive_focus` | `(&StateSource, &StateFile, &StateGraph, &[(StateSource, StateFile)]) -> DerivedFocus` | **Single-source derivation** — computes the expected `focus` from authored `tracks[]`: `now` = `in_progress` blocks; `next` = ready open blocks in `wave` order; `blocked` = open blocks with an unmet `depends_on`, each carrying the unmet subset as `blocked_by[]`. Called by both `check_focus_drift` and the emit planners. |
| `derive_cross_repo` | `(&[(StateSource, StateFile)]) -> Vec<CrossRepoEdge>` | For every `tracks[].blocks[].depends_on` entry of `{type:"block"}` whose `repo` differs from the owning repo, produces a `CrossRepoEdge`. Same-repo deps are excluded. |
| `tier_scope_for` | `(&StateFile, &BrainConfig) -> TierScope` | Determines the `TierScope` a brain file's `repos[]`/`focus` should be scoped to: `TierScope::Tier(t)` when `brain_file.repo` matches a `tier` value declared in `brain.toml`'s `[[repos]]`; `TierScope::All` (every repo) otherwise (the HQ root). MV.3B.U. |
| `derive_rollup` | `(&TierScope, &BrainConfig, &[RepoRollup], &StateGraph, &[(StateSource, StateFile)]) -> Vec<RepoRollup>` | Tier-scoped, non-destructive rollup (MV.3B.U). Iterates the in-scope `config.repos[]` (config order): if a loadable `kind == "project"` child exists, derives its headline via `derive_focus` and sets `tier` from config; else if `existing` (the brain file's current `repos[]`) already has an entry for that slug, preserves it verbatim (backfilling `tier`); else emits a tier-tagged empty stub. `RepoRollup.tier` is non-`None` in every branch, and a malformed/missing child can never silently drop a repo. |
| `derive_brain_focus` | `(&TierScope, &BrainConfig, &StateGraph, &[(StateSource, StateFile)]) -> Focus` | Derives a brain file's `focus.now/next/blocked` as the repo-tagged union of its in-scope children's `derive_focus` output (MV.3B.U). Iterates in-scope `config.repos[]` in config order; for each repo with a loadable child, appends its `now`/`next`/`blocked` blocks (each tagged `repo: Some(<slug>)`) in the child's own within-focus order, deduplicated by `(repo, id)` per list. Repos with no loadable child contribute nothing. |

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
| `TierScope` | MV.3B.U: the repo set a brain file's `repos[]`/`focus` should be scoped to — `Tier(String)` (one tier's `[[repos]]`) or `All` (every repo, the HQ root). Produced by `tier_scope_for`, consumed by `derive_rollup` and `derive_brain_focus`. |

#### Public library entry point

`validate_brain_state(root: &Path) -> anyhow::Result<Report>` (in `src/lib.rs`) runs the full OKF schema pass followed by the multi-step state pipeline (discovery → load → schema → graph build + check → cycle detection → status consistency → backlog integrity → rollup → focus drift) and appends all state diagnostics to the same `Report`. Invoked by `mev validate-brain --state`.

---

### Emit module (`src/brain/emit.rs`) — Phase 3 Block T

The emit module is the **single derivation engine** for all generated views declared by the v2 state schema. It is a pure compiler (files in → files out; no DB, no network), and its planners share the same `derive_focus` / `derive_rollup` / `derive_cross_repo` / `derive_brain_focus` helpers used by the validator's drift checks — so the emit is, by construction, the fixed point of `check_focus_drift` and `check_rollup`.

#### Public functions

| Function | Signature | Description |
|---|---|---|
| `wave_order` | `(&StateGraph, &[(StateSource, StateFile)]) -> Vec<String>` | All block keys (`"repo:id"`) sorted by `wave` ascending (`None` last), tiebreak by track iteration order then block array index. Full-roadmap sibling of `ready_order` — includes every block regardless of status. |
| `render_wave_table` | `(&str, &StateFile, &StateGraph) -> String` | Renders a Markdown table of one repo's blocks in wave order. Columns: `Wave \| Block \| Title \| Status \| Depends on`. Open blocks with an unmet `depends_on` render as `blocked` in the Status column. |
| `splice_generated` | `(&str, &str, &str) -> Result<String, EmitError>` | Idempotent sentinel-splice: replaces the text between `<!-- BEGIN generated:{marker} -->` and `<!-- END generated:{marker} -->` with `generated`, preserving every line outside the sentinels verbatim. Returns `EmitError` when a sentinel is missing or unbalanced. |
| `plan_state_json` | `(&[(StateSource, StateFile)], &StateGraph, &BrainConfig) -> EmitPlan` | For each loaded state file, clones it, regenerates derived sections (leaf: `focus`; brain: tier-scoped non-destructive `repos[]` via `tier_scope_for`/`derive_rollup`, `cross_repo[]`, and `focus` via `derive_brain_focus`), re-serializes, and adds an `EmitAction` only when the content actually changed. Authored fields survive the round-trip unchanged. MV.3B.U threaded `&BrainConfig` in to drive tier scoping. |
| `plan_master_plan_tables` | `(&[(StateSource, StateFile)], &StateGraph) -> EmitPlan` | For each state file, resolves the sibling `master-plan.md`; if it exists and carries the `wave-table` sentinels, splices the rendered table and adds an `EmitAction`. A missing file or sentinels yields `W_EMIT_NO_SENTINEL` (never invents sentinels). |
| `apply_plan` | `(&EmitPlan, bool) -> Vec<Diagnostic>` | When `write` is `true`, writes each action's `new_content` to its `path` and emits `I_EMIT_WROTE` per file. When `false` (dry-run), writes nothing and emits `W_EMIT_DRY_RUN` per planned action. Always surfaces the plan's own diagnostics. |

#### Emit types

| Type | Description |
|---|---|
| `EmitError` | thiserror error for sentinel failures: `MissingSentinel { marker, which }`. |
| `EmitAction` | A single proposed write: `path: PathBuf`, `new_content: String`, `note: String`. |
| `EmitPlan` | A collection of proposed writes and accompanying diagnostics: `actions: Vec<EmitAction>`, `diagnostics: Vec<Diagnostic>`. |

#### Diagnostic locators emitted by the emit module

| Locator | Severity | Condition |
|---|---|---|
| `W_EMIT_DRY_RUN` | Warning | Planned action in dry-run mode; no file written. |
| `I_EMIT_WROTE` | Warning | File written in `--write` mode. |
| `W_EMIT_NO_SENTINEL` | Warning | `master-plan.md` is missing the `wave-table` sentinel pair; file skipped. |
| `E_EMIT_WRITE_FAILED` | Error | IO error writing a file. |

#### Public library entry point

`emit_state(root: &Path, write: bool) -> anyhow::Result<Report>` (in `src/lib.rs`) resolves `brain.toml`, discovers and loads all state files, builds the graph, runs `plan_state_json` and `plan_master_plan_tables`, applies the plan with `apply_plan(write)`, and collects all diagnostics into a `Report`. Invoked by `mev emit-state`.

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
