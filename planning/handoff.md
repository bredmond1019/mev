---
type: Handoff
created: 2026-06-26
---

# Handoff — Block 2.F shipped; start Block G (Brain crawl)

> **For the next agent:** Read this immediately after `/prime`. Delete this file once consumed.

## What we're doing and why

`mev` is being generalised from a learn-ai-only validator into a two-consumer Markdown validator
(learn-ai content + Bastion Brain OKF frontmatter). The motivating deadline is **Block H of the
brain's `brain-rag-improvements` plan** — the first live RAG `--rebuild` against Mac Mini Postgres
— which needs `mev validate-brain ~/Dev/agentic-portfolio` as a pre-flight gate. Phase 2 of the
master-plan (`planning/master-plan.md`) drives this work; Blocks F → I in sequence.

## Completed this session

- **Block 2.F — `ContentValidator` trait + shared core — DONE (PR branch)**
  - Extracted `extract_frontmatter`, `is_kebab_case`, `non_empty` into `src/shared.rs` with unit tests
  - Defined associated-type `ContentValidator` trait in `src/validator.rs` (crawl + validate_item + default run driver)
  - Moved `crawl.rs` / `meta.rs` into `src/learn_ai/` module; added `LearnAiValidator` impl
  - Rewrote `pub fn validate()` in `src/lib.rs` as a thin wrapper over `LearnAiValidator.run()`
  - All 27 existing tests pass unchanged; public API preserved via `pub use`
  - Branch: `2.F-content-validator-trait-flow` (no remote — local-only repo)
  - Fixed a misleading `non_empty` docstring (commit `b8fe7f7`) in the post-flow review

## Remaining work

- **Block G — Brain crawl** (next) — `MdFile { path, rel, stem }` + `crawl_brain(root)` that walks all `.md`
  with a two-layer skip-list: name blocklist (`target/`, `node_modules/`, `.git/`) + nested-git pruning
  (any non-root dir with its own `.git` is skipped). Unit tests for pruning logic required.
- **Block H — Brain OKF frontmatter validator** — `OkfFrontmatter` serde struct; validate required fields
  (`type`, `title`, `description`), controlled vocab (`layer`, `project`, `status`), kebab-case `doc_id`,
  keyword count 3–7.
- **Block I — `validate-brain` subcommand + JSON reporter** — wire `BrainValidator` to a `mev validate-brain`
  subcommand; add `--json` flag emitting machine-readable envelope for the RAG indexer.
- **Block D / E** (Phase 1 learn-ai work) — deprioritised below Phase 2; do not start until I is done.

## Open questions / choices

- The `layer` field in OKF frontmatter is used as both a scalar and a list in the live corpus. Block H says
  "settle the scalar-vs-list question empirically against the live corpus" — this should be the first thing
  verified when starting H. Check `grep -r "^layer:" ~/Dev/agentic-portfolio/docs/ | head -20`.
- No GitHub remote exists for this repo — PRs are local-only branches. Merge the 2.F branch into `main`
  manually with `git merge` before starting Block G (or use `/clean-worktree`).

## Context the next agent needs

- The worktree for Block 2.F lives at `trees/2.F-content-validator-trait-flow/`. Run
  `/clean-worktree` (or `git merge 2.F-content-validator-trait-flow && git branch -d ...`) to land it
  on `main` before starting Block G.
- `status.md` is stale — it still shows Block D as "current focus" and doesn't reflect the Phase 2
  reframe or Block F completion. Update it after merging 2.F.
- The master-plan reframe was done this session (2026-06-26): `planning/master-plan.md` now describes
  both consumers and all Phase 2 blocks (F–I). It is the authoritative roadmap.
- There is no GitHub remote. `gh pr list` will fail. All branching is local.
- Harness gates: `cargo fmt --check && cargo clippy -- -D warnings && cargo test && cargo build --release`
  — these must all stay green after every block.

## First command after `/prime`

`/clean-worktree` (to merge 2.F-content-validator-trait-flow into main), then `/generate-tasks 2.G-brain-crawl`
