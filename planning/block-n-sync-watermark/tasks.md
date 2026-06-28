---
type: TaskSpec
title: Task Spec — Phase 3, Block M (HQ-R Block N) — synced_from watermark check
description: Decomposed task spec for `mev validate-brain --sync` — the cross-repo synced_from watermark check that catches brain cache ↔ sub-repo status drift.
doc_id: block-n-sync-watermark-tasks
layer: [factory, brain]
project: mev
status: active
keywords: [sync watermark, synced_from, validate-brain, chrono, cross-repo, D29]
related: [master-plan, status, D29-mev-brain-validation-engine]
---

# Task Spec — Phase 3, Block M (HQ-Restructure Block N) — `synced_from` watermark check

**Status:** Done · **Last run:** 2026-06-28 (PASS, 5/5 tasks, 196 tests)

## Goal
Implement `mev validate-brain --sync`: per `brain.toml` `[[repos]]` entry, assert the sub-repo's
`planning/status.md` `timestamp` equals the brain cache doc's `synced_from` (strict full-ISO datetime
compare), emitting a `Sync` error per mismatch so brain↔sub-repo status drift becomes a deterministic,
machine-caught failure.

## Context Pointers

- **Plan:** mev `planning/master-plan.md` → Phase 3, **Block M — Sync integrity** (the local
  numbering); the authoritative cross-repo charter is the HQ-Restructure master-plan **Block N**
  (`~/Dev/agentic-portfolio/planning/hq-restructure/master-plan.md:438`). Governed by brain **D29**
  (mev is the single validation engine).
