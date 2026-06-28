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

## Preferences

_Project-specific preferences (tooling, style, workflow) the operator has expressed._

- **Validator-first over compiler.** The operator explicitly chose to ship validation (exit codes + human/`--json` reports) before the `compile` step. The manifest is speculative until learn-ai chooses to consume it. Do not reopen this tradeoff without a concrete consumer.
  source: planning/decisions/D2-scope-and-sequence.md · date: 2026-06-18 · supersedes: — · freshness: 2026-06-27

- **Learn-ai adopts mev only after the binary is proven.** mev stays standalone in its own repo. learn-ai adopts it as a pre-build gate only once it is proven; no edits to learn-ai until then.
  source: planning/decisions/D2-scope-and-sequence.md · date: 2026-06-18 · supersedes: — · freshness: 2026-06-27

---

*Episodic + portable. For durable "how it works" knowledge see `knowledge.md`.*
