# Task Spec — Phase 3B, Block Q (Manifest Emit)

**Status:** Not started · **Last run:** never

## Goal
Emit the canonical file-list + per-file OKF metadata as JSON from the corpus crawl result, so the
Brain RAG indexer (`index_brain.py`) can consume mev's manifest instead of re-crawling — "what's
validated == what's embedded" holds by construction. Carries the D5 extract-once refactor: parse
frontmatter once during crawl and surface it on `CorpusEntry`.

## Context Pointers
- **Master-plan block:** `planning/master-plan.md` → `MV.3B.Q — Manifest emit`
- **D4:** `planning/decisions/D4-corpus-engine-and-knowledge-graph.md` — corpus engine outputs
- **D5:** `planning/decisions/D5-heterogeneous-format-ingest.md` — extract-once refactor deferred to
  this block; `CorpusEntry { …, metadata }` becomes load-bearing here
- **Existing seam:** `src/brain/graph.rs` → `read_doc_metadata()` / `DocMeta` — the D5 single-site
  that parses frontmatter from a `CorpusEntry`; this block collapses it to `entry.metadata`
- **Corpus types:** `src/brain/crawl.rs` → `CorpusEntry`, `Corpus`, `crawl_corpus()`
- **OKF struct:** `src/brain/okf.rs` → `OkfFrontmatter` (the serde target for all frontmatter fields)
- **CLI dispatch:** `src/main.rs` → `Command` enum; `src/lib.rs` → public API functions
- **Standing rules:** GEMINI.md — tests ship with every change; harness gates must pass

## Step-by-Step Tasks

### 1. Add parsed metadata to `CorpusEntry` (D5 extract-once refactor)
- **Files:** `src/brain/crawl.rs` (modify)
- Add an `Option<OkfFrontmatter>` field (or a new `EntryMetadata` struct wrapping the relevant
  fields) to `CorpusEntry`. The type must derive `serde::Serialize` so the manifest can emit it.
  `OkfFrontmatter` currently only derives `Deserialize`; add `Serialize` to it in `src/brain/okf.rs`.
- In `crawl_corpus()`, after determining corpus membership and before pushing the entry, read the
  file contents, call `extract_frontmatter()` + `serde_yaml::from_str::<OkfFrontmatter>()`, and
  store the parsed result on the entry. On any parse failure, store `None` (graceful degradation —
  the OKF validator will catch the error separately).
- Update the existing `Corpus` serialization unit test (`corpus_is_serializable_to_json`) to verify
  the metadata field round-trips.
- Add a new test: a corpus entry with valid frontmatter carries `Some(metadata)` with correct fields;
  one without frontmatter carries `None`.

### 2. Collapse `read_doc_metadata` seam to use `CorpusEntry.metadata`
- **Files:** `src/brain/graph.rs` (modify)
- Change `build_graph()` to read `doc_id` and `related` from `entry.metadata` (the new field) instead
  of calling `read_doc_metadata(entry)`. The `read_doc_metadata` function becomes dead code — remove
  it and its `RawFrontmatter` helper struct.
- Update `DocMeta` construction: derive `doc_id` and `related` from the parsed `OkfFrontmatter` on
  the entry. If `entry.metadata` is `None`, produce `DocMeta { doc_id: None, related: vec![] }` (same
  graceful degradation as before).
- All existing graph unit tests (`tests/brain_graph.rs` and in-module tests in `graph.rs`) must pass
  unchanged — the graph's behaviour is identical, only the metadata source changes.
- Update `src/brain/links.rs` → `collect_doc_ids()` if it re-reads frontmatter independently; it
  should also use `entry.metadata` now.

### 3. Build the manifest module and `ManifestEntry` type
- **Files:** `src/brain/manifest.rs` (new), `src/brain/mod.rs` (modify — add `pub mod manifest;`)
- Create `src/brain/manifest.rs` with:
  - `ManifestEntry` struct (derives `Serialize`): `rel` (String — relative path), `scope` (String),
    `doc_id` (Option<String>), `doc_type` (Option<String> — the `type` field), `title`
    (Option<String>), `description` (Option<String>), `layer` (Option<Vec<String>>), `project`
    (Option<String>), `status` (Option<String>), `keywords` (Option<Vec<String>>).
  - `Manifest` struct (derives `Serialize`): `version` (String, e.g. `"1"`), `root` (String — display
    path of the HQ root), `entries` (Vec<ManifestEntry>).
  - `build_manifest(root: &Path, corpus: &Corpus) -> Manifest` — iterates `corpus.entries`, maps each
    `CorpusEntry` to a `ManifestEntry` by extracting the relevant fields from `entry.metadata`.
