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
timestamp: "2026-07-03T18:30:00Z"
now: "4.A-emit-foundation shipped (PASS, 4 tasks). Generated-marker name constants added to src/brain/emit.rs; global_status_map helper maps repo:id -> status across all state files; render_wave_table now resolves cross-repo depends_on edges against that map, fixing the always-blocked bug. Next: MV.4.B (project caches + tier rollups) or MV.4.C (HQ board), which build on this foundation."
next: "MV.4.B (project caches + tier rollups) or MV.4.C (HQ board) — state-sync-loop Phase 4"
blocked: []
---

# STATUS — Current State & Progress

**Last updated:** 2026-07-03 — `4.A-emit-foundation` (Phase 4, state-sync-loop emit foundation) fully implemented and PASS (4 tasks). Task 1 added named generated-marker constants (`markers::WAVE_TABLE`, `PROJECT_CACHE`, `TIER_ROLLUP`, `HQ_BOARD`) to `src/brain/emit.rs`, with `plan_master_plan_tables` now referencing `markers::WAVE_TABLE` instead of a hardcoded string literal. Task 2 added a pure `global_status_map(files)` helper mapping every loaded state file's blocks to authored status keyed `"{repo_slug}:{block_id}"` across all repos, with 4 new unit tests (multi-repo namespacing, no-collision, absent-status, empty input). Task 3 threaded that map into `render_wave_table`, fixing the cross-repo `depends_on` bug: a block depending on a closed cross-repo block now renders `open` instead of the previous always-`blocked` conservative treatment; `plan_master_plan_tables` builds the map via `global_status_map(files)` and passes it through; 7 existing call sites updated plus 3 new cross-repo closed/open/absent tests. Task 4 confirmed all four harness gates green with no code changes needed. Final review verdict: PASS. This is Wave 1 of the state-sync-loop initiative — the spine `MV.4.A → {B,C} → E` — and unblocks `MV.4.B`/`MV.4.C`.

**Previous:** `ticket-ba15-12-okf-core-convergence` (mev-side okf-core format convergence, BA.15.12/D9/D15/D16) fully implemented and PASS (6 tasks). Task 1 added `okf-core` as an unpinned path dependency (`../bastion/crates/okf-core`, D15 discipline). Task 2 repointed `brain/okf.rs` at `okf_core::OkfFrontmatter` (struct deleted, `pub use`), adapting `validate_md_file`'s `layer`/`keywords` checks to the crate's `Vec<String>` (empty-means-absent) shape. Task 3 repointed `brain/state.rs` at `okf_core`'s state schema/loader/graph model (confirmed byte-for-byte port via diff before deleting mev's copies), keeping only mev-specific validation/derivation logic local. Task 4 repointed `brain/graph.rs` and `brain/graph_emit.rs` at `okf_core`'s graph/graph_emit model+resolution types, keeping only `build_graph`/`check_graph`'s corpus-walking logic local. Task 5 verified byte-identical `validate-brain --json`/`emit-state`/`manifest`/`emit-graph` output on the live brain corpus, before vs. after the repoint (md5-checksum confirmed). Task 6 confirmed all four harness gates green (fmt, clippy `-D warnings`, 312 lib tests + all integration suites, release build). Final review verdict: PASS. This closes out the ticket that had been blocked since D9 — bastion's `okf-core`-side BA.15.12 work had landed by the time this ticket ran.

**Previous:** `MV.3B.V` (one graph resolver: `emit-graph` ships resolved edges) fully implemented and PASS (5 tasks). Task 1 extracted `check_graph()`'s per-edge resolution loop into a pure `resolve_edge(artifact, edge) -> EdgeResolution` (`Resolved`/`LeafTarget`/`Dangling`) in `src/brain/graph.rs`, with `check_graph` now matching on it and producing byte-identical diagnostics; 4 new unit tests. Task 2 extended `graph_emit.rs` to build an export-local `ExportedEdge` (via `resolve_edge`) carrying nullable `target_node_id`/`target_doc_id`, and bumped `GraphExport.version` `"1"` → `"2"`; `graph.rs`'s shared `Edge` struct stays untouched. Task 3 added an integration parity test (`export_resolution_matches_check_graph_diagnostics`) asserting edge-by-edge agreement between `build_graph_export`'s resolved fields and `check_graph`'s `E_GRAPH_DANGLING_RELATED`/`W_GRAPH_LEAF_TARGET` diagnostics over a synthetic corpus. Task 4 updated `docs/cli.md`'s `emit-graph` Output shape to document version `"2"` and the two new nullable fields. Task 5 confirmed all four harness gates green (fmt, clippy `-D warnings`, 312+ tests, release build). Final review verdict: PASS.

