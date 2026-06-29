---
type: Handoff
created: 2026-06-29
---

# Handoff — 2.J-corpus-crawl merged; 2.J-graph-integrity is next

> **For the next agent:** Read this immediately after `/prime`. Delete this file once consumed.

## What we're doing and why

`mev` is the corpus engine for the Bastion Brain. Phase 2 (OKF validation, sync watermark) and the
corpus-crawl foundation (`2.J-corpus-crawl`) are now complete and on `main`. The architecture is
settled in **D4**: mev is a pure compiler producing diagnostics + manifest + graph as separate
emitted outputs; the knowledge graph lives in Postgres beside embeddings; two retrieval modes
(semantic + structural). The next block is **`2.J-graph-integrity`** — global `scope:doc_id` node
index, extensible edge model (`Edge { from, to_ref, kind }`), `related:` resolution, uniqueness
linting, and a `--graph` subcommand flag.

## Completed this session

- **`/sdlc-flow 2.J-corpus-crawl` ran to completion** (all 5 tasks, PASS): `src/brain/scope.rs`
  (registry-driven `scope_units`/`scope_for`/`owning_unit`, longest-prefix match, root-unit
  fallback), `crawl_corpus()` returning owned serializable `Corpus`/`CorpusEntry`, `BrainValidator`
  rewired to corpus crawl, OKF root-file exemption for unit-root `README.md`/`CLAUDE.md`,
  13-test integration suite over a 3-unit fixture tree.
- **Post-flow code review fix applied** — `is_root_instruction_file` in `src/brain/okf.rs` was
  checking only the filename; a `docs/README.md` in the corpus would have been silently OKF-exempt.
  Fixed to also verify the file's unit-relative path is exactly `README.md` or `CLAUDE.md` (using
  `owning_unit()` + `strip_prefix`). Regression test added (`is_root_instruction_file_false_for_deep_readme`).
  Commit: `753be87 fix: is_root_instruction_file must verify unit-root position, not just filename`.
- **PR #3 merged** on GitHub (`af6ef67 Merge pull request #3 from bredmond1019/2.J-corpus-crawl-flow`).
- **Worktree `trees/2.J-corpus-crawl-flow` cleaned** — branch deleted, local main rebased + pulled.
- **160 tests pass** (was 159; the regression test added one).
- `main` is 1 commit ahead of origin (`7a37755 plan: add D5 (heterogeneous-format ingest) + fold
  its two guardrails into Block J`) — needs a `git push` at the start of the next session.

## Remaining work

In order:

1. **`git push`** — push the 1 local-only commit to origin before starting any new work.
2. **`/sdlc-flow 2.J-graph-integrity`** — global `scope:doc_id` node index + edge integrity. Spec
   at `planning/2.J-graph-integrity/`. Honor D4 forward-compat: graph construction is a reusable
   module with `Serialize`-able node/edge structs; edge carries `kind` from day one.
3. **Phase 3B (after graph is done):** Block Q (manifest emit → `index_brain.py`), Block R (graph
   emit → Postgres edges + bastion/MCP structural queries), Block S (graph-aware RAG, orchestrator).
4. **Companion work (not mev code — flag to Brandon):** register tier sub-brains as scope units in
   `brain.toml`; switch `skip_dirs` to bare-component bloat list; refactor `index_brain.py` to
   consume mev's manifest; add Postgres edges table.

## Open questions / choices

None — architecture settled per D4; `2.J-graph-integrity` spec is ready as written.
Clear to proceed.

## Context the next agent needs

- **Branch:** `main`. All corpus-crawl work is merged. Need `git push` before new work.
- **Tests:** 160 green. Harness: `cargo fmt --check && cargo clippy -- -D warnings && cargo test && cargo build --release`.
- **Key source files added/modified this session:**
  - `src/brain/scope.rs` — scope registry resolver (new)
  - `src/brain/crawl.rs` — `Corpus`, `CorpusEntry`, `crawl_corpus()`, `is_corpus_member`, `is_ephemeral`, `rel_to_unit_root`
  - `src/brain/okf.rs` — `is_root_instruction_file` (now takes `config` to verify unit-root position), OKF exemption on no-frontmatter path
  - `src/brain/mod.rs` — `BrainValidator::crawl` delegates to `crawl_corpus`
  - `tests/brain_corpus.rs` — 13 integration tests over 3-unit fixture tree
- **brain.toml** at HQ root (`~/Dev/agentic-portfolio/brain.toml`): 9 repos, no tier units yet.
- **D5 is now in the plan** (`7a37755`): heterogeneous-format ingest guardrails are folded into
  Block J — check `planning/decisions/D5-*.md` for any constraints on `2.J-graph-integrity`.
- `2.J-graph-integrity` spec is at `planning/2.J-graph-integrity/`. Read its `tasks.md` before starting.

## First command after `/prime`

```
git push && /sdlc-flow 2.J-graph-integrity
```
