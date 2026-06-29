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

## Preferences

_Project-specific preferences (tooling, style, workflow) the operator has expressed._

- **Validator-first over compiler.** The operator explicitly chose to ship validation (exit codes + human/`--json` reports) before the `compile` step. The manifest is speculative until learn-ai chooses to consume it. Do not reopen this tradeoff without a concrete consumer.
  source: planning/decisions/D2-scope-and-sequence.md · date: 2026-06-18 · supersedes: — · freshness: 2026-06-27

- **Learn-ai adopts mev only after the binary is proven.** mev stays standalone in its own repo. learn-ai adopts it as a pre-build gate only once it is proven; no edits to learn-ai until then.
  source: planning/decisions/D2-scope-and-sequence.md · date: 2026-06-18 · supersedes: — · freshness: 2026-06-27

---

*Episodic + portable. For durable "how it works" knowledge see `knowledge.md`.*