**Current focus:** `4.A-emit-foundation` Done — the state-sync-loop initiative's Wave 1 emit foundation (generated-marker constants, cross-repo `global_status_map`, `render_wave_table` cross-repo `blocked`-bug fix) is landed, unblocking `MV.4.B`/`MV.4.C`. Phase 3B (corpus engine outputs, D4) remains fully closed (Q/R/S/T/U/V all Done or shipped-elsewhere). Next: `MV.4.B` (project caches + tier rollups) or `MV.4.C` (HQ board) per `core/planning/state-sync-loop/master-plan.md`, or the out-of-repo orchestrator follow-up (`load_brain_edges.py` deletes its own `build_node_maps()`/`resolve_ref()` and reads mev's exported `target_node_id`/`target_doc_id` fields directly). (Separately: the brain's own 84 `E_STRUCT_ORPHAN_FILE` findings are a brain-content cleanup, not a mev task.)

---

## Momentum

> Working board — keep all five queues live. **Never end a meaningful session with every queue
> empty.** The headlines of **now / next / blocked** mirror the frontmatter scalars above.

- **now** — **`4.A-emit-foundation` Done** (Phase 4, state-sync-loop Wave 1; 4 tasks, PASS). Generated-marker constants (`markers::WAVE_TABLE`/`PROJECT_CACHE`/`TIER_ROLLUP`/`HQ_BOARD`), a cross-repo `global_status_map(files)` helper, and a fix to `render_wave_table`'s cross-repo `depends_on` resolution (closed cross-repo dep now renders `open` instead of always `blocked`) are landed in `src/brain/emit.rs`. **Prior:** `ticket-ba15-12-okf-core-convergence` (mev-side okf-core format convergence, BA.15.12/D9/D15/D16; 6 tasks, PASS).
- **next** — `MV.4.B` (project caches + tier rollups) or `MV.4.C` (HQ board) per `core/planning/state-sync-loop/master-plan.md`, or the out-of-repo orchestrator follow-up (`load_brain_edges.py` deletes its own resolution logic and reads mev's exported fields).
- **blocked** — none currently. Live `mev validate-brain --state` on brain will still fail until the brain-side v2 `state.json` re-seed lands — that is a brain-side coordination step, not a mev blocker.
- **improve** — Phase 3B (D4) is now fully closed (Q/R/S/T/U/V). Cross-repo follow-up (own spec, orchestrator repo): `load_brain_edges.py` deletes `build_node_maps()`/`resolve_ref()` and reads mev's `target_node_id`/`target_doc_id` directly.
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
| MV.3.L | Structural coverage (`index.md` ↔ dir, D17) | Done | `src/brain/structure.rs::check_structure` (orphan + dangling-row detection); `validate_brain_structure` + `--structure` CLI flag; 7 unit + integration tests in `tests/brain_structure.rs`; docs updated; live-brain sanity run: 84 genuine `E_STRUCT_ORPHAN_FILE`, 0 dangling rows. |
| MV.3.P | State integrity (`state.json` schema + block graph) | Done | `StateFile` serde model + loader; `discover_state_files` + `check_schema` (4 rings); `StateGraph`/`build_state_graph`/`check_state_graph` (5 codes); `check_rollup` (`W_STATE_ROLLUP_DRIFT`); `validate_brain_state` + `--state` flag; 4 integration tests; 209 total tests pass. |
| MV.3.P2 | State-graph expansion validation (v2 schema) | Done | 8 tasks, PASS, 275 tests. v2 serde model; DAG from `depends_on[]`; `detect_cycles`/`ready_order`/`check_focus_drift`/`check_status_consistency`/`check_backlog_integrity`; wired into `validate_brain_state`; docs updated. Live brain run gated on brain-side v2 re-seed. |

### Phase 3B — The Brain as a queryable product (corpus engine outputs, D4)
| Block | What | Status | Notes |
|---|---|---|---|
| MV.3B.Q | Manifest emit (file-list + metadata JSON) | Done | 6 tasks, PASS. D5 extract-once: `OkfFrontmatter` Serialize, `CorpusEntry.metadata`, `read_doc_metadata` removed; `src/brain/manifest.rs`; `mev manifest <root>` (`--pretty`); docs updated; all harness gates green. |
| MV.3B.R | Graph emit + structural query surface | Done | 5 tasks, PASS. `src/brain/graph_emit.rs::GraphExport`/`build_graph_export` (nodes/edges/sorted leaves, D4 pure-compiler); `graph_brain` driver + `mev emit-graph [--pretty]` CLI; 3 unit + 4 integration tests; docs updated; live-brain sanity run: 411 nodes, 1062 edges, 101 leaves. mev's in-repo scope is JSON emit only — orchestrator's Postgres edges-table load + bastion/MCP structural query surface are out-of-repo companion work. |
| MV.3B.S | Graph-aware RAG (orchestrator) | Not started | Retrieval traverses edges to expand/rerank semantic hits + query router. Orchestrator-side; mev's edge model is the contract. |
| MV.3B.T | Table/rollup emit (from v2 state graph) | Done | 6 tasks, PASS, 275 tests. `derive_focus`/`derive_rollup`/`derive_cross_repo` single-sourced; `emit.rs` (`wave_order`, `render_wave_table`, `splice_generated`); `plan_state_json`/`plan_master_plan_tables`/`apply_plan`; `emit_state` driver + `emit-state` CLI (`--write`). |
| MV.3B.U | Brain rollup tier-scoping + brain-focus aggregation | Done | 6 tasks, PASS. `TierScope`/`tier_scope_for` + tier-scoped, non-destructive `derive_rollup` (derive/preserve/stub branches, `RepoRollup.tier` always populated); `derive_brain_focus` repo-tagged union; `plan_state_json`/`emit_state` thread `&BrainConfig` through; 8 new integration tests; docs + `D7` decision; carryover resolved. |
| MV.3B.V | One graph resolver: `emit-graph` ships resolved edges | Done | 5 tasks, PASS. `resolve_edge(artifact, edge) -> EdgeResolution` extracted from `check_graph`'s resolution loop (`graph.rs`); `graph_emit.rs`'s `ExportedEdge` carries nullable `target_node_id`/`target_doc_id` via `resolve_edge`; `GraphExport.version` `"1"` → `"2"`; `check_graph` diagnostics byte-identical; edge-by-edge parity test over synthetic corpus; `docs/cli.md` updated; all harness gates green. Phase 3B fully closed. |

### Phase 4 — state-sync-loop: status-surface generators (cross-repo initiative)
| Block | What | Status | Notes |
|---|---|---|---|
| MV.4.A | Emit foundation | Done | 4 tasks, PASS. Generated-marker constants (`markers::WAVE_TABLE`/`PROJECT_CACHE`/`TIER_ROLLUP`/`HQ_BOARD`) in `src/brain/emit.rs`; `plan_master_plan_tables` references `markers::WAVE_TABLE`; pure `global_status_map(files)` helper maps `"{repo_slug}:{block_id}" → status` across all repos; `render_wave_table` now resolves cross-repo `depends_on` edges against that map, fixing the always-`blocked` conservative bug (closed cross-repo dep → `open`); all four harness gates green. Wave 1 of the spine `MV.4.A → {B,C} → E`; see `core/planning/state-sync-loop/master-plan.md`. |
| MV.4.B | Project caches + tier rollups | Not started | Builds on MV.4.A's marker constants + status map. |
| MV.4.C | HQ board | Not started | Builds on MV.4.A's marker constants + status map. |
| MV.4.E | (spine terminus) | Not started | Depends on MV.4.B/MV.4.C. |

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
