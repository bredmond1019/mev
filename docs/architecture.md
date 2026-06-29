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
    ├── okf.rs      ← OkfFrontmatter, validate_md_file() — OKF field checks; root-instruction-file exemption
    ├── scope.rs    ← scope_units(), scope_for(), owning_unit() — registry-driven scope resolver
    ├── sync.rs     ← (internal) sync helpers
    ├── graph.rs    ← EdgeKind, Edge, Node, Graph, GraphArtifact, DocMeta; build_graph(), check_graph(), read_doc_metadata() — Phase 3 Block J
    └── state.rs    ← StateFile, StateGraph, StateNode, StateEdge, StateSource; discover_state_files(), load_state(), check_schema(), build_state_graph(), check_state_graph(), check_rollup() — Phase 3 Block P

tests/
├── brain_config.rs   ← integration tests for brain.toml loading + BrainConfig
├── brain_corpus.rs   ← integration tests for crawl_corpus() multi-root walk + scope resolution
├── brain_crawl.rs    ← integration tests for crawl_brain()
├── brain_graph.rs    ← integration tests for validate_brain_graph() end-to-end — Phase 3 Block J
├── brain_okf.rs      ← integration tests for validate_md_file()
├── brain_state.rs    ← integration tests for validate_brain_state() end-to-end — Phase 3 Block P
├── brain_validate.rs ← integration tests for BrainValidator end-to-end
├── smoke.rs          ← integration tests for the learn-ai validate() public API
└── fixtures/
    └── brain.toml    ← minimal fixture — NOT the live brain.toml
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
| `CorpusEntry` | Multi-root corpus entry: `path`, `rel`, `stem`, `scope` (owning-unit slug) |
| `Corpus` | Complete Brain corpus: `entries: Vec<CorpusEntry>`; serde-serializable for manifest emission |

### Scope resolution (`src/brain/scope.rs`)

| Function | Signature | Description |
|---|---|---|
| `scope_units` | `(&BrainConfig) -> Vec<(String, String)>` | All `(slug, repo_path)` pairs from the registry |
| `scope_for` | `(&Path, &BrainConfig) -> String` | Resolve a HQ-relative path to its owning unit's slug |
| `owning_unit` | `(&Path, &BrainConfig) -> (String, String)` | Resolve a HQ-relative path to its `(slug, repo_path)` pair |

Resolution algorithm: longest-prefix match using `Path::strip_prefix` (prevents `core/mev-extra` from matching `core/mev`). Root unit (`repo_path = "."`) is the fallback when no prefix matches.

### Knowledge graph (`src/brain/graph.rs`) — Phase 3 Block J

The graph module builds and validates the serializable `scope:doc_id` knowledge graph over the Brain corpus.

#### Public functions

| Function | Signature | Description |
|---|---|---|
| `read_doc_metadata` | `(&CorpusEntry) -> DocMeta` | D5 seam — reads `doc_id` and `related` from a corpus entry's inline frontmatter. Degrades gracefully on I/O or parse error (returns empty `DocMeta`). |
| `build_graph` | `(&Corpus, &BrainConfig) -> GraphArtifact` | Walks the corpus once; files with a `doc_id` become nodes, others become leaves. Returns the serializable graph plus lookup structures. |
| `check_graph` | `(&GraphArtifact) -> Vec<Diagnostic>` | Checks the built graph for integrity violations without re-walking the corpus. |

#### Graph types

| Type | Description |
|---|---|
| `EdgeKind` | Discriminant enum — `Related` is the only variant today (from `related:` frontmatter). |
| `Edge` | Directed edge: `from` (canonical `scope:doc_id`), `to_ref` (as-authored), `kind`. |
| `Node` | Graph node: `id` (canonical `scope:doc_id`), `scope`, `doc_id`, `rel` path. |
| `Graph` | Serializable D4 artifact: `nodes: Vec<Node>`, `edges: Vec<Edge>`. |
| `GraphArtifact` | Build output: `graph`, `node_map` (canonical id → node index), `leaf_keys` (files with no `doc_id`). |
| `DocMeta` | Metadata extracted by `read_doc_metadata`: `doc_id: Option<String>`, `related: Vec<String>`. |

