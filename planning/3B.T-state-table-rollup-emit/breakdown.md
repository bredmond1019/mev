# Task Breakdown — MV.3B.T Task 3: Emit planners (state.json focus+rollup + master-plan tables)

## Source Spec
`planning/3B.T-state-table-rollup-emit/tasks.md` — **Task 3** ("Emit planners: state.json (focus + rollup) + master-plan tables, with dry-run/write split")

## Goal
Add the planners that turn loaded state into proposed file writes without performing IO inline:
regenerate leaf `focus` and brain `repos[]`/`cross_repo[]` in each `state.json` (one rewrite per file),
splice the wave/dependency table into each sibling `master-plan.md` between sentinels, and gate all
writes behind a dry-run/`--write` split. Pure compiler: files in → proposed file contents out.

## How to Use
Work top to bottom. Each sub-step is a single atomic action. Run the inline **Verify** checks as you
go — do not batch them at the end. Each check must pass before continuing.

**Preconditions (from Tasks 1 & 2 — must already be merged when this runs):**
- `src/brain/state.rs` exposes `derive_focus(src, file, graph, files) -> DerivedFocus { now: Vec<String>, next: Vec<String>, blocked: Vec<(String, Vec<BlockedBy>)> }`, `derive_rollup(children, graph, files) -> Vec<RepoRollup>`, `derive_cross_repo(files) -> Vec<CrossRepoEdge>`.
- `src/brain/emit.rs` exists (registered `pub mod emit;` in `src/brain/mod.rs`) and exposes `EmitError` (thiserror), `wave_order(graph, files) -> Vec<String>`, `render_wave_table(repo_slug, file, graph) -> String`, and `splice_generated(original: &str, marker: &str, generated: &str) -> Result<String, EmitError>`.
- All emit work lives in `src/brain/emit.rs`; Task 2 and Task 3 both own this file, so they are **sequentially dependent** (Task 3 `Depends on` Task 2) — never run them as parallel tasks (see Notes).

---

## Steps

### Step 3: Emit planners (state.json + master-plan tables, dry-run/write split)

#### 3.1 Add the `EmitAction` and `EmitPlan` types
**File:** `src/brain/emit.rs`
**Action:** add two public structs near the top of the module (below the existing `EmitError`).
```rust
/// A single proposed file write produced by a planner. Pure data — no IO is performed
/// until [`apply_plan`] is called.
#[derive(Debug, Clone)]
pub struct EmitAction {
    /// Absolute path of the file to (over)write.
    pub path: std::path::PathBuf,
    /// The complete proposed new contents of the file.
    pub new_content: String,
    /// Human note describing what changed (for the dry-run/write diagnostic message).
    pub note: String,
}

/// The output of a planner: the proposed writes plus any diagnostics raised while planning
/// (e.g. a missing-sentinel warning).
#[derive(Debug, Default)]
pub struct EmitPlan {
    pub actions: Vec<EmitAction>,
    pub diagnostics: Vec<crate::Diagnostic>,
}

impl EmitPlan {
    /// Merge another plan's actions and diagnostics into this one.
    pub fn extend(&mut self, other: EmitPlan) {
        self.actions.extend(other.actions);
        self.diagnostics.extend(other.diagnostics);
    }
}
```
**Note:** import paths — reuse the module's existing `use` block; `StateSource`, `StateFile`,
`StateGraph`, `Focus`, `Block`, `BlockedBy`, `RepoRollup`, `CrossRepoEdge` come from
`crate::brain::state`. Add `use crate::brain::state::{...}` as needed.

#### 3.2 Add a private `id_index` helper (id → (title, status)) for title backfill
**File:** `src/brain/emit.rs`
**Action:** add a private helper that builds, for one `StateFile`, a map from block `id` to its
`(title, status)` so the derived focus entries (which `derive_focus` returns as bare id strings) can be
rehydrated into `Block { id, title, status, ... }`.
```rust
/// Map every `tracks[].blocks[]` id in one file to its (title, authored status).
fn id_index(file: &StateFile) -> std::collections::HashMap<String, (String, Option<String>)> {
    let mut map = std::collections::HashMap::new();
    for track in &file.tracks {
        for block in &track.blocks {
            map.insert(block.id.clone(), (block.title.clone(), block.status.clone()));
        }
    }
    map
}
```

