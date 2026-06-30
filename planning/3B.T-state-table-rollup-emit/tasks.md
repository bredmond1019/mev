# Task Spec — Phase 3B, Block MV.3B.T — State-graph table + rollup emit

**Status:** In progress (tasks 1-4 passed) · **Last run:** 2026-06-30T14:36:19Z

## Goal
Add `mev emit-state`: make mev the **single derivation engine** for every *generated* view the v2
state schema declares — the leaf `focus` snapshot (now/next/blocked), the brain `repos[]` /
`cross_repo[]` rollup, and the master-plan wave/dependency tables (spliced between sentinel comments so
narrative is never clobbered) — all computed from the authored `tracks[]` DAG, as a pure compiler
(files in → files out; no DB, no network). `/log-work` regenerates state by **shelling out to
`mev emit-state --write`** rather than re-deriving in a brain command, so the validator
(`validate-brain --state`) and the writer share one derivation and can never disagree.

## Context Pointers
- **Master-plan block:** `planning/master-plan.md` → *Phase 3B → MV.3B.T — State-graph table + rollup
  emit*. The `MV.3B.R` parallel: same pure-compiler model. Settles D3 (Option B): mev owns
  derived-view generation, not a conversational `/log-work` agent — `/log-work` invokes `mev
  emit-state` instead of re-deriving. This is the block that lets `MV.3.P2` flip derivation drift from
  warning → error (once the views are regenerable from one engine, drift becomes fixable and therefore
  enforceable).
- **Single-derivation-engine intent:** the derived `focus` is computed by the **same** `derive_focus`
  the validator's `check_focus_drift` uses (task 1 below). Folding focus regen into this block means
  `mev emit-state --write` immediately followed by `mev validate-brain --state` reports **zero**
  `W_STATE_FOCUS_DRIFT` / `W_STATE_ROLLUP_DRIFT` — the emit is, by construction, the fixed point of the
  drift check.
- **v2 schema contract:** `../planning/state-schema.md` (the `core` repo) — *Authored vs derived* table
  and the **Derivation rules** (`focus.now` = `in_progress`; `focus.blocked` = open blocks with an
  unmet `depends_on`, each carrying the unmet subset as `blocked_by[]`; `focus.next` = ready open
  blocks in `wave` order). Brain `repos[]` / `cross_repo[]` are derived from the union of children's
  `tracks[]`.
- **Existing reusable surface (`src/brain/state.rs`):** `StateGraph` / `build_state_graph`,
  `ready_order` (readiness ordering — the forward-compat hook MV.3.P2 left for this block),
  `check_focus_drift` (carries the focus-derivation logic to single-source against), `check_rollup`
  (the drift check this emit makes fixable), and the serde model (`StateFile`, `Track`, `TrackBlock`,
  `Focus`, `Block`, `RepoRollup`, `CrossRepoEdge`, `Endpoint`, `BlockedBy`, `Origin`, `Backlog`).
- **State pipeline driver:** `src/lib.rs::validate_brain_state` shows the discover → load → build-graph
  sequence to reuse (`discover_state_files`, `load_state`, `build_state_graph`).
- **CLI dispatch:** `src/main.rs` (`ValidateBrain` subcommand + `--json` global flag, `JsonReport`
  envelope, `find_brain_root`).
- **CLAUDE.md standing rules:** every new function/module ships with tests (rule 1); new `.md` docs
  carry OKF frontmatter and update the directory `index.md` (rule 2); decisions are append-only
  (rule 4). mev stays read-only over the corpus **except** this emit, which writes only derived views.

## Step-by-Step Tasks

