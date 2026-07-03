---
type: Plan
title: "Ticket: mev-side BA.15.12 (okf-core format convergence)"
description: Repoint mev's brain/okf.rs, brain/state.rs, brain/graph.rs, and brain/graph_emit.rs at bastion's okf-core crate as the single implementation of OKF frontmatter, state.json schema, and graph edge-resolution, deleting the duplicate struct/logic definitions here.
doc_id: ticket-ba15-12-okf-core-convergence
layer: [factory]
project: mev
status: draft
keywords: [okf-core, BA.15.12, D9, D15, D16, format convergence]
related: [D9-ba15-12-okf-core-convergence-mirror, master-plan, status]
---

# Ticket: mev-side BA.15.12 (okf-core format convergence)

## Metadata
prompt: `mev-side half of bastion's BA.15.12 (mev/okf-core format convergence, D15, scope widened
by D16, mirrored in this repo's D9-ba15-12-okf-core-convergence-mirror.md): repoint this repo at
bastion's okf-core crate as the single implementation of OKF frontmatter, state.json schema, and
graph edge-resolution, deleting the four duplicate modules once okf-core has grown the matching
models.`
status: Not started
last-run: never

## Description

`brain/okf.rs` (899 lines), `brain/state.rs` (5,383 lines), `brain/graph.rs` (807 lines), and
`brain/graph_emit.rs` (282 lines) each define a struct/logic set that duplicates something bastion's
`okf-core` crate also implements or is slated to implement — `OkfFrontmatter` parsing (`okf.rs` vs
`okf-core/src/frontmatter.rs`), the `state.json` schema + emit engine (`state.rs`, no `okf-core`
counterpart today), and graph edge resolution (`graph.rs`'s `resolve_edge`/`EdgeResolution` +
`graph_emit.rs`'s `GraphExport`/`ExportedEdge`, no `okf-core` counterpart today). bastion's D15
(scoped `okf.rs`+`state.rs`) and D16 (widened to add `graph.rs`+`graph_emit.rs`, after this repo
shipped `MV.3B.V`) name `okf-core` as the destination single implementation.

**This ticket is blocked on bastion's own BA.15.12 task spec landing first.** `okf-core` today has
only `frontmatter.rs` + `parse.rs` (605 lines total, per `crates/okf-core/src/*.rs` in the bastion
repo as of 2026-07-03) — no state schema, no reconciled `OkfFrontmatter` model, and no graph/edge-
resolution model exist yet for this repo to repoint at. Task 2 below (the actual repoint) cannot be
executed until `okf-core` ships those types; its concrete sub-steps are deliberately left to be
filled in against `okf-core`'s real shape once it exists, rather than guessing at an API that isn't
written yet (this repo's own `CLAUDE.md` standing rule: don't fabricate what can't be grounded).

## Relevant Files

- `Cargo.toml` — add `okf-core = { path = "../bastion/crates/okf-core" }` as an unpinned path
  dependency (same discipline as bastion's own `mev`/`bella-engine` deps, D15).
- `src/brain/okf.rs` — `OkfFrontmatter` struct + `validate_md_file`; the OKF-parsing duplicate.
- `src/brain/state.rs` — the `state.json` schema, graph, and emit engine; the largest duplicate.
- `src/brain/graph.rs` — `Graph`, `build_graph`, `check_graph`, `resolve_edge`/`EdgeResolution`
  (added in `MV.3B.V`); the graph-resolution duplicate D16 pulled into scope.
- `src/brain/graph_emit.rs` — `GraphExport`, `ExportedEdge`, `build_graph_export` (also `MV.3B.V`);
  consumes `graph.rs`'s `resolve_edge`, so it moves in lockstep with it.
- `src/lib.rs` — re-exports `brain::okf::{OkfFrontmatter, validate_md_file}`,
  `brain::graph::{Graph, build_graph, check_graph}`, `brain::graph_emit::{GraphExport,
  build_graph_export}` — these public re-exports are this crate's contract with `bastion`'s
  `brainval` pass-through (`bastion graph`/`validate-brain`/etc.) and must keep resolving to the
  same names even once their internals move to `okf-core`.
- `src/brain/links.rs`, `src/brain/manifest.rs`, `src/brain/structure.rs`, `src/brain/crawl.rs`,
  `src/brain/emit.rs` — confirmed (via grep) to import from `okf.rs`/`state.rs`/`graph.rs`/
  `graph_emit.rs` today; each needs its imports repointed once those modules delegate to
  `okf-core`, not fully rewritten.

### New Files

None expected — `okf-core` is the new-code destination, in the bastion repo, out of this ticket's
scope.

## Step by Step Tasks
See `tasks.json` in this directory — the task list is defined there, not here.

## Testing Strategy

- `tests/brain_okf.rs`, `tests/brain_state.rs`, `tests/brain_graph.rs`, `tests/brain_graph_emit.rs`
  (all four exist today) must keep passing unmodified in behavior — same assertions, same fixtures —
  proving the repoint changed *implementation*, not *observable behavior*. Update only what breaks
  on type-path changes (e.g. `okf::OkfFrontmatter` → `okf_core::OkfFrontmatter`), not on logic.
- **Parity test (new or extended):** run `cargo run -- validate-brain <live brain root> --json`,
  `cargo run -- emit-state <live brain root>`, `cargo run -- manifest <live brain root>`, and
  `cargo run -- emit-graph <live brain root>` before and after the repoint, on the full live brain
  corpus (`/Users/brandon/Dev/agentic-portfolio`), and diff the outputs — must be byte-identical.
  This mirrors the parity check bastion's own `BA.15.2` already ran against this crate's public API.
- Edge case: confirm `GraphExport.version` stays `"2"` and `ExportedEdge`'s nullable
  `target_node_id`/`target_doc_id` fields keep their current null/resolved semantics after the move —
  the repoint must not silently regress `MV.3B.V`'s behavior.

## Acceptance Criteria

- `Cargo.toml` depends on `okf-core` as an unpinned path dependency; `cargo build --release`
  succeeds.
- `brain/okf.rs`, `brain/state.rs`, `brain/graph.rs`, and `brain/graph_emit.rs` no longer contain
  duplicate struct/logic definitions — they delegate to `okf-core`'s types (or are deleted outright
  if `okf-core` provides direct equivalents with no mev-specific wrapping needed).
- `src/lib.rs`'s public re-exports (`OkfFrontmatter`, `validate_md_file`, `Graph`, `build_graph`,
  `check_graph`, `GraphExport`, `build_graph_export`) still resolve to the same names bastion's
  `brainval` pass-through depends on — no breaking change to the cross-repo path-dependency contract
  (D15).
- `cargo run -- validate-brain`/`emit-state`/`manifest`/`emit-graph` output on the live brain corpus
  is byte-identical before and after the repoint.
- All four existing test files (`tests/brain_okf.rs`, `tests/brain_state.rs`, `tests/brain_graph.rs`,
  `tests/brain_graph_emit.rs`) pass with unchanged assertions (only import paths may change).
- Combined test count is not lower than before this ticket.
- `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, `cargo build --release` all pass.

## Validation Commands

cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release

## Notes

- **Hard prerequisite, not yet met:** bastion's `okf-core`-side task spec for `BA.15.12` (tracked in
  the bastion repo at `planning/15.12-mev-okf-core-convergence/` once `/generate-tasks` runs there)
  must land and ship `okf-core`'s state schema, reconciled `OkfFrontmatter` model, and graph/edge-
  resolution model before Task 2 in `tasks.json` can actually be implemented. Running `/sdlc-task` on
  this ticket before that lands will fail at Task 2 for lack of a real `okf-core` API to repoint at.
- Do not delete `src/brain/graph.rs`'s `resolve_edge`/`EdgeResolution` helper (or `graph_emit.rs`'s
  consumption of it) without first confirming `okf-core`'s graph model reproduces the exact
  referrer-scope-only resolution semantics `MV.3B.V`'s parity test locked in — this repo's own
  `master-plan.md` calls those "the validated ones" versus the orchestrator's now-superseded,
  divergent resolution logic.
- Cross-repo coordination: this ticket and bastion's `BA.15.12` task spec should land in the same
  rough timeframe — a long gap leaves this repo pinned to a stale `okf-core` API or leaves bastion's
  `okf-core` growth unconsumed. No hard deadline is set; sequence, not calendar (this repo's own
  standing rule).
- **Task 5 parity verification result (2026-07-03):** ran all four commands against the live brain
  corpus (`/Users/brandon/Dev/agentic-portfolio`) with two release binaries — baseline built from
  `main`/commit `6c0e0fa` (pre-repoint, in the primary `core/mev` checkout) and post built from this
  worktree's tip after Tasks 1–4 (commit `dacc452`). Commands run against each binary:
  `mev validate-brain <root> --json`, `mev emit-state <root>`, `mev manifest <root>`,
  `mev emit-graph <root>`. Compared with `diff` (exit 0 on all four) and confirmed with `md5`
  checksums matching exactly: `validate-brain` output `ad3e510d132baa995f70fb117f371d85`,
  `emit-state` output `c605ece18e60fa1e479cba369853a043`, `manifest` output
  `62328f950fddb9ed0dca4eafba52ff60`, `emit-graph` output `303a87ab2824d37cd3d57f14f77f03f8` — all
  four byte-identical baseline vs. post-repoint, stderr empty on both sides for all four commands.
  Confirms Tasks 2–4's repoint changed implementation only, not observable behavior.

## Amendment Log
<!-- Append-only. Pipeline stages append one dated line here when they deviate from the plan. -->
- 2026-07-03 (Task 2): the blocker noted in Description/Notes had lifted since this ticket was
  drafted — bastion's `okf-core` now ships `frontmatter.rs`/`parse.rs`/`state.rs`/`graph.rs`/
  `graph_emit.rs` with a reconciled `OkfFrontmatter` model. Repointed `brain/okf.rs` at
  `okf_core::OkfFrontmatter` (`pub use`, struct deleted) and adapted `validate_md_file`'s
  `layer`/`keywords` checks to the model's `Vec<String>` (empty-means-absent) shape, replacing
  the old `Option<Vec<String>>` checks. Because the struct is shared crate-wide, also updated
  the two other real (non-test) consumers whose code touched the changed field shape directly —
  `brain/manifest.rs`'s `build_manifest` (added a `non_empty_vec` adapter so `ManifestEntry`'s
  `Option<Vec<String>>` fields, and its `null`-when-absent JSON output, are unchanged) and
  `brain/graph.rs`'s `related` extraction (`.and_then` → `.map(..).unwrap_or_default()`) — these
  two files are outside Task 2's nominal file list (they belong to Tasks 3/4) but needed a
  one-line shape fix to keep the crate compiling; their own struct/logic delegation to `okf-core`
  is left to Tasks 3/4. `tests/brain_okf.rs`'s 14 assertions pass unmodified; `src/lib.rs`'s
  `OkfFrontmatter`/`validate_md_file` re-export needed no change (re-exported transitively through
  `brain::okf`).
- 2026-07-03 (Task 3): repointed `brain/state.rs` at `okf_core`'s `state` module, which now ships
  a byte-for-byte identical port of this file's former schema/loader/graph-model section
  (`BlockedBy`/`Block`/`Focus`/`TrackBlock`/`Track`/`RepoRollup`/`Endpoint`/`CrossRepoEdge`/
  `TierEntry`/`Origin`/`Backlog`/`CarryoverScope`/`Carryover`/`StateFile`/`StateLoadError`/
  `load_state`/`StateSource`/`StateEdgeKind`/`StateEdge`/`StateNode`/`StateGraph`/
  `build_state_graph` — confirmed via `diff` against `okf-core/src/state.rs`: only doc-comment
  wording differs, every field name/type/order and derive is identical). Deleted all of those
  duplicate definitions from `brain/state.rs` and replaced them with one `pub use okf_core::{...}`
  block; the mev-specific validation/derivation logic that consumes them (`discover_state_files`,
  `check_schema`, `check_state_graph`, `check_status_consistency`, `check_backlog_integrity`,
  `check_rollup`, `detect_cycles`(`_dfs`), `ready_order`, `DerivedFocus`/`derive_focus`,
  `check_focus_drift`, `derive_cross_repo`, `TierScope`/`tier_scope_for`, `derive_rollup`,
  `derive_brain_focus`, `sorted_set`) is unchanged and now operates on the `okf_core` types.
  `src/lib.rs`'s and `src/brain/emit.rs`'s `brain::state::X` imports needed no change — they
  already used fully-qualified paths that the new `pub use` re-export continues to satisfy, so
  neither file is touched by this task (contrary to the file list mev's own repo scan predicted;
  no import repoint was actually needed downstream of `state.rs` itself). One pre-existing quirk
  surfaced: two `#[test]` functions (`carryover_array_deserializes`,
  `carryover_schema_checks`) live outside the `mod tests { }` block (after its closing brace) and
  so are only compiled under `#[cfg(test)]` implicitly via the `#[test]` attribute itself, not via
  the module — this predates this task (present in the Task-2 commit already) and was left as-is
  (out of scope to restructure); it only required keeping a `#[cfg(test)] use std::path::PathBuf;`
  import so both the release build (which doesn't need `PathBuf`) and the test build (which does,
  for these two orphaned tests) compile clean under `clippy -D warnings`. `tests/brain_state.rs`'s
  41 assertions and `tests/brain_emit.rs`'s assertions pass unmodified; combined suite is 312 lib
  tests + all integration suites green (no regression in count).
