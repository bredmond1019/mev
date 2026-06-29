---
type: TaskSpec
title: Task Spec — Phase 3, Block J-crawl — multi-root corpus crawl + scope registry
description: Decomposed task spec for the registry-driven scope resolver and the canonical multi-root Brain corpus crawl — the shared foundation for OKF validation, the doc_id graph, and the embedder.
doc_id: 2j-corpus-crawl-tasks
layer: [factory, brain]
project: mev
status: archived
keywords: [corpus crawl, multi-root, scope registry, brain.toml, skip_dirs, single source of truth]
related: [master-plan, status, block-j-namespacing-decision, D4-corpus-engine-and-knowledge-graph, D29-mev-brain-validation-engine]
---

<!-- Archived 2026-06-29 — residue distilled into planning/knowledge.md, planning/memory.md -->

# Task Spec — Phase 3, Block J-crawl — Multi-root corpus crawl + scope registry

**Status:** Done · **Last run:** 2026-06-28 (PASS, all 5 tasks)

## Goal
Make mev a multi-root validator: a registry-driven `scope_for(path)` resolver plus a canonical corpus
crawl that walks every registered unit from the HQ root and yields exactly the Brain corpus
(`planning/**` + `docs/**` + root `README`/`CLAUDE`, minus bloat + ephemeral) — the single file-list
both OKF validation and the graph check consume.

## Context Pointers

- **Authoritative design:** `planning/2.J-graph-integrity/namespacing-and-corpus-decision.md`, esp. the
  **"Corpus rules"** section and the **2026-06-28 Update** (registry-driven stable slugs; mev as
  multi-root validator; root files carry OKF frontmatter; single corpus definition). Read it first.
- **Destination architecture:** `planning/decisions/D4-corpus-engine-and-knowledge-graph.md` — mev is the
  **single corpus engine** (one crawl → diagnostics + **manifest** + graph). This block's crawl is the
  shared walk that, in Phase 3B Block Q, also emits the manifest the embedder (`index_brain.py`) consumes
  instead of re-crawling. Honor the forward-compat constraint below (decision 6).
- **Plan:** `planning/master-plan.md` → Phase 3, **Block J-crawl** (+ Phase 3B Block Q manifest emit).
  Governed by brain **D29**.
- **Repo files that apply:**
  - `src/brain/config.rs` — `BrainConfig` / `RepoEntry` already carry `slug` + `repo_path`; this block
    treats each `[[repos]]` entry as a **scope unit** (the root entry `repo_path = "."` → slug `brain`).
  - `src/brain/crawl.rs` — `crawl_brain` + `MdFile`; the current single-root walk with nested-git pruning
    and the `CLAUDE.md` file blocklist are what this block replaces/changes.
  - `src/brain/mod.rs` — `BrainValidator::crawl` delegates here; rewire to the corpus crawl.
  - `src/lib.rs` — `validate_brain` (the crawl swap flows through it).
- **CLAUDE.md standing rules:** every behaviour change ships with tests; all four harness gates green;
  existing brain (sync) + learn-ai tests stay green.

### Scoping decisions made at authoring time (do not relitigate)

1. **Scope units come from the `brain.toml` registry** (`[[repos]]` entries: `slug` + `repo_path`),
   not inferred from tier/path position. `scope_for(rel)` = longest-prefix match of the file's path
   against the registry; the root unit (`repo_path = "."`, slug `brain`) is the fallback. Registering
   **tier sub-brains** as units (so `core/docs/...` → scope `core` rather than `brain`) is a brain-side
   `brain.toml` edit — **out of scope here**; mev reads whatever units the registry declares, and the
   fixtures include tier units to prove the resolver.
2. **Corpus membership rule:** a `.md` file is in the corpus iff, relative to its **owning unit**
   (longest-prefix), its path is under `planning/` or `docs/`, **or** it is the unit's root `README.md`
   or `CLAUDE.md`. Everything else (a unit's `src/*.md`, stray root-level `.md`, files under an
   unregistered nested dir) is excluded.
3. **Bloat + ephemeral exclusion:** prune `skip_dirs` (from `brain.toml` `[crawl].skip_dirs`, matched as
   **bare components at any depth** — `target`, `node_modules`, `.git`, `.claude`, `.agent(s)`,
   `.repo-backups`, `archive`, `archived`, `trees`, `sdlc`, `venv`, `.venv`); exclude ephemeral files
   `handoff.md` and any `_`-prefixed `.md`.