### 1. Single-source the derivation in `state.rs` (`derive_focus` + `derive_rollup` + `derive_cross_repo`)
- In `src/brain/state.rs`, extract the inline focus-derivation logic currently inside
  `check_focus_drift` into a public, reusable function `derive_focus(src, file, graph, files) ->
  DerivedFocus`, where `DerivedFocus { now: Vec<String>, next: Vec<String>, blocked: Vec<(String,
  Vec<BlockedBy>)> }` returns derived block ids (and, for `blocked`, the **unmet subset** of each
  block's `depends_on`). Rewrite `check_focus_drift` to call `derive_focus` so the validator and the
  emitter share one derivation (no possibility of the check and the emit disagreeing).
- Add `derive_cross_repo(files: &[(StateSource, StateFile)]) -> Vec<CrossRepoEdge>`: for every leaf
  `tracks[].blocks[].depends_on` entry of `{type:"block"}` whose `repo` differs from the owning
  repo, produce a `CrossRepoEdge { from: {owner_repo, block.id}, to: {dep.repo, dep.id}, note: what }`.
- Add `derive_rollup(children: &[(StateSource, StateFile)], graph, files) -> Vec<RepoRollup>`: for each
  loaded leaf (`kind == "project"`), build a `RepoRollup { repo, tier, now, next, blocked }` from that
  child's `derive_focus` result (mapping ids back to `Block { id, title, status, blocked_by }` using
  the child's `tracks[]` for titles). `tier` may be left `None` here (not derivable from state alone).
- **Primary files:** `src/brain/state.rs`; tests in `tests/brain_state.rs`.
- Add unit/integration tests: `check_focus_drift` output is unchanged for the existing fixtures
  (regression guard); `derive_cross_repo` produces an edge for a cross-repo `depends_on` and none for
  same-repo deps; `derive_rollup` reproduces a child's derived focus.

### 2. New `emit` module: full wave ordering + table renderer + sentinel splice
- Create `src/brain/emit.rs` and register it with `pub mod emit;` in `src/brain/mod.rs`.
- `wave_order(graph: &StateGraph, files: &[(StateSource, StateFile)]) -> Vec<String>`: **all** block
  keys (`"repo:id"`) sorted by `wave` ascending (`None` last), tiebreak by track iteration order then
  block array index — the full-roadmap sibling of `ready_order` (which filters to ready/open only).
- `render_wave_table(repo_slug: &str, file: &StateFile, graph: &StateGraph) -> String`: a Markdown
  table for that repo's blocks in wave order. Columns: `Wave | Block | Title | Status | Depends on`,
  where `Status` shows the **derived** state (an open block with an unmet `depends_on` renders as
  `blocked`) and `Depends on` lists the block's `depends_on` targets (`repo:id`, plus `external:<what>`).
- `splice_generated(original: &str, marker: &str, generated: &str) -> Result<String, EmitError>`:
  replace the text between `<!-- BEGIN generated:{marker} -->` and `<!-- END generated:{marker} -->`
  with `generated`, preserving every line outside the sentinels verbatim. Idempotent (re-splicing the
  result yields identical output). Return an `EmitError` when a sentinel is missing or the pair is
  unbalanced. Define `EmitError` (thiserror) in this module.
- **Primary files:** `src/brain/emit.rs` (new), `src/brain/mod.rs` (add module line); tests in
  `tests/brain_emit.rs` (new).
- Tests: wave order matches a fixture DAG; an open-with-unmet-dep block renders `blocked`; splice
  preserves all non-sentinel lines and is idempotent; missing/unbalanced sentinels error.
- **Depends on task 1.**

### 3. Emit planners: state.json (focus + rollup) + master-plan tables, with dry-run/write split
- In `src/brain/emit.rs`, add the planners that turn loaded state into proposed file writes without
  performing IO inline:
  - `EmitAction { path: PathBuf, new_content: String, note: String }` and `EmitPlan { actions:
    Vec<EmitAction>, diagnostics: Vec<Diagnostic> }`.
  - `plan_state_json(loaded, graph, files) -> EmitPlan`: **one rewrite per `state.json`** so the two
    derived sections never collide on the same file. On a clone of each loaded `StateFile`:
    - **leaf** (`kind == "project"`): regenerate `focus` from `derive_focus` — `focus.now` = blocks
      with `status: in_progress`; `focus.next` = ready open blocks in `wave` order; `focus.blocked` =
      open blocks with an unmet `depends_on`, each carrying the unmet subset as `blocked_by[]`. Titles
      are filled from the file's own `tracks[]`.
    - **brain** (`kind == "brain"`): regenerate `repos[]` (via `derive_rollup`) and `cross_repo[]`
      (via `derive_cross_repo`). Brain `focus` aggregation across children is **out of scope** (its
      derivation rule is not settled in `state-schema.md`) — leave the brain file's `focus` untouched.
    - Re-serialize with `serde_json::to_string_pretty`; authored fields (`tracks[]`, `backlog[]`,
      `tiers[]`, `repo`, `kind`, `updated`, `note`, and — for brain — `focus`) survive the round-trip
      by value. Add one `EmitAction` per file whose derived section actually changed.
  - `plan_master_plan_tables(loaded, graph) -> EmitPlan`: for each loaded state file, resolve the
    sibling `master-plan.md` (`state.json`'s parent dir `/ master-plan.md`); if it exists and carries
    the `wave-table` sentinels, `splice_generated` the rendered table into it and add an `EmitAction`.
    A missing file or missing sentinels yields a `W_EMIT_NO_SENTINEL` warning diagnostic (not a hard
    error) — never invent sentinels into arbitrary prose.
- `apply_plan(plan: &EmitPlan, write: bool) -> Vec<Diagnostic>`: when `write` is true, write each
  action's `new_content` to its `path` and emit an `I_EMIT_WROTE` info/warning diagnostic per file;
  when false (dry-run), write nothing and emit a `W_EMIT_DRY_RUN` note per planned action. Always
  surface the plan's own diagnostics.
- **Primary files:** `src/brain/emit.rs`; tests in `tests/brain_emit.rs`.
- Tests: a leaf `state.json` has its `focus` regenerated to match the derivation rules while authored
  `tracks[]` is preserved; a brain `state.json` round-trips with derived `repos[]`/`cross_repo[]` and
  an untouched brain `focus` while authored `tracks[]`/`backlog[]`/`tiers[]` are preserved; master-plan
  splice writes only inside sentinels; a master-plan with no sentinels yields `W_EMIT_NO_SENTINEL` and
  no write; dry-run leaves files byte-identical; a file already at its derived fixed point produces no
  `EmitAction`.
- **Depends on tasks 1 and 2.**

### 4. CLI surface: `emit-state` subcommand + `emit_state` library driver
- In `src/lib.rs`, add `pub fn emit_state(root: &Path, write: bool) -> anyhow::Result<Report>`:
  resolve `brain.toml` (same `find_brain_config` + `E_CONFIG_NOT_FOUND` fallback as the other
  drivers), discover + load state files (reuse `discover_state_files` / `load_state`), `build_state_graph`,
  run `plan_master_plan_tables` and `plan_brain_rollup`, `apply_plan` with `write`, and collect all
  diagnostics into a `Report`. Re-export the emit entry points via `pub use brain::emit::{…}`.
- In `src/main.rs`, add an `EmitState { path: PathBuf (default "."), write: bool (`--write`) }`
  subcommand that calls `mev::emit_state`, prints diagnostics / `JsonReport` envelope exactly like
  `ValidateBrain`, and sets the exit code from `report.is_failure()`. Default is **dry-run**; `--write`
  performs the in-place writes. Document the subcommand in its `about`/arg help text.
- **Primary files:** `src/lib.rs`, `src/main.rs`; integration tests in `tests/brain_emit.rs`.
- Tests: `emit_state` on a fixture brain with `write=false` returns planned-action diagnostics and
  leaves files unchanged; with `write=true` updates the brain `state.json` and master-plan; `--json`
  envelope is valid JSON.
- **Depends on task 3.**

### 5. Documentation
- `docs/cli.md`: add the `emit-state` subcommand — purpose, `--write` vs dry-run default, the
  diagnostic codes (`W_EMIT_NO_SENTINEL`, `W_EMIT_DRY_RUN`, `I_EMIT_WROTE`), exit codes, and the
  `<!-- BEGIN generated:wave-table -->` sentinel contract.
- `docs/cli.md`: also note that `mev emit-state` is the single derivation engine `/log-work` shells
  out to (regenerates leaf `focus` + brain `repos[]`/`cross_repo[]` + master-plan tables).
- `docs/architecture.md`: add the `emit` module to the module map and a function table
  (`wave_order`, `render_wave_table`, `splice_generated`, `plan_state_json`,
  `plan_master_plan_tables`, `apply_plan`, `emit_state`) plus the `derive_*` helpers added to
  `state.rs`; note that `derive_focus` is shared with `check_focus_drift`.
- Update `docs/index.md` only if a new doc is added (no new file expected here).
- **Primary files:** `docs/cli.md`, `docs/architecture.md`.
- **Depends on task 4.**

### 6. Validate
- Run the Validation Commands listed below and confirm all pass.

## Acceptance Criteria
- `mev emit-state <brain-root>` runs; **without** `--write` it writes nothing (dry-run) and reports the
  planned actions; **with** `--write` it updates the derived views in place.
- The emitted master-plan wave/dependency tables match the authored DAG: rows are in `wave` order and
  the dependency column reflects each block's `depends_on`; an open block with an unmet dependency
  renders with derived status `blocked`.
- Regeneration preserves every line of narrative outside the `<!-- BEGIN generated:wave-table -->` …
  `<!-- END generated:wave-table -->` sentinels; re-running the emit is idempotent (no further change).
- A master-plan file lacking the sentinels is skipped with a `W_EMIT_NO_SENTINEL` warning — never
  spliced into arbitrary prose.
- A leaf `state.json`'s `focus` is regenerated to match the v2 derivation rules (`now` = `in_progress`;
  `next` = ready open blocks in `wave` order; `blocked` = open blocks with an unmet `depends_on`, each
  carrying the unmet subset as `blocked_by[]`), while authored `tracks[]` survives unchanged.
- The emitted brain rollup matches the children's `tracks[]`: `repos[]` reflects each child's derived
  focus and `cross_repo[]` reflects cross-repo `depends_on` edges, while authored `tracks[]`/`backlog[]`/
  `tiers[]` (and the brain file's own `focus`) survive the JSON round-trip unchanged.
- `derive_focus` is the single derivation used by **both** `check_focus_drift` and `mev emit-state`;
  the existing focus-drift tests still pass (no behavior change in the validator).
- **Fixed-point property:** running `mev emit-state --write` then `mev validate-brain --state` on the
  same corpus reports zero `W_STATE_FOCUS_DRIFT` and zero `W_STATE_ROLLUP_DRIFT` — the emit is the
  drift check's fixed point.
- mev writes nothing to any database or network; the only writes are to the derived sections of
  Markdown/JSON files on disk.
- All four harness gates are green.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes
<filled in as work happens>

## Amendment Log
<!-- Append-only. Pipeline stages append one dated line here when they deviate from the spec. -->
_No amendments yet._