#### 3.3 Add `derived_focus_for(src, file, graph, files) -> Focus` — rehydrate `DerivedFocus` into a `Focus`
**File:** `src/brain/emit.rs`
**Action:** add a private fn that calls `derive_focus` and maps the returned id lists back into the
`Focus` struct, filling titles from `id_index` and setting `now` items' `status` to `in_progress`,
`blocked` items' `blocked_by` to the unmet subset returned by `derive_focus`.
```rust
fn derived_focus_for(
    src: &StateSource,
    file: &StateFile,
    graph: &StateGraph,
    files: &[(StateSource, StateFile)],
) -> Focus {
    let idx = id_index(file);
    let d = crate::brain::state::derive_focus(src, file, graph, files);
    let title_of = |id: &str| idx.get(id).map(|(t, _)| t.clone()).unwrap_or_default();

    let now = d.now.iter().map(|id| Block {
        id: id.clone(),
        title: title_of(id),
        status: Some("in_progress".to_string()),
        note: None,
        repo: None,
        blocked_by: Vec::new(),
    }).collect();

    let next = d.next.iter().map(|id| Block {
        id: id.clone(),
        title: title_of(id),
        status: None,
        note: None,
        repo: None,
        blocked_by: Vec::new(),
    }).collect();

    let blocked = d.blocked.iter().map(|(id, unmet)| Block {
        id: id.clone(),
        title: title_of(id),
        status: None,
        note: None,
        repo: None,
        blocked_by: unmet.clone(),
    }).collect();

    Focus { now, next, blocked }
}
```
**Note:** match the actual `Block` field set in `src/brain/state.rs` (`id`, `title`, `status`,
`note`, `repo`, `blocked_by`). If `DerivedFocus`'s field shapes differ from the Task-1 signature
above, adapt the mapping — the contract is "ids in, `Focus` out, titles from this file's tracks".

**Verify:** `cargo build` → compiles (types line up with `state.rs`).

#### 3.4 Add `plan_state_json(files, graph) -> EmitPlan`
**File:** `src/brain/emit.rs`
**Action:** add the public planner. One rewrite per `state.json`: leaf → regenerate `focus`; brain →
regenerate `repos[]` + `cross_repo[]` (leave brain `focus` untouched). Emit an `EmitAction` only when
the re-serialized derived file differs from the re-serialized **original** (fixed-point check that
ignores on-disk whitespace).
```rust
pub fn plan_state_json(
    files: &[(StateSource, StateFile)],
    graph: &StateGraph,
) -> EmitPlan {
    let mut plan = EmitPlan::default();

    // Children (leaf project files) for the brain rollup derivation.
    let children: Vec<(StateSource, StateFile)> = files
        .iter()
        .filter(|(_, f)| f.kind == "project")
        .cloned()
        .collect();

    for (src, file) in files {
        let mut derived = file.clone();

        match file.kind.as_str() {
            "project" => {
                derived.focus = derived_focus_for(src, file, graph, files);
            }
            "brain" => {
                derived.repos = crate::brain::state::derive_rollup(&children, graph, files);
                derived.cross_repo = crate::brain::state::derive_cross_repo(files);
                // brain `focus` is intentionally left untouched (aggregation rule unsettled).
            }
            _ => continue, // unknown kind already flagged by check_schema
        }

        // Fixed-point check: compare canonical serializations, not on-disk bytes.
        let original = match serde_json::to_string_pretty(file) {
            Ok(s) => s,
            Err(e) => {
                plan.diagnostics.push(crate::Diagnostic::warning(
                    &src.abs_path,
                    "W_EMIT_SERIALIZE_FAILED",
                    format!("could not serialize original state for {}: {e}", src.repo_slug),
                ));
                continue;
            }
        };
        let new_content = match serde_json::to_string_pretty(&derived) {
            Ok(s) => s,
            Err(e) => {
                plan.diagnostics.push(crate::Diagnostic::warning(
                    &src.abs_path,
                    "W_EMIT_SERIALIZE_FAILED",
                    format!("could not serialize derived state for {}: {e}", src.repo_slug),
                ));
                continue;
            }
        };

        if new_content != original {
            let note = if file.kind == "project" {
                format!("regenerate focus for '{}'", src.repo_slug)
            } else {
                format!("regenerate repos[]/cross_repo[] for '{}'", src.repo_slug)
            };
            plan.actions.push(EmitAction {
                path: src.abs_path.clone(),
                // serde_json::to_string_pretty omits the trailing newline; add one for POSIX text.
                new_content: format!("{new_content}\n"),
                note,
            });
        }
    }

    plan
}
```
**Note:** `to_string_pretty` emits no trailing newline; the live files end with one, so both the
"original" baseline and `new_content` must be compared/written consistently. Above, the comparison is
between two `to_string_pretty` outputs (both newline-free) → equal when data matches; the written
content appends `\n`. This means the **first** write may add a missing trailing newline; that is a
one-time normalization, after which the file is a fixed point.

