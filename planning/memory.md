---
type: Reference
title: mev Memory
description: Repo-scoped durable memory for mev — episodic notes, preferences, superseded facts. Committed and portable.
doc_id: memory
layer: [factory]
project: mev
status: active
keywords: [memory, episodic, preferences, durable, portable]
related: [knowledge, context, status, planning-index]
---

# Memory — mev

Repo-scoped **durable memory**: episodic notes, operator preferences, and superseded facts that
must survive a handoff and travel with the repo. Committed and portable — distinct from the global
`~/.claude/.../memory/` auto-memory (which is operator-level and stays on one machine).

Use this for project facts worth remembering across sessions. Promote durable "how it works"
knowledge to `knowledge.md`; promote settled choices to `decisions/`. Do not duplicate the global
auto-memory here.

## Notes

_Dated episodic entries — what was tried, what was decided in-flight, what to remember next time._

- **Live triage 2026-06-26: 145 errors from 3 root causes.** First live `mev validate-brain` pass against the company-brain repo started at 145 cascading errors from three root issues: (1) `.claude`, `.repo-backups`, `.agent` directories not in the crawl skip-list — fixed by adding them to `is_blocklisted_name` in `src/brain/crawl.rs`; (2) decision filenames (`D<n>-…` convention) rejected by `is_valid_doc_id` — fixed by adding `is_decision_id()` helper in `src/brain/okf.rs`; (3) path-style `skip_dirs` entries silently ignored because `is_blocklisted_name` only compared leaf names — fixed in Block 2.M Task 6. After all three fixes: 0 errors, 3 warnings (benign keyword-count edge cases).
  source: log.md (2026-06-26 crawl-hardening entry) · date: 2026-06-26 · supersedes: — · freshness: 2026-06-27

- **Block 2.M shipped in a single SDLC-flow run (6 tasks, verdict PASS).** The `brain.toml` config reader was implemented across 6 tasks without any mid-flow rework. The config-flip integration test (Task 4) proved that a vocab-only `brain.toml` edit flips validation results without touching Rust source — the key acceptance criterion for config-driven vocab.
  source: log.md (2026-06-27 Block 2.M entry) · date: 2026-06-27 · supersedes: — · freshness: 2026-06-27

- **GitHub repo created 2026-06-27.** Code pushed to remote for the first time after Block 2.M completed. All 174+ tests passing at time of push.
  source: log.md (2026-06-27 Block 2.M complete entry) · date: 2026-06-27 · supersedes: — · freshness: 2026-06-27

- **`/update-docs --bootstrap` mode added to SDLC harness.** The `/update-docs` command was fixed to support a `--bootstrap` flag that skips the "invention check" (the guard against fabricating doc content) when scaffolding docs from scratch. This was needed for the first full doc pass on a newly created project. Propagated to mev and learn-ai during Block 2.M close-out.
  source: log.md (2026-06-27 Block 2.M complete entry) · date: 2026-06-27 · supersedes: — · freshness: 2026-06-27

- **`docs/harness-json.md` flagged NEEDS_REVIEW.** During the Block 2.H doc health sweep, `docs/harness-json.md` was flagged for future attention. Not blocking, but should be reviewed before the Phase 3 planning pass.
  source: log.md (2026-06-26 Block 2.H close-out) · date: 2026-06-26 · supersedes: — · freshness: 2026-06-27

- **Next planned block after 2.M is 2.J (graph-integrity check).** Block 2.J is the next and final block in Phase 2 per the master plan. It was deferred while 2.M (brain.toml reader) was prioritized as part of HQ Restructure.
  source: log.md (2026-06-26 Block 2.I close-out) · date: 2026-06-26 · supersedes: — · freshness: 2026-06-27

- **Block 2.J was split into 2.J-corpus-crawl + 2.J-graph-integrity.** The split happened because D5 guardrails (single metadata-extractor seam; authored-only graph) were added after `2.J-corpus-crawl` had already shipped to review. The graph block therefore consumed the crawl result rather than being one combined block. This delivery order is the reason D5 "guardrails applied" is noted in the 2.J-graph-integrity tasks.md amendment log rather than the corpus-crawl spec.
  source: planning/archive/2.J-graph-integrity/tasks.md · date: 2026-06-28 · supersedes: — · freshness: 2026-06-29

- **Integration tests in brain_okf.rs and brain_validate.rs required corpus-membership adjustment.** When corpus membership rules changed (Block 2.J-corpus-crawl Task 3), tests that placed fixture `.md` files at the root level of the temp dir stopped being validated — root-level stray `.md` files are not corpus members under the new rules. Tests must place fixture files under `planning/` or `docs/` to be picked up by `BrainValidator`/`validate_brain`.
  source: planning/archive/2.J-corpus-crawl/sdlc/worklog.md (Task 3 decision) · date: 2026-06-28 · supersedes: — · freshness: 2026-06-28