- 2026-07-03 (Task 4): repointed `brain/graph.rs` and `brain/graph_emit.rs` at `okf_core`'s
  `graph`/`graph_emit` modules, which now ship a byte-for-byte identical port of both files'
  former pure model/resolution sections (confirmed by inspection: `EdgeKind`/`Edge`/`Node`/
  `Graph`/`GraphArtifact`/`EdgeResolution`/`resolve_edge` in `graph.rs`, and the entirety of
  `graph_emit.rs` — `GraphExport`/`ExportedEdge`/`build_graph_export` — have no mev-specific
  logic layered on top; `graph_emit.rs`'s whole non-test body is now a `pub use`). Deleted all of
  those duplicate definitions from both files and replaced them with one `pub use okf_core::{...}`
  each (note: the crate's public path is `okf_core::{Edge, ...}` directly, not
  `okf_core::graph::{...}` — `okf-core`'s `graph`/`graph_emit` submodules are private, re-exported
  flat from its crate root). The mev-specific corpus-walking/diagnostic logic that consumes these
  types (`build_graph`, `check_graph`) is unchanged and now operates on the `okf_core` types.
  `src/lib.rs` needed no change — its `brain::graph::{Graph, build_graph, check_graph}` and
  `brain::graph_emit::{GraphExport, build_graph_export}` re-exports continue to resolve through
  the new `pub use` blocks in each module (contrary to the file list mev's own repo scan
  predicted). `tests/brain_graph.rs`'s 7 assertions and `tests/brain_graph_emit.rs`'s 5 assertions
  pass unmodified; combined suite is still 312 lib tests + all integration suites green (no
  regression in count). Confirmed `GraphExport.version` stays `"2"` and `ExportedEdge`'s
  `target_node_id`/`target_doc_id` null/resolved semantics are unchanged (both come straight from
  `okf-core`'s port, which its own tests already lock in). Ran `cargo run -- validate-brain
  /Users/brandon/Dev/agentic-portfolio --json` as a smoke check post-repoint (full before/after
  byte-identical diff across all four commands is Task 5's job, not this task's).
- 2026-07-03 (Task 5): built two release binaries — baseline from the primary `core/mev` checkout
  at commit `6c0e0fa` (tip of `main`, pre-repoint) and post from this worktree's tip after Tasks 1–4
  (commit `dacc452`) — and ran `validate-brain --json`, `emit-state`, `manifest`, and `emit-graph`
  against the live brain corpus (`/Users/brandon/Dev/agentic-portfolio`) with each binary. `diff`
  reported no differences on any of the four output pairs, confirmed with matching `md5` checksums
  and empty stderr on both sides for all four commands. See Notes for the full command log and
  checksums. No code changes were needed for this task; it is a verification-only task, and its file
  list (`tasks.md`) is updated with this record per the task's acceptance criteria.