**Verify:** `cargo build` → compiles.

#### 3.5 Add `plan_master_plan_tables(files, graph) -> EmitPlan`
**File:** `src/brain/emit.rs`
**Action:** add the public planner that splices the rendered wave table into each state file's sibling
`master-plan.md` under the `wave-table` marker. Missing file or missing sentinels → a
`W_EMIT_NO_SENTINEL` warning, no action (never invent sentinels).
```rust
pub fn plan_master_plan_tables(
    files: &[(StateSource, StateFile)],
    graph: &StateGraph,
) -> EmitPlan {
    let mut plan = EmitPlan::default();

    for (src, file) in files {
        // Sibling master-plan.md: state.json's parent dir / master-plan.md.
        let Some(planning_dir) = src.abs_path.parent() else { continue };
        let mp_path = planning_dir.join("master-plan.md");
        if !mp_path.exists() {
            plan.diagnostics.push(crate::Diagnostic::warning(
                &mp_path,
                "W_EMIT_NO_SENTINEL",
                format!("no master-plan.md beside '{}' state.json; skipping table emit", src.repo_slug),
            ));
            continue;
        }

        let original = match std::fs::read_to_string(&mp_path) {
            Ok(s) => s,
            Err(e) => {
                plan.diagnostics.push(crate::Diagnostic::warning(
                    &mp_path,
                    "W_EMIT_NO_SENTINEL",
                    format!("could not read master-plan.md for '{}': {e}", src.repo_slug),
                ));
                continue;
            }
        };

        let table = render_wave_table(&src.repo_slug, file, graph);
        match splice_generated(&original, "wave-table", &table) {
            Ok(new_content) => {
                if new_content != original {
                    plan.actions.push(EmitAction {
                        path: mp_path,
                        new_content,
                        note: format!("splice wave-table for '{}'", src.repo_slug),
                    });
                }
            }
            Err(_) => {
                plan.diagnostics.push(crate::Diagnostic::warning(
                    &mp_path,
                    "W_EMIT_NO_SENTINEL",
                    format!(
                        "master-plan.md for '{}' has no <!-- BEGIN generated:wave-table --> sentinels; skipping",
                        src.repo_slug
                    ),
                ));
            }
        }
    }

    plan
}
```
**Note:** `splice_generated` returns `Err(EmitError)` on missing/unbalanced sentinels (Task 2) — that
is the signal converted to `W_EMIT_NO_SENTINEL` here, not a hard error.

**Verify:** `cargo build` → compiles.