- **`BrainConfig` requires `#[derive(Clone)]` for `validate_brain_sync`.** `validate_brain_sync` needs to pass config to both `BrainValidator::new(config.clone())` and `check_sync(root, &config)` without borrow conflicts. If you add a new function that calls multiple consumers of `BrainConfig`, add `Clone` before trying to fight the borrow checker.
  source: planning/archive/block-n-sync-watermark/sdlc/worklog.md (Task 3 decision) · date: 2026-06-28 · supersedes: — · freshness: 2026-06-28

- **Rollup-row sync check deferred.** Block N also called for asserting tier-rollup rows match their cache's `synced_from`. The current rollup (`core/docs/projects/index.md`) is a plain link list with no per-row `synced_from` field — no grounded format to validate against. The per-repo cache↔source watermark check was implemented; rollup-row assertion lands later once rollups start stamping `synced_from`.
  source: planning/archive/block-n-sync-watermark/tasks.md · date: 2026-06-28 · supersedes: — · freshness: 2026-06-28

- **`tempfile` dev-dep was missing from Cargo.toml.** When adding fixture-based unit tests in `src/brain/graph.rs` (Block 2.J-graph-integrity Task 1), `tempfile` was absent from `[dev-dependencies]` even though other test modules already used it. Adding it unblocked the unit tests.
  source: planning/archive/2.J-graph-integrity/sdlc/worklog.md (Task 1 decision) · date: 2026-06-29 · supersedes: — · freshness: 2026-06-29

- **`check_graph` was co-located in Task 1 of 2.J-graph-integrity.** The spec deferred `check_graph` to Task 2, but it was implemented in Task 1 because its logic depends directly on `GraphArtifact` defined in the same module, and the Task 1 unit tests for `E_GRAPH_DUPLICATE_DOC_ID` and `W_GRAPH_LEAF_TARGET` required it. Task 2 consequently only added the one missing unit test (bare ref to another scope's `doc_id` is dangling). When structuring graph specs in future, co-locate the check function with the artifact it consumes.
  source: planning/archive/2.J-graph-integrity/sdlc/worklog.md (Task 1 decision) · date: 2026-06-29 · supersedes: — · freshness: 2026-06-29

- **`E_CONFIG_NOT_FOUND` cannot be reliably triggered in dev-environment tests.** `find_brain_config` walks up from any directory. On developer machines, the real `brain.toml` may be discovered via walk-up from any temp dir, so tests asserting `E_CONFIG_NOT_FOUND` must use a **lenient/no-panic smoke assertion** rather than a strict diagnostic-code check. Reserve the strict assertion for CI-only environments.
  source: planning/archive/2.M-brain-toml-reader/sdlc/worklog.md (Task 4 decision) · date: 2026-06-27 · supersedes: — · freshness: 2026-06-27

- **Byte-scan state machines over brain content must advance by UTF-8 char width, not `i += 1`.** `extract_links()` originally stepped one byte at a time, which can land mid-multibyte-sequence and make `contents[i..].starts_with(...)` panic on real brain content. Fix: gate the `file://` check on `bytes[i] == b'f'` first, then advance `i` by the char width derived from the leading byte. Only surfaced on the live brain run, not in unit fixtures.
  source: planning/archive/3.K-link-integrity/tasks.md · date: 2026-06-30 · supersedes: — · freshness: 2026-07-02

- **`--structure` orphan detection is purely link-based.** A file named in an `index.md` table as plain backtick text (not a `[text](path)` markdown/`file://` link) counts as an ORPHAN. This is why the live brain shows 84 `E_STRUCT_ORPHAN_FILE` — most brain `index.md` tables list filenames in backticks, which `check_structure` cannot resolve as coverage. Not a false positive; a deliberate consequence of the coverage definition. Any future "reduce orphan noise" work must decide whether to broaden coverage to backtick-name matching or fix the index files.
  source: planning/archive/3.L-structural-coverage/tasks.md · date: 2026-07-02 · supersedes: — · freshness: 2026-07-02

- **`#[serde(alias = "block")]` on `Block.id`/`Endpoint.id` is the v2-migration linchpin.** v2 renames the focus/endpoint ID key `block`→`id`; a pure rename would make every v1 fixture AND the 5 live v1 `state.json` files fail to deserialize on a missing required `id`. The alias keeps them readable through the transition and — critically — lets live v1 files reach the v2 *checks* (rich diagnostics) instead of dying at a parse error. Intended to be removed in a later cleanup once all files are re-seeded to v2.
  source: planning/archive/3.P2-state-graph-validation/breakdown.md · date: 2026-06-30 · supersedes: — · freshness: 2026-07-02

- **Two dedup guards in the v2 state checks, both to avoid double-reporting.** (1) `check_focus_drift` silently skips files with empty `tracks[]` — aggregated brain files derive focus from child repos and would otherwise false-positive; (2) `check_status_consistency` silently skips deps whose key isn't in the status map (dangling), leaving those to `check_state_graph`'s `E_STATE_DANGLING_BLOCKED_BY`.
  source: planning/archive/3.P2-state-graph-validation/worklog.md · date: 2026-06-30 · supersedes: — · freshness: 2026-07-02

- **`emit-state --write` silently drops unmodeled `state.json` fields.** It re-serializes via `serde_json::to_string_pretty(&StateFile)`, which emits ONLY modeled struct fields. `StateFile` is extras-tolerant on read (no `deny_unknown_fields`) but does NOT capture unknown keys — so any live field absent from the struct is dropped on emit. Safe today (v2 fully modeled), but if the schema gains a field, add it to `StateFile` BEFORE running `emit-state --write` against live files. No `preserve_order` feature, so output key order follows struct field order. Also: `to_string_pretty` omits a trailing newline but live files end with one — the first `--write` against such a file performs a one-time newline normalization (counts as a change); don't mistake that for derivation drift.
  source: planning/archive/3B.T-state-table-rollup-emit/breakdown.md · date: 2026-06-30 · supersedes: — · freshness: 2026-07-02

- **Brain `state.json` files are concurrently edited by multiple repos' tooling — re-read before writing, prefer restoring committed content.** A mev-side edit to `core/planning/state.json` (or the HQ root `planning/state.json`) to resolve a carryover silently clobbered legitimate external edits (other repos' focus/rollup entries) that had landed since. Recovery was `git checkout` back to the already-correct committed content, NOT re-overwriting. When touching brain-side `state.json`, re-read/rebase against HEAD immediately before writing.
  source: planning/archive/3B.U-brain-rollup-tier-scoping/sdlc/worklog.md · date: 2026-07-02 · supersedes: — · freshness: 2026-07-02

