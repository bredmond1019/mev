---
type: TaskSpec
title: MV.3.L — Structural coverage (index.md ↔ directory)
description: Task spec for the bidirectional index.md coverage check — orphan files and dangling rows (D17 / Standing Rule 7).
doc_id: 3L-structural-coverage-tasks
layer: [factory]
project: mev
status: active
keywords: [structural coverage, index.md, orphan detection, dangling row, D17, validate-brain]
related: [master-plan, status, D17-index-md-convention]
---

# Task Spec — Phase 3, Block L (MV.3.L)

**Status:** Not started · **Last run:** never

## Goal
Enforce CLAUDE.md Standing Rule 7 / D17: every corpus file in a directory appears in that directory's `index.md` (orphan detection), and every `index.md` row points at a file that exists (dangling-row detection) — bidirectional, surfaced via `mev validate-brain --structure`.

## Context Pointers
- **Plan section:** `planning/master-plan.md` → *MV.3.L — Structural coverage (`index.md` ↔ directory, D17)* (the What/Acceptance) and the Phase 3 wave table.
- **Governing decision:** company-brain `docs/decisions/D17-index-md-convention.md` — `index.md` (not `README.md`) is the directory-listing file across `planning/` and its subfolders; and CLAUDE.md Standing Rule 7 ("adding a file to a directory requires updating `index.md`").
- **Pattern to mirror:** `src/lib.rs::validate_brain_graph` / `validate_brain_links` (schema pass → crawl corpus once → run check → extend report) and the `--graph` / `--links` CLI flags in `src/main.rs` (`ValidateBrain` dispatch-precedence chain).
- **Reuse:** `src/brain/links.rs::extract_links` (index.md rows are markdown `[text](path)` links — do NOT write a new parser); `src/brain/crawl.rs` `Corpus` / `CorpusEntry` (fields `path`, `rel`, `stem`, `scope`) as the authoritative set of "files in scope" (already skip-pruned and ephemeral-filtered — this keeps orphan detection from flagging skip-dir/ephemeral files).
- **CLAUDE.md sections:** Standing rules 1 (tests ship with every change), 2 (OKF frontmatter + index.md row on new files), 4 (decisions append-only).

## Design notes (scope of the check)
- **Orphan detection is per-directory, direct children only.** For each directory that contains an `index.md` corpus member, gather the corpus entries whose parent directory *is* that directory (siblings of the `index.md`), excluding the `index.md` itself. Subdirectories are covered by their own `index.md`, so do not recurse for orphan purposes. A direct-child file is "covered" iff some markdown / `file://` link in the `index.md` resolves to its path. Uncovered → `E_STRUCT_ORPHAN_FILE` (located at the orphan file).
- **Dangling-row detection.** For each `index.md`, resolve every extracted markdown / `file://` link relative to the `index.md`'s directory. A link whose target lies inside the corpus root but does not exist on disk → `E_STRUCT_DANGLING_ROW` (located at the `index.md`). External links (`http(s)://`, `mailto:`, anchors) are already skipped by `extract_links`; `[[wikilink]]` targets are out of scope here (owned by MV.3.K). Links resolving *outside* the corpus root are ignored (not this check's job).
- **Both are errors** (`E_STRUCT_*`), consistent with `E_GRAPH_DANGLING_RELATED` / `E_LINK_*` — Rule 7 / D17 is mandatory. Any `E_STRUCT_*` makes `report.is_failure()` → exit 1.
- **Path normalization** must match the cross-platform discipline already in the crawl (compare using `rel` paths / `PathBuf` component comparison, not raw string equality) so a `./foo.md` vs `foo.md` vs a link with mixed separators all resolve to the same target.

## Step-by-Step Tasks

### 3.L.1 Structure-check module (`src/brain/structure.rs`)
- Create `src/brain/structure.rs`. Register it with `pub mod structure;` in `src/brain/mod.rs` (single additive line — no other edits to `mod.rs`).
- Implement `check_structure(corpus: &Corpus, root: &Path) -> Vec<Diagnostic>`:
  - Build a map of directory → its `index.md` `CorpusEntry` (match on `stem == "index"` / filename `index.md`).
  - Build the set of direct-child corpus entries per directory (parent-dir grouping over `corpus.entries`).
  - For each `index.md`: read the file, call `links::extract_links`, keep `Markdown` + `FileUri` kinds, resolve each `target` against the `index.md`'s directory into a normalized path.
    - **Dangling row:** resolved target inside `root` and not present on disk → `Diagnostic::error(index_path, "E_STRUCT_DANGLING_ROW", …)`.
    - **Orphan:** for each direct-child corpus entry (excluding the `index.md`) not present in the index's resolved-target set → `Diagnostic::error(child_path, "E_STRUCT_ORPHAN_FILE", …)`.
  - Directories with no `index.md` are skipped entirely (no coverage obligation → no orphan flags).
