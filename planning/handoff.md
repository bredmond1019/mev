---
type: Handoff
created: 2026-06-26
---

# Handoff — Crawl hardening done; Block J (graph integrity) is next

> **For the next agent:** Read this immediately after `/prime`. Delete this file once consumed.

## What we're doing and why

Phase 3 of `mev` adds corpus-wide integrity checks on top of the OKF frontmatter validator
(Phase 2). Before starting Block J we did a live run of `mev validate-brain` against the
actual Brain repo and triaged every diagnostic. The crawl skip-lists were too narrow —
`.claude/`, `.repo-backups/`, `.agent/`, `CLAUDE.md`, `GEMINI.md`, and `handoff.md` were all
being validated as OKF docs. Those are now excluded. We also discovered that the Brain's
`doc_id` convention for decision files (`D15-okf-lowercase-doc-names`) violates the old
kebab-case regex, so we widened the validator to accept the `D<n>-…` format. A config-system
decision (D3) was written to track eventual extraction of all these corpus-specific rules into
a `.mev.toml` file (sequenced after Phase 3). The Brain now validates to **0 errors, 3 warnings**
(the warnings are honest: three files have 8 keywords where the rule is 3–7).

## Completed this session

- **Live triage of `mev validate-brain ~/Dev/agentic-portfolio`** — diagnosed all 145 original
  errors; reduced to 0 errors / 3 warnings across three targeted fixes
- **`fix(brain/crawl)`: dir skip-list** (`4cd7843`) — added `.claude`, `.repo-backups`, `.agent`
  to `is_blocklisted_name` in `src/brain/crawl.rs`; 3 new unit tests
- **`fix(brain/okf)`: decision-id format** (`1790c64`) — added `is_decision_id` and
  `is_valid_doc_id` helpers in `src/brain/okf.rs` to accept `D<n>-…` alongside standard
  kebab-case; removed unused `is_kebab_case` import; 7 new unit tests; 152 total tests pass
- **`docs(decisions)`: D3 — corpus config system** (`d727c7c`) — written as `planning/decisions/D3-corpus-config-system.md`; registered in `planning/decisions/index.md`; status `draft`, sequenced post-Phase 3
- **File-level skip-list** (uncommitted) — added `is_blocklisted_file` to `src/brain/crawl.rs`
  blocking `CLAUDE.md`, `CLAUDE.local.md`, `GEMINI.md`, `handoff.md`; 4 new unit tests;
  154 total tests pass

## Remaining work

- **Block J — Graph integrity (`related:` edges)** (start here)
  - Build a corpus-wide `doc_id` index (every `.md`'s `doc_id`, defaulting to filename stem)
  - Flag every `related:` entry pointing at an undefined `doc_id` (dangling edge)
  - Flag duplicate `doc_id`s
  - Acceptance: renamed/deleted `doc_id` is flagged; duplicate `doc_id`s are flagged; clean corpus passes
- **Block K — Link integrity** (markdown `[text](path)`, `file:///`, `[[wikilinks]]`)
- **Block L — Structural coverage** (`index.md` ↔ directory bidirectional check, D17)
- **Keyword warnings** (3 warnings in Brain corpus) — left as honest signal; will become
  configurable via D3 config system. Not a blocker.

## Open questions / choices

None — clear to proceed with Block J.

## Context the next agent needs

- Repo is on `main`; no open worktrees. No GitHub remote.
- Harness gates: `cargo fmt --check && cargo clippy -- -D warnings && cargo test && cargo build --release`
- **154 tests currently passing** (after this session's uncommitted crawl.rs changes land).
  Keep all green after every block.
- `mev validate-brain ~/Dev/agentic-portfolio` now exits 0 (0 errors, 3 warnings).
- The 3 remaining warnings are `keywords` count > 7 in:
  - `planning/bastion-ui/plan.md`
  - `planning/brain-rag-improvements/plan.md`
  - `docs/projects/markdown-engine-validator.md`
  These are real minor violations in the Brain corpus, not validator bugs.
- D3 decision (`planning/decisions/D3-corpus-config-system.md`) captures the plan to move
  all current hardcodes (skip-lists, doc_id patterns, vocab sets) into a per-corpus `.mev.toml`.
  Current hardcodes are interim and should eventually carry `// TODO(D3): move to config`.
- Block J will likely need `src/brain/graph.rs` with `build_doc_id_index` and
  `check_related_edges`; wire into `BrainValidator::run()` or a new `validate_brain_graph()`
  entry point.

## First command after `/prime`

`/generate-tasks 2.J-graph-integrity`
