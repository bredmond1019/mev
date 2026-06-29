---
type: ProjectStatus
title: mev Status
description: Current state and progress tracker for mev.
doc_id: status
layer: [factory]
project: mev
status: active
keywords: [block progress, phase status, mev, Rust, validation]
related: [master-plan, context]
timestamp: "2026-06-29"
now: "2.J-corpus-crawl complete + merged (PR #3, 160 tests); is_root_instruction_file correctness fix applied. Ready to build 2.J-graph-integrity"
next: "/sdlc-flow 2.J-graph-integrity (global scope:doc_id node index + edge integrity + leaf lint via --graph)"
blocked: []
---

# STATUS — Current State & Progress

**Last updated:** 2026-06-29 — 2.J-corpus-crawl merged (PR #3). Post-flow code review fix: is_root_instruction_file now verifies unit-root position (not just filename) — prevents docs/README.md from being silently OKF-exempt. 160 tests pass.
**Current focus:** 2.J-graph-integrity — global `scope:doc_id` node index + edge integrity + leaf lint (`--graph`)

---

## Momentum

> Working board — keep all five queues live. **Never end a meaningful session with every queue
> empty.** The headlines of **now / next / blocked** mirror the frontmatter scalars above.

- **now** — **2.J-corpus-crawl MERGED** (PR #3, all 5 tasks PASS). Registry-driven `scope_for`/`scope_units`, `crawl_corpus` (owned serializable `Corpus`), `BrainValidator` rewired to corpus crawl, OKF root-file exemption, 13-test integration suite over 3-unit fixture tree. Post-flow fix: `is_root_instruction_file` now verifies unit-root position (not just filename). 160 tests pass.
- **next** — `/sdlc-flow 2.J-graph-integrity` (global `scope:doc_id` node index + extensible edge model + `related:` resolution + leaf lint via `--graph`)
- **blocked** — nothing blocked
- **improve** — Phase 3B (D4): **Block Q** manifest emit → `index_brain.py` consumes it (kill double crawl); **Block R** graph emit → Postgres edges table + structural query surface (bastion/MCP); **Block S** graph-aware RAG (orchestrator). Companion: register tier units + bare-bloat `skip_dirs` in `brain.toml`.
- **recurring** — none yet

## Metrics

> Cheap, hand-maintained signals (leading + lagging). Do **not** push these into frontmatter —
> they are multi-valued and volatile.

- tasks completed / verified this period; intervention rate; retry rate; regression rate
- reusable assets created since last milestone
- days since last eval improvement; days since last new skill/workflow
- % of runs ending with an explicit next action

---

## How to Read / Update This File

- Status values: `Not started` · `In progress` · `Done` · `Blocked` · `Skipped`
- Keep `Current focus` and `Last updated` accurate; update as work happens.
- This file is **state only**. For what the work means, see `master-plan.md`.

---

## Progress Table

### Phase 0 — Foundation
| Block | What | Status | Notes |
|---|---|---|---|
| Block A | Foundation setup | Done | Rust binary `mev` scaffolded; clap CLI + `Diagnostic`/`Report` lib; smoke tests; all four harness gates green |

### Phase 1 — Core: learn-module validation
| Block | What | Status | Notes |
|---|---|---|---|
| Block B | Crawl & classify | Done | `walkdir` + `Corpus`; filename conventions; 16 tests (7 unit + 7 integration + 2 smoke) |
| Block C | Frontmatter & JSON struct validation | Done | All tasks complete (1–7): struct/frontmatter validation module (`src/meta.rs`) with serde-based deserialization; `ModuleMeta` (LearnModuleJson), PathMetadataJson, and ModuleMdx frontmatter validation with required field checks, enum validation (difficulty, type, level), format validation (kebab-case id, duration pattern); YAML frontmatter parsing with proper error handling; fixture-driven tests (good + broken variants) for all cases; all four harness gates green (fmt, clippy, test, build). |
| Block D | Cross-file integrity | Not started | Anchor-slice, pair existence, ID coherence, callout types |
| Block E | pt-BR parity & reporter polish | Not started | Locale mirror checks; ANSI + `--json` output |

### Phase 2 — Generalize: ContentValidator trait + Brain OKF validation
| Block | What | Status | Notes |
|---|---|---|---|
| Block F | `ContentValidator` trait + shared core | Done | All tasks (1–5) complete: extracted `extract_frontmatter`, `is_kebab_case`, `non_empty` into `src/shared.rs`; defined associated-type `ContentValidator` trait in `src/validator.rs`; moved learn-ai code (`crawl.rs`, `meta.rs`) into `src/learn_ai/` module with `LearnAiValidator` impl; rewrote `validate()` as thin wrapper; public API preserved; all 27 tests pass (including post-flow code-review fix to `non_empty` docstring); all harness gates green. |
| Block G | Brain crawl | Done | `MdFile { path, rel, stem }` + `crawl_brain(root)` with two-layer skip-list (name blocklist + nested-git rule); 8 integration tests + unit tests for pruning helpers; all 96 tests pass |
| Block H | Brain OKF frontmatter validator | Done | OkfFrontmatter struct, validate_md_file, BrainValidator (ContentValidator impl), vocab helpers, 30 unit tests + 14 integration tests; 142 total tests pass |
| Block I | `validate-brain` subcommand + `--json` | Done | `mev validate-brain <root>` (default `..`), global `--json` flag, `JsonReport` envelope, `Serialize` on `Severity`/`Diagnostic`, `validate_brain()` public fn; 5 integration tests; 145 total tests pass |
| Block 2.M | brain.toml config reader (HQ-R) | Done | `BrainConfig` (toml crate), `load_brain_config`/`find_brain_config` walk-up; `crawl_brain` skip_dirs from config; `is_valid_layer`/`is_valid_status`/`is_valid_project` config-driven; `validate_brain` resolves config via walk-up; path-style skip_dirs matching (`planning/archive`); D3 superseded; 10 config + 5 validate integration tests; all harness gates green |
| Block N | `synced_from` watermark check (HQ-R) | Done | `mev validate-brain --sync`; `synced_from` on `OkfFrontmatter`; `parse_watermark` (strict RFC3339); `check_sync` emitting `E_SYNC_FILE_MISSING`/`E_SYNC_WATERMARK_MISSING`/`E_SYNC_WATERMARK_MALFORMED`/`E_SYNC_DRIFT`; `validate_brain_sync()` public API; `--sync` CLI flag; 4 integration tests (in-sync, drift, re-align, JSON); 196 total tests pass |

### Phase 3 — Brain integrity: graph + sync
| Block | What | Status | Notes |
|---|---|---|---|
| Block J-crawl | Multi-root corpus crawl + scope registry | Done | `scope_units`/`scope_for`/`owning_unit` in `src/brain/scope.rs`; `crawl_corpus` → owned serializable `Corpus`; `BrainValidator` rewired; OKF root-file exemption; 13-test integration suite; all 160 tests pass. Post-flow fix: `is_root_instruction_file` now verifies unit-root position (commit `753be87`). PR #3 merged. |
| Block J | Graph integrity (global `scope:doc_id`) | Not started | Spec ready (`planning/2.J-graph-integrity/`). Global node index + extensible edge model + uniqueness + `related:` resolution (bare=same scope, qualified=cross) + leaf lint; `--graph`. Depends on J-crawl. See `namespacing-and-corpus-decision.md`. |
| Block K | Link integrity (markdown/`file://`/`[[wiki]]`) | Not started | Per master-plan |
| Block L | Structural coverage (`index.md` ↔ dir, D17) | Not started | Per master-plan |

### Phase 3B — The Brain as a queryable product (corpus engine outputs, D4)
| Block | What | Status | Notes |
|---|---|---|---|
| Block Q | Manifest emit (file-list + metadata JSON) | Not started | mev emits canonical file-list; `index_brain.py` consumes it → "validated == embedded" by construction. Depends on 2.J-corpus-crawl. |
| Block R | Graph emit + structural query surface | Not started | mev emits graph JSON; orchestrator loads Postgres edges table beside `brain_documents`; bastion/MCP structural queries (free/exact). Depends on 2.J-graph-integrity. |
| Block S | Graph-aware RAG (orchestrator) | Not started | Retrieval traverses edges to expand/rerank semantic hits + query router. Orchestrator-side; mev's edge model is the contract. |

---

## Decisions & Deviations Log

*Record deviations from the plan and notable in-flight choices here. Promote durable ones to
`decisions/` via `/log-work`.*

---

## Quick Self-Check

- Is `Current focus` accurate?
- Any `In progress` rows that are actually `Done`?
- Anything `Blocked` that needs surfacing?

---

*State only. For what things mean, see master-plan.md. For orientation, see context.md.*
