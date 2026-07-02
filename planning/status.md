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
timestamp: "2026-07-02T08:23:55Z"
now: "MV.3B.U shipped — tier-scoped, non-destructive brain rollup + brain-focus union derived; all 4 harness gates green. Next: MV.3.L (structural coverage) or MV.3B.R (graph emit / Phase 3B)."
next: "MV.3.L (structural coverage: index.md ↔ dir, D17); MV.3B.R (graph emit / Phase 3B)"
blocked: []
---

# STATUS — Current State & Progress

**Last updated:** 2026-07-02 — `MV.3B.U` (brain rollup tier-scoping + brain-focus aggregation) fully implemented and PASS (6 tasks). Added `TierScope` + `tier_scope_for(brain_file, config)` and rewrote `derive_rollup` to iterate the in-scope config repos (tier-filtered, or all for HQ), deriving a headline where a child `state.json` loads, else **preserving** the brain file's existing `repos[]` entry verbatim, else emitting a tier-tagged empty stub — no repo is ever silently dropped again; `RepoRollup.tier` is now always populated (Task 1). New `derive_brain_focus(scope, config, graph, files)` computes brain-kind `focus.now/next/blocked` as the repo-tagged, deduped (by `(repo, id)`, per-list), config-ordered union of in-scope children's derived focus (Task 2). `plan_state_json` now takes `&BrainConfig` and wires the brain arm through `tier_scope_for`/`derive_rollup`/`derive_brain_focus`, replacing the transitional all-scope stub; `lib.rs::emit_state` threads its resolved `BrainConfig` through (Task 3). 8 new end-to-end integration tests cover tier-scoped rollup (derived/preserved/stub branches), the malformed-child-JSON preserve regression, repo-tagged focus union, HQ all-tier aggregation, and write/re-emit idempotence (Task 4). `state-schema.md`, `docs/cli.md`, `docs/architecture.md` updated + new decision `D7-brain-rollup-tier-scoping-and-preserve.md` (Task 5). All four harness gates green; non-destructive dry-run verified against the live company brain (no repo drops); `mev-brain-rollup-tier-scoping` carryover resolved in `core/planning/state.json` (Task 6). Final review verdict: PASS.

**Current focus:** `MV.3B.U` Done — next: `MV.3.L` (structural coverage: `index.md` ↔ dir, D17) or `MV.3B.R` (graph emit / Phase 3B); see master-plan.md for ordering

---

## Momentum

> Working board — keep all five queues live. **Never end a meaningful session with every queue
> empty.** The headlines of **now / next / blocked** mirror the frontmatter scalars above.

- **now** — **`MV.3B.U` Done** (brain rollup tier-scoping + brain-focus aggregation; 6 tasks, PASS). `TierScope`/`tier_scope_for` + tier-scoped, non-destructive `derive_rollup` (derive/preserve/stub, `RepoRollup.tier` always populated); `derive_brain_focus` repo-tagged union; `plan_state_json`/`emit_state` thread `&BrainConfig`; 8 new integration tests; docs + `D7`; carryover resolved; live-brain dry-run verified no repo drops. **Prior:** `MV.3B.Q` (manifest emit; 6 tasks, PASS, PR #9 merged).
- **next** — **`MV.3.L`** (structural coverage: `index.md` ↔ dir, D17) or **`MV.3B.R`** (graph emit / Phase 3B). Coordinate brain-side v2 `state.json` re-seed (5 files) for live `--state` validation. See master-plan.md for ordering.
- **blocked** — nothing hard-blocked; live `mev validate-brain --state` on brain will fail until the brain-side v2 `state.json` re-seed lands — that is a brain-side coordination step, not a mev blocker.
- **improve** — Phase 3B (D4): **`MV.3B.R`** graph emit → Postgres edges table + structural query surface (bastion/MCP); **`MV.3B.S`** graph-aware RAG (orchestrator). `index_brain.py` can now consume `mev manifest` → kill double crawl. Companion: register tier units + bare-bloat `skip_dirs` in `brain.toml`.
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
| MV.3.P2 | State-graph expansion validation (v2 schema) | Done | 8 tasks, PASS, 275 tests. v2 serde model; DAG from `depends_on[]`; `detect_cycles`/`ready_order`/`check_focus_drift`/`check_status_consistency`/`check_backlog_integrity`; wired into `validate_brain_state`; docs updated. Live brain run gated on brain-side v2 re-seed. |

### Phase 3B — The Brain as a queryable product (corpus engine outputs, D4)
| Block | What | Status | Notes |
|---|---|---|---|
| MV.3B.Q | Manifest emit (file-list + metadata JSON) | Done | 6 tasks, PASS. D5 extract-once: `OkfFrontmatter` Serialize, `CorpusEntry.metadata`, `read_doc_metadata` removed; `src/brain/manifest.rs`; `mev manifest <root>` (`--pretty`); docs updated; all harness gates green. |
| MV.3B.R | Graph emit + structural query surface | Not started | mev emits graph JSON; orchestrator loads Postgres edges table beside `brain_documents`; bastion/MCP structural queries (free/exact). Depends on `MV.3.J`. |
| MV.3B.S | Graph-aware RAG (orchestrator) | Not started | Retrieval traverses edges to expand/rerank semantic hits + query router. Orchestrator-side; mev's edge model is the contract. |
| MV.3B.T | Table/rollup emit (from v2 state graph) | Done | 6 tasks, PASS, 275 tests. `derive_focus`/`derive_rollup`/`derive_cross_repo` single-sourced; `emit.rs` (`wave_order`, `render_wave_table`, `splice_generated`); `plan_state_json`/`plan_master_plan_tables`/`apply_plan`; `emit_state` driver + `emit-state` CLI (`--write`). |
| MV.3B.U | Brain rollup tier-scoping + brain-focus aggregation | Done | 6 tasks, PASS. `TierScope`/`tier_scope_for` + tier-scoped, non-destructive `derive_rollup` (derive/preserve/stub branches, `RepoRollup.tier` always populated); `derive_brain_focus` repo-tagged union; `plan_state_json`/`emit_state` thread `&BrainConfig` through; 8 new integration tests; docs + `D7` decision; carryover resolved. |

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