#### Diagnostic locators emitted by `check_graph`

| Locator | Severity | Condition |
|---|---|---|
| `E_GRAPH_DUPLICATE_DOC_ID` | Error | Two or more corpus files share the same `scope:doc_id`. |
| `related` | Error | A `related:` entry resolves to no node and no leaf (`E_GRAPH_DANGLING_RELATED`). |
| `related` | Warning | A `related:` entry resolves to a real corpus file that has no `doc_id` (`W_GRAPH_LEAF_TARGET`). |

#### Public library entry point

`validate_brain_graph(root: &Path) -> anyhow::Result<Report>` (in `src/lib.rs`) runs the full OKF schema pass followed by the graph pass — crawls the corpus once, builds the graph, and appends graph diagnostics to the same `Report`. Invoked by `mev validate-brain --graph`.

---

### State integrity (`src/brain/state.rs`) — Phase 3 Block P

The state module discovers, loads, and validates all `planning/state.json` files across the registered repos, then builds and checks the cross-repo block-dependency graph.

#### Public functions

| Function | Signature | Description |
|---|---|---|
| `discover_state_files` | `(&Path, &BrainConfig) -> (Vec<StateSource>, Vec<Diagnostic>)` | Discovers state.json paths for the HQ brain, tier sub-brains (via `tiers[].rollup`), and leaf repos (via `[[repos]]` in `brain.toml`). Missing files emit `W_STATE_FILE_MISSING`. |
| `load_state` | `(&Path) -> Result<StateFile, StateLoadError>` | Deserializes a `planning/state.json` file into a `StateFile`. Returns `StateLoadError::Malformed` on schema mismatch or `StateLoadError::Io` on I/O failure. |
| `check_schema` | `(&StateSource, &StateFile) -> Vec<Diagnostic>` | Schema-ring validation: kind membership, `updated` non-empty, status enum values, `blocked_by` well-formedness, kind-appropriate section presence. |
| `build_state_graph` | `(&[(StateSource, StateFile)]) -> StateGraph` | Builds the cross-repo block-dependency graph: nodes from `tracks[]` blocks, edges from `blocked_by` and `cross_repo[]` entries. |
| `check_state_graph` | `(&StateGraph, &[(StateSource, StateFile)]) -> Vec<Diagnostic>` | Graph integrity checks: duplicate block IDs, dangling focus references, unknown repos, dangling blocked_by, dangling cross_repo edges. |
| `check_rollup` | `(&Path, &StateFile, &HashMap<String, StateFile>) -> Vec<Diagnostic>` | Rollup drift check: compares brain `repos[]` now/next headline entries against each child's actual `focus` values. Emits `W_STATE_ROLLUP_DRIFT` on mismatch. |

#### State types

| Type | Description |
|---|---|
| `StateFile` | Top-level deserialized `state.json`: `kind`, `updated`, `focus`, `tracks`, `repos`, `tiers`, `cross_repo`, `note`. |
| `Focus` | Current focus entry: `now`, `next`, `blocked_by`. |
| `Block` | A single `tracks[]` block: `id`, `title`, `status`, `blocked_by`. |
| `BlockedBy` | Internally-tagged enum for block dependencies: `BlockRef { repo, id }` or `External { description }`. |
| `StateSource` | Discovery result: `abs_path`, `repo_slug`, `kind` (`hq`, `tier`, `leaf`). |
| `StateGraph` | Cross-repo block-dependency graph: `nodes: Vec<StateNode>`, `edges: Vec<StateEdge>`. Serde-serializable. |
| `StateNode` | Graph node: `repo`, `id`, `title`, `status`, `source_path` (skipped in serialization). |
| `StateEdge` | Graph edge: `from_repo`, `from_id`, `to_repo`, `to_id`, `kind` (`blocked_by` or `cross_repo`), `source_path` (skipped in serialization). |

#### Public library entry point

`validate_brain_state(root: &Path) -> anyhow::Result<Report>` (in `src/lib.rs`) runs the full OKF schema pass followed by the five-step state pipeline (discovery → load → schema → graph → rollup) and appends all state diagnostics to the same `Report`. Invoked by `mev validate-brain --state`.
