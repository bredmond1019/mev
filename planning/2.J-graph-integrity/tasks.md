---
type: TaskSpec
title: Task Spec — Phase 3, Block J — graph integrity (related: edges)
description: Decomposed task spec for the corpus-wide doc_id index, dangling related-edge detection, and duplicate doc_id detection surfaced via `mev validate-brain --graph`.
doc_id: 2j-graph-integrity-tasks
layer: [factory, brain]
project: mev
status: active
keywords: [graph integrity, related edges, doc_id index, duplicate doc_id, validate-brain, D29]
related: [master-plan, status, D29-mev-brain-validation-engine]
---

# Task Spec — Phase 3, Block J — Graph integrity (`related:` edges)

**Status:** Not started · **Last run:** never

## Goal
Build a corpus-wide `doc_id` index (every `.md`'s `doc_id`, defaulting to filename stem) and flag
every `related:` entry that points at a `doc_id` no document defines (a dangling edge) plus every
duplicate `doc_id`, surfaced via a new `mev validate-brain --graph` mode.

## Context Pointers

- **Plan:** mev `planning/master-plan.md` → Phase 3, **Block J — Graph integrity (`related:` edges)**
  (the generalization of the learn-ai anchor-slice contract). Governed by brain **D29** (mev is the
  single validation engine; these checks are read-only and `--json`-able).
- **Repo files that apply:**
  - `src/brain/okf.rs` — `OkfFrontmatter` already exposes `doc_id: Option<String>` and
    `related: Option<Vec<String>>`; reuse it (plus `extract_frontmatter`) to read each file's edges.
  - `src/brain/crawl.rs` — `MdFile { path, rel, stem }`; `stem` is the doc_id default.
  - `src/brain/mod.rs` — `BrainValidator` + `ContentValidator` impl (register the new module here;
    the graph check is a corpus-wide pass over the full crawled `MdFile` list, not a per-item check).
  - `src/lib.rs` — `validate_brain()` (the sibling to add `validate_brain_graph()` beside, with the
    same `E_CONFIG_NOT_FOUND` fallback); `Diagnostic` / `Report` / `JsonReport` (the `--json`
    envelope serializes every diagnostic, so a graph finding flows through it unchanged).
  - `src/main.rs` — `ValidateBrain` subcommand + global `--json` flag; the `--sync` flag added in the
    Block N spec is the sibling pattern to mirror for `--graph`.
- **CLAUDE.md standing rules:** every block ships with tests (rule 1); all four harness gates stay
  green; existing learn-ai + brain tests must keep passing.

### Scoping decisions made at authoring time (do not relitigate)

1. **CLI surface = a `--graph` flag** on `validate-brain`, parallel to Block N's `--sync`. Default
   `validate-brain` stays a fast per-file schema check (the pre-commit tier); `--graph` is the fuller
   pre-push tier. `--graph` runs the schema pass **plus** the graph check.
2. **Duplicate-`doc_id` detection considers only files that successfully parse OKF frontmatter.**
   Files without parseable frontmatter (e.g. the five `CLAUDE.md` instruction files, which all default
   to stem `CLAUDE`) are **excluded from duplicate flagging** — verified against the live corpus, this
   is the only stem collision and it is not a real corpus-doc conflict. Real corpus docs declare an
   explicit `doc_id`, so the live corpus stays clean.
3. **The resolution index (for `related:` edges) includes every crawled `.md`** with effective doc_id =
   explicit `doc_id` if present and non-empty, else the filename `stem` (per the master-plan). So a
   `related:` ref can resolve to any crawled file; only *duplicate flagging* is restricted (decision 2).
4. **`Graph` findings reuse the existing `Diagnostic` currency** as `Error`-severity diagnostics with
   distinct `E_GRAPH_*` locator codes (below). No new `Severity` variant; `JsonReport` serializes them
   automatically, so `--json` needs no envelope change.

### Locator codes (the `Graph` diagnostic vocabulary)

- `E_GRAPH_DANGLING_RELATED` — a `related:` entry names a doc_id no document defines.
- `E_GRAPH_DUPLICATE_DOC_ID` — two or more OKF-frontmatter files claim the same effective doc_id.

## Step-by-Step Tasks

### 1. Graph module foundation — `doc_id` index + edge parsing
- Create `src/brain/graph.rs` and register it with `pub mod graph;` in `src/brain/mod.rs`.
- Add an internal helper that, given an `MdFile`, returns its effective doc_id (explicit non-empty
  `doc_id` else `stem`), its `related` list (empty when absent), and a `has_okf_frontmatter` flag —
  reusing `extract_frontmatter` + `OkfFrontmatter` deserialization (a read/parse failure ⇒
  `has_okf_frontmatter = false`, effective doc_id falls back to `stem`, related = empty).
- Add `build_doc_id_index(items: &[MdFile])` producing both (a) the **resolution keyset** of every
  effective doc_id across all crawled files, and (b) a map `doc_id -> Vec<rel-path>` restricted to
  files where `has_okf_frontmatter` is true (the duplicate-detection input).
- Unit tests: explicit doc_id wins over stem; stem fallback when doc_id absent; a frontmatter-less
  file contributes its stem to the resolution keyset but is excluded from the duplicate map.
- Files: `src/brain/graph.rs`, `src/brain/mod.rs`.

### 2. Graph checks — dangling `related` + duplicate `doc_id`
- In `src/brain/graph.rs`, add `pub fn check_graph(items: &[MdFile]) -> Vec<Diagnostic>` built on the
  Task 1 index:
  - **Dangling:** for each file, for each `related:` entry not present in the resolution keyset, emit
    `E_GRAPH_DANGLING_RELATED` (locator `related`, `file` = the referring file's rel path, message
    naming the unresolved target id).
  - **Duplicate:** for each effective doc_id claimed by ≥2 files **in the duplicate map** (OKF docs
    only), emit one `E_GRAPH_DUPLICATE_DOC_ID` (message listing the colliding rel paths).
- Unit tests with temp-dir fixtures (mirror the `okf.rs` temp-dir style): clean corpus → 0; a doc with
  `related: [does-not-exist]` → exactly one `E_GRAPH_DANGLING_RELATED`; two docs declaring the same
  explicit doc_id → exactly one `E_GRAPH_DUPLICATE_DOC_ID`; two frontmatter-less files sharing a stem
  → **0** duplicate diagnostics (excluded per decision 2); a `related:` ref that resolves (explicit or
  stem) → 0.
- Files: `src/brain/graph.rs`. (Depends on Task 1.)

### 3. Public API + `--graph` CLI flag
- In `src/lib.rs`, add `pub fn validate_brain_graph(root: &Path) -> anyhow::Result<Report>` beside
  `validate_brain`: resolve `brain.toml` via `find_brain_config` (reuse the `E_CONFIG_NOT_FOUND`
  fallback), crawl once via `BrainValidator`, run the per-item OKF schema validation, then append
  `brain::graph::check_graph(&items)` into the same `Report`. Re-export `check_graph` (or expose only
  `validate_brain_graph`) consistent with the module's existing `pub use` style.
- In `src/main.rs`, add a `--graph` flag to the `ValidateBrain` subcommand. When set, dispatch to
  `mev::validate_brain_graph`; the existing `--json` / human and exit-code branches stay unchanged
  (a graph error makes `report.is_failure()` true → exit 1). Update the subcommand help text to
  mention the `--graph` corpus integrity check.
- Files: `src/lib.rs`, `src/main.rs`. (Depends on Task 2.)

### 4. Integration tests — end-to-end `--graph` over a fixture tree
- Add `tests/brain_graph.rs` that builds a temp HQ-root fixture (a `brain.toml` with full `[vocab]`
  so the schema pass is clean, plus a handful of OKF-clean `.md` docs with `doc_id` + `related`).
- Tests:
  - Clean corpus (all `related:` edges resolve, all doc_ids unique) → report with **0 errors**.
  - A doc whose `related:` names a renamed/deleted doc_id → **exactly one** `E_GRAPH_DANGLING_RELATED`
    for that referring file (re-adding the target, or fixing the ref, clears it).
  - Two docs declaring the same `doc_id` → **exactly one** `E_GRAPH_DUPLICATE_DOC_ID`.
  - (Optional, if cheap) a `--json` round-trip asserting a graph diagnostic appears in the envelope.
- Files: `tests/brain_graph.rs`. (Depends on Task 3.)

### 5. Validate
- Run the Validation Commands listed below and confirm all pass.

## Acceptance Criteria
- A corpus-wide effective-`doc_id` index is built over every crawled `.md` (explicit `doc_id` else
  filename stem).
- `mev validate-brain --graph` exists, runs the schema pass plus the graph check, and exits 1 when any
  graph error is present.
- A `related:` entry pointing at a renamed/deleted (undefined) `doc_id` is flagged
  `E_GRAPH_DANGLING_RELATED`; a `related:` entry that resolves is not flagged.
- Two OKF-frontmatter docs claiming the same `doc_id` are flagged `E_GRAPH_DUPLICATE_DOC_ID`;
  frontmatter-less files sharing a stem (e.g. `CLAUDE.md`) are **not** flagged.
- A clean corpus (edges resolve, doc_ids unique) passes with zero graph errors.
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