- Factor a small private path-resolution/normalization helper; add ≥6 unit tests in `#[cfg(test)]` covering: clean dir (no diagnostics), one orphan file, one dangling row, both together, a directory with no `index.md` (no flags), and `./`-prefixed / mixed-separator link normalization.
- **Owns:** `src/brain/structure.rs` (new), `src/brain/mod.rs` (append `pub mod structure;` only).

### 3.L.2 Library driver + `--structure` CLI flag (in progress)
- In `src/lib.rs`, add `pub fn validate_brain_structure(root: &Path) -> anyhow::Result<Report>` mirroring `validate_brain_graph`: resolve `brain.toml` via `find_brain_config` (same `E_CONFIG_NOT_FOUND` fallback), run the `BrainValidator` schema pass, crawl the corpus once, call `structure::check_structure`, and extend the report.
- In `src/main.rs`, add a `structure: bool` field (`#[arg(long)]`) to the `ValidateBrain` subcommand with a doc comment matching the existing `--graph`/`--links` style, and insert it into the dispatch precedence chain (place it adjacent to `--links`/`--state`; document the chosen precedence in the doc comment so it matches behaviour). Update the `ValidateBrain` top-level doc comment line listing the flags.
- **Owns:** `src/lib.rs`, `src/main.rs`.

### 3.L.3 Integration tests (`tests/brain_structure.rs`)
- Create `tests/brain_structure.rs` exercising the wired `validate-brain --structure` path end-to-end against temp-dir fixtures with a minimal `brain.toml`: (1) clean tree with a correct `index.md` → exit 0, no `E_STRUCT_*`; (2) an orphan corpus file not listed in its `index.md` → `E_STRUCT_ORPHAN_FILE`, exit 1; (3) an `index.md` row pointing at a deleted/nonexistent file → `E_STRUCT_DANGLING_ROW`, exit 1; (4) the `--json` envelope carries the `E_STRUCT_*` codes.
- Follow the fixture-construction style already used in `tests/brain_graph.rs` / `tests/brain_links.rs`.
- **Owns:** `tests/brain_structure.rs` (new).

### 3.L.4 Documentation
- `docs/cli.md`: document the `--structure` flag under `validate-brain` (purpose, dispatch precedence, the two `E_STRUCT_ORPHAN_FILE` / `E_STRUCT_DANGLING_ROW` diagnostic codes, exit behaviour) — match the `--links` section shape.
- `docs/architecture.md`: add the `structure.rs` module to the brain module map and its `check_structure` function/row to the function table; note `validate_brain_structure` as the lib driver.
- **Owns:** `docs/cli.md`, `docs/architecture.md`.

### 3.L.5 Validate
- Run the Validation Commands listed below and confirm all pass.
- Sanity: run `mev validate-brain --structure ..` against the live company brain and eyeball that findings are genuine (real orphan files / dangling rows), not false positives from scope/normalization bugs.

## Acceptance Criteria
- A corpus file present in a directory but not referenced by that directory's `index.md` is flagged with `E_STRUCT_ORPHAN_FILE` located at the orphan file.
- An `index.md` row (markdown or `file://` link) whose target does not exist on disk is flagged with `E_STRUCT_DANGLING_ROW` located at the `index.md`.
- A directory with no `index.md` produces no orphan diagnostics.
- `[[wikilink]]`, external (`http(s)://`), and out-of-corpus-root link targets produce no `E_STRUCT_*` diagnostics (owned elsewhere / out of scope).
- `mev validate-brain --structure <root>` exits 1 when any `E_STRUCT_*` is present and 0 on a clean tree; `--json` emits the codes in the envelope.
- All four harness gates pass (fmt, clippy `-D warnings`, `cargo test`, release build). New code ships with unit + integration tests.

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
