---
type: Log
title: mev Development Log
description: Chronological log of work completed for mev.
doc_id: log
layer: [factory]
project: mev
status: active
keywords: [work log, development history, session entries, block completion]
related: [status]
timestamp: "2026-08-01T00:00:00Z"
---

# Log — mev

*Append-only working log. One dated entry per session. Newest entries at the top.*

---

## [run: 2026-08-01]

### Phase 11 orchestrated end to end — MV.11.A + MV.11.B both PASS and merged

- **What:** Drove the handoff's Phase 11 sequence to completion as an orchestrator, delegating each
  block to a subagent and reviewing before merge. **`MV.11.A — Epic weight + status vocabulary`**
  (PR #24 → `a7020cd`): `weight: Option<u8>` on `okf_core::Epic` (okf-core `ff86475`,
  `skip_serializing_if` so an absent weight stays byte-identical), `E_STATE_EPIC_BAD_WEIGHT` at an
  inclusive max of 100, `focused` as a fourth epic status made active-equivalent in `plan_sync_epics`
  (`complete` already existed and was not re-added; `plan_resume_epic` still sets `active`), and
  warn-only `W_STATE_EPIC_ALL_CLOSED` guarded by `total > 0` so it cannot double-fire with
  `W_STATE_EPIC_EMPTY`. Nothing touched `derive_rollup` — the CR's rollup hop does not exist. One
  unspecced change was necessary and accepted: `render_epic_board` is a strict two-way partition that
  **silently drops** any status in neither `RENDERED_EPIC_STATUSES` nor `COLLAPSED_EPIC_STATUSES`, so
  a `focused` epic — the current-priority one — would have vanished from the board; `focused` was
  added to the rendered list with a test. **`MV.11.B — mev set-block-status`** (PR #25 → `5478dc9`):
  `src/brain/blocks.rs::plan_set_block_status`, one more sibling in the shipped `epics.rs` planner
  family, behind `mev [--json] set-block-status <repo:id> <status> [path] [--write]` — dry-run by
  default (verified at hash level to change zero bytes and create no lock file), `--write` taking the
  worktree guard → advisory lock → incomplete-corpus guard → apply → chained `emit_state(write=true)`.
  Validates against `VALID_TRACK_BLOCK_STATUSES` and rejects `blocked` as the derived-only lane it is;
  unqualified ids rejected rather than guessed; no-op set is a clean exit 0. 880 tests green (from
  857), all four gates green on merged `main`. Both blocks were then closed **using the command this
  phase shipped**, which incidentally reconciled a stale derived surface in `orchestrator` (`OR.X2`
  had been closed in its authored `tracks[]` without a re-emit — authored data untouched, only
  `focus.next`, the wave table, and the status frontmatter). Live corpus: 0 errors, `emit-state` at a
  fixed point. Repaired the cross-repo fallout in `bastion` (7 `Epic` struct literals, commit
  `a2fd8d1`, local).
- **Why:** The handoff had three specs authored and zero executed. Running them in sequence rather
  than parallel avoided the `epics.rs` merge-conflict risk the specs were serialized for, and let
  `MV.11.B` build on `MV.11.A`'s landed state instead of alongside it.
- **Caveats recorded (three new carryovers, all needing attention):** the `Workflow` SDLC engine was
  **unavailable to delegated subagents**, so both blocks were driven manually and neither produced an
  `sdlc/` state trail or a formal end-of-flow review — an independent review agent was run in place of
  each, both PASS, and both found real defects that were fixed before merge; `defer-epic` /
  `resume-epic` / `sync-epics` still write **without** the advisory lock (`MV.11.B`'s spec wrongly
  asserted they took it); and `Epic.weight` is **not yet readable by any consumer** — bastion's
  `EpicDto` has no such field, so `MV.11.A` delivers no user-visible value until bastion projects it
  and bastion-web drops its hardcoded `EPIC_WEIGHTS`. Also unmasked 6 pre-existing clippy lints in
  bastion (previously hidden behind the compile error) and left one true-positive
  `W_STATE_EPIC_ALL_CLOSED` unflipped on the live corpus: `state-graph-view` has all 7 members closed
  but is still `active` — an authoring call, not an agent one.
- **Refs:** `planning/phase-11-orchestration/notes.md` (full run detail), PRs
  [#24](https://github.com/bredmond1019/mev/pull/24) / [#25](https://github.com/bredmond1019/mev/pull/25)

```
5478dc9 MV.11.B — mev set-block-status: the first block-level mutation (#25)
a7020cd MV.11.A — Epic weight + status vocabulary (#24)
```

---

### Triaged the bastion-web arch-review CR; decomposed Phase 11 (MV.11.A/B + content-epic chore)

- **What:** Reviewed `planning/arch-review-asks-bastion-web/notes.md` against the real code and
  found two defects: (1) its central routing claim — that the epic `weight` should "flow through
  mev's rollup" — describes a code path that **does not exist** (`bastion/src/serve/handlers/epics.rs`
  reads `okf_core::Epic` directly and documents "no rollup/graph"; `derive_rollup` never touches the
  epic registry), and (2) the doc covered only two of the four asks its own HQ backlog entry
  advertises. Rewrote the CR: four asks, a status table, correct per-repo ownership, and a triage
  note. Added the evidence the original lacked — `EPIC_WEIGHTS` (`bastion-web/lib/board-view.ts:555-570`)
  carries **11 keys against 13 epics**, so `brain-quality` and `outreach-machine` silently score at
  the 60 fallback; the predicted drift has already happened twice. Verified the other two backlog
  asks (`last_action_at`, terminal-stage flag) are already filed in **bastion's** CR addendum, and
  that `last_action_at` is a pure bastion change (its `handlers/pipeline.rs` already parses
  `actions[].at`; only the summary projection omits it). Recorded a finding neither CR had: the
  pipeline stage vocabulary exists in **three** independent copies (mev's `VALID_STAGES`,
  `pipeline.md` prose, bastion-web's `CLOSED_STAGES`). Fixed the CR's dangling `related`
  (`epic-taxonomy-v2` → a spec `tasks.md`, which is never crawled); brain-wide graph errors 23 → 22.
  Added two asks from the operator review — a `focused` epic status plus a warn-only
  all-blocks-closed diagnostic, and `mev set-block-status` as the first block-level mutation. Then
  decomposed the mev-side work into three specs (`epic-weight-and-status/`,
  `ticket-set-block-status/`, `content-epic/`), registered `MV.11.A` / `MV.11.B` /
  `MV.chore.content-epic` in `state.json` under a new Phase 11 track, and updated `planning/index.md`.
  A corpus survey run for the content epic found the queue's regex matches **mostly false positives**
  (brazilianportugui payments, engine-rs divestment) and that genuine content work is largely not
  block-tracked at all — so that chore is parked on an operator membership call rather than specced
  as executable. `validate-brain --state` 0 errors; `emit-state --write` propagated all three blocks.
- **Why:** The CR was about to be promoted to a plan. Promoting it as written would have sent an
  implementer looking for a rollup hop that does not exist, and would have silently dropped half its
  scope. Planning-only session — no mev source changed.
- **Refs:** `planning/arch-review-asks-bastion-web/notes.md`, `planning/handoff.md`,
  `core/_planning/bastion/arch-review-asks-bastion-web/notes.md` (addendum)

### MV.ticket.distill-freshness-lane — Read D35's `freshness:` stamp; PASS

Ran `/sdlc-flow distill-freshness-lane` on branch `distill-freshness-lane-flow`, 6 tasks all PASS,
review verdict PASS. Task 1 extended `AttentionThresholds` (`src/brain/config.rs`) with
`knowledge_days` (45) / `memory_days` (30) defaults and a `distill_threshold(stem)` helper (longest
fallback for unknown stems). Task 2 added a new module `src/brain/distill.rs` with a hand-rolled line
scanner (no new dependency) parsing D35-format `knowledge.md`/`memory.md` entries under both the
`  source:` and `  - source:` (amistad) shapes, wrapped-claim recovery, and the shared predicate
`distill_stale_age` (anchor = `max(date, freshness)`, strictly `>`, unparseable freshness never
stale — mirrors `backlog_stale_age`); resolved a pre-existing `clippy::type_complexity` warning at
`state.rs:7323` (unrelated to this spec) via a `PriorityBlockSpec` type alias so `--all-targets -D
warnings` stayed green. Task 3 added `check_distill_staleness`, wired into `validate_brain_state` to
emit a warning-severity `W_DISTILL_STALE` diagnostic per stale entry (silent skip if the file is
absent; exit code unchanged), naming the three remedies (re-affirm / supersede / archive). Task 4 gave
the Attention board a 4th, capped "Stale distilled knowledge" lane (10 rows + explicit "…and N more"),
fed by a per-repo knowledge/memory read-once cache in `plan_attention_board`, reusing the identical
tier-scoping predicate the carryover union uses, spliced into the existing `markers::ATTENTION` region
(no new sentinel). Task 5 confirmed the mechanical 3→4 lane golden-test update was already covered by
task 4's commit; no further changes needed. Task 6 closed D35's deferred freshness-format
documentation thread and updated `docs/decisions/D35-memory-distillation-loop.md`,
`docs/state/{overview,state-schema}.md`, and the `/attention` skill+command (root + all 5 tier
mirrors) to document the lane's bump-freshness disposition — `/snooze`'s scope stays unchanged
(carryover + backlog only, since distilled entries have no stable id); this task's changes are all
brain-repo docs, committed in `agentic-portfolio`, not `core/mev`. All four harness gates green (842
tests, up from 822); live-corpus `mev validate-brain --sync --graph --state --links --structure`: 0
errors. No new frontmatter fields, no GC/summarizer, per the spec's explicit scope boundary.
Next: pick the next phase/block per `master-plan.md`.

```
84cedf1 docs: update docs for distill-freshness-lane
a08f59c feat: implement distill-freshness-lane-task4
0d058e5 feat: implement distill-freshness-lane-task3
71cf0b7 fix: fix pass 1 for distill-freshness-lane-task2
4cda94f feat: implement distill-freshness-lane-task2
dbfcd9c feat: add knowledge_days/memory_days attention thresholds
```

---

## [run: 2026-07-29]

### MV.10.D — Derive `last_touched` per block; PASS

Ran `/sdlc-flow 10.D-derive-last-touched` on branch `10.D-derive-last-touched-flow`, 6 tasks all
PASS, review verdict PASS. Task 1 added `RepoEntry.prefix: Option<String>` to `src/brain/config.rs`
(54 struct literals repaired) so block IDs can resolve against prefix-stripped spec folders (mev's
own convention). Task 2 added `src/brain/last_touched.rs` — `derive_last_touched(root, config,
loaded) -> HashMap<String, String>`, keyed `"repo:id"`, reading all four SDLC state-file kinds
(`sdlc-flow-state.json`/`sdlc-task-state.json`/`sdlc-run-state.json`/`sdlc-state.json`), resolving
folders under full-ID, bare-ID, and prefix-stripped naming with a name-boundary match, walking
`planning/archive/`, newest-`updated_at`-wins lexicographically, no fabricated fallback for
never-started blocks. Task 3 added a 10-test integration suite covering all three folder-naming
conventions in one fixture corpus. Task 4 wired `BlockGraphNode.last_touched: Option<String>`,
populated corpus-wide before the scope pipeline (mirroring `dependent_count`'s precedent),
scope-stable across scoped/unscoped exports. Task 5 documented the field in `docs/cli.md` and the
HQ `docs/state/state-schema.md` as derived, never authored. Task 6 validated end to end: all four
harness gates green, two consecutive `emit-block-graph` runs diff clean (nothing written), and a
live-corpus smoke check confirmed non-null `last_touched` for `MV.10.C` (closed, under
`--include-closed`) and `MV.3B.Q` (archived), and null for `BW.2.A` (never-started). Notable
decision: the transport is a new public `mev` lookup function, not an addition to
`okf_core::Block`/`derive_rollup`, so bastion's `BA.11.S` calls the same function rather than
re-deriving or persisting a derived timestamp into `state.json`. Unblocks bastion's `BA.11.S`.

Next: pick up `MV.ticket.distill-freshness-lane` (D35 freshness read + capped 4th Attention lane
for stale distilled knowledge), or the next phase/block per `master-plan.md`.

```
f669736 docs: update docs for 10.D-derive-last-touched
944e783 feat: implement 10.D-derive-last-touched-task5
ab181b7 feat: implement 10.D-derive-last-touched-task4
9791cd3 feat: implement 10.D-derive-last-touched-task3
a852a20 feat: implement 10.D-derive-last-touched-task2
1c0caa9 feat: implement 10.D-derive-last-touched-task1
```

---

## [run: 2026-07-28]

### MV.10.C — mev emit-block-graph CLI subcommand; PASS

Ran `/sdlc-task 10.C-emit-block-graph-cli` in-place on `main`, 6 tasks all PASS (commits
`4d83548`..`6f6972a`). Added `dependent_count: u32` to `BlockGraphNode` — a corpus-wide,
scope-stable count of distinct in-corpus `BlockedBy` dependents (`CrossRepo` excluded).
Added epic-slug validation to `block_graph_brain` (unknown/blank `--epic` now a hard `Err`
before any corpus loading). Wired `mev emit-block-graph`, an 8-flag clap subcommand (`--scope
hq|tier|repo|epic`, `--tier`, `--epic`, `--repo`, `--include-closed`, `--include-boundary`,
`--max-nodes`, `--pretty`) mirroring `emit-graph`'s CLI pattern, with pre-filesystem validation
of scope/flag pairing. 14 new CLI integration tests (`tests/emit_block_graph_cli.rs`) covering
every scope flag, validation exit codes, determinism, and the disk-untouched guarantee.
`docs/cli.md` documents the subcommand. Two amendment-logged deviations: `--include-boundary`
added as an eighth flag to reach all seven scope-pipeline stages from the CLI; epic validation
lives in `block_graph_brain` rather than `main.rs` since it needs the loaded corpus. Low-effort
`/code-review --fix` found no issues. `/close-out` found `docs/architecture.md` stale against
this session's two new behaviors (`dependent_count` field, epic-registry validation) and
patched it surgically (commit `54ad725`). All four harness gates green throughout (415+ tests).
Unblocks `MV.10.D`.

### MV.10.B close-out — manual testing found and fixed a truncation/edges bug

Before merging PR #21, manually exercised `block_graph_brain` against the live brain corpus
(362 nodes / 562 edges full scope) via a scratch example script across five scope combinations
(tier/repo/epic/boundary/closed) — all behaved correctly. Found one real issue: `max_nodes`
truncation (Stage 7) capped `nodes` but left `edges` computed against the pre-truncation scope
(Stage 6), so a truncated export (e.g. 5 of 362 nodes) still returned all 562 edges, most naming
`from`/`target_node_id` keys absent from the returned `nodes` array — internally inconsistent for
any graph-rendering consumer (`MV.10.C`'s CLI, bastion's `BA.17.A`). Fixed by re-filtering
`edges` against the truncated node set in Stage 7, mirroring Stage 6's rule; added a regression
test (`max_nodes_truncation_drops_edges_whose_endpoints_were_truncated_out`) proving a
single-node truncation drops every edge in the fixture. `docs/architecture.md`'s seven-stage
pipeline section and the module-level doc comment updated to state the re-filter. All four
harness gates re-confirmed green (full suite passing, including the new regression test).
Pushed to PR #21.

### MV.10.B — Enriched block-graph exporter (block_graph.rs); PASS

Ran `/sdlc-flow 10.B-block-graph-exporter` (branch `10.B-block-graph-exporter-flow`), 6 tasks all
PASS, final review verdict PASS. Task 1 built the `src/brain/block_graph.rs` scaffold —
`BlockGraphExport`/`BlockGraphNode`/`BlockGraphEdge`/`BlockLane`/`BlockGraphScope`/
`BlockGraphScopeEcho` types plus the full-corpus derivation pipeline (lane from `derive_focus` with
repo-slug prefixing, cycle-safe longest-path `layer` restricted to `BlockedBy` edges with back-edges
contributing 0, `in_cycle` from `cycle_paths`), registered in `src/brain/mod.rs`. Task 2 layered the
seven-stage scope filter (tier → repo → epic-overrides-tier → closed → boundary → edges → truncate)
strictly after derivation via a `node_meta` index, retaining boundary/dangling edges and tracking
`truncated`/`total_nodes`. Task 3 added the `block_graph_brain` driver in `src/lib.rs` (mirrors
`graph_brain`, reuses `emit_state`'s corpus-load pipeline, skips malformed `state.json` files) and
re-exported the public surface from the crate root. Task 4 added `tests/brain_block_graph.rs` — a
9-node multi-repo/multi-tier fixture covering HQ/tier/repo/epic scoping, `include_closed`/
`include_boundary`, `max_nodes` truncation, byte-identical determinism, a dual-role brain fixture, and
a dangling edge. Task 5 documented the exporter in `docs/architecture.md` alongside the existing
Graph exporter section. Task 6 was validation-only — all four harness gates green (fmt, clippy `-D
warnings`, 413 tests, release build), scope boundaries confirmed clean (no `okf-core` changes, no CLI
subcommand, `docs/cli.md` untouched per MV.10.C's scope). No amendments — all decisions were
within-spec interpretations of stated ambiguity (e.g. `layer` computed only over `BlockedBy` edges,
matching `cycle_paths`/`effective_priorities` precedent). This closes Phase 10's `MV.10.A → MV.10.B`
spine and unblocks `MV.10.C` (the CLI subcommand) and bastion's `BA.17.A` endpoint.

Next: `MV.10.C` — `mev emit-block-graph` CLI subcommand, or `MV.ticket.distill-freshness-lane`.

```
45ac2cc feat: implement 10.B-block-graph-exporter-task5
a08c337 feat: implement 10.B-block-graph-exporter-task4
aad39d6 feat: implement 10.B-block-graph-exporter-task3
b98e2ce feat: implement 10.B-block-graph-exporter-task2
c66acf1 fix: fix pass 1 for 10.B-block-graph-exporter-task1
cd1bac3 feat: implement 10.B-block-graph-exporter-task1
1fd061f docs: document topo_order + cycle_paths public API (MV.10.A close-out)
50296da feat: implement 10.A-topo-order-cycle-paths-task2
```

---

## [run: 2026-07-28]

### MV.10.A — Extract topo_order + cycle_paths primitives; close-out

Ran `/sdlc-task 10.A-topo-order-cycle-paths` (in-place, main), 3 tasks all PASS. Task 1 extracted
`pub fn topo_order(graph, files) -> Vec<String>` (`src/brain/emit.rs`) — the cycle-safe, wave-seeded
DFS topological sort previously inlined in `epic_members` — leaving `epic_members` a thin filter
over it with an unchanged signature; 4 new unit tests. Task 2 extracted `pub fn cycle_paths(graph)
-> Vec<CyclePath>` (`src/brain/state.rs`) — canonical-rotation-deduplicated cycle finder — out of
`detect_cycles`'s DFS, rewriting `detect_cycles` as a thin formatter over it that preserves
byte-identical `E_STATE_CYCLE` messages; new unit tests plus a parity test against `detect_cycles`.
Task 3 was validation-only (no source changes needed).

Closed out with `/close-out`: all four harness gates green (fmt, clippy `-D warnings`, cargo test,
release build) plus the emoji gate; coverage confirmed adequate (both new public functions carry
dedicated tests, no blocking gaps); `/code-review low` (run directly by the user, since
`/code-review` cannot be invoked by the agent — `disable-model-invocation`) found zero issues,
confirming the refactor is behavior-preserving with no stale call sites; `docs/architecture.md`
patched to document `topo_order` and `cycle_paths`/`CyclePath` in their respective Public
functions / State types tables (previously undocumented new public API).

**Refs:** `MV.10.A`, unblocks `MV.10.B` (enriched block-graph exporter, consumes both primitives).

### MV.9.A — Generic doc-materializer engine + CLI + Opportunity command family

Implemented the mev-side half of D53 (mev writes source `.md`; engine-rs executes) across six
tasks, all PASS. Task 1 built `mev::doc::plan_document`, a generic `EmitPlan` planner over any
`okf_core::doc::model::BrainDocModel` — proven against all three sketched models (`Opportunity`,
`LearningArtifact`, `Proposal`) through the same code path — deriving the target path from
`IndexIntent`, re-splicing `Generated` sections via the existing `splice_generated` seam, and
no-opping with `W_DOC_UNCHANGED` when nothing changed; a missing sentinel pair raises
`W_DOC_MISSING_SENTINEL` rather than silently clobbering hand-edited content. Task 2 added
`plan_index_reconcile`, an idempotent `index.md` row upsert (matched on `link_target`) merged into
`plan_document` via `EmitPlan::extend`. Task 3 added `src/doc/opportunity.rs`'s four planners
(`plan_ingest` with kind auto-detect and a `job-posting` kind, `plan_set_stage`, `plan_add_action`,
`plan_merge_contacts`), all idempotent, backed by a real Anthropic `CompanyBrief` fixture and 13
integration tests. Task 4 wired `mev doc materialize` and `mev doc opportunity
{ingest,set-stage,add-action,merge-contacts}` CLI subcommands plus stable `mev::doc_*` library
runners, exercised end-to-end against the built binary. Task 5 documented the full `doc` command
family and the materializer's place in the `EmitPlan`/`apply_plan` seam in `docs/cli.md` and
`docs/architecture.md`. Task 6 confirmed all four harness gates green and ran a dry-run opportunity
ingest against the real brain corpus — it wrote nothing and `validate-brain` reported zero errors.
Final review verdict: **PASS**. Notable decisions: an `E_DOC_UNKNOWN_MODEL` diagnostic was added
beyond the original spec's list so `doc materialize --model <bad>` still exits 1 with a reportable
code; `add-action --at` defaults to today's date when omitted; `load_opportunity()` maps both a
missing file and a frontmatter parse/reconstruct failure to the single documented `E_DOC_NOT_FOUND`
code. Closes Phase 9's opening block (`MV.9.A`); engine-rs node/workflow wiring (`EN.7.A`/`EN.7.B`)
and the Synapse harvest hop (`EN.7.C`) remain out of scope per the block's Notes.

Next: pick the next phase/block per `master-plan.md` (no open mev-tracked spec).

```
c906ef7 feat: implement 9.A-doc-materializer-task5
733d995 feat: implement 9.A-doc-materializer-task4
6c6533f feat: implement 9.A-doc-materializer-task3
80779fd feat: implement 9.A-doc-materializer-task2
a21f478 feat: implement 9.A-doc-materializer-task1
```

---

## [2026-07-24]

### MV.8.A — Epics: a cross-repo initiative axis for `state.json`

- **What:** Added `epics` — a **multi-valued** membership field on blocks plus an HQ-only `epics[]`
  registry — and everything that consumes it.
  - **Schema (`okf-core`, D15/D16 single source):** `epics: Vec<String>` on `TrackBlock`, `Block`,
    and `StateNode`; new `Epic` struct (`slug`/`title`/`description`/`status`/`plan`/`repos`);
    `epics: Vec<Epic>` on `StateFile`; membership copied onto graph nodes by `build_state_graph`.
    Every field is `skip_serializing_if` empty — verified against a HEAD-built binary that all ~292
    untagged blocks stay byte-identical (identical emit plans on the same corpus).
  - **Validation (`src/brain/state.rs`):** `check_epics` — corpus-level, like
    `check_backlog_integrity` rather than the per-file `check_field_policy`, since membership is
    checked against a registry in another file. Six locators: `E_STATE_UNKNOWN_EPIC`,
    `E_STATE_DUPLICATE_EPIC_SLUG`, `E_STATE_EPIC_BAD_STATUS`, `W_STATE_EPIC_EMPTY`,
    `W_STATE_EPIC_UNREACHABLE_DEP`, `W_STATE_EPIC_REGISTRY_IGNORED`.
  - **Derivation:** `derive_epic_focus` filters `derive_brain_focus`'s *output* (so an epic board
    cannot disagree with the unified board); `derive_epic_edges` computes cross-epic relationships
    from the block `depends_on` graph — **no epic-level `depends_on` was added**, per the D36
    graph-vs-narrative litmus; `epic_members` delegates ordering to `wave_order`.
  - **Emit:** `render_epic_board` / `render_epic_sequence_table` / `plan_epic_boards` /
    `plan_epic_sequences` + the `epic-board` and `epic-sequence` markers, wired into `emit_state`
    **after** the first apply batch. Also parameterized `render_unified_board_section` with a
    heading level — epic lanes were rendering `## NOW` beneath an `### Epic` heading.
  - **Data:** 3-epic registry (`bastion-os`, `bastion-surfaces`, `engine-split`), **157 core-tier
    blocks tagged**, sentinels in both status docs, and new `core/planning/epics/` sequence docs.
  - Refactored the six `Block` construction sites in `derive_rollup` / `derive_brain_focus` onto
    shared `track_block_index` + `focus_block` helpers (a 3-tuple lookup MV.6.B had already widened
    once).
- **Why:** `tracks[]` groups work *within* one repo and `tier` groups repos organizationally.
  Neither can express a program like "Bastion Web + UI against the `bastion serve` endpoint", which
  spans three repos and interleaves with unrelated work in every existing view — so answering "what
  is the sequence for just this initiative, and what gates it" meant reading three `state.json`
  files and a prose master plan and joining them by hand. The graph already had `depends_on`,
  `wave`, `status`, and effective priority; the only missing piece was a grouping label that crosses
  repo boundaries.
- **Verification:** mev 625 / okf-core 56 / bastion 1457 tests passing; `fmt` + `clippy -D warnings`
  clean in all three; `validate-brain --state` 0 errors with zero epic diagnostics; `emit-state`
  reaches a fixed point.
- **Follow-ups:** four, captured as `carryover[]` rather than blocks (not yet scoped) —
  `bastion-web-external-deps-not-block-edges` and `epic-taxonomy-open-calls` (core tier),
  `emit-state-same-file-batching` and `epic-sequence-wave-scale` (mev). See `planning/handoff.md`.
- **Refs:** `planning/handoff.md`, `docs/state/state-schema.md` (`epics[]`),
  `core/planning/epics/index.md`

---

## [2026-07-14]

### 7.A-effective-priority-inheritance shipped — full spec, PASS
- **What:** Implemented `effective_priorities(graph, files) -> HashMap<String, u8>`
  (`src/brain/state.rs`): a memoized, cycle-safe reverse-topological `min`-propagation over the
  `depends_on` DAG — `effective(n) = min(own(n), min{ effective(m) : m depends_on n })`, keyed by
  `"repo:id"`. On hitting a node already on the DFS recursion stack the pass returns that node's own
  priority rather than re-recursing, guaranteeing termination on a cycle without hanging or
  panicking. Threaded through the unified board's `NEXT` sort: `render_unified_board` gained an
  `effective: &HashMap<String, u8>` parameter; `sort_unified_board_next` now sorts via a new
  `effective_priority_for` helper (effective map → raw `priority` → `u8::MAX` fallback chain) instead
  of raw `priority` directly; `plan_unified_board` computes the map once via
  `effective_priorities(graph, files)` and passes it down. 5 new unit tests in `state.rs` (gating
  inheritance, two-hop chain propagation, no-hotter-dependents, absent-priority, cycle termination)
  plus 2 new integration tests in `tests/brain_emit.rs` (explicit-override sort behavior, and a full
  `mev::emit_state` gating + idempotency proof); all 9 pre-existing `render_unified_board` call sites
  updated for the new parameter. Docs patched: `docs/architecture.md` (`effective_priorities`,
  `render_unified_board`, `plan_unified_board` rows) and `docs/cli.md` (`emit-state` unified-board
  `NEXT` sort description); `core/planning/state-schema.md` was also edited (brain repo, one level
  up) but correctly left uncommitted here — that's a separate brain-level commit.
- **Review:** PASS on the first attempt — all acceptance criteria met, all four gating checks (fmt,
  clippy `-D warnings`, cargo test, release build) green.
- **Decisions:** reverse-adjacency direction confirmed against the spec's "gates" language (the
  *dependency* node inherits its hottest *dependent*'s priority, i.e. `biz depends_on eng` means
  `eng`'s effective value rises to match `biz`'s); `effective_priority_for`'s three-way fallback chain
  keeps `render_unified_board` callable with an empty map for non-gating tests while degrading
  gracefully to pre-MV.7.A sort behavior; cycle-guard returns the stuck node's own priority as a
  deterministic tie-break rather than `u8::MAX` or a panic.
- **Refs:** `planning/7.A-effective-priority-inheritance/tasks.md`; commits `5498ece` (impl),
  `5f836ad` (docs). This closes Phase 7 (Priority inheritance) entirely.

Next: no open mev-tracked spec — pick the next phase/block per `master-plan.md`.

```
5f836ad docs: update docs for 7.A-effective-priority-inheritance
5498ece feat: implement 7.A-effective-priority-inheritance
9beaccb docs: log MV.5.A tracker reconciliation
d3fca28 test(brain-emit): remove unused PathBuf import
80722d1 chore: sync harness command updates
```

---

### MV.5.A tracker reconciliation — block was already shipped
- **What:** A `/generate-tasks MV.5.A` request revealed that MV.5.A ("Status Frontmatter
  Reconciler", the ad-hoc block in `planning/plan-state-yaml-drift/plan.md`) was **already fully
  implemented and tested** — it landed in `46b2e4e` but the trackers were never reconciled, leaving
  `state.json` `open`, `status.md` `next` listing it, and `tasks.md` "Not started." Confirmed the
  deliverables exist and pass: `reconcile_status_scalars` (`src/brain/emit.rs:1005`),
  `plan_status_frontmatter` (`src/brain/emit.rs:1573`), wired into `emit_state` (`src/lib.rs:574`),
  3 green tests in `tests/brain_emit.rs`. Marked the block `closed` in `state.json`, ran
  `mev emit-state --write` (0 errors — dropped MV.5.A from derived `focus`, leaving `MV.7.A`), flipped
  `tasks.md` to Done, and removed an unused `PathBuf` import from the block's test module. All four
  harness gates green (fmt, clippy `-D warnings`, 19 test binaries / 0 failures / 0 warnings, release).
- **Why:** Shipped work left `open` in `state.json` is a recurring drift (this repo has hit it before
  with engine-rs `state-json-block-status-stale` and the two tickets reconciled 2026-07-14). Closing
  the gap keeps `focus` derivation and every generated surface honest, and prevents an SDLC engine from
  being pointed at an already-done block.
- **Refs:** `planning/plan-state-yaml-drift/plan.md` (MV.5.A), commit `46b2e4e`.

---

## [2026-07-05]

### D44 core Cargo workspace REMOVED (D45) — worktree-validation break resolved
- **What:** Resolved the `core-cargo-workspace-breaks-worktree-validation` carryover by **removing
  the workspace outright** rather than adding shim/relocation machinery. Re-verified the root cause
  empirically (bug reproduces; `workspace.exclude` cannot carve a path out of a member's own directory
  — tested `mev/trees`, `mev/trees/<pkg>`, `mev/trees/*`, all fail with the same "believes it's in a
  workspace" error). Then assessed the workspace's actual payoff and found it mostly unrealized: no
  automation runs `cargo test --workspace` (every core repo's `harness.json` runs plain
  `cargo test`/`clippy`/`build` from its own dir); `mev` + `claude-code-rs` already carried their own
  `Cargo.lock` so the "single lock" invariant wasn't even true; and shared `core/target/` is forfeited
  in worktrees under any candidate fix anyway. Deleted `core/Cargo.toml` + `core/Cargo.lock`. Verified
  all four members (`bastion`, `okf-core`, `mev`, `claude-code-rs`) resolve **standalone** via unchanged
  path-deps, and a nested `core/<member>/trees/<branch>` package now builds — the break is gone with
  **zero tooling changes** (worktrees nest exactly as the shared `.claude/workflows/*` engines already
  create them). mev gates green standalone: fmt PASS, clippy `-D warnings` clean, 567 tests 0 failed,
  release build OK. Authored `docs/decisions/D45` (supersedes D44's workspace clause **only** — the
  okf-core promotion to a first-class repo and the rest of D44 stand); updated `.gitignore`,
  `core/README.md`, `core/planning/core-rust-workspace/notes.md`, and the decisions index;
  `bastion` + `okf-core` regenerated their own standalone locks. Cleared the carryover, consumed the
  handoff.
- **Why:** The prior session left the fix as a deferred user decision. Investigation this session showed
  `exclude` cannot fix a nested-under-member path (no cheap third option), and that the workspace's
  benefits were largely latent while it permanently broke the primary dev workflow (nested SDLC
  worktrees) for all four core-tier Rust repos. Removal was the lowest-complexity resolution that keeps
  every functional property (path-dep coupling) intact.
- **Refs:** `docs/decisions/D45-revert-core-cargo-workspace.md`, `docs/decisions/D44-core-cargo-workspace.md`,
  `core/planning/core-rust-workspace/notes.md`.

### D44 worktree-validation break — root cause confirmed, fix deferred
- **What:** Investigated the `core/Cargo.toml` D44 worktree-validation break carried over from the
  prior close-out (`core-cargo-workspace-breaks-worktree-validation`). Root cause confirmed by direct
  testing: `workspace.exclude` cannot carve an exception out of a path nested inside an
  already-declared member's own directory — a sibling scratch crate excludes cleanly, but the
  identical crate nested one level inside `bastion/` fails regardless of exclude pattern (literal or
  glob). This is a hard Cargo limitation, not a syntax error. Two real fix options identified, neither
  implemented — deferred to a fresh session per user request: (1) relocate SDLC worktrees for the four
  core-tier Rust repos to a sibling path like `core/.worktrees/<repo>/<branch>`, which requires editing
  the shared `.claude/workflows/*.js` worktree-creation logic; (2) keep `trees/` nested and formalize
  the manual empty-`[workspace]`-table shim into an automated wrapper. Also ran
  `mev emit-state --write` from `main` (deferred from the prior close-out session) — succeeded,
  unblocked `MV.7.A`, and populated the unified-board region at the HQ root for the first time.
- **Why:** The prior close-out session left this as an open known-issue carryover; this session's job
  was to pin down the actual mechanism before committing to either fix, and to run the deferred
  brain-wide `emit-state` regeneration now that `main` had the merged changes.
- **Refs:** `planning/handoff.md` (full investigation + both fix options), `core/Cargo.toml` (D44),
  `.claude/workflows/*.js` (worktree-creation logic, fix option 1)

### `6.B-generate-hq-board-flow` — close-out: coverage-gap fix + state reconciliation
- **What:** Closed out `6.B-generate-hq-board-flow` (MV.6.B, unified priority board). Found and
  fixed a blocking test-coverage gap: `plan_unified_board` had zero tests despite being new public
  API wired into `emit_state` — added 6 tests mirroring the `plan_hq_board` suite (splice, missing
  sentinel, missing file, tier-skip, fixed-point, marker constant). All four harness gates
  (fmt/clippy/test/build) pass; code review clean; docs already current. Marked `MV.6.B` `closed`
  in `planning/state.json` (was drifted from `status.md`).
- **Why:** Close-out review surfaced the gap between "new public API wired into the emit pipeline"
  and "covered by tests," which the standing rule (every behaviour change ships with tests) requires
  closing before merge.
- **Refs:** `planning/state.json` (`MV.6.B`), `planning/handoff.md` (carryover), `core/Cargo.toml` (D44)
- Added a known-issue carryover: `core/Cargo.toml`'s new unified workspace (D44) breaks `cargo`
  commands run from inside any `core/*/trees/*` worktree; `workspace.exclude` does not fix it — the
  verified workaround is a local, never-committed empty `[workspace]` table in the worktree's own
  `Cargo.toml`. `mev emit-state --write` correctly refused to run from this linked worktree
  (`E_EMIT_LINKED_WORKTREE`) — brain-wide regeneration is deferred to a run from `main` after merge.

### `6.B-generate-hq-board` — carry priority/due through focus + unified HQ board region
- **What:** Ran `/sdlc-flow 6.B-generate-hq-board` to completion (6/6 tasks passed, review PASS). Task 1
  fixed both derived-focus rehydration paths — `emit.rs::derived_focus_for` and
  `state.rs::derive_brain_focus` — to copy the source `TrackBlock`'s `priority`/`due` onto the
  constructed `Block` instead of hardcoding `None` (introduced `BlockIndexEntry` type alias in
  `emit.rs` to satisfy `clippy::type_complexity`). Task 2 added `markers::UNIFIED_BOARD` and a pure
  `render_unified_board(focus, edges, config, today)` rendering NOW/NEXT/BLOCKED/DUE-SOON sections
  tagged `[BIZ]`/`[ENG]` by source-repo tier, with `NEXT` stably re-sorted by `(priority asc, due asc)`
  (wave implicit tiebreak) and DUE-SOON (due ≤ today+14d, overdue included) sorted by due ascending.
  Task 3 added `plan_unified_board` (mirroring `plan_hq_board`) and wired it into `emit_state`
  (`src/lib.rs`) after `plan_hq_board`, using `chrono::Local::now().date_naive()` as the reference
  date. Task 4 added an end-to-end integration test driving `emit_state` over a business+engineering
  fixture, asserting tagging, priority-based ordering, DUE-SOON window behavior, and fixed-point
  idempotence. Task 5 added the `unified-board` sentinel region to the HQ root
  `../../planning/status.md` (applied directly in the main company-brain checkout, outside the mev
  worktree — no mev-repo commit for this task). Task 6 confirmed all four harness gates green
  (fmt, clippy `-D warnings`, test, release build). Final review verdict: PASS. The existing
  `hq-board` sentinel, per-domain lanes, and `/biz-status` are unchanged. This closes out
  `6.B-generate-hq-board` in full.
- **Why:** Second step of operationalizing the Statify Business roadmap inside `mev` — gives the HQ
  brain a single priority-ranked board unioning business and engineering blocks, per
  `core/planning/statify-business/master-plan.md` (D43).
- **Refs:** `planning/6.B-generate-hq-board/spec.md`, `planning/master-plan.md`.
  ```
  7dcbb94 chore: flow state — docs
  1bdd8de docs: update docs for 6.B-generate-hq-board
  3d949ed chore: flow state — task 6 passed
  2c630ad chore: flow state — task 5 passed
  ae4ea5e chore: flow state — task 4 passed
  6b460fd feat: implement 6.B-generate-hq-board-task4
  a7864d7 chore: flow state — task 3 passed
  b8d26a2 feat: implement 6.B-generate-hq-board-task3
  ```

### `6.A-validate-new-fields` — implement validation for new block fields
- **What:** Ran `/sdlc-task 6.A-validate-new-fields` to completion (all tasks passed). Implemented validation for the four new optional `Block` / `TrackBlock` fields (`priority`, `due`, `sdlc_workflow`, `model`) inside `src/brain/state.rs`. Added tests verifying `priority` range (0..=3), `due` format (YYYY-MM-DD), and `sdlc_workflow`/`model` enums, resulting in 4 new diagnostic codes (`E_STATE_PRIORITY_RANGE`, `E_STATE_DUE_FORMAT`, `E_STATE_SDLC_WORKFLOW_ENUM`, `E_STATE_MODEL_ENUM`). Refactored struct initializers in tests and the CLI to support the new fields explicitly. Added integration tests to `tests/brain_state.rs` and patched `docs/cli.md` with the new codes. Reconciled `MV.6.A` to `closed` in `planning/state.json`. All four harness gates (fmt, clippy, test, build) and the emoji gate passed cleanly.
- **Why:** This is the first step (Block MV.6.A) in operationalizing the Statify Business roadmap inside `mev`, introducing these fields to the state graph ahead of generating the unified HQ board region (MV.6.B).
- **Refs:** `planning/master-plan.md`, `core/planning/state-schema.md`.

## [2026-07-04]

### `update-write-state-in-trees` — guard emit-state --write against linked worktrees
- **What:** Ran `/sdlc-task update-write-state-in-trees` in place on `main` (5/5 tasks, all passed).
  Added `is_linked_worktree(path: &Path) -> bool` to `src/brain/config.rs` (compares canonicalized
  `git rev-parse --git-dir`/`--git-common-dir`, fails open to `false` on any git error or non-repo
  path); gated `Command::EmitState { path, write }` in `src/main.rs` to refuse with a new
  `E_EMIT_LINKED_WORKTREE` diagnostic and non-zero exit when `--write` is invoked from inside a
  linked worktree, leaving dry-run and main-tree behavior unchanged; added CLI-level test coverage
  (`tests/brain_emit.rs`) covering the refusal, the dry-run pass-through, and a main-tree regression
  guard; documented the new code in `docs/cli.md`'s `emit-state` section. All four harness gates
  green. Committed directly to `main` (`66cfd1a`..`957b4f1`). Also merged PR #18
  (`4.E-emit-state-wiring`, carried over from the prior session) and ran `mev emit-state --write`
  from the main checkout to regenerate the brain-wide derived views — flipped
  `MV.ticket.update-write-state-in-trees` to `closed` in `planning/state.json` first so the focus
  derivation picked it up; `focus` is now empty (no open mev-tracked work).
- **Why:** Live investigation in the agentic-portfolio brain traced a mysterious "reverting" of
  uncommitted `state.json` edits to `emit-state --write` always resolving every repo's derived-file
  paths from `brain.toml` (never CWD) — so running it from a linked `trees/<slug>` worktree silently
  clobbered the *main* checkout's files with no git operation involved. Concurrent `sdlc-flow`/
  `sdlc-block` worktrees calling `emit-state --write` (via `/log-work` and `sdlc-block.js`) turned
  this into a last-writer-wins race on shared files. Refusing from a worktree is the correct fix —
  a worktree's local state isn't the right input for regenerating shared derived surfaces anyway.
- **Refs:** `planning/update-write-state-in-trees/tasks.md`; PR #18 (merged 2026-07-04T12:52:52Z);
  `planning/handoff.md` (now consumed, to be deleted).

## [run: 2026-07-04]

Shipped `4.E-emit-state-wiring` (Phase 4, state-sync-loop spine terminus) across the full spec, final verdict PASS (4 tasks). Task 1 wired all three previously-standalone planners into `emit_state` (`src/lib.rs`): it now calls `plan_state_json`, `plan_master_plan_tables`, `plan_project_caches`, `plan_tier_rollups`, and `plan_hq_board` — in that stable order — applying each via `apply_plan(&plan, write)` and merging diagnostics; the doc comment was updated to name all five generated surfaces. No planner signatures or behaviour changed — pure wiring, per the task spec. Task 2 added a new `mv4e_ripple` integration test module in `tests/brain_emit.rs`, building a real on-disk fixture (brain.toml + HQ + one tier sub-brain + two leaf project repos) and proving that a single `emit_state(&dir, true)` call ripples a close-A-unblocks-B cross-repo dependency change (repo-a's `RA.1.A` flipped `in_progress` → `closed`, unblocking repo-b's `RB.1.A`) across every generated surface at once: the leaf `state.json` focus, the leaf project-cache doc + its `synced_from` watermark, the tier rollup table, the HQ operating board (the closed block drops out of BLOCKED/NOW/NEXT everywhere; the dependent block moves from BLOCKED to open), and the master-plan wave table — plus a fixed-point check that a second pass over the emitted corpus is byte-identical with zero `I_EMIT_WROTE` diagnostics. Task 3 patched `docs/cli.md`'s `emit-state` section to document the three newly wired surfaces (project caches, tier rollups, HQ board) and their sentinel markers, generalizing the sentinel-contract prose to name all four markers rather than duplicating near-identical blocks. Task 4 ran the full validation suite (fmt, clippy `-D warnings`, test, release build) — all four gates green, no fixes needed, no commit required (nothing to commit). Final review verdict: PASS. This closes Phase 4 (state-sync-loop) entirely — the spine `MV.4.A → {B,C,D} → E` is fully Done, with a single `mev emit-state --write` now refreshing every human-read status surface (leaf focus, brain rollup, master-plan tables, project caches, tier rollups, HQ board) in one pass. Next: pick the next phase/spec per `master-plan.md` — Phase 4 is complete; the out-of-repo orchestrator follow-up (`load_brain_edges.py` reading mev's exported edge fields directly) remains outstanding as separate work.

```
216c7fa chore: flow state — docs
605eff0 docs: update docs for 4.E-emit-state-wiring
4454d0d chore: flow state — task 4 passed
90aeaae chore: flow state — task 3 passed
471d0d5 docs: update docs/cli.md emit-state section for MV.4.E surfaces
d7de03c chore: flow state — task 2 passed
92a1c02 feat: implement 4.E-emit-state-wiring-task2
616a097 chore: flow state — task 1 passed
```

---

## [2026-07-04]

### Close-out: 4.E-emit-state-wiring
- **What:** Closed out `4.E-emit-state-wiring`: gates green, coverage adequate, docs already patched in-flow. Reconciled `MV.4.E` to `closed` in `planning/state.json` (Phase 4 state-sync-loop fully done). PR #18 open, not yet merged.
- **Why:** Standard close-out gate after `4.E-emit-state-wiring` (Phase 4 spine terminus) shipped via `sdlc-flow` — confirm gates/coverage/docs are genuinely green and reconcile the authored state tracker so Phase 4 is provably closed before merge.
- **Refs:** `core/planning/state-sync-loop/master-plan.md` (spine `MV.4.A → {B,C,D} → E`); PR #18; `planning/state.json`.

### Close-out: 4.D-sync-comparator-hardening
- **What:** Ran `/close-out` for `4.D-sync-comparator-hardening` following its `/sdlc-run` completion (PASS, commits landed directly on `main` — no worktree/PR this time). Re-verified all four harness gates (`cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test` — 314 tests, `cargo build --release`) plus the emoji gate, all green. Coverage scan found the one behavioral change (the `.to_utc()` instant-comparison hardening in `check_sync`) already covered by the two new regression tests added in-flow — no blocking gaps. Docs audit found no STALE/MISSING items (`docs/cli.md`'s `--sync` section was already patched in-flow to describe the instant-based comparison). Hand-edited `planning/state.json`'s authored `tracks[]`: `MV.4.D` flipped `open` → `closed`. Ran `mev emit-state --write` from the main checkout per protocol — regenerated derived `focus`: `next` is now just `MV.4.E` with `blocked_by: []` (all three prerequisites — `MV.4.B`, `MV.4.C`, `MV.4.D` — now closed), `blocked` is now empty. 0 errors.
- **Why:** Standard close-out gate after `4.D-sync-comparator-hardening` shipped directly to `main` — confirm gates/coverage/docs are genuinely green and reconcile the authored state tracker so `MV.4.E` (the Phase 4 spine terminus) is provably unblocked before the next session picks it up.
- **Refs:** `core/planning/state-sync-loop/master-plan.md` (spine `MV.4.A → {B,C} → E`); `src/brain/sync.rs`; `planning/state.json`; commits `c991a30`, `b0dea52`, `e723c9c`.

---

## [2026-07-04 — run date +%Y-%m-%d]

Shipped `4.D-sync-comparator-hardening` (Phase 4, state-sync-loop Wave 3) across the full spec, final verdict PASS (review attempt 1 of 3). Hardened `check_sync`'s watermark comparison in `src/brain/sync.rs` (~line 211) from a bare `DateTime` `!=` to an explicit UTC-instant compare (`source_dt.to_utc() != cache_dt.to_utc()`), with a new doc comment stating the invariant that a `-03:00` and a `Z` watermark denoting the same moment are in sync. This was an investigation-confirmed hardening rather than a bug fix: chrono's `DateTime<FixedOffset>` `PartialEq`/`Ord` was already instant-based, so no behavior changed for any existing case — the change makes that guarantee explicit and legible so a future refactor can't silently regress it into a string/offset-sensitive compare. Closed the test gap the spec identified: two new regression tests in the inline `#[cfg(test)] mod tests` — `same_instant_across_offsets_produces_no_e_sync_drift` (`2026-06-27T00:00:00Z` vs `2026-06-26T21:00:00-03:00`, same instant, no diagnostics) and `different_instant_across_offsets_produces_e_sync_drift` (same offsets, different instant, exactly one `E_SYNC_DRIFT`). All four harness gates green (`cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test` — 314 tests, `cargo build --release`); review re-ran all four fresh and confirmed. `docs/cli.md`'s `--sync` section was patched to describe the instant-based comparison and the `E_SYNC_DRIFT` locator wording updated to "denote different instants". This closes Wave 3 of the state-sync-loop spine, leaving `MV.4.E` (spine terminus, wires `MV.4.B`+`MV.4.C` planners into `emit_state`) as the sole remaining Phase 4 block. Next: `MV.4.E`.

```
b0dea52 docs: update docs for 4.D-sync-comparator-hardening
c991a30 feat: implement 4.D-sync-comparator-hardening
caf52f1 chore: add spec for 4.D-sync-comparator-hardening
```

---

## [run: 2026-07-04]

Shipped `4.C-hq-board-emit` (Phase 4, state-sync-loop Wave 2) across 3 tasks, final verdict PASS. Task 1 added a pure `render_hq_board(focus, edges) -> String` (`src/brain/emit.rs`), rendering NOW/NEXT/BLOCKED sections as `repo:id — title` lines with cross-repo-edge/`blocked_by` annotations, plus a `task1_render_hq_board` test module in `tests/brain_emit.rs` covering rendering, ordering, edge-note annotation, `what`-fallback, external deps, multi-blocker joins, no-trailing-newline, and empty-focus cases; empty NOW/NEXT/BLOCKED sections render a literal `_none_` line rather than being omitted, and the renderer preserves the input `Focus` vector order rather than re-sorting (derive_brain_focus already establishes deterministic ordering). Task 2 added `plan_hq_board`, which locates the HQ brain's sibling `status.md` (the `state.json` parent, for the brain-kind file whose `tier_scope_for` resolves to `TierScope::All`), builds the board via `render_hq_board(derive_brain_focus(...), derive_cross_repo(...))`, and splices it into the `markers::HQ_BOARD` sentinel with fixed-point/no-sentinel-warning semantics mirroring `plan_project_caches`/`plan_tier_rollups`; confirmed the target-doc resolution against the real company-brain repo (`planning/state.json` sits beside `planning/status.md`, which already carries a hand-maintained "## Operating Board" section this planner will eventually generate); not wired into `emit_state` (MV.4.E's job). 6 new tests in a `task2_plan_hq_board` module cover splice-produces-expected-content, missing-sentinel-warns, missing-file-warns, tier-sub-brain-is-skipped, fixed-point-no-action-on-second-pass, and a marker-constant sanity check. Task 3 confirmed all four harness gates green (`cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test` — 76 tests, `cargo build --release`) with no code changes needed. Final review verdict: PASS. This closes Wave 2 of the state-sync-loop spine (`MV.4.A → {B,C} → E`) — both `MV.4.B` and `MV.4.C` are now Done, unblocking `MV.4.E`. Next: `MV.4.E` (spine terminus, wires `MV.4.B`+`MV.4.C` planners into `emit_state`).

```
7ab7b40 docs: update docs for 4.C-hq-board-emit
657d9d2 chore: flow state — task 3 passed
58b72a6 chore: flow state — task 2 passed
39c1c51 feat: implement 4.C-hq-board-emit-task2
02ea055 chore: flow state — task 1 passed
29d94e7 feat: implement 4.C-hq-board-emit-task1
9f21c53 chore: init worktree 4.C-hq-board-emit-flow
```

---

## [2026-07-04]

### Close-out: 4.C-hq-board-emit

- **What:** Ran `/close-out --clean-worktree` for `4.C-hq-board-emit` following its `/sdlc-flow` run (3 tasks, PASS, PR #17 opened, not yet merged). Re-verified all four harness gates (`cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test` — 312 unit tests + integration suites, `cargo build --release`) plus the emoji gate, all green. Coverage scan found no blocking gaps (test diff ~3x the source diff; `tests/brain_emit.rs` now carries 76 tests). Docs audit found `docs/architecture.md` already fully documents `render_hq_board`/`plan_hq_board` (patched in-flow by `sdlc-flow`'s own docs stage) — no STALE or MISSING items, no doc changes made this session. Hand-edited this worktree's `planning/state.json` authored `tracks[]`: `MV.4.C` flipped `open` → `closed` (it's in fact done — this reconciliation happens during close-out since the flow itself doesn't do it). Ran `mev emit-state --write` from the worktree per protocol; confirmed (as in the prior 4.B close-out session) it resolves `brain.toml`'s `[[repos]]` entry for `mev` to the canonical `core/mev` path (main checkout, still on `MV.4.C: open` since PR #17 hasn't merged), so it regenerated the main checkout's derived tables — not this worktree's own `state.json` — and left the main checkout git-clean. This worktree's derived `focus` section remains stale and will self-correct once the branch merges to main and `emit-state --write` runs there; not hand-edited.
- **Why:** `4.C-hq-board-emit` (`render_hq_board` + `plan_hq_board`, Phase 4 state-sync-loop Wave 2) shipped via `sdlc-flow` and needed the standard close-out verification pass before merge — confirm gates are still green, coverage is adequate, docs are current, and the authored state tracking is reconciled, so the branch is provably ready for `/clean-worktree`.
- **Refs:** `core/planning/state-sync-loop/master-plan.md` (spine `MV.4.A → {B,C} → E`); PR #17.

### Close-out: 4.B-cache-rollup-emit

- **What:** Ran `/close-out --clean-worktree` for `4.B-cache-rollup-emit` following its `/sdlc-flow` run (3 tasks, PASS, PR #16 opened, not yet merged). Re-verified all four harness gates (`cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test` — 312 tests, `cargo build --release`) plus the emoji gate, all green. Coverage scan found no blocking gaps (test file ~3x the size of the changed source, covers all new public functions `plan_project_caches`/`plan_tier_rollups`). Docs audit found `docs/architecture.md` already fully documents both functions (patched in-flow by `sdlc-flow`'s own docs stage) — no STALE or MISSING items, no doc changes made this session. Hand-edited this worktree's `planning/state.json` authored `tracks[]`: `MV.4.A` and `MV.4.B` flipped `open` → `closed` (both are in fact done; the file hadn't been synced since `4.A` merged). Attempted `mev emit-state --write` from the worktree per protocol; confirmed it resolves `brain.toml`'s `[[repos]]` entry for `mev` to the canonical `core/mev` path (main checkout), so it regenerated the main checkout's `state.json`/derived tables, not this worktree's — the main checkout stayed git-clean afterward and this worktree's own diff was unchanged. This worktree's derived `focus` section remains stale and will self-correct once the branch merges to main and `emit-state --write` runs there; not hand-edited.
- **Why:** Standard close-out gate before merging `4.B-cache-rollup-emit` — confirm gates/coverage/docs are actually green (not just claimed) and reconcile the state tracker before handing the branch off for merge, per the `/close-out` protocol.
- **Refs:** PR #16 (open); `core/planning/state-sync-loop/master-plan.md` (MV.4.A → {B,C} → E spine); prior entry below (`4.B-cache-rollup-emit` implementation, PASS).

## [run: 2026-07-04]

Shipped `4.B-cache-rollup-emit` (Phase 4, state-sync-loop Wave 2) across 3 tasks, final verdict PASS. Task 1 added `pub fn plan_project_caches` (`src/brain/emit.rs`), which splices a derived focus-line (new `render_focus_line` helper, format `**Current focus:** ... Next: ... Blocked: ...`) into each project-kind repo's `docs/projects/<slug>.md` project-cache sentinel and reconciles its `synced_from` watermark via a separate line-based splice (`reconcile_synced_from`) rather than routing through `okf_core::serialize_frontmatter` (which deliberately never emits `synced_from`), preserving all other frontmatter/prose verbatim; target-doc path resolved via `root.join(entry.cache_doc)` from `brain.toml`'s `[[repos]]` (matching `check_sync`'s existing convention, since real entries vary — e.g. `README.md` for the HQ root); 5 new integration tests. Task 2 added `pub fn plan_tier_rollups` (+ `render_tier_rollup_table` helper), resolving each tier rollup doc as `<tier state.json parent>/status.md` (sibling-to-state-file, mirroring `plan_master_plan_tables`'s resolution), splicing `derive_rollup`'s tier-scoped rows into `markers::TIER_ROLLUP`, and skipping any tier whose `tier_scope_for` resolves to `TierScope::All` (the HQ root — MV.4.C's `plan_hq_board` responsibility) with no diagnostic; rendered columns `Repo | Now | Next | Blocked`. Task 3 confirmed all four harness gates green (`cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, `cargo build --release`) with no code changes needed. Final review verdict: PASS. Neither planner is wired into `emit_state` — that remains `MV.4.E`'s job. This is Wave 2 of the state-sync-loop spine (`MV.4.A → {B,C} → E`). Next: `MV.4.C` (HQ board) or `MV.4.E` (spine terminus, once `MV.4.C` also lands).

```
6633959 chore: flow state — docs
18b4baa docs: update docs for 4.B-cache-rollup-emit
882431c chore: flow state — task 3 passed
b468302 chore: flow state — task 2 passed
38b6533 feat: implement 4.B-cache-rollup-emit-task2
22a1b1a chore: flow state — task 1 passed
300859d feat: implement 4.B-cache-rollup-emit-task1
```

---

## [run: 2026-07-03]

Shipped `4.A-emit-foundation` (Phase 4, state-sync-loop Wave 1) across 4 tasks, final verdict PASS. Task 1 added named generated-marker constants to `src/brain/emit.rs` — a `pub mod markers { pub const ... }` grouping (`WAVE_TABLE`, `PROJECT_CACHE`, `TIER_ROLLUP`, `HQ_BOARD`) — and updated `plan_master_plan_tables` to reference `markers::WAVE_TABLE` instead of the hardcoded `"wave-table"` string literal; no behavioural change. Task 2 added a pure `global_status_map(files)` helper (placed immediately after `render_wave_table`) that maps every loaded state file's `tracks[].blocks[]` to authored status keyed `"{repo_slug}:{block_id}"` across all repos, with 4 new unit tests covering multi-repo namespacing, no-collision, absent-status→`None`, and empty input. Task 3 threaded that map into `render_wave_table`, fixing the previously always-unmet cross-repo `depends_on` bug: a block depending on a closed cross-repo block now resolves `open` instead of `blocked` (absent/open cross-repo deps still render `blocked`); same-repo derivation and the unused `graph: &StateGraph` parameter were left unchanged per the spec; `plan_master_plan_tables` now builds the map via `global_status_map(files)` and threads it through; 7 existing `render_wave_table` call sites updated to pass an empty map (no behavior change for same-repo fixtures) plus 3 new cross-repo closed/open/absent tests. Task 4 confirmed all four harness gates green (`cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, `cargo build --release`) with no code changes needed. Final review verdict: PASS. This is Wave 1 of the state-sync-loop initiative's spine (`MV.4.A → {B,C} → E`) and unblocks `MV.4.B` (project caches + tier rollups) and `MV.4.C` (HQ board). Next: `MV.4.B` or `MV.4.C` per `core/planning/state-sync-loop/master-plan.md`.

```
780cb2b docs: update docs for 4.A-emit-foundation
9e07fc4 chore: flow state — task 4 passed
551e0e5 chore: flow state — task 3 passed
c07b102 feat: implement 4.A-emit-foundation-task3
8777f69 chore: flow state — task 2 passed
4ed69f2 feat: implement 4.A-emit-foundation-task2
cb75878 chore: flow state — task 1 passed
07006af feat: implement 4.A-emit-foundation-task1
```

---

## [2026-07-03]

### 4.A-emit-foundation close-out: gates re-verified, coverage scan clean, docs confirmed patched, handoff rewritten
- **What:** Ran a `/close-out` pass on `4.A-emit-foundation` after `/sdlc-flow` completed it (4 tasks, PASS, PR #15). Re-verified all four harness gates in the worktree — `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, `cargo build --release` — plus the emoji gate; all green. Ran a coverage gap scan over the session's changed source files (`src/brain/emit.rs`, `tests/brain_emit.rs`): every new public function has 5–27 direct test references, no blocking gaps found. Ran `/update-docs --patch`: `docs/cli.md` and `docs/architecture.md` already fully document `emit-state`/`emit_state` (patched in-flow by the earlier docs stage) — no STALE or MISSING items. Wrote a fresh `planning/handoff.md` superseding the stale one left over from the prior `ticket-ba15-12-okf-core-convergence` session, recommending the next agent run `/clean-worktree 4.A-emit-foundation-flow` to merge PR #15, then pick up `MV.4.B` or `MV.4.C`.
- **Why:** Standard end-of-flow close-out to confirm `4.A-emit-foundation`'s work is fully verified and documented, with no new gaps, before handing off cleanly to the next agent. No code changes were made — pure verification + a handoff rewrite.
- **Refs:** PR #15; `src/brain/emit.rs`; `tests/brain_emit.rs`; `docs/cli.md`; `docs/architecture.md`; `planning/handoff.md`.

---

### ticket-ba15-12-okf-core-convergence wrap-up: PR merge, worktree/stash cleanup, state.json carryover resolved, handoff
- **What:** Ran `/sdlc-flow` on `ticket-ba15-12-okf-core-convergence` (mev's half of bastion's `BA.15.12`/D9/D15/D16 cross-repo format convergence) to completion — all 6 tasks PASS, final review PASS, PR #14 opened. Ran `/code-review low` on the diff (clean, no findings), then merged PR #14. Fast-forward merged local `main` to `origin/main`, removed the completed worktree (`trees/ticket-ba15-12-okf-core-convergence-flow`), and deleted the local + remote feature branch. Found a stale pre-session git stash carrying an earlier draft of this ticket's `tasks.md`/`tasks.json`; diffed it against the merged, completed version, confirmed it was fully superseded, and dropped it. Removed the now-resolved `ba15-12-okf-core-convergence` carryover entry from `planning/state.json` and ran `mev emit-state --write` to reconcile derived rollups. Wrote `planning/handoff.md` for the next agent.
- **Why:** Standard end-of-flow cleanup after a shipped ticket — merge, prune the worktree/branch, and reconcile `state.json` so derived focus/rollup stay accurate; the stale stash was pre-session clutter left over from before this ticket's work began and needed to be verified-safe before dropping so no in-flight draft content was lost.
- **Refs:** PR #14; `planning/ticket-ba15-12-okf-core-convergence/tasks.md`; `planning/state.json`; `planning/handoff.md`; D9/D15/D16.

---

## [run: 2026-07-03]

Shipped `ticket-ba15-12-okf-core-convergence` (mev-side half of bastion's `BA.15.12`, D9/D15/D16) across 6 tasks, final verdict PASS. Task 1 added `okf-core` as an unpinned path dependency (`../bastion/crates/okf-core`, D15 discipline); `cargo build --release` succeeded with the dependency present but unused (a local untracked symlink worked around the worktree's extra directory depth for in-worktree validation only). Task 2 repointed `brain/okf.rs` at `okf_core::OkfFrontmatter` (struct deleted, `pub use`), adapting `validate_md_file`'s `layer`/`keywords` checks to the crate's `Vec<String>` (empty-means-absent) shape, and applied minimal one-line shape fixes to `brain/manifest.rs` and `brain/graph.rs` to keep the crate compiling ahead of Tasks 3/4. Task 3 repointed `brain/state.rs` at `okf_core`'s state schema/loader/graph model (confirmed byte-for-byte via diff before deleting mev's copies), keeping mev-specific validation/derivation logic (`discover_state_files`, `check_schema`, `check_state_graph`, `derive_focus`/`derive_rollup`/`derive_cross_repo`/`derive_brain_focus`, cycle detection, tier scoping) local. Task 4 repointed `brain/graph.rs` and `brain/graph_emit.rs` at `okf_core`'s graph/graph_emit model+resolution types (`EdgeKind`, `Edge`, `Node`, `Graph`, `EdgeResolution`, `resolve_edge`, `GraphExport`, `ExportedEdge`, `build_graph_export`), keeping only `build_graph`/`check_graph`'s corpus-walking logic local. Task 5 verified byte-identical `validate-brain --json`/`emit-state`/`manifest`/`emit-graph` output on the live brain corpus, baseline (pre-repoint, commit `6c0e0fa`) vs. post (commit `dacc452`), confirmed by `diff` + matching md5 checksums. Task 6 confirmed all four harness gates green: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test` (312 lib tests + all integration suites, 0 failures), `cargo build --release`. Notable decision: the ticket's stated blocker (waiting on bastion's own `okf-core`-side `BA.15.12` spec) had already lifted by the time this ticket ran — `okf-core` shipped `frontmatter.rs`/`parse.rs`/`state.rs`/`graph.rs`/`graph_emit.rs` with a matching model — so the ticket proceeded rather than reporting blocked. This closes out D9's pending cross-repo dependency on the mev side. Next: Phase 4 (`BlogValidator` as a fourth `ContentValidator` impl) per master-plan.md, or the out-of-repo orchestrator follow-up (`load_brain_edges.py` cleanup).

```
2689b56 chore: flow state — docs
dba383a docs: update docs for ticket-ba15-12-okf-core-convergence
8b5413d chore: flow state — task 6 passed
7a06737 chore: flow state — task 5 passed
f5bbd23 feat: implement ticket-ba15-12-okf-core-convergence-task5
dacc452 chore: flow state — task 4 passed
6fa1777 feat: implement ticket-ba15-12-okf-core-convergence-task4
```

---

## [2026-07-03]

### Shipped MV.3B.V — emit-graph ships resolved edges; Phase 3B closed
- **What:** Shipped `MV.3B.V` (emit-graph resolved edges) — spec authored, `/sdlc-flow` PASS, code-review low clean, PR #13 merged to main. `emit-graph` now exports `target_node_id`/`target_doc_id` via a shared `resolve_edge` pure function (both `check_graph` and `build_graph_export` call it); output `version` bumped `"1"` → `"2"`. Closed `MV.3B.V` in `state.json`, added a deferred carryover for the orchestrator `load_brain_edges.py` cleanup (gates the embed pass `OR.H`), and ran `mev emit-state --write`. mev Phase 3B roadmap is now clear (focus empty).
- **Why:** Single-source the edge resolution so the exported graph carries resolved target IDs directly, letting the orchestrator loader drop its own `build_node_maps()`/`resolve_ref()` logic. Closes Phase 3B (corpus engine outputs, D4) entirely.
- **Refs:** `planning/emit-graph-resolved-edges/tasks.md` ; PR #13 ; master-plan §MV.3B.V.

---

## [run: 2026-07-03]

`MV.3B.V` (one graph resolver: `emit-graph` ships resolved edges) complete across 5 tasks, final verdict PASS. Task 1 extracted `check_graph()`'s per-edge resolution loop (`src/brain/graph.rs:232–281` — qualify a bare `to_ref` to the referrer's own scope, look up `node_map`, else classify leaf/dangling) into a pure `pub(crate) resolve_edge(artifact, edge) -> EdgeResolution` (`Resolved`/`LeafTarget`/`Dangling`), with `check_graph` now matching on it and producing byte-identical diagnostics; used Rust let-chains to satisfy `clippy::collapsible_if`; `EdgeResolution` derives `Debug`/`Clone`/`PartialEq`/`Eq`; 4 new unit tests. Task 2 extended `graph_emit.rs` with an export-local `ExportedEdge` (built via `resolve_edge`) carrying nullable `target_node_id`/`target_doc_id`, and bumped `GraphExport.version` `"1"` → `"2"`; `graph.rs`'s shared `Edge` struct was left untouched per the spec's disjoint-structs guidance; fixed the pre-existing `tests/brain_graph_emit.rs::graph_brain_export_round_trips_as_json` assertion (`version == "1"` → `"2"`) since it directly breaks under the version bump; added a null-target-fields unit test and extended the round-trip test to assert the two new JSON keys. Task 3 added an integration parity test (`export_resolution_matches_check_graph_diagnostics`) over a synthetic corpus (resolved, leaf-target, dangling `related:` edges) asserting edge-by-edge agreement between `build_graph_export`'s resolved fields and `check_graph`'s `E_GRAPH_DANGLING_RELATED`/`W_GRAPH_LEAF_TARGET` diagnostics, matching by locator code + `to_ref` substring in the diagnostic message. Task 4 updated `docs/cli.md`'s `emit-graph` "Output shape" section to document `version` `"2"` and the two new nullable fields with resolved + dangling example edges, leaving the unrelated `manifest` command's own `version: 1` section untouched. Task 5 confirmed all four harness gates green: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test` (312+ tests, 0 failures), `cargo build --release`. No deviations from the spec surfaced across any task — all five passed on the first attempt. This closes Phase 3B (corpus engine outputs, D4) entirely — Q/R/S/T/U/V all Done or shipped elsewhere. Next: Phase 4 (`BlogValidator` as a fourth `ContentValidator` impl), or the out-of-repo orchestrator follow-up where `load_brain_edges.py` deletes its own `build_node_maps()`/`resolve_ref()` and reads mev's exported `target_node_id`/`target_doc_id` fields directly.

```
02d6021 chore: flow state — docs
7544ba7 docs: update docs for emit-graph-resolved-edges
86e108d chore: flow state — task 5 passed
05af010 chore: flow state — task 4 passed
642f7e2 docs: document emit-graph resolved edge fields, version 2
f5d061c chore: flow state — task 3 passed
e8123f5 test: add resolve_edge/check_graph/export parity test
```

---

## [2026-07-02]

### Archive 9 completed Phase 3/3B block folders; add graph-integrity check to /archive
- **What:** Retired 9 cold block folders into `planning/archive/` via `/archive` with history-preserving `git mv`: `3.K-link-integrity`, `3.L-structural-coverage`, `3.P-state-integrity`, `3.P2-state-graph-validation`, `3B.Q-manifest-emit`, `3B.R-graph-emit`, `3B.T-state-table-rollup-emit`, `3B.U-brain-rollup-tier-scoping`, and the never-executed `ticket-review-frontmatter` ticket. Ran the D35 distillation ratchet BEFORE moving: promoted ~11 entries into `knowledge.md` (D5 extract-once supersession of the removed `read_doc_metadata` seam — old entry marked SUPERSEDED; manifest/graph emit module facts; lexical link path resolution; FileUri-vs-Markdown resolution; state-file discovery off HQ root; v2 state schema DAG + `ready_order`/`detect_cycles`; single-sourced `derive_focus`; `validate-brain` flag dispatch precedence; wikilink scope semantics; ManifestEntry naming; sentinel-splice markers; no-info-severity Diagnostic; v2 diagnostic-code table; `state-schema.md` cross-repo commit split) and ~8 entries into `memory.md` (UTF-8 char-width byte scanning fix; `--structure` orphan detection is link-based → the 84 E_STRUCT_ORPHAN_FILE; `#[serde(alias="block")]` v2-migration linchpin; two v2 dedup guards; `emit-state --write` drops unmodeled fields + trailing-newline normalization; brain state.json concurrent-edit clobber; ~20 files carrying filler OKF frontmatter with the ticket never executed; post-P2 in-flight state). Set `status: archived` on the 5 moved files that carried `status: active`. Updated `planning/index.md` (Active Concept Folders now just `herdr-mev-patterns/`) and `planning/archive/index.md` (9 new registry rows). Then hardened `/archive` itself: added a Step 3 `mev validate-brain --graph` baseline capture before the move and a new Step 4.5 that diffs after and fails only on net-new dangling edges — applied to mev's `.claude/commands/archive.md` AND the canonical base-template sources (`.claude/commands/archive.md` + `.claude/commands/brain/archive.md`) so it survives `/sync-downstream-harness`. Re-validated post-archive: `mev validate-brain --graph ~/Dev/agentic-portfolio` → 7 errors + 1 warning but 0 net-new from the archive (all 7 dangling `related:` edges are pre-existing brain-content issues in bastion/orchestrator/core-planning, unrelated to mev). Commits: mev `4ff79ac`, base-template `7f2cbad`.
- **Why:** Nine completed block specs had accumulated in `planning/` as clutter; retiring them keeps the live planning surface clean while the D35 ratchet preserves their durable residue. The graph-check addition turns the manual verification done this session into a permanent guardrail so future archives can't silently introduce dangling `related:` edges. Housekeeping only — no block landed; focus is unchanged (`MV.3B.S` remains next).
- **Refs:** commits mev `4ff79ac`, base-template `7f2cbad`; D35 (memory-distillation loop); decisions D4–D8; `.claude/commands/archive.md`; `planning/archive/index.md`

### MV.3B.R wrap-up: PR merge, worktree cleanup, state.json update, handoff
- **What:** Ran `/sdlc-flow` for MV.3B.R to completion, merged PR #12 (squash), reconciled local `main` via `git reset --hard origin/main` (verified content-identical first), ran `/clean-worktree 3B.R-graph-emit-flow` to remove the worktree/branch, flipped `MV.3B.R` to closed/done in `planning/state.json`'s `tracks[]`, ran `mev emit-state --write` to regenerate focus (promoting `MV.3B.S` to next), and wrote `planning/handoff.md` for the next agent.
- **Why:** The initial `/sdlc-flow` invocation failed twice before starting: once because extra free-text instructions were mistakenly passed as a task-range argument, and once because `planning/3B.R-graph-emit/tasks.md` used non-standard `### 3B.R.N Title` task headings instead of the project's `### N. Title` convention (confirmed against 6+ other specs), which sdlc-flow's D16 task-heading parser rejected with "No task headings". Fixed by renumbering the 5 headings to plain `N.` form (commit `2635d93` on `main`, merged into the worktree branch). Separately, discovered the worktree (created by `/init-worktree`) had a corrupted sparse-checkout: `.git/worktrees/<name>/info/sparse-checkout` mixed non-cone patterns (`/*`, `!*/`) with cone-mode `/<dir>/` entries, silently disabling cone mode and dropping nested subdirectories (including the newly added spec file) from the checkout. Fixed by re-running `git sparse-checkout init --cone` + `git sparse-checkout set $(git ls-tree HEAD --name-only -d | tr '\n' ' ')` inside the worktree; root cause in `/init-worktree` itself was not investigated — logged as a `known_issue` carryover in `state.json` (`sdlc-flow-worktree-sparse-checkout-cone-bug`). After these fixes, `/sdlc-flow` completed cleanly (5/5 tasks PASS, review PASS, docs already current) and opened PR #12; `/code-review low` on the diff found 0 findings. While hand-editing the `state.json` carryover, hit and fixed two schema mistakes caught by `mev validate-brain --state`: `related[]` must be `Vec<BlockedBy>` objects (`{"type":"block","repo":...,"id":...,"what":null}`), not bare strings; `scope` requires exactly one of `repo`/`tier`/`cross_repo` set, not zero or multiple.
- **Refs:** PR #12; `planning/3B.R-graph-emit/tasks.md`; `planning/state.json`; `planning/handoff.md`; carryover `sdlc-flow-worktree-sparse-checkout-cone-bug`

---

## [run: 2026-07-02]

MV.3B.R (graph emit + structural query surface, D4) complete across 5 tasks, final verdict PASS. Task 1 added `src/brain/graph_emit.rs::GraphExport { version, root, nodes, edges, leaves }` (deriving `serde::Serialize`, reusing `Node`/`Edge` from `graph.rs`) and `build_graph_export(root, &GraphArtifact) -> GraphExport`, cloning `artifact.graph.nodes`/`edges` directly and building `leaves` from `artifact.leaf_keys` sorted for deterministic output, per the spec's "pure compiler" (D4) guidance; 3 unit tests (node/edge/leaf mapping, empty corpus, JSON round-trip); registered `pub mod graph_emit` in `src/brain/mod.rs`. Task 2 added `graph_brain(root)` to `src/lib.rs`, mirroring `manifest_brain` exactly (`find_brain_config` → `crawl_corpus` → `build_graph` → `build_graph_export`), re-exported `GraphExport`/`build_graph_export` from the crate root alongside `Manifest`, and wired a new `mev emit-graph [--pretty] [path]` CLI subcommand (placed beside `GenerateGraph` with a doc comment distinguishing JSON emit from the existing HTML visual) that crawls the corpus, builds the graph, and prints the envelope to stdout. Task 3 added `tests/brain_graph_emit.rs`, mirroring `tests/brain_manifest.rs` conventions, covering node/edge/leaf counts, JSON round-trip, `related:`-edge resolution, and the missing-`brain.toml` error path. Task 4 documented the `emit-graph` subcommand (synopsis, sample envelope, exit codes, distinction from `generate-graph`) in `docs/cli.md`, placed between `manifest` and `emit-state` as the closer sibling grouping, and added `graph_emit.rs`/`GraphExport`/`brain_graph_emit.rs` to `docs/architecture.md`'s module map and knowledge-graph section; `docs/index.md` left unchanged as anticipated (no new top-level doc file). Task 5 confirmed all four harness gates green (fmt, clippy `-D warnings`, `cargo test`, release build) and sanity-ran `mev emit-graph` against the live company brain: 411 nodes, 1062 edges, 101 leaves, confirming pure stdout emit with no DB/file writes. No deviations from the spec surfaced across any task — all five passed on the first attempt. Next: `MV.3B.S` (graph-aware RAG, orchestrator-side; mev's edge model is the contract) or the deferred brain-content cleanup (84 live `E_STRUCT_ORPHAN_FILE` findings).

```
48eef75 chore: flow state — docs
b8f50c3 chore: flow state — task 5 passed
c8d99dd feat: implement 3B.R-graph-emit-task5
f5bfd0f chore: flow state — task 4 passed
3af8c27 feat: implement 3B.R-graph-emit-task4
3183410 chore: flow state — task 3 passed
e7a98de feat: implement 3B.R-graph-emit-task3
396e209 chore: flow state — task 2 passed
```

---

## [run: 2026-07-02]

MV.3.L (structural coverage: `index.md` ↔ directory, D17) complete across 5 tasks, final verdict PASS. Task 1 added `src/brain/structure.rs::check_structure(corpus, root)` — for each directory containing an `index.md` corpus member, gathers its direct-child corpus entries and the `index.md`'s extracted `Markdown`/`FileUri` link targets (via `links::extract_links` plus a local normalization helper mirroring `links.rs::normalize_path`), emitting `E_STRUCT_ORPHAN_FILE` for any uncovered direct child and `E_STRUCT_DANGLING_ROW` for any link resolving inside the corpus root to a nonexistent file; directories with no `index.md` are skipped entirely; 7 unit tests. Task 2 added the `validate_brain_structure(root)` library driver (schema pass → crawl → `check_structure`, mirroring `validate_brain_graph`) and wired a `--structure` CLI flag into `ValidateBrain`, dispatched ahead of `--state`/`--graph`/`--sync` but after `--links`. Task 3 added `tests/brain_structure.rs` — clean tree, orphan file, dangling row, both together, and the `--json` envelope carrying both codes, plus a CLI subprocess end-to-end exit-code check. Task 4 documented the `--structure` flag (dispatch precedence, both `E_STRUCT_*` codes, examples) in `docs/cli.md` and added a full "Structural coverage" section (module map, function/diagnostic tables) to `docs/architecture.md`. Task 5 confirmed all four harness gates green (fmt, clippy `-D warnings`, `cargo test`, release build) and sanity-ran `mev validate-brain --structure` against the live company brain: 84 genuine `E_STRUCT_ORPHAN_FILE` findings (files named in plain backtick text in index tables rather than as markdown links — correctly flagged per this check's link-based coverage definition), 0 `E_STRUCT_DANGLING_ROW`, no false positives. Notable decision: fixing the brain's own 84 orphan findings is out of scope for this validator-behavior block, deferred as a separate brain-content cleanup. Next: `MV.3B.R` (graph emit + structural query surface / Phase 3B).

```
48c3eb3 chore: flow state — docs
8dd2571 chore: flow state — task 5 passed
34c611e feat: implement 3.L-structural-coverage-task5
39b2be6 chore: flow state — task 4 passed
34e3a3f feat: implement 3.L-structural-coverage-task4
009333b chore: flow state — task 3 passed
c76daa2 feat: implement 3.L-structural-coverage-task3
f362c31 chore: flow state — task 2 passed
```

---

## 2026-07-02

### MV.3.L merged (PR #11) — squash-merge reconciliation, MV.3B.R now next

- **What:** `MV.3.L` (structural coverage) shipped, reviewed, and merged via PR #11 (squash-merged on GitHub); the SDLC worktree was cleaned up afterward. Because the PR was squash-merged remotely while local `main` still carried its own unpushed commits from the earlier same-day carryover-resolution session (the `state.json` kind:"portfolio" / block-ID naming-convention work), reconciling local `main` with `origin/main` required an explicit merge step rather than a fast-forward: a conflict surfaced in `log.md`/`planning/status.md` frontmatter, resolved in favor of the newer, post-3.L values (the squash commit's content superseded the pre-merge state). Re-ran the full test suite after reconciliation — green — then pushed the reconciled `main` to origin. `MV.3.L` is now closed; `MV.3B.R` (graph emit + structural query surface, Phase 3B) is the only remaining unstarted mev feature block and is next up.
- **Why:** GitHub's squash-merge collapses the PR's branch history into a single commit on `main`, which diverges from a local `main` that already has unpushed commits of its own — a normal merge/rebase was needed to reconcile the two histories cleanly rather than assuming a fast-forward would apply.
- **Refs:** PR #11; `planning/master-plan.md` (Phase 3 / 3B); carryover entry below.

**Carryover (deferred, non-mev):** `MV.3.L`'s sanity run of `mev validate-brain --structure` against the live company brain surfaced 84 genuine `E_STRUCT_ORPHAN_FILE` findings — files referenced only as plain backtick text (not markdown links) in various `index.md` tables across the brain. These are real findings, correctly flagged, but fixing them is brain-content cleanup, not a mev validator-behavior task; deferred to a separate future session/repo (the company-brain root), out of scope for this repo's log.

### Cross-repo: state.json portfolio kind, block-ID naming convention, /update-state command

- **What:** Resolved `mev emit-state` / `mev validate-brain --state` warnings surfaced against the live brain. Added a new `kind:"portfolio"` to mev's state.json schema (`discover_state_files`, `check_schema`, `plan_master_plan_tables`), with new tests, `docs/cli.md` updates, and decision `D8-portfolio-kind-terminal-repos.md` (already recorded earlier this session). Applied `kind:"portfolio"` to the three live portfolio-tier repos (rag-engine-rs, workflow-engine-rs, claude-sdk-rs), which are terminal (published to GitHub, no roadmap) and were being wrongly flagged as incomplete `kind:"project"` repos (`E_STATE_SCHEMA_MISSING_FIELD` on empty `tracks[]`). Separately, adopted the `<Prefix>.<Phase>.<Letter>` block-ID naming convention (already used by mev, e.g. `MV.3B.U`) across amistad (`AM`) and price-scout (`PS`) — renamed master-plan.md/status.md headings and cross-references; the client-repo (`BP`) rename is deferred (blocked by a concurrent SDLC flow in that repo — tracked as a carryover in `planning/state.json`). Updated `/generate-master-plan` (plain + brain-flavored variants) and `/new-project` to use the bare-ID heading convention and auto-derive+register a unique prefix. Shipped a new `/update-state` command (plain + brain-flavored variants) documenting the safe procedure for editing any repo's `state.json` — the authored-vs-derived field boundary, the `kind` decision table, the block-ID rename checklist, and the edit → validate → `emit-state --write` → `validate-brain --state` loop — distributed to all repos and registered in `run_syncs.sh`.
- **Why:** Investigating the two warning classes (`W_EMIT_NO_SENTINEL`, `E_STATE_SCHEMA_MISSING_FIELD`) revealed the schema had no way to represent a terminal, roadmap-less repo, and that block-ID conventions had drifted inconsistently across sub-repos, risking ambiguity in cross-repo tooling and this command's own `PREFIX.PHASE.BLOCK` lookups.
- **Refs:** `planning/decisions/D8-portfolio-kind-terminal-repos.md`; `planning/state.json` carryover entries `client-repo-block-id-rename-pending`, `agents-skills-generate-master-plan-mirror-drift`.

### MV.3B.U complete: brain rollup tier-scoping + brain-focus aggregation (6 tasks, PASS)

Implemented the full Block 3B.U fix for `emit-state --write` corruption of brain-kind `state.json` files across 6 tasks (all PASS). Task 1 added `TierScope` and `tier_scope_for(brain_file, config)` — a brain file's `repo` slug is matched against `brain.toml`'s tier map (a match scopes to that single tier, no match, e.g. the HQ root, scopes to all repos) — and rewrote `derive_rollup` to iterate the in-scope config repos in config order: derive the headline where a child `state.json` loads, else **preserve** the brain file's existing `repos[]` entry verbatim (backfilling `tier`), else emit a tier-tagged empty stub; no repo is ever silently dropped, and `RepoRollup.tier` is now always populated. Task 2 added `derive_brain_focus(scope, config, graph, files)`, computing brain-kind `focus.now/next/blocked` as the repo-tagged, deduped-by-`(repo, id)`-per-list, config-ordered union of in-scope children's derived focus. Task 3 threaded `&BrainConfig` through `plan_state_json` (brain arm now calls `tier_scope_for`/`derive_rollup`/`derive_brain_focus` instead of the transitional all-scope stub from task 1) and `lib.rs::emit_state`. Task 4 added 8 end-to-end integration tests covering tier-scoped rollup (derived/preserved/stub branches, all repos retained), the malformed-child-JSON preserve regression, repo-tagged focus union, HQ all-tier aggregation, and write/re-emit fixed-point idempotence. Task 5 documented the tier-scoped/non-destructive rollup and brain-focus union in `state-schema.md` (company-brain `core` repo), `docs/cli.md`, `docs/architecture.md`, and new decision `D7-brain-rollup-tier-scoping-and-preserve.md`. Task 6 confirmed all four harness gates green, ran a non-destructive tier-scoped dry-run of `mev emit-state` against the live company brain (only pre-existing malformed-JSON warnings for orchestrator/bastion, no repo drops), and resolved the `mev-brain-rollup-tier-scoping` carryover in `core/planning/state.json`. Final review verdict: PASS (no findings). This closes the corruption risk flagged at the end of `MV.3B.T` — `mev emit-state --write` is now safe to run against brain-kind files. Next: `MV.3.L` (structural coverage) or `MV.3B.R` (graph emit / Phase 3B).

```
a7d46d1 chore: flow state — docs
6220f95 chore: flow state — task 6 passed
57db0ec feat: implement 3B.U-brain-rollup-tier-scoping-task6
78b46ba chore: flow state — task 5 passed
e4543fa feat: implement 3B.U-brain-rollup-tier-scoping-task5
8df7c85 chore: flow state — task 4 passed
5d612b3 feat: implement 3B.U-brain-rollup-tier-scoping-task4
88967d0 chore: flow state — task 3 passed
```

---

## 2026-06-30

### MV.3B.Q complete: manifest emit + D5 extract-once refactor (`manifest` subcommand, 6 tasks, PASS)

Implemented the full Block 3B.Q manifest emit pipeline across 6 tasks (all PASS). Task 1 applied the D5 extract-once refactor: added `Serialize` to `OkfFrontmatter`, added `Option<OkfFrontmatter>` field to `CorpusEntry`, and wired frontmatter parsing into `crawl_corpus()` via a single read + parse chain (I/O or YAML errors produce `None`); two new tests verify metadata round-trip. Task 2 collapsed the `read_doc_metadata` seam: `build_graph()` reads `doc_id`/`related` directly from `entry.metadata`; `RawFrontmatter`, `DocMeta`, and `read_doc_metadata` removed from `graph.rs`; `collect_doc_ids()` in `links.rs` likewise updated; test helpers patched to pre-populate `entry.metadata` as `crawl_corpus` does. Task 3 created `src/brain/manifest.rs` with `ManifestEntry` and `Manifest` structs (all derive `Serialize`) and `build_manifest()` function; `rel` paths normalised to forward slashes via `replace(MAIN_SEPARATOR, '/')` for cross-platform JSON portability; registered as `pub mod manifest` in `brain/mod.rs`; 3 unit tests covering entry mapping, serialization, and empty-corpus handling. Task 4 added `manifest_brain()` library driver in `src/lib.rs` and the `mev manifest <root>` CLI subcommand in `src/main.rs` with `--pretty` flag (compact JSON default); 5 integration tests in `tests/brain_manifest.rs` asserting on compact JSON string patterns. Task 5 updated `docs/cli.md` with the manifest subcommand reference (arguments, `--pretty`, output shape, exit codes, sample JSON) and `docs/architecture.md` with the manifest module map, `ManifestEntry`/`Manifest` types, `build_manifest` function, D5 refactor note, and removal of `read_doc_metadata`. Task 6 confirmed all four harness gates green: `cargo fmt --check`, `clippy -D warnings`, `cargo test` (all tests pass), `cargo build --release`. Final review verdict: PASS (no findings). `mev manifest <root>` now emits a canonical file-list consumable by `index_brain.py` — "validated == embedded" holds by construction. Next: `MV.3.L` (structural coverage) or `MV.3B.R` (graph emit).

```
a04d830 chore: flow state — docs
bd1e715 chore: flow state — task 6 passed
a479818 chore: flow state — task 5 passed
59dc133 docs: document manifest subcommand and D5 refactor in cli.md and architecture.md
e4cfd42 chore: flow state — task 4 passed
71d55c3 feat: implement 3B.Q-manifest-emit-task4
f185749 chore: flow state — task 3 passed
ed6bbc0 feat: implement 3B.Q-manifest-emit-task3
9436033 chore: flow state — task 2 passed
1e1a8cd feat: implement 3B.Q-manifest-emit-task2
67694a2 chore: flow state — task 1 passed
87ce81b feat: implement 3B.Q-manifest-emit task 1 — D5 extract-once refactor
```

### MV.3B.Q shipped: manifest emit + D5 extract-once refactor
- **What:** `mev manifest <root>` subcommand implemented (6 tasks, PASS). D5 extract-once refactor: `OkfFrontmatter` derives `Serialize`; `CorpusEntry` carries `Option<OkfFrontmatter>` parsed once in `crawl_corpus()`; `read_doc_metadata` seam removed from `graph.rs`. New `src/brain/manifest.rs` with `ManifestEntry`/`Manifest`/`build_manifest()`. `manifest_brain()` lib driver + CLI subcommand with `--pretty`. All harness gates green. Worktree merged + cleaned. PR #9 merged to main.
- **Why:** Phase 3B: turn the validated Brain corpus into a queryable product. The manifest is the single source for `index_brain.py` — "what's validated == what's embedded" by construction.
- **Refs:** `planning/3B.Q-manifest-emit/tasks.md`, PR #9

---

## 2026-06-30

### MV.3B.T complete: state-graph table + rollup emit (`emit-state` subcommand, 6 tasks, PASS, 275 tests)

Implemented the full Block 3B.T single-derivation emit engine across 6 tasks (all PASS). Task 1 extracted `derive_focus`, `derive_rollup`, and `derive_cross_repo` from `check_focus_drift` into reusable public functions in `src/brain/state.rs` — `DerivedFocus { now, next, blocked }` returning derived block ids (with unmet `depends_on` subsets for `blocked`); `check_focus_drift` now delegates to `derive_focus` so the validator and emitter share one derivation; 8 integration tests added. Task 2 created `src/brain/emit.rs` with `EmitError` (thiserror), `wave_order` (all-block wave sort, `None`-last), `render_wave_table` (Markdown table with derived `blocked` status), and `splice_generated` (idempotent sentinel-aware splice of `<!-- BEGIN/END generated:{marker} -->` regions); 18 integration tests in `tests/brain_emit.rs`. Task 3 added `EmitAction`/`EmitPlan` types, `plan_state_json` (leaf focus regen + brain rollup regen with fixed-point check), `plan_master_plan_tables` (sentinel-aware wave-table splice — missing sentinels yield `W_EMIT_NO_SENTINEL`, never forced into prose), and `apply_plan` (dry-run/write split); 14 integration tests covering fixed-point property, idempotency, and dry-run/write behaviour. Task 4 added the `emit_state` library driver (`src/lib.rs`) and the `emit-state` CLI subcommand (`src/main.rs`) with `--write` flag; default is dry-run; exits from `report.is_failure()`; 4 integration tests. Task 5 updated `docs/cli.md` (subcommand reference, sentinel contract, diagnostic codes) and `docs/architecture.md` (emit module map + function table, `derive_*` helpers). Task 6 confirmed all four harness gates green: `cargo fmt --check`, `clippy -D warnings`, `cargo test` (275 tests, 0 failures), `cargo build --release`. Final review verdict: PASS (no findings). `mev emit-state --write` is now the fixed point of `mev validate-brain --state`: running emit then validate reports zero `W_STATE_FOCUS_DRIFT` / `W_STATE_ROLLUP_DRIFT`. Next: `MV.3.L` (structural coverage) or `MV.3B.Q` (manifest emit / Phase 3B).

```
8bffa82 chore: flow state — docs
decdef0 chore: flow state — task 6 passed
f85af39 chore: flow state — task 5 passed
9082dae feat: implement 3B.T-state-table-rollup-emit-task5
c6d8595 chore: flow state — task 4 passed
4b8bb76 feat: implement 3B.T-state-table-rollup-emit-task4
61dd986 chore: flow state — task 3 passed
0309e40 feat: implement 3B.T-state-table-rollup-emit-task3
f943487 chore: flow state — task 2 passed
55e9653 feat: implement 3B.T-state-table-rollup-emit-task2
3b86689 chore: flow state — task 1 passed
c141664 feat: implement 3B.T-state-table-rollup-emit-task1
```

---

## 2026-06-30

### MV.3.P2 merged via PR #7 — v2 state-graph validator (8 tasks, 275 tests, PASS)

- **What:** Ran `/sdlc-flow 3.P2-state-graph-validation` to completion (8 tasks, PASS, 275 tests) and merged it via **PR #7** (merge commit `460d0cd`). MV.3.P2 migrates `src/brain/state.rs` to the v2 `state.json` schema: `depends_on` DAG on track blocks, `detect_cycles` (`E_STATE_CYCLE`), `ready_order`, `check_status_consistency` (`E_STATE_STATUS_INCONSISTENT`), `check_backlog_integrity` (`E_STATE_DANGLING_PROMOTION`), `check_focus_drift` (`W_STATE_FOCUS_DRIFT`), and `E_STATE_AUTHORED_BLOCKED` — all wired into `validate_brain_state`. Post-merge `/code-review low` found no code issues; a follow-up doc fix (commit `1edbd21`) corrected `docs/architecture.md`: the `check_focus_drift` signature (was missing the 4th `files` arg), backlog integrity wrongly attributed to `check_state_graph` (it is `check_backlog_integrity`), and two missing function-table rows (`check_status_consistency`, `check_backlog_integrity`). Local `main` fast-forwarded cleanly; worktree `trees/3.P2-state-graph-validation-flow` removed, branch deleted; `main == origin/main` at `460d0cd`.
- **Why:** Completes the v2 state-graph validation layer — the work-block graph twin of the doc-graph corpus engine — and keeps the architecture docs accurate after review. **Gotcha:** live `mev validate-brain --state` will fail until the brain-side re-seed of the 5 live `state.json` files from v1→v2 lands — expected, not a regression.
- **Refs:** PR #7 (merge `460d0cd`); doc fix `1edbd21`; `planning/3.P2-state-graph-validation/`; `core/planning/state-schema.md` (v2 contract).

---

## 2026-06-30 — MV.3.P2 complete: v2 state-graph expansion validator (8 tasks, PASS, 275 tests)

Implemented the full Block P2 v2 state-graph expansion validator across 8 tasks (all PASS, single attempt each). Task 1 migrated `src/brain/state.rs` to the v2 serde model: `Origin`/`Backlog` structs, `depends_on`/`wave`/`origin` added to `TrackBlock`, `Block.block`/`Endpoint.block` renamed to `id` with serde aliases for v1 backward compat, `backlog[]` added to `StateFile`, and all in-file fixture JSON migrated to canonical v2 form. Task 2 re-sourced DAG edges from `tracks[].blocks[].depends_on[]` (replacing the v1 `focus.blocked_by[]` edge source), added `E_STATE_AUTHORED_BLOCKED` for track blocks with a hand-authored `"blocked"` status, and extended `check_schema` to validate `backlog[].status` ∈ `{idea, ready, promoted}`. Task 3 added `detect_cycles` (DFS-based, emits `E_STATE_CYCLE` with cycle path) and a standalone `ready_order` (wave-ordered ready+open blocks, built as a reusable function for the future `MV.3B.T` emit step). Task 4 added `check_status_consistency` (`E_STATE_STATUS_INCONSISTENT` when a closed block depends on a non-closed block) and `check_backlog_integrity` (`E_STATE_DANGLING_BLOCKED_BY` for unresolvable backlog deps, `E_STATE_DANGLING_PROMOTION` for promoted backlog nodes whose `block` pointer resolves to nothing). Task 5 added `check_focus_drift` which recomputes `focus.now/next/blocked` from `tracks[]` and emits `W_STATE_FOCUS_DRIFT` (warning, exit 0) on mismatch, reusing `ready_order`. Task 6 wired all four new checks into the `validate_brain_state` pipeline in `src/lib.rs`, migrated the integration fixtures in `tests/brain_state.rs` to v2, and added 6 new end-to-end tests (10 total). Task 7 updated `docs/cli.md` (new diagnostic codes, expanded `--state` pipeline description) and `docs/architecture.md` (v2 types, function map). Task 8 confirmed all four harness gates green: `cargo fmt --check`, `clippy -D warnings`, `cargo test` (275 tests, 0 failures), `cargo build --release`. Final review verdict: PASS (no findings). Next: coordinate the brain-side re-seed of the 5 live `state.json` files to v2 to enable live `--state` validation, then `MV.3.L` (structural coverage) or `MV.3B.Q` (manifest emit).

```
ce336e4 chore: flow state — task 8 passed
2032d44 chore: mark Task 8 validate passed — all harness gates green (275 tests)
25c3626 chore: flow state — task 7 passed
78a606b docs: update cli.md and architecture.md for 3.P2 state-graph v2 (Task 7)
0f47c99 chore: flow state — task 6 passed
cbd16f2 feat: implement 3.P2-state-graph-validation-task6
e495bb9 chore: flow state — task 5 passed
833f404 feat: implement 3.P2-state-graph-validation-task5
```

---

## 2026-06-30 — MV.3.K complete: link integrity validator (`validate-brain --links`)

Implemented the full Block K link integrity layer (all 6 tasks, PASS verdict, 236 tests). Task 1 added `src/brain/links.rs` with a `LinkKind`/`LinkRef` model and `extract_links()` — a single-pass byte-scan state machine (no new regex dependency) that captures markdown `[text](path)`, bare/embedded `file://` URIs, and `[[wikilink]]` targets while correctly skipping external URLs and pure in-page anchors. Task 2 added `check_links()` resolving all three link kinds against the corpus: dead markdown refs emit `E_LINK_DEAD_MARKDOWN`, dead `file://` paths emit `E_LINK_DEAD_FILE_URI`, unknown wikilink slugs emit `E_LINK_DANGLING_WIKILINK`; a `collect_doc_ids()` helper reuses the existing `read_doc_metadata` D5 seam to avoid re-parsing frontmatter. Task 3 added `read_moves_pending()` and `check_moved_references()` consuming the `.brain-moves-pending` ephemeral hook file, emitting `E_LINK_MOVED_REFERENCE` for stale markdown/file:// refs pointing at moved paths (WikiLink slug-resolution intentionally excluded as scope-correct). Task 4 wired everything into a `validate_brain_links()` public API and a `--links` CLI flag on the `ValidateBrain` subcommand, with a UTF-8 boundary panic in the byte scanner fixed during this task; 9 end-to-end integration tests cover all four diagnostic codes plus the clean-corpus and JSON-envelope paths. Task 5 documented the `--links` flag and all four `E_LINK_*` codes in `docs/cli.md` and added the `links.rs` module to `docs/architecture.md`. Task 6 confirmed all four harness gates green (fmt, clippy, 236 tests, release build). A live run against the company brain confirmed genuine findings: dangling `[[bin]]`/`[[test]]` wikilinks in claude-sdk-rs status docs, dead `file://` URIs with placeholder paths, and dead markdown links in SECURITY.md. Next: `MV.3.L` (structural coverage: `index.md` ↔ dir) or `MV.3B.Q` (manifest emit / Phase 3B).

```
e62f481 chore: flow state — docs
34b01f1 chore: flow state — task 6 passed
32ea092 chore: flow state — task 5 passed
7223483 feat: implement 3.K-link-integrity-task5
8b70af3 chore: flow state — task 4 passed
e1c86e6 feat: implement 3.K-link-integrity-task4
b1c3989 chore: flow state — task 3 passed
782f4ad feat: implement 3.K-link-integrity-task3
```

### MV.3.K merged (PR #6) + `--links` dispatch precedence fix
- **What:** `3.K-link-integrity` merged via PR #6 (merge commit `334ae4a`). Ran `/sdlc-flow 3.K-link-integrity` to completion (6 tasks, PASS), then a post-merge `/code-review low`. The review caught a real docs/code mismatch: the `validate-brain` dispatch ladder in `src/main.rs` placed the `--links` branch **last** (lowest precedence), contradicting `docs/cli.md`, `docs/architecture.md`, and the recorded task decision that `--links` outranks `--state`. Fix: moved `links` to the **top** of the ladder; added binary-spawning integration test `links_flag_outranks_state_in_dispatch` in `tests/brain_links.rs` (commit `973b3df`). Test count now **237** (was 236). Local `main` rebased to preserve an unpushed planning-doc commit → `main` at `b1fb953`, in sync with `origin/main`. Worktree `trees/3.K-link-integrity-flow` removed, branch deleted. `mev validate-brain --links` is live.
- **Why:** Block K is the link-integrity sibling of the doc-graph corpus engine; the precedence bug would have let `--links` silently lose to `--state` at the CLI, contradicting documented behavior — fixing it keeps the dispatch contract consistent with docs/spec.
- **Refs:** PR #6; commit `973b3df`; merge `334ae4a`; `main` `b1fb953`; `planning/3.K-link-integrity/`; `master-plan.md`.

### State-graph expansion design + MV.3.P2 / MV.3B.T planning
- **What:** Settled 4 design decisions + 7 refinements for the state-graph expansion (recorded in the Resolutions section of `core/planning/state-graph-design-decisions/notes.md`), then rewrote `core/planning/state-schema.md` to **v2**: `depends_on` DAG (replacing the ad-hoc `blocked_by`), derived focus/rollup, `backlog[]`, `id` standardization, and **blocked-is-derived** (a block's blocked status is computed from its `depends_on` edges, not hand-set). Added two blocks to the mev master-plan — **MV.3.P2** (state-graph expansion validation) and **MV.3B.T** (table/rollup emit) — and specced **MV.3.P2** at `planning/3.P2-state-graph-validation/tasks.md` (8 tasks). Concurrent in a separate worktree: **MV.3.K** link integrity was implemented, reviewed, and merged (PR #6, 237 tests). Key commits: mev `b1fb953` (master-plan MV.3.P2+MV.3B.T), `7f20ca8` (MV.3.P2 spec); core repo `4693dce` (schema v2 + settled decisions). Note: two separate git repos — mev at `core/mev`, brain/core at `core/`. Next: re-seed the 5 live `state.json` files to v2, then run `/sdlc-flow 3.P2-state-graph-validation`.
- **Why:** The state-graph (work-block graph) is the twin of the doc-graph corpus engine; before re-seeding the live `state.json` files we needed to settle the v2 schema shape and plan the mev validator that will guard it — design and validator spec first, schema migration second.
- **Refs:** D36; `core/planning/state-schema.md` (v2); `core/planning/state-graph-design-decisions/notes.md`; `planning/3.P2-state-graph-validation/tasks.md`; mev `b1fb953`, `7f20ca8`; core `4693dce`.

---

## 2026-06-29 — MV.3.P complete: post-flow code-review fix + PR #5 merged

- **What:** MV.3.P (`validate-brain --state`) fully implemented and merged. All 7 tasks PASS; 209 tests green. Post-flow `/code-review low --fix` removed dead `(usize, &PathBuf)` tuple from `node_counts` in `check_state_graph` (path stored but never read). PR #5 merged, worktree cleaned, `main` pushed to origin.
- **Why:** Block P was the planned next step after MV.3.J (graph integrity) — the work-block analogue of the document graph. Completing it gives `mev` machine-caught validation of the cross-repo block-dependency graph (`blocked_by` edges, brain rollup drift), closing a silent-rot risk in the state.json files.
- **Refs:** `planning/3.P-state-integrity/tasks.md`; PR #5; commits `e8a6e69`–`fe94b25`.

---

## 2026-06-29 — MV.3.P Done: state.json integrity validator (`validate-brain --state`)

Implemented the full Block P state integrity layer (all 7 tasks, PASS verdict, 209 tests). Task 1 added `src/brain/state.rs` with the complete serde model for `state.json` (`StateFile`, `Focus`, `Block`, `BlockedBy` internally-tagged enum, `Track`, `RepoRollup`, `CrossRepoEdge`, `TierEntry`) plus `load_state()`, registered in `mod.rs`. Task 2 added `StateSource`, `discover_state_files`, and `check_schema` — registry discovery from HQ brain + tier sub-brains (via `tiers[].rollup`) + leaf repos (via `brain.toml [[repos]]`), plus four validation rings (JSON+schema, blocked_by well-formedness, kind-appropriate section checks). Task 3 built the state graph: `StateGraph`/`StateNode`/`StateEdge` (all `Serialize`), `build_state_graph` (nodes from tracks[], edges from `blocked_by` + `cross_repo[]`), and `check_state_graph` emitting five diagnostic codes including the marquee `E_STATE_DANGLING_BLOCKED_BY`. Task 4 added `check_rollup` — brain `repos[]` headline drift detection emitting `W_STATE_ROLLUP_DRIFT`. Task 5 wired everything into `validate_brain_state` public API and the `--state` CLI flag on `mev validate-brain`. Task 6 delivered `tests/brain_state.rs` with four end-to-end integration tests (clean, dangling blocked_by, rollup drift, JSON round-trip). Task 7 confirmed all four harness gates green and the live run clean: 0 `E_STATE_*`/`W_STATE_*` diagnostics across all five live `planning/state.json` files. Next: `MV.3.K` (link integrity) or `MV.3B.Q` (manifest emit / Phase 3B).

```
32b9300 chore: flow state — docs
464046c docs: update docs for 3.P-state-integrity
129ecee chore: flow state — task 7 passed
b1e2e2c chore(3.P): task 7 validate — all harness gates pass, live state check clean
d0d540c chore: flow state — task 6 passed
a8ab973 feat(3.P): add integration tests for validate_brain_state (Task 6)
095352b chore: flow state — task 5 passed
511ab64 feat(state): add validate_brain_state public API + --state CLI flag
```

---

## 2026-06-29 — MV.3.P spec authored: state.json integrity validator (`validate-brain --state`)

- **What:** Authored and committed the task spec for a NEW Phase 3 block, **MV.3.P — State integrity** (`mev validate-brain --state`). The work-block analogue of MV.3.J (graph integrity): where MV.3.J validates the *document* graph (`scope:doc_id` nodes, `related:` edges), MV.3.P validates the *work-block* graph — it discovers every repo's `planning/state.json`, validates each against the canonical schema (`core/planning/state-schema.md`), and checks the cross-repo block-dependency graph for referential integrity. Marquee check `E_STATE_DANGLING_BLOCKED_BY` is the direct port of MV.3.J's `E_GRAPH_DANGLING_RELATED` from docs to blocks. Follows MV.3.M's cross-repo read mode (state.json files live in gitignored nested-git sub-repos invisible to the corpus walk). Four validation rings: JSON+schema, intra-repo (focus↔tracks), cross-repo (blocked_by/cross_repo edges resolve), and brain rollup drift (a warning — the rollup lags by design). Builds a Serialize-able state graph for D4 forward-compat (future emit + auto-generated brain rollup = "Direction 2"). **Spec only this session — no code written; block is Not started / spec drafted.**
- **Why:** The user asked how mev relates to the new `planning/state.json` files (v2 self-describing schema seeded last session). The relationship: they're two graphs over one corpus, and mev is already the corpus-graph engine — so mev should validate state.json (and eventually emit/generate the brain rollup). User chose to spec the validator now. This block also mechanically closes the denormalization-cost open question in state-schema.md (rollup drift becomes machine-caught instead of silent).
- **Refs:** spec `planning/3.P-state-integrity/tasks.md`; schema `core/planning/state-schema.md`; D29; commit `f5ca298`.

---

## 2026-06-29 — Post-flow code-review fixes: diagnostic locator codes + stale doc wording

- **What:** Post-flow `/code-review low --fix` pass on the 2.J-graph-integrity output. Both edge-resolution diagnostic locators in `check_graph` were using the generic `"related"` string instead of the documented locator codes. Fixed: leaf-target warning → `W_GRAPH_LEAF_TARGET`; dangling-edge error → `E_GRAPH_DANGLING_RELATED`. Updated all matching tests (unit + integration) to expect the correct codes. Also removed stale "(Task 2) will accept" future-tense wording from the module doc. PR #4 merged; worktree `trees/2.J-graph-integrity-flow` cleaned; branch deleted. Main is 17 commits ahead of origin.
- **Why:** Locator mismatch would have silently broken any downstream tooling (RAG gate, CI scripts) keying on the documented locator codes. Standard post-flow quality pass.
- **Refs:** `src/brain/graph.rs`, `tests/brain_graph.rs`, commit `70e07dd`

---

## 2026-06-29 — 2.J-graph-integrity complete (PASS): global scope:doc_id knowledge graph

Implemented the full Block J graph integrity layer (all 5 tasks, PASS). Task 1 defined the serializable, emittable graph model (`EdgeKind`, `Edge`, `Node`, `Graph` — all `Serialize`) and `build_graph` in `src/brain/graph.rs`, with the `read_doc_metadata` seam (D5 forward-compat: the single site that calls `extract_frontmatter`/`OkfFrontmatter`, keeping future foreign-format ingest a one-function swap); `check_graph` was co-located here since its logic depended directly on `GraphArtifact`. Task 2 completed by adding the one missing unit test: bare ref to a `doc_id` that exists only in another scope correctly flags dangling. Task 3 added `validate_brain_graph()` to `src/lib.rs` and `--graph` to the `ValidateBrain` CLI subcommand (mutually exclusive with `--sync` by precedence), re-exporting `build_graph`/`Graph`/`check_graph` for Phase 3B Block R. Task 4 delivered `tests/brain_graph.rs` with 7 end-to-end integration tests over a multi-unit (brain/core/mev) fixture: clean corpus, same doc_id across scopes, duplicate detection, cross-scope resolution, dangling edges, leaf-target warnings, and JSON envelope round-trip. Task 5 confirmed all four harness gates green: `fmt`, `clippy -D warnings`, 175 unit + 57 integration = 232 tests, release build. The `Graph` struct is `Serialize`-able (D4 forward-compat) — Phase 3B Block R can emit it directly to Postgres with no re-walk. Next: Block K (link integrity) or Block Q (manifest emit / Phase 3B).

```
ce18100 docs: update docs for 2.J-graph-integrity
7b159a1 chore: flow state — task 5 passed
83609cc feat: implement 2.J-graph-integrity-task5
bf19087 chore: flow state — task 4 passed
5ec919b feat: implement 2.J-graph-integrity-task4
51e13e6 chore: flow state — task 3 passed
05e48e8 feat: implement 2.J-graph-integrity-task3
11d56aa chore: flow state — task 2 passed
0b96651 feat(graph): check_graph + uniqueness/edge-resolution/leaf-lint tests (task 2, block J)
55817f9 chore: flow state — task 1 passed
6bcf0e7 feat(graph): serializable graph model + build_graph (task 1, block J)
```

---

## 2026-06-29 — 2.J-corpus-crawl merged; code-review fix for is_root_instruction_file

- **What:** Ran `/sdlc-flow 2.J-corpus-crawl` to completion (5 tasks, PASS). Post-flow code review found `is_root_instruction_file` in `src/brain/okf.rs` was checking only the filename — a `docs/README.md` in the corpus would be silently OKF-exempt. Fixed to use `owning_unit()` + `strip_prefix` to verify the file sits exactly at its owning unit's root; added regression test `is_root_instruction_file_false_for_deep_readme`. Commit `753be87`. 160 tests pass. PR #3 merged; worktree cleaned.
- **Why:** Normal post-flow quality pass; the bug was a latent false-negative that would have silently skipped OKF validation for any `docs/README.md` in the corpus.
- **Refs:** `src/brain/okf.rs`, `planning/2.J-corpus-crawl/tasks.md`, PR #3

---

## 2026-06-28 — Block 2.J-corpus-crawl complete (PASS): multi-root corpus crawl + scope registry

Implemented the full registry-driven corpus crawl foundation (all 5 tasks, PASS). Task 1 added `src/brain/scope.rs` with `scope_units`, `scope_for`, and `owning_unit` (longest-prefix registry match, root-unit fallback, 9 unit tests). Task 2 added owned serializable `Corpus`/`CorpusEntry` types and `crawl_corpus()` to `src/brain/crawl.rs`, with `CLAUDE.md` removed from the file blocklist so root instruction files join the corpus. Task 3 wired `crawl_corpus` into `BrainValidator::crawl` and added the OKF exemption for root files (`README.md`/`CLAUDE.md`) — they are valid corpus leaves without frontmatter; existing integration tests updated to place files under `planning/` as corpus members. Task 4 delivered a 13-test integration suite (`tests/brain_corpus.rs`) over a 3-unit fixture tree (brain/core/mev), covering all positive corpus members, all spec-listed exclusions, scope correctness, and `serde_json` round-trip. Task 5 confirmed all four harness gates green: `fmt`, `clippy -D warnings`, 159 tests across 10 suites, release build. The `Corpus` struct is `Serialize`-able (D4 forward-compat) — Phase 3B Block Q can emit it as the embedder manifest with no re-crawl. Next: `2.J-graph-integrity` — global `scope:doc_id` node index, extensible edge model, `related:` resolution, leaf lint via `--graph`.

```
fa68e1e chore: flow state — docs
73d2580 docs: update docs for 2.J-corpus-crawl
d57545e chore: flow state — task 5 passed
9d3e538 feat: validate 2.J-corpus-crawl-task5 — all harness gates green
e220baf chore: flow state — task 4 passed
b4d8ccc feat: implement 2.J-corpus-crawl-task4
d425e38 chore: flow state — task 3 passed
6d1e166 feat: wire crawl_corpus into BrainValidator + exempt root files from OKF
```

---

## 2026-06-28 — Destination architecture settled (D4): mev as corpus engine; graph as emitted product

### Reviewed knowledge_graph service; settled corpus-engine + knowledge-graph architecture (D4)
- **What:** Reviewed the `workflow-engine-rs` `knowledge_graph` service — verdict: **don't adopt**
  for the brain (UUID/Dgraph/inferred edges; wrong model for an authored `scope:doc_id` graph).
  Settled the destination architecture in new decision **D4**: mev becomes the single **corpus
  engine** (one crawl → diagnostics + manifest + graph), a pure side-effect-free compiler; the
  knowledge graph is a **first-class emitted artifact** stored in **Postgres beside the embeddings**;
  an **extensible `Edge { from, to_ref, kind }`** model; **two retrieval modes** (semantic +
  structural) fusing into graph-aware RAG. Refreshed `master-plan.md` with **Phase 3B (Blocks
  Q/R/S)** and added forward-compat constraints on the queued Block J specs; updated `status.md`
  and `decisions/index.md`.
- **Why:** Before building the graph specs, needed to settle division of labor against the existing
  Dgraph-backed graph service and lock the destination (where the graph lives, its node/edge
  contract, how retrieval uses it) so the queued Block J work is built forward-compatible.
- **Refs:** [D4](./planning/decisions/D4-corpus-engine-and-knowledge-graph.md) ·
  `planning/master-plan.md` (Phase 3B, Blocks Q/R/S) · `planning/2.J-corpus-crawl/` ·
  `planning/2.J-graph-integrity/` · `planning/status.md`

---

## 2026-06-28 — Block J reshaped into a global knowledge graph; split into corpus-crawl + graph specs

Design session with Brandon that substantially reshaped Phase 3 Block J. Confirmed Block N
(`--sync` watermark) was already done/merged (PR #2); cleaned up a stale handoff and a redundant
regenerated spec. Then pressure-tested the `doc_id` concept: separated **embedding** (keys on
`file_path`, needs no `doc_id`) from the **`related:` graph** (the only thing `doc_id` is for), and
settled on a **global cross-repo knowledge graph** built to scale and to be reusable for client
knowledge bases. Decisions: canonical node id = **`scope:doc_id`** with **registry-driven stable
slugs** from `brain.toml` (never inferred from tier/path); **mev becomes a multi-root validator**;
**extensible edge model** (`Edge { from, to_ref, kind }`, `related` first, typed edges later);
**corpus = planning/ + docs/ + root README/CLAUDE** across all registered repos minus bloat/ephemeral;
**root-file OKF frontmatter is optional** (embed-as-leaf, promote-by-`doc_id`); **mev owns the single
corpus definition** the embedder should consume.

Split Block J into two specs (committed): `planning/2.J-corpus-crawl/` (foundation: scope registry +
`scope_for` + multi-root `crawl_corpus` + root-file OKF exemption) → `planning/2.J-graph-integrity/`
(global `scope:doc_id` node index + edge integrity + leaf lint via `--graph`). Appended the 2026-06-28
refinements to `namespacing-and-corpus-decision.md`; updated `master-plan.md` (inserted Block J-crawl,
reshaped Block J) and `index.md`. Wrote `planning/handoff.md` flagging the **priority review of
`workflow-engine-rs/services/knowledge_graph/`** (existing Dgraph-backed graph service) before
building — to settle division of labor (mev validates; that service stores/queries) and a shared
node-id/edge contract. Removed the stale `/sdlc-flow` worktree (init commit only; nothing lost). No
production code changed this session — planning only.

```
(planning docs only — specs, decision doc, master-plan, status, log, handoff)
```

---

## 2026-06-28 — Block N (sync-watermark) shipped via PR #2; code-review fix landed

Block N (sync-watermark) shipped via PR #2. Code-review caught E_SYNC_FILE_MISSING misclassification for read/parse errors on existing files — fixed to E_SYNC_WATERMARK_MALFORMED at src/brain/sync.rs:126,139. All 196 tests pass. Worktree cleaned, rebased main pushed. Next: Block 2.J cross-file graph integrity.

---

## 2026-06-28 — Block N complete: `synced_from` watermark check shipped (verdict: PASS)

Implemented `mev validate-brain --sync` end-to-end across five tasks, all passing in a single SDLC-flow run (verdict: PASS, 196 tests). Task 1 added the `chrono` crate, a `synced_from: Option<String>` field to `OkfFrontmatter`, and `src/brain/sync.rs` with a strict RFC3339 `parse_watermark` function and 5 unit tests. Task 2 implemented `check_sync` — the core per-`[[repos]]` loop that reads each sub-repo `status_file` and cache `cache_doc`, compares `timestamp` vs `synced_from`, and emits `E_SYNC_FILE_MISSING`, `E_SYNC_WATERMARK_MISSING`, `E_SYNC_WATERMARK_MALFORMED`, or `E_SYNC_DRIFT` diagnostics; 8 unit tests covering all four locator codes. Task 3 wired everything into the public API: `validate_brain_sync()` in `lib.rs` and a `--sync` flag on the `validate-brain` subcommand in `main.rs`; `BrainConfig` derived `Clone` to allow both the OKF schema pass and the watermark check to run without borrow conflicts. Task 4 added `tests/brain_sync.rs` with 4 integration tests over a temp HQ-root fixture: in-sync (0 errors), drift detection (exactly 1 `E_SYNC_DRIFT`), cache re-alignment clearing the error, and JSON round-trip of a `Sync` diagnostic. Task 5 was a full harness validation pass confirming all four gates green (`fmt`, `clippy -D warnings`, 196 tests, `build --release`). Next: Block 2.J cross-file graph integrity.

```
22b03c3 chore: flow state — docs
93cf905 docs: update docs for block-n-sync-watermark
2c6139e chore: flow state — task 5 passed
e35bb2e chore: flow state — task 4 passed
3c90758 feat(block-n): Task 4 — integration tests for validate_brain_sync end-to-end
9e1cc73 chore: flow state — task 3 passed
9b294cc feat(block-n): Task 3 — validate_brain_sync public API + --sync CLI flag
f492670 chore: flow state — task 2 passed
823f8c0 feat(block-n): Task 2 — check_sync logic, WatermarkFrontmatter, unit tests
2fc70b6 chore: flow state — task 1 passed
dd15112 feat(block-n): Task 1 — chrono dep, synced_from field, parse_watermark
```

---

## 2026-06-27 — Block 2.M complete; GitHub repo created; docs bootstrapped; harness fixed

Block 2.M shipped (PASS, 6 tasks). GitHub repo created and code pushed. Wrote the first complete round of codebase docs: `cli`, `architecture`, `brain-toml`, and `okf-schema` reference pages. Fixed the SDLC harness doc pipeline — added `--bootstrap` mode to `/update-docs` (skips invention check when scaffolding from scratch), tightened the `/document` no-invention rule, wired `/new-project` to generate stack-aware doc stubs, and propagated all harness changes to `mev` and `learn-ai`.

---

## 2026-06-27 — Block 2.M: brain.toml config reader (HQ-R)

Implemented the `brain.toml` TOML config reader end-to-end across six tasks, all passing in a single SDLC-flow run (verdict: PASS). Task 1 added `BrainConfig` struct, `load_brain_config`, and `find_brain_config` walk-up resolver with 10 integration tests and the `toml` crate. Task 2 wired `crawl_brain` skip_dirs to config, converting `BrainValidator` from a unit struct to a config-bearing struct. Task 3 made all vocab validation (`is_valid_layer`, `is_valid_status`, `is_valid_project`) config-driven, removing every hardcoded string array from production source. Task 4 threaded `find_brain_config` through `validate_brain` in `lib.rs` and added a config-flip integration test proving a vocab-only `brain.toml` edit flips validation results without touching Rust source. Task 5 marked `planning/decisions/D3-corpus-config-system.md` as superseded (the `.mev.toml` per-corpus proposal retired in favour of the shared `brain.toml`). Task 6 fixed a latent bug where path-style `skip_dirs` entries (e.g. `planning/archive`) were silently ignored because `is_blocklisted_name` only compared leaf names; extended the helper to accept a relative-path parameter so path-style entries prune correctly. All four harness gates green; 174+ tests pass; `mev validate-brain` exits 0 with 0 errors against the live company-brain repo. Next: Block 2.J graph-integrity check.

```
a52e3f0 chore: flow state — docs
80f4c60 docs: update docs for 2.M-brain-toml-reader
3386b62 chore: flow state — task 6 passed
07845a9 fix(brain/crawl): support path-style skip_dirs entries from brain.toml
4f76797 chore: flow state — task 5 passed
553ea17 docs(decisions): mark D3 superseded by brain.toml Block M
6a0da1e chore: flow state — task 4 passed
17604d7 feat(brain/lib): thread find_brain_config through validate_brain; config-flip integration test (Task 4)
```

---

## 2026-06-26 — Crawl hardening + validate-brain live triage

Ran a live `mev validate-brain` pass against the actual company-brain repo to validate Phase 2 readiness. Starting point: 145 original errors cascading from three root issues. Diagnosed and fixed all three: (1) Directory skip-list hardening — added `.claude`, `.repo-backups`, `.agent` to the crawl blocklist in `src/brain/crawl.rs` (these directories contain non-OKF files that were flagged as missing frontmatter). (2) Decision-file doc_id pattern — implemented `is_decision_id()` and `is_valid_doc_id()` helpers in `src/brain/okf.rs` to accept the Brain's `D<n>-…` convention (e.g., `D3-corpus-config-system`) in addition to standard kebab-case, fixing validation false-positives on decision files. (3) Corpus config design — wrote `planning/decisions/D3-corpus-config-system.md` capturing the long-term architecture for moving hardcoded corpus rules into per-corpus `.mev.toml` config, planned post-Phase 3. Live run now passes with 0 errors / 3 warnings (benign: keywords count edge cases). Current focus remains 2.J-graph-integrity.

```diff
README.md           | 15 ++++++---
planning/decisions/D3-corpus-config-system.md | 81 ++++++++++++++++++++++++
src/brain/crawl.rs  | 35 +++++++++++
src/brain/okf.rs    | 40 ++++++++++++
4 files changed, 165 insertions(+), 6 deletions(-)
```

---

## 2026-06-26 — Close-out for Block 2.I (validate-brain subcommand + --json)

Completed Phase 2, Block I close-out activities: verified the validation suite with all 145 tests passing (all four harness gates green: fmt, clippy, test, build). Updated README.md to document the `validate-brain` subcommand (validates Brain OKF YAML frontmatter across the company-brain repo) and the global `--json` flag for machine-readable JSON output. Wrote `planning/handoff.md` to orient Block 2.J (graph-integrity check) — the next and final block in Phase 2. Phase 2 fully complete.

```diff
README.md           | 15 +++++++---
 planning/handoff.md | 84 ++++++++++++++++++++++++++---------------------------
 2 files changed, 53 insertions(+), 46 deletions(-)
```

---

## 2026-06-26 — 2.I-validate-brain-subcommand: validate-brain subcommand + --json flag (PASS)

Completed Phase 2, Block I: wired `BrainValidator` to a `mev validate-brain <root>` subcommand and added a global `--json` flag emitting a machine-readable `JsonReport` envelope. Added `serde::Serialize` to `Severity` (lowercase via `rename_all`) and `Diagnostic` in `src/lib.rs`, plus `pub fn validate_brain(root)` and `pub struct JsonReport` with `new()` constructor and `to_json()` method. In `src/main.rs`, added the `ValidateBrain { path }` subcommand variant (default `..`), a global `--json` flag on `Cli`, dispatch to `validate_brain`, JSON/human branching in both subcommand arms, and updated `about` text naming both consumers. Five new integration tests in `tests/brain_validate.rs` cover OKF violation detection, nested-git skip enforcement, clean-file no-error case, JSON envelope key/count validation, and `Severity` lowercase serialization. Total tests: 145 (91 unit + 54 integration) — all pass. All four harness gates green. Review passed on first attempt with all 8 acceptance criteria MET. Next: Block 2.J (graph integrity).

```
232bd3f docs: update docs for 2.I-validate-brain-subcommand
97ebe48 feat: implement 2.I-validate-brain-subcommand
f8da0c4 chore: add spec for 2.I-validate-brain-subcommand
```

---

## 2026-06-26 — Close-out for Block 2.H (Brain OKF frontmatter validator)

Completed Block 2.H close-out activities: verified the validation suite with all 142 tests passing (all four harness gates green: fmt, clippy, test, build), performed a doc health sweep (no stale sections identified; flagged `docs/harness-json.md` as NEEDS_REVIEW for future attention), and wrote `planning/handoff.md` to orient Block 2.I work on the validate-brain subcommand and --json flag. Block 2.H is fully closed; ready to hand off to Block 2.I.

```diff
 planning/handoff.md | 78 ++++++++++++++++++++++++++++++-----------------------
 1 file changed, 45 insertions(+), 33 deletions(-)
```

---

## 2026-06-26 — 2.H-brain-okf-validator: Brain OKF frontmatter validator (PASS)

Completed Phase 2, Block H: added the Brain OKF frontmatter validation layer on top of Block G's crawl infrastructure. Created `src/brain/okf.rs` with the `OkfFrontmatter` serde struct (all fields `Option`, `layer` as `Option<Vec<String>>`, extras tolerated), the `validate_md_file` entry point (read → extract → parse → field-check pipeline with short-circuit errors for missing/malformed frontmatter), and three vocab helpers (`is_valid_layer`, `is_valid_project`, `is_valid_status`) covering the three closed sets from D27. Required-field checks (`type`, `title`, `description`) each emit their own `error` with precise locators; controlled-vocab errors fire only when a field is present; `doc_id` uses `is_kebab_case` from shared; `keywords` count outside 3–7 emits a `warning`. `BrainValidator` was assembled in `src/brain/mod.rs` as the second `ContentValidator` impl (`type Item = MdFile`, crawl delegates to `crawl_brain`, validate_item delegates to `okf::validate_md_file`). Re-exports added to `src/lib.rs`. 30 unit tests in `src/brain/okf.rs` and 14 integration tests in `tests/brain_okf.rs` cover every rule, boundary cases, and end-to-end `BrainValidator::run`. Total test count: 142 (91 unit + 51 integration) — all pass. Review passed on first attempt with all 8 acceptance criteria MET. Next: Block 2.I (validate-brain subcommand + --json flag).

```
b6702d3 docs: update docs for 2.H-brain-okf-validator
24b6996 feat: implement 2.H-brain-okf-validator
2e38aba chore: add spec for 2.H-brain-okf-validator
```

---

## 2026-06-26 — Close-out for Block 2.G: Brain crawl (verification + docs + handoff)

Completed Block 2.G close-out activities: verified all 96 tests pass (61 unit + 35 integration), ran coverage scan with no gaps flagged, and patched `README.md` to add `src/brain/` to the directory map (was missing the new module from the source-tree overview). Wrote `planning/handoff.md` to orient Block 2.H work: context on the OKF frontmatter schema, pointers to the Brain docs in the parent repo, test fixtures, and the validation rules needed. Block 2.G is fully closed; ready to hand off to Block 2.H (Brain OKF frontmatter validator).

```diff
 README.md           |  3 ++-
 planning/handoff.md | 72 +++++++++++++++++++++++++----------------------------
 2 files changed, 36 insertions(+), 39 deletions(-)
```

---

## 2026-06-26 — 2.G-brain-crawl: Brain crawl entry point (PASS)

Completed Phase 2, Block G: added a parallel Brain crawl entry point alongside the existing learn-ai crawl. Created `src/brain/mod.rs` and `src/brain/crawl.rs` defining `MdFile { path, rel, stem }`, two pruning helpers (`is_blocklisted_name` for `target/`, `node_modules/`, `.git/` dirs, and `has_nested_git` for the depth>0 nested-git rule), and `pub fn crawl_brain(root: &Path) -> (Vec<MdFile>, Vec<Diagnostic>)` using `filter_entry`-based directory pruning. Re-exported `MdFile` and `crawl_brain` from `src/lib.rs`. Eight integration tests in `tests/brain_crawl.rs` cover root-level finds, all blocklist prunes, nested-git pruning, non-.md skips, and `rel`/`stem` correctness. All 96 tests (61 unit + 35 integration) pass; all four harness gates green. Review passed on first attempt with all 7 acceptance criteria MET. The document pass flagged `README.md` as needing a `src/brain/` row in the source-tree directory map (manual follow-up). Next: Block 2.H (Brain OKF frontmatter validator).

```
d64c0dd docs: update docs for 2.G-brain-crawl
52daf32 feat: implement 2.G-brain-crawl
6fc27de chore: add spec for 2.G-brain-crawl
```

---

## 2026-06-26 — 2.F-content-validator-trait: ContentValidator trait + shared core (PASS)

Completed Phase 2, Block F: the full refactor to introduce a generic `ContentValidator` trait. Extracted shared helpers (`extract_frontmatter`, `is_kebab_case`, `non_empty`) into `src/shared.rs`, defined the associated-type `ContentValidator` trait in `src/validator.rs`, moved the learn-ai code (`crawl.rs`, `meta.rs`) into a new `src/learn_ai/` module, implemented `LearnAiValidator` behind the trait, and rewrote `validate()` as a thin wrapper. All 27 tests pass. A post-flow code review fixed a misleading `non_empty` docstring. The public API (`mev::{ContentFile, Corpus, FileKind, Locale, crawl, validate_file, validate, Diagnostic, Severity}`) is preserved via `pub use`, so all existing integration tests pass unchanged. Next: Block 2.G (Brain crawl).

```
b8fe7f7 fix(shared): correct misleading non_empty docstring — returns original string, not trimmed
6810c65 chore: flow state — wrap-up (PASS)
eefd181 chore: wrap up 2.F-content-validator-trait
23a270a chore: flow state — docs
2d64850 docs: update docs for 2.F-content-validator-trait
31805de chore: flow state — task 5 passed
34e7596 feat: implement 2.F-content-validator-trait-task5
f23c530 chore: flow state — task 4 passed
2cf8b82 feat: implement 2.F-content-validator-trait-task4
ae7937a chore: flow state — task 3 passed
97894e6 feat: implement 2.F-content-validator-trait-task3
```

---

## 2026-06-24 — Harness pull from base-template (b8ebbf7)

Pulled the full current `base-template` harness (commit `b8ebbf71c20445de65195037aa24bfe00bbf080b`)
into `.claude/`. Added the **`/sdlc-flow`** engine (D30–D33; shared-worktree sequential flow, one end
review, PR wrap-up), **`/generate-master-plan`** + the **block-definition planning seam** (D34:
`/generate-tasks --from`, `/plan`-as-block, hardened block skeleton), the **plan-quality floor** (D35:
clarify-or-abort, never fabricate), and the TAC8 commands (`/patch`, `/conditional_docs`, the `e2e/`
template library). All engines `node --check` clean; command/engine files byte-identical to base.
`planning/harness.json` untouched. Provenance stamped in `planning/.template-version`.


### 2026-06-20 (task 7 — Run validation suite and confirm all gates green)

Task 7 executed the final harness validation gates: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, and `cargo build --release`. All four gates passed cleanly. The implementation from tasks 1–6 (struct deserialization, enum/format validation, YAML frontmatter parsing, and comprehensive fixture tests) was verified to meet the acceptance criteria: required-field diagnostics, enum/format violations, malformed YAML, and missing frontmatter blocks all surface correctly as errors with precise locators; no false positives or panics. Review verdict: PASS. Block C is feature-complete and ready for merge. Next: Task 1 of phase1-blockD — anchor-slice contract and pair existence validation.

```
26e71e2 docs: update docs for phase1-blockC-task7
1aa6378 feat: implement phase1-blockC-task7
76a9f46 chore: init worktree phase1-blockc-task7
```

---

## 2026-06-20 (task 6 — tests against fixtures)

Implemented comprehensive fixture-driven test suite for metadata and MDX frontmatter validation. Added temp-dir fixtures covering good cases (all required fields, valid enums/formats) and deliberately-broken variants (missing fields, bad enum values, malformed formats, empty sections, missing frontmatter). All tests assert correct diagnostics with precise locators and severities. Existing smoke and crawl tests remain green. PASS verdict on first review attempt with no changes required. Next: Task 7 — Validate (run harness gates and optionally test against live corpus).

```
000f1c6 docs: update docs for phase1-blockC-task6
8df75c6 feat: implement phase1-blockC-task6
2560225 chore: init worktree phase1-blockc-task6
```

---

## 2026-06-20 (task 5 — Wire the checks into `validate()`)

Implemented integration of all struct and frontmatter validation checks into the main `validate()` function. The implementation iterates through the corpus files and dispatches each file by kind to its corresponding validator (ModuleMeta for JSON modules, PathMetadataJson for path metadata, MDX frontmatter for Markdown files), with all diagnostics appended to the Report while preserving Block B filename diagnostics and the public contract. Review passed with no findings. Full test suite passes; all harness gates (fmt, clippy, test, build) remain green. Next: Task 6 — Tests against fixtures.

```
6b98f8b docs: update docs for phase1-blockC-task5
3e5faf5 feat: implement phase1-blockC-task5
b963389 chore: init worktree phase1-blockc-task5
```

---

## 2026-06-20 (task 4 — Parse and validate MDX frontmatter as real YAML)

Task 4 implemented full MDX frontmatter parsing using YAML deserialization and strict field validation. Frontmatter blocks are extracted between `---` fences, parsed with `serde_yaml`, and validated for required fields (`title, description, duration, difficulty, lastUpdated`) with proper error diagnostics for missing/malformed content. Format and enum validation (difficulty ∈ `beginner | intermediate | advanced`, duration format) are reused from shared helpers. All test fixtures covering good files and deliberately-broken variants (missing frontmatter, missing fields, malformed YAML) pass with expected diagnostics. Review verdict: PASS (1 attempt). All four harness gates remain green. Next: Task 5 — Wire the checks into `validate()`.

```
90886af docs: update docs for phase1-blockC-task4
b3228a1 feat: implement phase1-blockC-task4
eddc19a chore: init worktree phase1-blockc-task4
```

---

### 2026-06-20 (task 3 — define and validate path metadata.json struct)

Task 3 successfully implemented `PathMetadataJson` struct validation requiring fields `id, title, description, level, duration, version, lastUpdated, topics, modules`, with case-insensitive `level` enum validation matching `beginner`, `intermediate`, `advanced`. All required field diagnostics, format validation, and fixture-driven tests passed on first review. Next: Task 4 — Parse and validate MDX frontmatter as real YAML.

```
a1a7f02 docs: update docs for phase1-blockC-task3
d6b9421 feat: implement phase1-blockC-task3
b18fd11 chore: init worktree phase1-blockc-task3
```

---

## 2026-06-19 (task 2 — define and validate `ModuleMeta` struct for `LearnModuleJson`)

Implemented the `ModuleMeta` serde struct in `src/meta.rs` with full validation for `FileKind::LearnModuleJson` files. All required fields (`id`, `pathId`, `title`, `description`, `duration`, `type`, `difficulty`, `order`, `objectives`, `tags`, `version`, `lastUpdated`, and non-empty `sections[]` with `id/type/order`) are enforced, emitting an error-severity `Diagnostic` with a precise locator for each missing field. Enum validation covers `difficulty` (beginner/intermediate/advanced), module `type` (theory/concept/practice/project/assessment), and section `type` (content/quiz/exercise/project/assessment). Format validation covers kebab-case `id` and `duration` (`^\d+\s+(minutes?|hours?)$`) using hand-written helpers without the `regex` crate. Fixture-driven tests in `tests/meta.rs` cover the good case and each broken variant; existing Block B and smoke tests stayed green. All four harness gates (`fmt`, `clippy -D warnings`, `test`, `build`) passed on the first review attempt. Next: Task 3 — Define and validate path `metadata.json` (`FileKind::PathMetadataJson`).

```
c8c6061 docs: update docs for phase1-blockC-task2
244f533 feat: implement phase1-blockC-task2
92c2763 chore: init worktree phase1-blockc-task2
```

---

## 2026-06-19 (task 1 — add validate struct/frontmatter module)

Added `src/meta.rs` (re-exported from `lib.rs`) to hold the serde structs and per-file validation functions for Block C. The module reads each file's contents from `ContentFile.path` and surfaces read/parse failures as `error`-severity `Diagnostic` values without panicking or aborting the run. `crawl.rs` remains focused on the filesystem walk. Review passed on the first attempt with all four harness gates green (`fmt`, `clippy -D warnings`, `test`, `build`). Next: Task 2 — Define and validate the `ModuleMeta` struct (`FileKind::LearnModuleJson`).

```
2fe498a docs: update docs for phase1-blockC-task1
0c5b84e feat: implement phase1-blockC-task1
d940a34 chore: init worktree phase1-blockc-task1
```

---

## 2026-06-18

Project initialized from `base-template` (commit `00ad2834e232d3243a3578132b02db01a7be40ab`) via `/new-project`.
Planning infrastructure scaffolded: `planning/context.md`, `planning/status.md`,
`planning/master-plan.md`, `planning/index.md`, `planning/harness.json`, `planning/decisions/`,
and the root `CLAUDE.md` / `README.md`. Concept folders (`planning/<concept>/`) are created on
demand by the SDLC pipeline. Curated SDLC harness (`.claude/`) in place.

Next step: run `/generate-tasks` for the first Phase 0 block to begin the pipeline.

```diff
(no code changes — planning files only)
```
