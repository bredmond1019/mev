---
type: Plan
title: "MV.3.K — Link integrity task spec"
description: Decomposed task spec for the link-integrity validator — markdown / file:// / [[wikilink]] resolution plus .brain-moves-pending re-check.
doc_id: 3-K-link-integrity-tasks
layer: [factory, brain]
project: mev
status: active
keywords: [link integrity, dead links, wikilinks, brain-moves-pending, validate-brain, Phase 3]
related: [master-plan, status, 3-P-state-integrity-tasks]
---

# Task Spec — Phase 3, Block K (Link integrity)

**Status:** Passed (6/6 tasks) · **Last run:** 2026-06-30 09:15 UTC

## Goal
Add `mev validate-brain --links`: flag markdown `[text](path)`, `file://`, and `[[wikilink]]` references that do not resolve to an existing file / known `doc_id`, and consume `.brain-moves-pending` to surface references still pointing at moved/deleted paths.

## Context Pointers
- **Plan section:** `planning/master-plan.md` → Phase 3 → **MV.3.K — Link integrity**. Governed by brain decision **D29** (mev is the single validation engine; read-only diagnostics, never mutates the corpus — D25).
- **Sibling implementations to mirror (same shape):**
  - `src/brain/graph.rs` — serializable model (`Graph`/`Node`/`Edge`), `build_*` + `check_*` split over the corpus, `read_doc_metadata` D5 seam, `E_GRAPH_*` locator vocabulary, dense `#[cfg(test)]` unit tests. **This is the closest analogue — link integrity is the on-disk/wikilink complement of `related:`-edge integrity.**
  - `src/brain/state.rs` — second example of the build/check split + a new diagnostic-code family.