#### 3.6 Add `apply_plan(plan, write) -> Vec<Diagnostic>`
**File:** `src/brain/emit.rs`
**Action:** add the public executor. `write == true` → write each action's `new_content` to its
`path` and emit `I_EMIT_WROTE`; `write == false` (dry-run) → write nothing and emit `W_EMIT_DRY_RUN`.
Always pass through the plan's own diagnostics.
```rust
pub fn apply_plan(plan: &EmitPlan, write: bool) -> Vec<crate::Diagnostic> {
    let mut diags = plan.diagnostics.clone();

    for action in &plan.actions {
        if write {
            match std::fs::write(&action.path, action.new_content.as_bytes()) {
                Ok(()) => diags.push(crate::Diagnostic::warning(
                    &action.path,
                    "I_EMIT_WROTE",
                    format!("wrote: {}", action.note),
                )),
                Err(e) => diags.push(crate::Diagnostic::error(
                    &action.path,
                    "E_EMIT_WRITE_FAILED",
                    format!("failed to write {}: {e}", action.path.display()),
                )),
            }
        } else {
            diags.push(crate::Diagnostic::warning(
                &action.path,
                "W_EMIT_DRY_RUN",
                format!("would write (dry-run): {}", action.note),
            ));
        }
    }

    diags
}
```
**Note:** `Diagnostic` has only `error`/`warning` constructors (no info level) — `I_EMIT_WROTE` and
`W_EMIT_DRY_RUN` are Warning severity so they surface in the human + `--json` reporter without failing
the exit code. `E_EMIT_WRITE_FAILED` is the only Error-severity emit code (a real IO failure should
fail the run).

**Verify:** `cargo build && cargo clippy -- -D warnings` → clean.

#### 3.7 Re-export the new planner surface from the module
**File:** `src/brain/emit.rs`
**Action:** ensure `EmitAction`, `EmitPlan`, `plan_state_json`, `plan_master_plan_tables`, and
`apply_plan` are `pub` (they are, above). No `pub use` needed here — Task 4 adds the crate-root
re-export in `src/lib.rs`.

**Verify:** `cargo build` → compiles.

#### 3.8 Add `tests/brain_emit.rs` integration tests for the planners
**File:** `tests/brain_emit.rs` (created in Task 2 — append a new `mod task3_planners { ... }` section,
or add the tests at the end of the existing file).
**Action:** reuse the fixture pattern from `tests/brain_state.rs` (copy the `write_file`, `temp_dir`,
`write_json`, `write_brain_toml`, and leaf/brain `state.json` builders, or factor them into the file).
Add these tests. Each builds a fixture, calls the planners directly via the public API, and asserts on
the returned `EmitPlan`/diagnostics — no writes unless the test asserts the `write=true` path.

Suite `describe: plan_state_json`:
- `leaf_focus_regenerated_from_tracks` — build a leaf whose stored `focus.now` lists a block that is
  `closed` in `tracks[]` while another block is `in_progress`; load via the same discover→load→build
  pipeline `validate_brain_state` uses (expose or replicate: `discover_state_files` + `load_state` +
  `build_state_graph` are all `pub` in `state.rs`); call `plan_state_json(&loaded, &graph)`; assert
  exactly one `EmitAction` whose `path` is the leaf's `state.json` and whose `new_content` parses to a
  `focus.now` of the `in_progress` block (not the stale `closed` one).
- `brain_rollup_regenerated_preserves_authored` — build a brain file with stale `repos[]` and a leaf
  that advanced; call the planner; assert the brain `EmitAction`'s `new_content` parses to `repos[]`
  matching the child's derived focus AND still contains the authored `backlog[]`/`tiers[]` intact.
- `fixed_point_no_action` — build a clean fixture whose stored `focus`/`repos[]` already match the
  derivation; assert `plan_state_json` returns zero `actions` for those files.
- `brain_focus_untouched` — assert the brain file's `focus` in any emitted `new_content` equals the
  authored brain `focus` (the planner must not rewrite it).

Suite `describe: plan_master_plan_tables`:
- `splices_table_inside_sentinels` — write a `master-plan.md` beside a leaf `state.json` containing
  narrative + `<!-- BEGIN generated:wave-table -->\n<!-- END generated:wave-table -->`; call the
  planner; assert the `EmitAction.new_content` contains the rendered table between the sentinels and
  every narrative line outside them is byte-identical to the original.
- `no_sentinels_warns_no_action` — write a `master-plan.md` with no sentinels; assert zero actions for
  that file and one `W_EMIT_NO_SENTINEL` diagnostic.
- `missing_master_plan_warns` — a state file with no sibling `master-plan.md`; assert one
  `W_EMIT_NO_SENTINEL` and no action.

