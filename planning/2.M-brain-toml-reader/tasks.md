---
type: Plan
title: Task Spec — HQ-R Block M, mev reads brain.toml
description: Task breakdown for wiring mev's validate-brain to read vocab, crawl rules, and project slugs from brain.toml, retiring all hardcoded is_valid_* match arms.
doc_id: 2.M-brain-toml-reader-tasks
layer: [factory]
project: mev
status: active
keywords: [brain.toml, config, vocab, crawl, toml crate, walk-up, is_valid]
related: [D3-corpus-config-system, status, master-plan]
---

# Task Spec — HQ-R Block M: mev reads `brain.toml`

**Status:** Not started · **Last run:** never

## Goal

Add the `toml` crate and have `validate-brain` resolve and read `brain.toml` (via walk-up from the corpus root) for `[vocab]` layer/status lists, `[crawl].skip_dirs`, and `[[repos]]`-derived project slugs — retiring all hardcoded `is_valid_*` match arms and skip-list entries, and marking mev D3 superseded.

## Context Pointers

- **Block definition:** HQ Restructure master plan `planning/hq-restructure/master-plan.md` → Phase 4, Block M
- **D3 (to supersede):** `planning/decisions/D3-corpus-config-system.md` — the `.mev.toml` idea, now realized via `brain.toml`
- **`brain.toml`:** `agentic-portfolio/brain.toml` (two levels above this repo's root: `../../brain.toml`)
- **Files to retire hardcodes from:**
  - `src/brain/crawl.rs` — `is_blocklisted_name` skip-list
  - `src/brain/okf.rs` — `is_valid_layer`, `is_valid_status`, `is_valid_project` match arms
  - `src/brain/mod.rs` — `BrainValidator` wiring
  - `src/lib.rs` + `src/main.rs` — public API + CLI
- **New files:** `src/brain/config.rs`, `tests/brain_config.rs`, `tests/fixtures/brain.toml`
- **Harness gates:** `cargo fmt --check && cargo clippy -- -D warnings && cargo test && cargo build --release`
- **Standing rule:** every block ships with tests; OKF frontmatter on all new `.md` files

## Step-by-Step Tasks

### 1. Add `toml` crate + `BrainConfig` struct + walk-up resolver

**Files:** `Cargo.toml` (modified), `src/brain/config.rs` (new), `tests/fixtures/brain.toml` (new), `tests/brain_config.rs` (new)

- Add `toml = "0.8"` (or latest) to `[dependencies]` in `Cargo.toml`.
- Create `src/brain/config.rs` with:
  - `VocabConfig { layer: Vec<String>, status: Vec<String> }` — parsed from `[vocab]`
  - `CrawlConfig { skip_dirs: Vec<String> }` — parsed from `[crawl]`
  - `RepoEntry { slug: String, tier: String, repo_path: String, status_file: String, cache_doc: String, heading: String }` — parsed from `[[repos]]`
  - `BrainConfig { vocab: VocabConfig, crawl: CrawlConfig, repos: Vec<RepoEntry> }` — top-level
  - `impl BrainConfig { pub fn projects(&self) -> Vec<&str> }` — derives project vocab as the set of `[[repos]]` slugs
  - `pub fn find_brain_config(start: &Path) -> Result<BrainConfig, ConfigError>` — walks up from `start`, looking for `brain.toml`; returns a typed error if not found rather than falling back to hardcodes
  - `pub fn load_brain_config(path: &Path) -> Result<BrainConfig, ConfigError>` — parse from a given path (used by tests and by `find_brain_config`)
  - Re-export `BrainConfig` from `src/brain/mod.rs` (`pub use config::BrainConfig;`)
- Create `tests/fixtures/brain.toml` — minimal fixture with at least one `[vocab]` layer/status set, one `[crawl].skip_dirs` entry, and two `[[repos]]` blocks (one with slug `brain`, one with slug `mev`). Must parse to a valid `BrainConfig`.
- Create `tests/brain_config.rs` with:
  - Test: loading the fixture parses `VocabConfig.layer`, `VocabConfig.status`, `CrawlConfig.skip_dirs`, and two `RepoEntry`s
  - Test: `BrainConfig::projects()` returns the repo slugs
  - Test: `find_brain_config` resolves by walking up from a subdirectory of the fixture directory
  - Test: `find_brain_config` from a path with no ancestor `brain.toml` returns `Err`
  - All harness gates must pass after this task alone.

### 2. Config-driven crawl skip-list

**Files:** `src/brain/crawl.rs` (modified), `src/brain/mod.rs` (modified — struct field only)

**dependsOn:** Task 1

- In `src/brain/mod.rs`: add `config: BrainConfig` field to `BrainValidator`; update its constructor (likely `new` or `with_config`) to accept `BrainConfig`; keep `BrainValidator::run()` compiling (temporarily pass config to crawl, OKF wiring comes in Task 3).
- In `src/brain/crawl.rs`:
  - Change `crawl_brain(root: &Path)` to `crawl_brain(root: &Path, skip_dirs: &[String]) -> Vec<MdFile>` (or take `&BrainConfig`).
  - Replace the `is_blocklisted_name` hardcoded list with a lookup against the provided `skip_dirs` slice.
  - The file-level blocklist (`is_blocklisted_file` for `CLAUDE.md`, `handoff.md`, etc.) is a separate concern — keep it as-is for now (it is not covered by `brain.toml`'s `skip_dirs`; mark with `// TODO(D3): extend config for file-level blocklist` if desired).
  - Add `// TODO(D3): superseded by brain.toml` comment removal — just delete any existing `// TODO(D3)` markers on skip-list lines.
- Update existing `tests/brain_crawl.rs` tests that call `crawl_brain` to pass a skip_dirs slice (can use a hard-coded slice in tests, or a minimal `BrainConfig` from the fixture).
- All harness gates must pass after this task alone.

### 3. [~] Config-driven vocab validation

**Files:** `src/brain/okf.rs` (modified), `src/brain/mod.rs` (modified — `run()` wiring)

**dependsOn:** Task 2

- In `src/brain/okf.rs`:
  - Change `is_valid_layer(s: &str) -> bool`, `is_valid_status(s: &str) -> bool`, `is_valid_project(s: &str) -> bool` to each accept a `&BrainConfig` parameter (or a slice ref): e.g. `is_valid_layer(s: &str, config: &BrainConfig) -> bool`.
  - Replace the literal match arms / `.contains()` against hardcoded vecs with lookups against `config.vocab.layer`, `config.vocab.status`, and `config.projects()` respectively.
  - Change `validate_md_file(path, rel, content) -> Vec<Diagnostic>` to `validate_md_file(path, rel, content, config: &BrainConfig) -> Vec<Diagnostic>` and propagate the config down to the three `is_valid_*` call sites.
  - Delete all hardcoded vocabulary lists — no `["brain", "engine", ...]` literal in Rust source after this task.
- In `src/brain/mod.rs`:
  - Update `BrainValidator::run()` to call `validate_md_file(..., &self.config)` instead of the old signature.
- Update `tests/brain_okf.rs` to pass a `BrainConfig` (loaded from `tests/fixtures/brain.toml` or constructed inline with `BrainConfig { vocab: VocabConfig { layer: vec![...], ... }, ... }`) everywhere `is_valid_*` or `validate_md_file` is called.
- All harness gates must pass after this task alone.

### 4. Thread config through public API + CLI; integration-test the config-flip criterion

**Files:** `src/lib.rs` (modified), `src/main.rs` (modified), `tests/brain_validate.rs` (modified)

**dependsOn:** Task 3

- In `src/lib.rs`:
  - Update `validate_brain(root: &Path) -> Report` to resolve `BrainConfig` via `find_brain_config(root)` before constructing `BrainValidator`.
  - If `find_brain_config` returns `Err` (no `brain.toml` found by walk-up), surface it as a fatal `Diagnostic` in the returned `Report` (severity `Error`, code `E_CONFIG_NOT_FOUND` or similar) rather than panicking.
  - Construct `BrainValidator::new(config)` (or however the constructor was written in Task 2).
- In `src/main.rs`:
  - No changes required to the CLI surface — `validate-brain <root>` already calls `validate_brain(root)`. Verify it still compiles and wires correctly.
- In `tests/brain_validate.rs`:
  - Add an integration test that:
    1. Writes a temp directory with a minimal `brain.toml` containing a custom `layer` value (e.g. `["custom-layer"]`).
    2. Places a `.md` file in that directory with `layer: [custom-layer]` in its OKF frontmatter.
    3. Calls `validate_brain()` and asserts no `Error`-severity layer diagnostics.
    4. Then **removes** `custom-layer` from `brain.toml` (config-only edit, no source change) and calls `validate_brain()` again — asserts an `Error`-severity layer diagnostic appears.
    This test is the direct evidence for the acceptance criterion "config-only change flipping a result".
- All harness gates must pass after this task alone.

### 5. Mark D3 superseded (additive doc edit)

**Files:** `planning/decisions/D3-corpus-config-system.md` (additive append)

**dependsOn:** none (independent, additive)

- Append a `## Superseded` section at the end of `planning/decisions/D3-corpus-config-system.md` with frontmatter `status: superseded` change (update the `status` field in the YAML header from `draft` to `superseded`) and a body note:

  ```
  ## Superseded

  **Superseded by `brain.toml` — HQ Restructure Block M (2026-06-27).**

  D3's `.mev.toml` proposal is retired. The corpus-level config is instead the shared
  `brain.toml` at the HQ root, consumed directly by both `mev` (this block) and
  `index_brain.py` (HQ-R Block I). The walk-up resolution and the vocab/crawl/manifest
  surface are as specified in D3's "Decision" section; only the filename and the
  "each consumer carries its own" model differ. See the HQ Restructure master plan, Block M.
  ```

- Do NOT edit any other content in D3 (decisions are append-only).
- This task is safe to run in parallel with Tasks 1–4 since it touches only a planning doc.

### 6. Validate

- Run the Validation Commands listed below and confirm all pass.
- Manually run `mev validate-brain ~/Dev/agentic-portfolio` and confirm it exits 0 with the same or fewer diagnostics as before this block (the Brain corpus still validates to 0 errors, ≤3 warnings).
- Confirm `grep -r 'is_valid_layer\|is_valid_status\|is_valid_project' src/` returns only the function definitions (no call sites passing hardcoded strings).
- Confirm `grep -r '\["brain"' src/ | grep -v test` returns nothing (no literal vocab arrays in non-test source).

## Acceptance Criteria

- `mev validate-brain` resolves `brain.toml` by walk-up from the corpus root and uses it for all vocab + skip-dir decisions — no hardcoded vocab lists remain in `src/`.
- A config-only edit to `brain.toml` (adding or removing a vocab value) flips a validation result without any Rust source change — proven by the integration test in Task 4.
- `is_valid_layer`, `is_valid_status`, `is_valid_project` contain no literal string arrays; they delegate to `BrainConfig`.
- The hardcoded skip-list in `crawl.rs` (`is_blocklisted_name`) is driven by `config.crawl.skip_dirs`.
- `find_brain_config` walks up from the given root and returns a typed error if `brain.toml` is not found.
- All existing tests continue to pass (≥154 tests; the test count may rise as new tests are added).
- `planning/decisions/D3-corpus-config-system.md` frontmatter `status` is `superseded` and the doc carries a `## Superseded` section citing Block M.
- All four harness gates green: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, `cargo build --release`.

## Validation Commands

```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes

- `brain.toml` lives two levels above this repo root (`../../brain.toml` from `core/mev/`). The walk-up naturally handles this: starting from the corpus root argument (e.g. `~/Dev/agentic-portfolio`) it finds `brain.toml` immediately at level 0.
- The file-level blocklist (`is_blocklisted_file` for `CLAUDE.md`, `handoff.md`, etc.) is NOT driven by `brain.toml` — it is a mev-internal concern. Leave it hardcoded for now; D3's successors can extend the config later.
- In tests, construct `BrainConfig` inline or load from `tests/fixtures/brain.toml` — do not rely on `../../brain.toml` being present in CI.
- `BrainConfig::projects()` must return a `Vec<&str>` (or `Vec<String>`) — the union of `[[repos]]` slugs. There is intentionally no separate `[vocab].project` list in `brain.toml` (derived, not declared).
- The `[[repos]]` manifest is parsed and stored in `BrainConfig` but Block M only uses the slugs for project vocab. The full manifest (paths, status_file, etc.) is needed by Block N's `--sync` check — expose the `repos` field publicly so Block N can read it without a config-schema change.

## Amendment Log

_No amendments yet._