- **~20 brain markdown files carry FILLER OKF frontmatter injected only to pass validation.** A bulk validation-error cleanup auto-patched ~20 files across sub-repos (orchestrator, workflow-engine-rs, claude-sdk-rs, rag-engine-rs, client/brazilianportugui) with generic frontmatter — `type: Reference`, auto-generated titles, `layer: [meta]`, `description: Documentation for…` — solely to pass `mev`. The ticket to replace filler with accurate per-doc metadata (`ticket-review-frontmatter`) was NEVER executed. Those files still validate green while carrying meaningless frontmatter; detectable by grepping for `layer: [meta]` + `description: Documentation for`. File list is in the archived `ticket-review-frontmatter/tasks.md`.
  source: planning/archive/ticket-review-frontmatter/tasks.md · date: 2026-06-30 · supersedes: — · freshness: 2026-07-02

- **In-flight after MV.3.P2 (state v2).** (1) The 5 live `state.json` files are still v1, so `mev validate-brain --state` on the live brain correctly FAILS until a coordinated brain-side v2 re-seed lands — a clean live run was deliberately NOT an acceptance criterion. (2) Derivation drift is warning-only for now (`W_STATE_FOCUS_DRIFT`, `W_STATE_ROLLUP_DRIFT`); the warn→error flip is deferred until the `/log-work` derived-view writer ships. (3) `check_rollup` reached v2 deriving brain `repos[]` from child `tracks[]` in MV.3B.T/U.
  source: planning/archive/3.P2-state-graph-validation/tasks.md · date: 2026-06-30 · supersedes: — · freshness: 2026-07-02

## Preferences

_Project-specific preferences (tooling, style, workflow) the operator has expressed._

- **Validator-first over compiler.** The operator explicitly chose to ship validation (exit codes + human/`--json` reports) before the `compile` step. The manifest is speculative until learn-ai chooses to consume it. Do not reopen this tradeoff without a concrete consumer.
  source: planning/decisions/D2-scope-and-sequence.md · date: 2026-06-18 · supersedes: — · freshness: 2026-06-27

- **Learn-ai adopts mev only after the binary is proven.** mev stays standalone in its own repo. learn-ai adopts it as a pre-build gate only once it is proven; no edits to learn-ai until then.
  source: planning/decisions/D2-scope-and-sequence.md · date: 2026-06-18 · supersedes: — · freshness: 2026-06-27

---

*Episodic + portable. For durable "how it works" knowledge see `knowledge.md`.*