- Add unit tests in-module:
  - A corpus with two entries (one with metadata, one without) produces a manifest with matching
    `ManifestEntry` values.
  - The manifest serializes to JSON containing `"version"`, `"root"`, `"entries"`.

### 4. Add `manifest` CLI subcommand and library driver
- **Files:** `src/lib.rs` (modify), `src/main.rs` (modify), `tests/brain_manifest.rs` (new)
- In `src/lib.rs`: add `pub use brain::manifest::{Manifest, ManifestEntry, build_manifest};` and a
  public `manifest_brain(root: &Path) -> anyhow::Result<Manifest>` function that resolves
  `brain.toml`, calls `crawl_corpus`, and calls `build_manifest`.
- In `src/main.rs`: add a `Manifest` variant to the `Command` enum with a `path` argument (default
  `.`) and optional `--pretty` flag (default: compact JSON). Dispatch calls `mev::manifest_brain()`,
  serializes the result to stdout via `serde_json`, and exits 0 on success / 1 on config error.
  No `--json` envelope needed — the output *is* JSON.
- In `tests/brain_manifest.rs`: create an integration test that builds a temp dir with a `brain.toml`
  and several `.md` files under `planning/` and `docs/`, calls `manifest_brain()`, and asserts:
  - The manifest `entries` count matches the expected corpus size.
  - Each entry's `scope`, `doc_id`, `title` match the fixture files.
  - A file without frontmatter produces `doc_id: null` in the entry.
  - The manifest serializes to valid JSON (`serde_json::to_string`).

### 5. Update documentation
- **Files:** `docs/cli.md` (modify), `docs/architecture.md` (modify)
- In `docs/cli.md`: add the `manifest` subcommand reference — arguments, flags (`--pretty`), output
  shape, exit codes, and a sample JSON snippet showing the manifest schema.
- In `docs/architecture.md`: add the `manifest` module to the module map and describe the
  `build_manifest` function and `ManifestEntry` type. Note that `CorpusEntry` now carries parsed
  metadata (the D5 extract-once refactor) and `read_doc_metadata` is removed.

### 6. Validate
- Run the Validation Commands listed below and confirm all pass.
- Confirm existing test count is maintained plus new tests added.
- Verify `cargo run -- manifest --help` prints the subcommand help.

## Acceptance Criteria
- `CorpusEntry` carries parsed `OkfFrontmatter` metadata from the crawl; frontmatter is read exactly
  once per file (no double-parse between OKF validation and graph/manifest).
- `read_doc_metadata()` in `graph.rs` is removed; `build_graph()` reads metadata from the entry.
- All existing tests pass unchanged (graph, links, OKF, state, emit, crawl).
- `mev manifest <root>` emits a JSON manifest listing every corpus file with scope, doc_id, and
  OKF metadata fields.
- The manifest lists exactly the files `crawl_corpus` returns (same file set, same scope).
- A file with no frontmatter appears in the manifest with null/absent metadata fields (graceful
  degradation).
- The manifest serializes to valid JSON consumable by `serde_json::from_str`.
- `docs/cli.md` documents the `manifest` subcommand; `docs/architecture.md` reflects the new module.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes
- The `OkfFrontmatter` struct needs `Serialize` added — it currently only derives `Deserialize`.
  This is a one-line change in `src/brain/okf.rs`.
- The D5 extract-once refactor is the heaviest lift (Task 1–2). Tasks 3–4 are additive.
- The manifest is a **pure output** — mev writes nothing to disk (consistent with D4 pure compiler
  model); it prints to stdout and the orchestrator consumes it.
- Out of scope: refactoring `index_brain.py` to consume the manifest (orchestrator-side work).

## Amendment Log
<!-- Append-only. Pipeline stages append one dated line here when they deviate from the spec. -->
_No amendments yet._