4. **Root files are included; OKF frontmatter on them is *optional*.** Remove `CLAUDE.md` from the file
   blocklist so it joins the corpus. A root file (`CLAUDE.md`/`README.md`) **without** frontmatter is a
   valid **leaf** — it must **not** raise the OKF "missing frontmatter" error; one **with** a `doc_id`
   is treated as a normal node. `handoff.md` stays blocklisted. (No frontmatter backfill is required;
   this matches HQ CLAUDE.md Standing Rule 6, which exempts root `README`/`CLAUDE`.)
5. **OKF-exemption mechanism.** The OKF validator must skip the "missing/!frontmatter" error for
   root instruction files (`README.md`, `CLAUDE.md`) — they are corpus members but not required to be
   OKF docs. All other corpus files keep the existing required-frontmatter behaviour.
6. **D4 forward-compat — the crawl yields a clean owned corpus result.** `crawl_corpus` must return a
   first-class, **owned** corpus structure (not state buried inside a validation pass), with each entry
   carrying its computed `scope` (resolved once, here — the graph block and the manifest both need it).
   Derive `serde::Serialize` on that structure so Phase 3B Block Q can emit it as the manifest with no
   re-crawl, and so "what's validated == what's embedded" holds by construction. Keep diagnostics a
   **separate** return value from the corpus data. Do *not* build the manifest emitter here — only ensure
   the crawl produces the consumable, serializable result.

## Step-by-Step Tasks

### 1. Scope-unit registry + `scope_for` resolver
- Create `src/brain/scope.rs` and register `pub mod scope;` in `src/brain/mod.rs`.
- Add `scope_units(config) -> Vec<(slug, repo_path)>` (every `[[repos]]` entry) and
  `scope_for(rel: &Path, config: &BrainConfig) -> String`: longest-prefix match of `rel` against unit
  `repo_path`s (excluding `"."` from prefix comparison); on no match, return the root unit's slug
  (`repo_path == "."`, i.e. `brain`). Also expose `owning_unit(rel, config) -> (slug, repo_path)` for
  the crawl's membership test.
- Unit tests: `core/mev/planning/x.md` → `mev`; `core/docs/x.md` → `core` (when a `core` unit is
  registered); `planning/x.md` and `README.md` → `brain`; longest-prefix wins (mev over core);
  stability — the same file keyed by slug regardless of a simulated tier rename in the fixture.
- Files: `src/brain/scope.rs`, `src/brain/mod.rs`.

### 2. Canonical corpus crawl (owned, serializable result)
- In `src/brain/crawl.rs`, define an owned corpus result per **decision 6**: a `CorpusEntry`
  (`path`, `rel`, `stem`, `scope: String`) and a `Corpus { entries: Vec<CorpusEntry> }` (or equivalent),
  both `#[derive(serde::Serialize)]` — this is the manifest seed Phase 3B Block Q will emit.
- Add `crawl_corpus(root, config) -> (Corpus, Vec<Diagnostic>)`: walk `root` once, pruning `skip_dirs`
  (bare-component match at any depth — the existing `is_blocklisted_name` name-mode already does this) so
  bloat subtrees never descend. For each `.md` file, compute its owning unit + `scope` (Task 1) and keep
  it only if its path relative to that unit is under `planning/` or `docs/`, or equals
  `README.md`/`CLAUDE.md`; drop ephemeral (`handoff.md`, `_`-prefixed). Each kept file becomes a
  `CorpusEntry` carrying its resolved `scope`. Diagnostics are returned **separately** from `Corpus`.
  Remove `CLAUDE.md` from `is_blocklisted_file`; keep `handoff.md`.
- Drop the nested-git pruning for the corpus crawl (unit-ownership now bounds membership); keep
  `crawl_brain` if still referenced, or migrate callers — no dead code, clippy clean.
- Unit/inline tests for the membership helper (under-planning / under-docs / root-file → in;
  `src/x.md` / stray root `.md` / unregistered-dir file → out) and for `scope` being populated per entry;
  a `serde_json::to_string(&corpus)` round-trip proves the result is serializable.
- Files: `src/brain/crawl.rs`.

