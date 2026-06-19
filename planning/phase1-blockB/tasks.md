# Task Spec — Phase 1, Block B — Crawl & classify

> **Note on the slug.** Invoked as `phase0 blockB`, but the master plan defines **Block B
> ("Crawl & classify") under Phase 1**, and `status.md`'s current focus is "Phase 1, Block B".
> Normalized to `phase1-blockB` accordingly.

## Goal
`walkdir` the content root, classify each file as `learn-module-json`, `path-metadata-json`, or
`module-mdx`, build a `Corpus` grouped by path-id / module-id, and surface filename-convention
violations as diagnostics.

## Context Pointers
- **Plan:** `planning/master-plan.md` → *Phase 1 → Block B — Crawl & classify*. Also the Phase 1
  preamble: the bar is a *superset of* `learn-ai/scripts/validate-content.ts` (D2); the universal
  currency is the `Diagnostic` (`error` → exit 1, `warning` → exit 0); only the reporter prints.
- **Decisions:** `planning/decisions/D2-scope-and-sequence.md`.
- **Repo files:** `src/lib.rs` (existing `Diagnostic` / `Severity` / `Report` / `validate()` stub —
  extend, do not rewrite), `src/main.rs` (CLI dispatch; reporter stays a stub until Block E),
  `tests/smoke.rs` (existing test style — temp dirs + fixtures).
- **CLAUDE.md standing rules:** every block ships with tests (rule 1); all four harness gates must
  stay green (`fmt`, `clippy -D warnings`, `test`, `build`).
- **Dependencies:** `walkdir = "2.5.0"` is already in `Cargo.toml` — no new deps expected.

## Step-by-Step Tasks

### 1. Define classification + corpus types
- Add a `crawl` module (`src/crawl.rs`, re-exported from `lib.rs`).
- Define `FileKind` enum: `LearnModuleJson`, `PathMetadataJson`, `ModuleMdx` (plus an internal
  `Unknown`/skip path for non-content files so the walk does not choke on README/schema files).
- Define `ContentFile { path: PathBuf, kind: FileKind, path_id: String, module_id: Option<String> }`
  — `path_id` is the immediate content-section directory (the path slug); `module_id` is the
  module file stem (`None` for `path-metadata-json`).
- Define `Corpus` grouping `ContentFile`s by `path_id`, then by `module_id`, with accessors the
  later blocks (C/D/E) will consume (e.g. iterate modules, look up a module's json/mdx pair).
- Keep the types `pub` so integration tests in `tests/` can construct/inspect them.

### 2. Classify files during the walk
- Implement classification from filename + location:
  - `metadata.json` at a path-section root → `PathMetadataJson`.
  - `^\d{2}-[a-z0-9-]+\.json$` → `LearnModuleJson`.
  - `^\d{2}-[a-z0-9-]+\.mdx$` → `ModuleMdx`.
  - Anything else (e.g. `schemas/`, `README.md`, dotfiles) → skipped, not an error.
- Decide and document (in code comments) how `path_id` / `module_id` are derived from the relative
  path so Block D can pair `.json` ↔ `.mdx` by `(path_id, module_id)`.

### 3. Port the filename-convention checks (`validateFileName`)
- Emit `Diagnostic`s for content files that violate the conventions:
  - no spaces in the filename;
  - lowercase filename;
  - module files (json/mdx) must match `^\d{2}-[a-z0-9-]+\.(json|mdx)$`.
- A filename violation still gets classified/added to the corpus where possible (so downstream
  checks still see the file) — the violation is reported, not fatal to enumeration.
- Match severity to the TS validator's behavior (filename issues are errors).

### 4. Build the corpus and wire into `validate()`
- Replace the Phase 0 stub body of `validate(root)`: crawl `root`, classify, accumulate filename
  diagnostics into the `Report`, and build the `Corpus`.
- Return the `Corpus` alongside the `Report` (e.g. change the return to carry both, or expose a
  `crawl(root) -> (Corpus, Vec<Diagnostic>)` that `validate` calls) so Blocks C–E can consume it.
  Keep `validate()`'s public contract (returns a `Report` driving the exit code) intact for `main.rs`.
- Handle IO/permission errors from `walkdir` gracefully (surface as a diagnostic or `anyhow` error;
  do not panic).

### 5. Tests against fixtures
- Add `tests/crawl.rs` (or extend the existing test module) with a small fixture tree built in a
  temp dir (good modules: `01-intro.json` + `01-intro.mdx`, a `metadata.json`; broken: a file with
  spaces, an uppercase name, a module missing the `NN-` prefix).
- Assert: corpus enumerates every content file with the correct `FileKind` and grouping; each
  filename violation produces exactly the expected diagnostic; non-content files (`README.md`,
  `schemas/…`) are skipped; an empty tree still produces a clean report (keep `smoke.rs` green).

### 6. Validate
- Run the Validation Commands listed below and confirm all pass.
- Optionally run `cargo run -- validate ../learn-ai/content/learn` if the sibling checkout exists,
  to sanity-check the corpus enumerates the live tree (filename diagnostics only at this block).

## Acceptance Criteria
- `Corpus` enumerates the live content tree, grouping files by `path_id` and `module_id` with the
  correct `FileKind` for each (`learn-module-json` / `path-metadata-json` / `module-mdx`).
- Non-content files (schemas, READMEs, dotfiles) are skipped without error.
- Every filename-convention violation (spaces, uppercase, missing `NN-` prefix / wrong pattern)
  surfaces as a `Diagnostic` with the correct severity and file locator.
- New fixture-driven tests cover good + deliberately-broken filenames and pass.
- All four harness gates are green; the existing `smoke.rs` tests still pass.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes
<!-- filled in as work happens -->