Suite `describe: apply_plan`:
- `dry_run_writes_nothing` — capture the on-disk bytes of a fixture file, build a plan with one action
  targeting it, call `apply_plan(&plan, false)`; assert the file bytes are unchanged and the returned
  diagnostics include `W_EMIT_DRY_RUN`.
- `write_true_persists` — same plan, call `apply_plan(&plan, true)`; assert the file now equals the
  action's `new_content` and the diagnostics include `I_EMIT_WROTE`.

**Verify:** `cargo test --test brain_emit` → all new tests green.

**Verify (group):** `cargo fmt --check && cargo clippy -- -D warnings && cargo test` → all gates green.

---

## Acceptance Criteria
*(from the spec — the Task-3-relevant subset; full list in `tasks.md`)*
- A leaf `state.json`'s `focus` is regenerated to match the v2 derivation rules (`now` = `in_progress`;
  `next` = ready open blocks in `wave` order; `blocked` = open blocks with an unmet `depends_on`, each
  carrying the unmet subset as `blocked_by[]`), while authored `tracks[]` survives unchanged.
- The emitted brain rollup matches the children's `tracks[]`: `repos[]` reflects each child's derived
  focus and `cross_repo[]` reflects cross-repo `depends_on` edges, while authored `tracks[]`/`backlog[]`/
  `tiers[]` (and the brain file's own `focus`) survive the JSON round-trip unchanged.
- Regeneration preserves every line of narrative outside the `<!-- BEGIN generated:wave-table -->` …
  `<!-- END generated:wave-table -->` sentinels; re-running the emit is idempotent (no further change).
- A master-plan file lacking the sentinels is skipped with a `W_EMIT_NO_SENTINEL` warning — never
  spliced into arbitrary prose.
- Without `--write` the planners produce actions but `apply_plan` writes nothing (dry-run); a file
  already at its derived fixed point produces no `EmitAction`.
- mev writes nothing to any database or network; the only writes are to derived sections of files.
- All four harness gates are green.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes
- **Disjoint-ownership flag.** Task 2 and Task 3 both edit `src/brain/emit.rs`, and Task 3 also appends
  to `tests/brain_emit.rs` (created by Task 2). These are **sequentially dependent** — Task 3 `Depends
  on` Task 2 in `tasks.md`, so `/sdlc-flow`'s sequential execution is safe. Do **not** run them as
  parallel tasks or they collide at merge. Task 1's edits (`src/brain/state.rs`, `tests/brain_state.rs`)
  are disjoint from both.
- **serde round-trip drops un-modeled fields.** `StateFile` is extras-tolerant on *read* (no
  `deny_unknown_fields`) but does **not** capture unknown keys, so `to_string_pretty(&StateFile)` emits
  only the modeled fields. Any field present in a live `state.json` but absent from the struct would be
  lost on emit. The v2 schema (`../planning/state-schema.md`) is fully modeled by the current struct, so
  this is safe today — but if the schema gains a field, add it to `StateFile` **before** relying on
  `emit-state --write` against live files. `serde_json` has no `preserve_order` feature enabled here, so
  output key order follows the `StateFile` struct field order (deterministic), not the on-disk order.
- **`Diagnostic` has no info severity** (`error`/`warning` only — see `src/lib.rs`). `I_EMIT_WROTE` and
  `W_EMIT_DRY_RUN` are Warning-severity so they report without failing the exit code; only
  `E_EMIT_WRITE_FAILED` (a real IO error) is Error-severity. Confirm these codes are documented in
  `docs/cli.md` in Task 5.
- **Fixed-point depends on Task 1 fidelity.** The "no action when already correct" behaviour and the
  `MV.3.P2` warn→error precondition both hinge on `plan_state_json` deriving exactly what
  `check_focus_drift`/`check_rollup` expect. Because both call the **same** `derive_focus`/`derive_rollup`
  (Task 1), they cannot diverge — do not reimplement the derivation inline here.
- **`render_wave_table` / `wave_order` are Task 2's** — this breakdown calls them but does not define
  them. If Task 2's `splice_generated` marker convention differs from `"wave-table"`
  (`<!-- BEGIN generated:wave-table -->`), reconcile the marker string in 3.5 to match Task 2.