### 3. Wire the corpus crawl into `BrainValidator` + exempt root files from OKF
- In `src/brain/mod.rs`, change `BrainValidator::crawl` to call `crawl_corpus(root, &self.config)` and
  feed its `Corpus` entries into the validation pass (map each `CorpusEntry` to the trait's `Item`, or
  validate entries directly — keep the owned `Corpus` intact so callers that want the manifest can reuse
  it). Confirm `validate_brain` / `validate_brain_sync` (in `src/lib.rs`) still compile and behave; update
  any doc comments naming the old single-root walk.
- **OKF exemption for root files:** in the OKF validation path (`src/brain/okf.rs::validate_md_file` or
  the `validate_item` dispatch), when the file is a root instruction file (`README.md`/`CLAUDE.md`,
  detected by leaf name) and has **no** frontmatter, return **no** diagnostic (it is a leaf, not a
  required OKF doc) instead of the usual "missing frontmatter" error. A root file that *does* carry
  frontmatter is validated normally. Unit tests cover both branches.
- Files: `src/brain/mod.rs`, `src/lib.rs`, `src/brain/okf.rs`.

### 4. Integration tests — multi-root corpus over a fixture tree (in progress)
- Add `tests/brain_corpus.rs` building a temp HQ-root fixture: a `brain.toml` registering `brain`
  (`.`), a tier unit (`core`), and a repo unit (`mev` → `core/mev`); files placed across each unit's
  `planning/`, `docs/`, root `README.md`/`CLAUDE.md`, plus negative cases — a `sdlc/` file, an
  `archive/` file, a `trees/` file, a `handoff.md`, a `core/mev/src/notes.md`, and a file under an
  unregistered nested dir.
- Assert the crawl includes exactly the corpus files with correct owning scope and excludes every
  negative case; assert `CLAUDE.md`/`README.md` are included.
- Files: `tests/brain_corpus.rs`.

### 5. [x] Validate
- Run the Validation Commands listed below and confirm all pass.

## Acceptance Criteria
- `scope_for` resolves a file to its owning unit's stable slug via longest-prefix over the `brain.toml`
  registry; the root unit is the fallback (`brain`); a simulated tier/path rename in a fixture does not
  change a file's scope when keyed by slug.
- `crawl_corpus` returns, for every registered unit, that unit's `planning/**` + `docs/**` + root
  `README.md`/`CLAUDE.md`, and nothing else.
- Bloat dirs (`sdlc`, `archive`, `archived`, `trees`, `target`, `node_modules`, `.git`, `.venv`, …),
  ephemeral files (`handoff.md`, `_`-prefixed), unit `src/*.md`, stray root `.md`, and files under
  unregistered nested dirs are all excluded.
- `CLAUDE.md` is included in the corpus (no longer file-blocklisted); `handoff.md` remains excluded.
- A root `README.md`/`CLAUDE.md` **without** frontmatter produces **no** OKF "missing frontmatter"
  error (treated as a leaf); one **with** frontmatter is validated normally.
- `crawl_corpus` returns an **owned, `Serialize`-able** `Corpus` (each entry carries its resolved
  `scope`), separate from the diagnostics, and `serde_json` serializes it cleanly (D4 manifest seed).
- `BrainValidator`/`validate-brain` use the corpus crawl; existing brain + learn-ai tests stay green.
- All four harness gates pass (`fmt`, `clippy -D warnings`, `test`, `build`).

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes
- Amended 2026-06-28 for **D4**: `crawl_corpus` returns an owned, `Serialize`-able `Corpus` (entries carry
  `scope`), separate from diagnostics — the manifest seed Phase 3B Block Q emits for the embedder.

## Amendment Log
<!-- Append-only. Pipeline stages append one dated line here when they deviate from the spec. -->
- 2026-06-28 [task 2] `rel_to_unit_root` returns `Option<&Path>` (None on prefix mismatch) rather than panicking — unexpected mismatches surface as a diagnostic and are skipped; spec implied a simpler string-based path model.
- 2026-06-28 [task 3] Integration tests in `brain_okf.rs` and `brain_validate.rs` updated to place files under `planning/` so they are corpus members; the new corpus-membership rule excludes root-level stray `.md` files, which prior tests assumed were valid validation targets.
- 2026-06-28 [task 3] `BrainValidator::crawl` kept `Item=MdFile` and maps `CorpusEntry→MdFile` rather than changing the trait `Item` type — avoids touching the `ContentValidator` trait definition and `validate_item` signatures across all callers; not specified in the original task.