- **Corpus crawl (input):** `src/brain/crawl.rs` — `crawl_corpus(root, &config) -> (Corpus, Vec<Diagnostic>)`; `CorpusEntry { path, rel, stem, scope }`. The crawl already prunes `skip_dirs`/nested-git; reuse it verbatim — do **not** add a new walk.
- **API + CLI wiring pattern:** `src/lib.rs` → `validate_brain_graph` / `validate_brain_state` (schema pass via `BrainValidator`, then append the new pass's diagnostics); `src/main.rs` → the `--graph` / `--state` flags on `ValidateBrain` (mutually-exclusive `else if` ladder, exit-code via `report.is_failure()`).
- **`.brain-moves-pending` source + format:** company-brain `hooks/README.md` §`post-commit`. The file lives at the **brain repo root**, is gitignored/ephemeral, and each line is `<ISO-date> <rel-path1> [rel-path2 ...]` — space-separated repo-relative paths deleted/renamed in one commit. `hooks/README.md` explicitly names this block ("future Block K") as the consumer.
- **Doc_id index for wikilinks:** the set of authored `doc_id`s already comes out of `build_graph` (`GraphArtifact.node_map` keys are `scope:doc_id`). Reuse `read_doc_metadata` / the graph node set rather than re-parsing frontmatter.
- **Standing rules (`CLAUDE.md`):** every new fn/module ships with tests (rule 1); OKF frontmatter + `index.md` row on every new `.md` under `docs/`/`planning/` (rule 2); decisions are append-only (rule 4). Harness gates in `planning/harness.json`.

## Step-by-Step Tasks

### 1. Link model + extractor (`src/brain/links.rs`)
- Create `src/brain/links.rs` and register it in `src/brain/mod.rs` (`pub mod links;` — append-only, beside the existing `graph`/`state` module declarations).
- Define a serializable link model (mirror the graph model's `serde::Serialize` derives for D4 consistency, even though K emits no artifact today):
  - `enum LinkKind { Markdown, FileUri, WikiLink }` (`#[serde(rename_all = "snake_case")]`).
  - `struct LinkRef { kind: LinkKind, raw: String, target: String }` — `raw` is the as-authored reference; `target` is the path/slug portion with any `#anchor` suffix stripped (anchors are out of scope for K).
- Implement `extract_links(contents: &str) -> Vec<LinkRef>` over a file **body**:
  - Markdown inline links `[text](target)` → `LinkKind::Markdown`.
  - `file://` / `file:///…` URIs (in markdown link targets or bare in prose) → `LinkKind::FileUri`.
  - `[[wikilink]]` → `LinkKind::WikiLink` (`target` = the inner slug).
  - **Skip** external/non-local references: `http://`, `https://`, `mailto:`, `tel:`, protocol-relative `//`, and pure in-page anchors (`#section`). These produce no `LinkRef`.
- Unit tests (`#[cfg(test)] mod tests`) covering: a relative markdown link, a `file://` link, a `[[wikilink]]`, an `http(s)` link skipped, a pure-anchor link skipped, and a `path#anchor` link whose `target` has the anchor stripped.
- **Owns:** `src/brain/links.rs` (new), `src/brain/mod.rs` (one-line module decl, append-only).

### 2. Resolution + integrity checks (`check_links`)
- In `src/brain/links.rs`, add `check_links(corpus: &Corpus, root: &Path, doc_ids: &HashSet<String>) -> Vec<Diagnostic>`:
  - For each `CorpusEntry`, read its contents (graceful degrade on I/O error — skip the file, mirroring `read_doc_metadata`), `extract_links`, and resolve each `LinkRef`:
    - **`Markdown`** (relative/local): resolve `target` relative to the referring file's **directory** (`entry.path.parent()`); if the resolved path does not exist on disk → `E_LINK_DEAD_MARKDOWN` (error), located at `entry.rel` with the raw target in the message.
    - **`FileUri`**: strip the `file://`/`file:///` scheme to an absolute path; if it does not exist on disk → `E_LINK_DEAD_FILE_URI` (error).
    - **`WikiLink`**: if `target` (a bare slug) is not present in `doc_ids` → `E_LINK_DANGLING_WIKILINK` (error). (Wikilinks resolve to a **bare** known `doc_id`, matching memory-doc `[[name]]` usage — they are scope-agnostic.)
- Build the `doc_ids: HashSet<String>` of authored bare `doc_id`s from the corpus — reuse `crate::brain::graph::read_doc_metadata` (or `build_graph`'s node set) rather than re-parsing frontmatter (D5 single-seam discipline). Add a small helper for this (e.g. `collect_doc_ids(corpus) -> HashSet<String>`).
- Unit tests: a dead relative markdown link is flagged; a live one passes; a dead `file://` path is flagged; a live `file://` passes; a `[[wikilink]]` to an unknown slug is flagged; a `[[wikilink]]` to a real `doc_id` passes; external/anchor links never flagged.
- **Owns:** `src/brain/links.rs` (extends task 1; sequential within the flow).

### 3. `.brain-moves-pending` moved-reference re-check
- In `src/brain/links.rs`, add `read_moves_pending(root: &Path) -> Vec<String>` (the set of moved/deleted repo-relative paths) and `check_moved_references(corpus, root, moved_paths) -> Vec<Diagnostic>`:
  - Parse `<root>/.brain-moves-pending`: each line is `<ISO-date> <path...>`; collect every path token after the leading date. Missing file → empty set, no diagnostics (the hook is optional/ephemeral).
  - For each moved path, scan the corpus for any markdown/`file://` reference that still resolves to that path and emit `E_LINK_MOVED_REFERENCE` (error) at the referring file, naming the moved target. This is the **targeted re-check** the acceptance criterion calls for.
- Unit tests: a `.brain-moves-pending` entry plus a doc still linking the moved path → flagged; no `.brain-moves-pending` file → no diagnostics; a moved path with no remaining references → no diagnostics.
- **Owns:** `src/brain/links.rs` (extends tasks 1–2).

### 4. Public API + `--links` CLI flag + integration tests
- `src/lib.rs`: add `pub fn validate_brain_links(root: &Path) -> anyhow::Result<Report>` mirroring `validate_brain_graph` — resolve `brain.toml` via `find_brain_config` (same `E_CONFIG_NOT_FOUND` fallback), run the `BrainValidator` schema pass, then `crawl_corpus` once and append `check_links` + `check_moved_references` diagnostics. Re-export the public link surface (`pub use brain::links::{...}`) alongside the existing `build_graph`/`check_graph` re-exports.
- `src/main.rs`: add a `--links` bool flag to the `ValidateBrain` subcommand (doc-comment in the same style as `--graph`/`--state`); extend the mutually-exclusive `else if` dispatch ladder so `--links` calls `validate_brain_links`. Update the subcommand `about`/doc text to mention `--links`.
- `tests/brain_links.rs` (new): end-to-end integration tests over a temp brain fixture (mirror `tests/brain_graph.rs` setup) — dead markdown link flagged; dead `file://` flagged; dangling wikilink flagged; a `.brain-moves-pending` entry drives an `E_LINK_MOVED_REFERENCE`; a clean corpus passes; `--json` envelope is well-formed.
- Run `cargo run -- validate-brain --links ~/Dev/agentic-portfolio` against the **live** brain and record the result in Notes (clean, or a real-findings list to triage).
- **Owns:** `src/lib.rs`, `src/main.rs`, `tests/brain_links.rs` (new).

### 5. Documentation
- `docs/cli.md`: document the `--links` flag (purpose, exit-code behaviour) and add the four new diagnostic codes (`E_LINK_DEAD_MARKDOWN`, `E_LINK_DEAD_FILE_URI`, `E_LINK_DANGLING_WIKILINK`, `E_LINK_MOVED_REFERENCE`) to the diagnostics reference.
- `docs/architecture.md`: add `src/brain/links.rs` to the module map with a one-line role.
- Update `docs/index.md` only if a row's scope changed (per CLAUDE.md rule 2 / brain Standing Rule 7); no new doc file is created, so no new `index.md` row is required.
- **Owns:** `docs/cli.md`, `docs/architecture.md` (+ `docs/index.md` only if scope shifts).

### 6. Validate
- Run the Validation Commands listed below and confirm all pass.

## Acceptance Criteria
- `mev validate-brain --links <brain-root>` runs the OKF schema pass **and** the link-integrity pass, emitting `E_LINK_*` diagnostics; any error-severity diagnostic produces exit code 1.
- A markdown `[text](path)` link to a **moved or deleted** file is flagged (`E_LINK_DEAD_MARKDOWN`); a live one is not.
- A `file://` link to a nonexistent path is flagged (`E_LINK_DEAD_FILE_URI`); a live one is not.
- A `[[wikilink]]` to an unknown slug is flagged (`E_LINK_DANGLING_WIKILINK`); one matching a real authored `doc_id` is not.
- A `.brain-moves-pending` entry drives a targeted re-check that flags every reference still pointing at the moved/deleted path (`E_LINK_MOVED_REFERENCE`); a missing `.brain-moves-pending` file produces no diagnostics.
- External links (`http(s)`, `mailto:`, `tel:`, protocol-relative) and pure in-page anchors (`#…`) are never flagged.
- The pass is read-only — it mutates nothing in the corpus (D25); `--json` emits a well-formed envelope.
- All harness gates (`fmt`, `clippy -D warnings`, `cargo test`, release build) are green; the full pre-existing test suite still passes.

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
- 2026-06-30 [task 4] UTF-8 boundary panic fix: `extract_links()` advanced `i += 1` byte-by-byte, which can step into the middle of a multi-byte UTF-8 sequence and cause `starts_with()` to panic on real brain content. Fix gates the `file://` check on `bytes[i] == b'f'` and advances `i` by the char width derived from the leading byte. Not in the original spec — discovered during the live brain run.
