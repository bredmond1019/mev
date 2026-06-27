---
type: Handoff
created: 2026-06-26
---

# Handoff — Block 2.G done; start Block 2.H (Brain OKF validator)

> **For the next agent:** Read this immediately after `/prime`. Delete this file once consumed.

## What we're doing and why

`mev` is being generalised from a learn-ai-only validator into a two-consumer tool: learn-ai
content validation (Phase 1) and Bastion Brain OKF frontmatter validation (Phase 2). The
motivating goal is `mev validate-brain ~/Dev/agentic-portfolio` as a pre-flight gate for the
Brain RAG indexer. Phase 2 blocks F → I drive this work in sequence; Blocks F and G are now
done — Block H (OKF frontmatter validator) is the current target.

## Completed this session

- **Block 2.G — Brain crawl — DONE** (`52daf32`)
  - `MdFile { path, rel, stem }` type + `crawl_brain(root)` in `src/brain/crawl.rs`
  - Two-layer directory pruning: name blocklist (`target/`, `node_modules/`, `.git/`, etc.)
    + nested-git rule (any non-root dir with its own `.git` is skipped at depth > 0)
  - `src/brain/mod.rs` re-exports; `src/lib.rs` re-exports `brain` module
  - 8 integration tests in `tests/brain_crawl.rs` + 5 unit tests inline in `crawl.rs`
  - All 96 tests pass; all four harness gates green
- **Close-out: README.md patched** — added `src/brain/` row to the directory map (the
  NEEDS_REVIEW flag from the 2.G wrap-up is resolved)

## Remaining work

- **Block 2.H — Brain OKF frontmatter validator** (start here)
  - `OkfFrontmatter` serde struct; validate required fields (`type`, `title`, `description`)
  - Controlled vocab: `layer` (brain|engine|factory|console|surface|infra|business|content|meta),
    `project` (controlled slug), `status` (active|draft|deprecated|superseded|archived)
  - Kebab-case `doc_id`; keyword count 3–7
  - First: empirically settle the scalar-vs-list question for `layer` against the live corpus:
    `grep -r "^layer:" ~/Dev/agentic-portfolio/docs/ | head -20`
- **Block 2.I — `validate-brain` subcommand + `--json` reporter** — wire `BrainValidator` to CLI
- **Block D / E** (Phase 1 learn-ai) — deprioritised; do not start until Phase 2 is complete
- **NEEDS_REVIEW (non-blocking):** `docs/harness-json.md` is referenced from
  `docs/workflows/index.md` but does not exist — needs a dedicated doc task when time allows

## Open questions / choices

- The `layer` field may be scalar or list in the live Brain corpus — settle empirically before
  writing the serde struct for Block H (see Remaining work above).

## Context the next agent needs

- Repo is on `main`; no open worktrees. There is no GitHub remote — `gh pr list` will fail.
- Harness gates: `cargo fmt --check && cargo clippy -- -D warnings && cargo test && cargo build --release`
- 96 tests currently passing (61 unit + 35 integration/smoke). Keep all green after every block.
- Source layout after 2.G: `src/brain/` holds `crawl.rs` + `mod.rs`; `src/learn_ai/` holds the
  learn-ai validator; `src/validator.rs` holds the `ContentValidator` trait; `src/lib.rs` re-exports both.
- OKF schema canonical reference: `~/Dev/agentic-portfolio/docs/okf-frontmatter.md` (brain repo)

## First command after `/prime`

`/generate-tasks 2.H-brain-okf-validator`