- **Watermark contract (grounded in the live tree):**
  - Source watermark = the `timestamp` frontmatter scalar in each sub-repo's `status_file`
    (e.g. mev's `planning/status.md` carries `timestamp: "2026-06-27"`).
  - Cache watermark = the `synced_from` frontmatter scalar in the brain cache doc `cache_doc`
    (e.g. `core/docs/projects/mev.md` carries `synced_from: "2026-06-27"`).
  - Both paths come from `brain.toml` `[[repos]]` (`status_file`, `cache_doc`) — **relative to the
    HQ root** that `validate-brain` is pointed at.
- **Repo files that apply:**
  - `src/brain/config.rs` — `BrainConfig` / `RepoEntry` already carry `status_file` + `cache_doc`
    (no config change needed). `BrainConfig::projects()` shows the `[[repos]]` access pattern.
  - `src/brain/okf.rs` — `OkfFrontmatter` struct (add `synced_from` here); `extract_frontmatter`
    usage pattern.
  - `src/brain/mod.rs` — `BrainValidator` + `ContentValidator` impl (register the new module here).
  - `src/lib.rs` — `validate_brain()` (the sibling to add `validate_brain_sync()` beside),
    `Diagnostic` / `Report` / `JsonReport` (the `--json` envelope already serializes every
    diagnostic — a `Sync` finding flows through it unchanged).
  - `src/main.rs` — `ValidateBrain` subcommand + global `--json` flag (add `--sync` here).
- **CLAUDE.md standing rules:** every block ships with tests (rule 1); all four harness gates stay
  green; decisions are append-only.

### Scoping decisions made at authoring time (do not relitigate)

1. **Rollup-row check is deferred.** Block N's "What" also mentions asserting each tier-rollup row
   matches its cache's `synced_from`. The current rollup (`core/docs/projects/index.md`) is a plain
   link list with **no per-row `synced_from`** — there is no grounded format to validate against.
   This spec implements only the per-repo cache↔source watermark check, which fully satisfies the
   Block N acceptance criterion. The rollup-row assertion lands later, once rollups stamp `synced_from`.
2. **Strict full-ISO (RFC3339) parsing.** Watermarks are parsed strictly as RFC3339 datetimes. A
   date-only or otherwise non-RFC3339 watermark is a `Sync` error (malformed watermark), enforcing
   the precision contract — it does **not** silently pass.
3. **`Sync` findings reuse the existing `Diagnostic` currency** as `Error`-severity diagnostics with
   distinct `E_SYNC_*` locator codes (below). No new `Severity` variant; the existing `JsonReport`
   serializes them automatically, so `--json` needs no envelope change.
4. **Brain-side git hooks are out of scope for this repo.** The `pre-commit`/`pre-push` hook wiring
   named in Block N happens in the HQ repo under `hooks/`, committed separately there. This spec is
   the mev `--sync` check only.

### Locator codes (the `Sync` diagnostic vocabulary)

- `E_SYNC_DRIFT` — both watermarks parsed but differ (`timestamp != synced_from`).
- `E_SYNC_WATERMARK_MISSING` — `status_file` lacks `timestamp`, or `cache_doc` lacks `synced_from`.
- `E_SYNC_WATERMARK_MALFORMED` — a watermark is present but not valid RFC3339.
- `E_SYNC_FILE_MISSING` — the `status_file` or `cache_doc` does not exist at `root.join(...)`.

## Step-by-Step Tasks

### 1. Foundation — `chrono` dependency, `synced_from` field, RFC3339 parser
- Add the `chrono` crate to `Cargo.toml` (default features are sufficient; no `serde` feature needed).
- In `src/brain/okf.rs`, add `pub synced_from: Option<String>` to `OkfFrontmatter` (a tolerated
  optional field, like the others). Add one unit test asserting a doc with `synced_from` still
  validates clean (the field is not vocab/format-checked by the OKF schema — it is the sync check's input).
- Create `src/brain/sync.rs` and register it with `pub mod sync;` in `src/brain/mod.rs`.
- In `src/brain/sync.rs`, add `fn parse_watermark(s: &str) -> Result<chrono::DateTime<chrono::FixedOffset>, ...>`
  that parses **strictly as RFC3339** (`DateTime::parse_from_rfc3339`). Unit tests: a full RFC3339
  value parses; a date-only `"2026-06-27"` value is rejected; a garbage value is rejected.
- Files: `Cargo.toml`, `src/brain/okf.rs`, `src/brain/mod.rs`, `src/brain/sync.rs`.

### 2. Sync check logic + unit tests
- In `src/brain/sync.rs`, add a small serde struct (e.g. `WatermarkFrontmatter { timestamp, synced_from }`,
  both `Option<String>`, extras tolerated) plus a helper that reads a file at an absolute path,
  runs `extract_frontmatter`, and returns the parsed watermark struct (or a read/parse failure).
- Add `pub fn check_sync(root: &Path, config: &BrainConfig) -> Vec<Diagnostic>` that, for each
  `[[repos]]` entry: resolves `root.join(repo.status_file)` and `root.join(repo.cache_doc)`; reads
  the `timestamp` from the source and `synced_from` from the cache; and emits diagnostics per the
  locator-code table:
  - file missing → `E_SYNC_FILE_MISSING`; watermark absent → `E_SYNC_WATERMARK_MISSING`;
    watermark present but not RFC3339 → `E_SYNC_WATERMARK_MALFORMED`; both parse but differ →
    `E_SYNC_DRIFT`; equal → no diagnostic. Each diagnostic's `file` is the offending path (rel to
    `root` where practical) and `message` names the repo slug + both values.
- Unit tests with temp-dir fixtures (mirror the `okf.rs` temp-dir test style): in-sync repo → 0
  diagnostics; drifted repo → exactly one `E_SYNC_DRIFT`; missing source `timestamp` → one
  `E_SYNC_WATERMARK_MISSING`; date-only watermark → one `E_SYNC_WATERMARK_MALFORMED`; missing file
  → one `E_SYNC_FILE_MISSING`.
- Files: `src/brain/sync.rs`. (Depends on Task 1.)

### 3. Public API + CLI `--sync` flag
- In `src/lib.rs`, add `pub fn validate_brain_sync(root: &Path) -> anyhow::Result<Report>` beside
  `validate_brain`: resolve `brain.toml` via `find_brain_config` (reuse the existing
  `E_CONFIG_NOT_FOUND` fallback), run the normal `BrainValidator` schema pass, then append
  `brain::sync::check_sync(root, &config)` diagnostics into the same `Report`. Re-export `check_sync`
  (or keep it crate-internal and only expose `validate_brain_sync`) consistent with the module's
  existing `pub use` style.
- In `src/main.rs`, add a `--sync` flag to the `ValidateBrain` subcommand. When set, dispatch to
  `mev::validate_brain_sync` instead of `mev::validate_brain`; the existing `--json` / human and
  exit-code branches stay unchanged (a `Sync` error makes `report.is_failure()` true → exit 1).
  Update the subcommand `about`/help text to mention the `--sync` watermark check.
- Files: `src/lib.rs`, `src/main.rs`. (Depends on Task 2.)

### 4. Integration tests — end-to-end `--sync` over a fixture tree
- Add `tests/brain_sync.rs` that builds a temp HQ-root fixture: a `brain.toml` with two `[[repos]]`
  entries (full `[vocab]` so the schema pass is clean), each repo's `status_file` and `cache_doc`
  present and OKF-clean, with full-ISO RFC3339 watermarks.
- Tests:
  - In-sync fixture → `validate_brain_sync` returns a report with **0 errors**.
  - Bump one repo's source `timestamp` without updating its cache `synced_from` → **exactly one**
    error, locator `E_SYNC_DRIFT`, for that repo (and re-aligning the cache clears it).
  - (Optional, if cheap) a `--json` round-trip asserting the `Sync` diagnostic appears in the
    serialized envelope.
- Files: `tests/brain_sync.rs`. (Depends on Task 3.)

### 5. Validate
- Run the Validation Commands listed below and confirm all pass.

## Acceptance Criteria
- `OkfFrontmatter` carries a `synced_from: Option<String>` field; an OKF doc bearing `synced_from`
  still validates clean.
- `mev validate-brain --sync` exists, runs the schema pass plus the watermark check, and exits 1 when
  any `Sync` error is present.
- Per `brain.toml` `[[repos]]`, the check compares the source `status_file` `timestamp` against the
  cache `cache_doc` `synced_from` resolved relative to the HQ root.
- Watermarks are parsed strictly as RFC3339; a date-only/non-RFC3339 watermark yields an
  `E_SYNC_WATERMARK_MALFORMED` `Sync` error (it does not silently pass).
- Bumping a sub-repo `status.md` `timestamp` without syncing its cache makes
  `mev validate-brain --sync --json` report **exactly one** `Sync` error (`E_SYNC_DRIFT`) for that
  repo; re-aligning the cache `synced_from` clears it.
- An in-sync project passes with zero `Sync` errors.
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
