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
timestamp: "2026-06-30T06:33:51-0300"
now: "MV.3.K Done — link integrity validator implemented + merged (PR #6, 237 tests, PASS verdict). Next: MV.3.P2 (state-graph v2 validator — spec drafted, gated on brain re-seed), MV.3.L (structural coverage) or MV.3B.Q (manifest emit / Phase 3B)"
next: "MV.3.P2 (v2 state-graph validator — spec drafted at planning/3.P2-state-graph-validation/, gated on brain-side v2 state.json re-seed); also MV.3.L (structural coverage, D17), MV.3B.Q (manifest emit / Phase 3B), MV.3B.T (table/rollup emit) — see master-plan.md for ordering"
blocked: []
---

# STATUS — Current State & Progress

**Last updated:** 2026-06-30 — `MV.3.K` (link integrity) fully implemented, reviewed, and merged via PR #6. All 6 tasks passed (PASS verdict); 237 tests total. `mev validate-brain --links` is live; live brain run confirmed real findings (dangling wikilinks, dead file:// URIs, dead markdown links). Post-review fix: `--links` now takes **highest** dispatch precedence (was placed last → lowest, contradicting docs/spec), covered by a new binary-spawning test.
**State-graph expansion planned (2026-06-30):** Settled the state-graph v2 design (schema rewritten to v2 in `core/planning/state-schema.md`: `depends_on` DAG, derived focus/rollup, `backlog[]`, blocked-is-derived); added blocks **MV.3.P2** (state-graph expansion validation) + **MV.3B.T** (table/rollup emit) to the master-plan and **specced MV.3.P2** at `planning/3.P2-state-graph-validation/tasks.md` (8 tasks). MV.3.P2 is gated on the brain-side re-seed of the 5 live `state.json` files to v2.

**Current focus:** `MV.3.K` Done — next block: `MV.3.P2` (v2 state-graph validator — spec drafted, gated on brain re-seed), `MV.3.L` (structural coverage) or `MV.3B.Q` (manifest emit / Phase 3B); see master-plan.md for ordering

---

## Momentum

> Working board — keep all five queues live. **Never end a meaningful session with every queue
> empty.** The headlines of **now / next / blocked** mirror the frontmatter scalars above.

- **now** — **`MV.3.K` Done & merged (PR #6)** (link integrity; 6 tasks, PASS, 237 tests). `LinkKind`/`LinkRef` model + `extract_links()` single-pass byte-scan; `check_links()` resolving Markdown/`FileUri`/`WikiLink` refs with four `E_LINK_*` diagnostic codes; `check_moved_references()` consuming `.brain-moves-pending`; `validate_brain_links()` public API + `--links` CLI flag (highest dispatch precedence); 10 integration tests. Live brain run confirmed real findings (dangling `[[bin]]`/`[[test]]` wikilinks, dead `file://` URIs with placeholder paths, dead markdown links in SECURITY.md). **Prior:** `MV.3.P` state integrity DONE (209 tests).
- **next** — **`MV.3.P2`** (state-graph v2 validator) — spec drafted at `planning/3.P2-state-graph-validation/tasks.md` (8 tasks); **gated on the brain-side re-seed of the 5 live `state.json` files to v2** (`depends_on` DAG, derived focus/rollup, `backlog[]`, blocked-is-derived per `core/planning/state-schema.md` v2). Also queued: `MV.3.L` (structural coverage: `index.md` ↔ dir, D17), `MV.3B.Q` (manifest emit / Phase 3B) — check master-plan.md for ordering.
- **blocked** — nothing hard-blocked; `MV.3.P2` implementation waits on the brain-side v2 `state.json` re-seed (not a mev-side blocker).
- **improve** — Phase 3B (D4): **`MV.3B.Q`** manifest emit → `index_brain.py` consumes it (kill double crawl); **`MV.3B.R`** graph emit → Postgres edges table + structural query surface (bastion/MCP); **`MV.3B.S`** graph-aware RAG (orchestrator); **`MV.3B.T`** table/rollup emit (newly planned — derived focus/rollup tables from the v2 state graph). Companion: register tier units + bare-bloat `skip_dirs` in `brain.toml`.
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
| MV.0.A | Foundation setup | Done | Rust binary `mev` scaffolded; clap CLI + `Diagnostic`/`Report` lib; smoke tests; all four harness gates green |

### Phase 1 — Core: learn-module validation
| Block | What | Status | Notes |
|---|---|---|---|
| MV.1.B | Crawl & classify | Done | `walkdir` + `Corpus`; filename conventions; 16 tests (7 unit + 7 integration + 2 smoke) |
| MV.1.C | Frontmatter & JSON struct validation | Done | All tasks complete (1–7): struct/frontmatter validation module (`src/meta.rs`) with serde-based deserialization; `ModuleMeta` (LearnModuleJson), PathMetadataJson, and ModuleMdx frontmatter validation with required field checks, enum validation (difficulty, type, level), format validation (kebab-case id, duration pattern); YAML frontmatter parsing with proper error handling; fixture-driven tests (good + broken variants) for all cases; all four harness gates green (fmt, clippy, test, build). |
| MV.1.D | Cross-file integrity | Not started | Anchor-slice, pair existence, ID coherence, callout types |
| MV.1.E | pt-BR parity & reporter polish | Not started | Locale mirror checks; ANSI + `--json` output |

### Phase 2 — Generalize: ContentValidator trait + Brain OKF validation
| Block | What | Status | Notes |
|---|---|---|---|
| MV.2.F | `ContentValidator` trait + shared core | Done | All tasks (1–5) complete: extracted `extract_frontmatter`, `is_kebab_case`, `non_empty` into `src/shared.rs`; defined associated-type `ContentValidator` trait in `src/validator.rs`; moved learn-ai code (`crawl.rs`, `meta.rs`) into `src/learn_ai/` module with `LearnAiValidator` impl; rewrote `validate()` as thin wrapper; public API preserved; all 27 tests pass (including post-flow code-review fix to `non_empty` docstring); all harness gates green. |
| MV.2.G | Brain crawl | Done | `MdFile { path, rel, stem }` + `crawl_brain(root)` with two-layer skip-list (name blocklist + nested-git rule); 8 integration tests + unit tests for pruning helpers; all 96 tests pass |
| MV.2.H | Brain OKF frontmatter validator | Done | OkfFrontmatter struct, validate_md_file, BrainValidator (ContentValidator impl), vocab helpers, 30 unit tests + 14 integration tests; 142 total tests pass |
| MV.2.I | `validate-brain` subcommand + `--json` | Done | `mev validate-brain <root>` (default `..`), global `--json` flag, `JsonReport` envelope, `Serialize` on `Severity`/`Diagnostic`, `validate_brain()` public fn; 5 integration tests; 145 total tests pass |
| MV.2.M | brain.toml config reader (HQ-R) | Done | `BrainConfig` (toml crate), `load_brain_config`/`find_brain_config` walk-up; `crawl_brain` skip_dirs from config; `is_valid_layer`/`is_valid_status`/`is_valid_project` config-driven; `validate_brain` resolves config via walk-up; path-style skip_dirs matching (`planning/archive`); D3 superseded; 10 config + 5 validate integration tests; all harness gates green |
| MV.2.N | `synced_from` watermark check (HQ-R) | Done | `mev validate-brain --sync`; `synced_from` on `OkfFrontmatter`; `parse_watermark` (strict RFC3339); `check_sync` emitting `E_SYNC_FILE_MISSING`/`E_SYNC_WATERMARK_MISSING`/`E_SYNC_WATERMARK_MALFORMED`/`E_SYNC_DRIFT`; `validate_brain_sync()` public API; `--sync` CLI flag; 4 integration tests (in-sync, drift, re-align, JSON); 196 total tests pass |

### Phase 3 — Brain integrity: graph + sync
| Block | What | Status | Notes |
|---|---|---|---|
| MV.3.J-crawl | Multi-root corpus crawl + scope registry | Done | `scope_units`/`scope_for`/`owning_unit` in `src/brain/scope.rs`; `crawl_corpus` → owned serializable `Corpus`; `BrainValidator` rewired; OKF root-file exemption; 13-test integration suite; all 160 tests pass. Post-flow fix: `is_root_instruction_file` now verifies unit-root position (commit `753be87`). PR #3 merged. |
| MV.3.J | Graph integrity (global `scope:doc_id`) | Done | Serializable Graph model, build_graph + read_doc_metadata seam, check_graph (3 diagnostic codes), validate_brain_graph API, --graph CLI flag, 7 integration tests. 232 total tests pass. Post-flow fix: diagnostic locators corrected (`W_GRAPH_LEAF_TARGET`, `E_GRAPH_DANGLING_RELATED`); PR #4 merged. |
| MV.3.K | Link integrity (markdown/`file://`/`[[wiki]]`) | Done | `LinkRef` model + `extract_links()`; `check_links()` (4 `E_LINK_*` codes); `check_moved_references()` (.brain-moves-pending); `validate_brain_links()` + `--links` flag; 10 integration tests; 237 total tests pass. PR #6 merged. Post-review fix: `--links` moved to highest dispatch precedence (matches docs/spec) + dispatch-precedence test. |
| MV.3.L | Structural coverage (`index.md` ↔ dir, D17) | Not started | Per master-plan |
| MV.3.P | State integrity (`state.json` schema + block graph) | Done | `StateFile` serde model + loader; `discover_state_files` + `check_schema` (4 rings); `StateGraph`/`build_state_graph`/`check_state_graph` (5 codes); `check_rollup` (`W_STATE_ROLLUP_DRIFT`); `validate_brain_state` + `--state` flag; 4 integration tests; 209 total tests pass. |
| MV.3.P2 | State-graph expansion validation (v2 schema) | Not started | v2 state-graph validator — spec drafted at `planning/3.P2-state-graph-validation/tasks.md` (8 tasks); validates `depends_on` DAG, derived focus/rollup, `backlog[]`, blocked-is-derived (`core/planning/state-schema.md` v2). Gated on brain re-seed of 5 live `state.json` files. |

### Phase 3B — The Brain as a queryable product (corpus engine outputs, D4)
| Block | What | Status | Notes |
|---|---|---|---|
| MV.3B.Q | Manifest emit (file-list + metadata JSON) | Not started | mev emits canonical file-list; `index_brain.py` consumes it → "validated == embedded" by construction. Carries the D5 extract-once refactor (adds `metadata` to `CorpusEntry`, parses frontmatter once; `MV.3.J`'s `read_doc_metadata` seam collapses to `entry.metadata`). Depends on `MV.3.J-crawl`. |
| MV.3B.R | Graph emit + structural query surface | Not started | mev emits graph JSON; orchestrator loads Postgres edges table beside `brain_documents`; bastion/MCP structural queries (free/exact). Depends on `MV.3.J`. |
| MV.3B.S | Graph-aware RAG (orchestrator) | Not started | Retrieval traverses edges to expand/rerank semantic hits + query router. Orchestrator-side; mev's edge model is the contract. |
| MV.3B.T | Table/rollup emit (from v2 state graph) | Not started | mev emits derived focus/rollup tables from the v2 state graph (`depends_on` DAG → computed blocked + rollup). Companion to MV.3.P2's validation; planned alongside the state-graph expansion. |

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
