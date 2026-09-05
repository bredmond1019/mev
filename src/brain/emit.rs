//! Emit module — derived-view generator for `mev emit-state`.
//!
//! This module is the **single derivation engine** for every generated view the
//! v2 state schema declares.  The public surface for Task 2:
//!
//! - [`EmitError`] — error type for sentinel-related failures.
//! - [`wave_order`] — all block keys (`"repo:id"`) sorted by `wave` ascending.
//! - [`render_wave_table`] — Markdown table of a repo's blocks in wave order.
//! - [`render_block_graph_reconcile_failed`] — per-block `(reconcile_failed)`
//!   annotation over a `BlockGraphExport`'s nodes.
//! - [`render_hq_board`] — NOW/NEXT/BLOCKED Operating Board Markdown from a
//!   brain-derived [`Focus`] + `cross_repo[]` edges.
//! - [`render_unified_board`] — NOW/NEXT/BLOCKED/DUE-SOON unified priority
//!   board unioning engineering + business blocks, tagged `[BIZ]`/`[ENG]`
//!   (`MV.6.B`).
//! - [`splice_generated`] — idempotent sentinel-splice into an existing Markdown
//!   document.
//!
//! Tasks 3 and 4 extend this file with the planners (`EmitAction`, `EmitPlan`,
//! `plan_state_json`, `plan_master_plan_tables`, `apply_plan`) and the library
//! entry point (`emit_state`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::brain::carryover::{
    CarryoverLane, CarryoverVerdict, TriageLane, clears_when_display, rank_carryover,
};
use crate::brain::config::BrainConfig;
use crate::brain::distill::{DistilledEntry, distill_stale_age, parse_distilled};
use crate::brain::state::{
    ApprovalDep, Backlog, Block, BlockDep, BlockedBy, Carryover, CrossRepoEdge, Epic, EpicEdge,
    EpicEdges, ExternalDep, Focus, OperatorDep, RepoRollup, StateFile, StateGraph, StateSource,
    TierScope, TrackBlock, backlog_stale_age, carryover_kind_str, carryover_stale_age,
    derive_brain_focus, derive_cross_repo, derive_epic_edges, derive_epic_focus, derive_focus,
    derive_rollup, effective_priorities, is_snoozed, staleness_anchor, tier_scope_for,
};

// ---------------------------------------------------------------------------
// Generated-region marker constants
// ---------------------------------------------------------------------------

/// Named constants for the `<!-- BEGIN generated:{marker} -->` / `<!-- END
/// generated:{marker} -->` sentinel markers used across the state-sync-loop
/// status generators.
///
/// [`WAVE_TABLE`] is used by [`plan_master_plan_tables`] today; the remaining
/// constants name the markers `MV.4.B`/`MV.4.C` target (project caches, tier
/// rollups, and the HQ board) so every generator references a single shared
/// source of truth instead of ad-hoc string literals.
pub mod markers {
    /// Marker for the per-repo wave-order roadmap table spliced into a
    /// repo's `master-plan.md`.
    pub const WAVE_TABLE: &str = "wave-table";

    /// Marker for a project's synced status cache (`docs/projects/<slug>.md`
    /// in the brain, or the equivalent per-tier cache).
    pub const PROJECT_CACHE: &str = "project-cache";

    /// Marker for a tier's rolled-up status summary.
    pub const TIER_ROLLUP: &str = "tier-rollup";

    /// Marker for the cross-repo HQ status board.
    pub const HQ_BOARD: &str = "hq-board";

    /// Marker for the unified priority-ranked NOW/NEXT/BLOCKED/DUE-SOON board
    /// unioning engineering + business blocks (`MV.6.B`). Separate from
    /// [`HQ_BOARD`], which stays untouched.
    pub const UNIFIED_BOARD: &str = "unified-board";

    /// Marker for the Attention board — stale carryover, aging backlog, and
    /// orphaned captures. Emitted tier-scoped into every brain-level `status.md`
    /// (HQ = all repos; each tier sub-brain = that tier's repos).
    pub const ATTENTION: &str = "attention";

    /// Marker for the per-epic board — one NOW/NEXT/BLOCKED + progress +
    /// cross-epic relationships section per active cross-repo initiative.
    /// Emitted into every brain-level `status.md` that carries the sentinels.
    pub const EPIC_BOARD: &str = "epic-board";

    /// Marker for one epic's cross-repo sequence table, spliced into that
    /// epic's own `plan` document. Distinct from [`EPIC_BOARD`]: the board is a
    /// live focus snapshot, this is the full ordered roadmap for one initiative.
    pub const EPIC_SEQUENCE: &str = "epic-sequence";

    /// Marker for the initiative index + per-phase block sections spliced into
    /// a repo's `master-plan.md` by [`crate::brain::master_plan`]
    /// (`MV.ticket.master-plan-generator`). Distinct from [`WAVE_TABLE`]: the
    /// wave table is a flat roadmap table, this is the fuller narrative-style
    /// rendering grouped by initiative and phase.
    pub const MASTER_PLAN_BODY: &str = "master-plan-body";
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors produced by the emit module.
#[derive(Debug, Error)]
pub enum EmitError {
    /// The `<!-- BEGIN generated:{marker} -->` sentinel is missing from the
    /// document, or the `END` sentinel does not follow the `BEGIN` sentinel.
    #[error("missing or unbalanced sentinels for marker '{marker}' in document")]
    MissingSentinel { marker: String },
}

// ---------------------------------------------------------------------------
// wave_order — full roadmap ordering (all blocks, not only ready/open)
// ---------------------------------------------------------------------------

/// Return all block keys (`"repo:id"`) across `files`, sorted by `wave` ascending
/// (`None` last), with ties broken by track iteration order then block array index.
///
/// This is the full-roadmap sibling of `ready_order` (which filters to ready/open
/// blocks only).  `wave_order` includes every block regardless of status so that
/// [`render_wave_table`] can produce a complete roadmap table.
///
/// The `graph` parameter is accepted for API symmetry with `ready_order` and
/// future forward-compat (e.g. cycle-aware ordering); it is not used today.
pub fn wave_order(_graph: &StateGraph, files: &[(StateSource, StateFile)]) -> Vec<String> {
    // Collect (wave_sort_key, iteration_index, "repo:id") for every block.
    let mut entries: Vec<(i64, usize, String)> = Vec::new();
    let mut iteration_index: usize = 0;

    for (src, file) in files {
        for track in &file.tracks {
            for block in &track.blocks {
                let wave_key = block.wave.unwrap_or(i64::MAX);
                let key = format!("{}:{}", src.repo_slug, block.id);
                entries.push((wave_key, iteration_index, key));
                iteration_index += 1;
            }
        }
    }

    // Primary sort: wave asc (None → i64::MAX → last). Tiebreak: iteration order (stable).
    entries.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    entries.into_iter().map(|(_, _, key)| key).collect()
}

// ---------------------------------------------------------------------------
// epic_members — one epic's blocks, in cross-repo dependency order
// ---------------------------------------------------------------------------

/// Cycle-safe DFS topological order over the full `depends_on` graph (every
/// block, every repo), seeded in [`wave_order`] so that within a component
/// with no dependency constraint — most block pairs, which just aren't
/// linked by an edge — the order still reads in the same stable
/// wave-then-iteration order [`wave_order`] would produce on its own. Only
/// `{type:"block"}` deps that resolve to a real node in this corpus
/// participate; `external` deps have no target node and cannot constrain
/// ordering.
///
/// Cycle-safe: a node already on the current DFS stack short-circuits
/// instead of recursing again, mirroring the guard in
/// [`crate::brain::state::effective_priorities`] — this does not assume
/// `MV.3.P2`'s cycle check already rejected the corpus.
///
/// Returns every block's `"repo:id"` key, in order.
pub fn topo_order(graph: &StateGraph, files: &[(StateSource, StateFile)]) -> Vec<String> {
    use std::collections::HashSet;

    // Index every block by "repo:id".
    let mut by_key: HashMap<String, (String, &TrackBlock)> = HashMap::new();
    for (src, file) in files {
        for track in &file.tracks {
            for block in &track.blocks {
                by_key.insert(
                    format!("{}:{}", src.repo_slug, block.id),
                    (src.repo_slug.clone(), block),
                );
            }
        }
    }

    // Forward deps: key -> [dep keys it depends_on], block-type only, and
    // only where the target actually resolves to a node in this corpus.
    let mut deps: HashMap<&str, Vec<String>> = HashMap::new();
    for (key, (_, block)) in &by_key {
        let ds = block
            .depends_on
            .iter()
            .filter_map(|dep| match dep {
                BlockedBy::Block(BlockDep { repo, id, .. }) => {
                    let dep_key = format!("{repo}:{id}");
                    by_key.contains_key(&dep_key).then_some(dep_key)
                }
                BlockedBy::External(_) | BlockedBy::Operator(_) | BlockedBy::Approval(_) => None,
            })
            .collect();
        deps.insert(key.as_str(), ds);
    }

    fn visit(
        key: &str,
        deps: &HashMap<&str, Vec<String>>,
        visited: &mut HashSet<String>,
        on_stack: &mut HashSet<String>,
        output: &mut Vec<String>,
    ) {
        if visited.contains(key) || on_stack.contains(key) {
            return;
        }
        on_stack.insert(key.to_string());
        if let Some(ds) = deps.get(key) {
            for d in ds {
                visit(d, deps, visited, on_stack, output);
            }
        }
        on_stack.remove(key);
        visited.insert(key.to_string());
        output.push(key.to_string());
    }

    let mut visited: HashSet<String> = HashSet::new();
    let mut on_stack: HashSet<String> = HashSet::new();
    let mut order: Vec<String> = Vec::new();
    for key in wave_order(graph, files) {
        visit(&key, &deps, &mut visited, &mut on_stack, &mut order);
    }

    order
}

/// Every block claiming `slug`, across all repos, in dependency-respecting
/// order.
///
/// Returns `(repo_slug, block)` pairs — the cross-repo sequence for one
/// initiative, which is what an epic's sequence table renders.
///
/// Cross-repo `wave` numbers are **not** on a shared scale (per-repo authors
/// pick their own range — bastion uses 1-7, bastion-web 10-60, bastion-ui
/// 1-20 — see the `epic-sequence-wave-scale` carryover this superseded), so
/// sorting a multi-repo table by raw `wave` alone can misrepresent an
/// initiative's real work order or strand a `None`-wave block at the bottom.
/// This filters [`topo_order`]'s full-graph topological order (every block,
/// every repo — not just this epic's members) down to `slug`'s members.
/// Walking the *full* graph (rather than only edges between two members)
/// means a member gated by a same-repo, out-of-epic prerequisite still sorts
/// after it correctly.
pub fn epic_members<'a>(
    graph: &StateGraph,
    files: &'a [(StateSource, StateFile)],
    slug: &str,
) -> Vec<(String, &'a TrackBlock)> {
    // Index every block (regardless of epic membership — dependency chains
    // can pass through non-member nodes) by "repo:id".
    let mut by_key: HashMap<String, (String, &TrackBlock)> = HashMap::new();
    for (src, file) in files {
        for track in &file.tracks {
            for block in &track.blocks {
                by_key.insert(
                    format!("{}:{}", src.repo_slug, block.id),
                    (src.repo_slug.clone(), block),
                );
            }
        }
    }

    topo_order(graph, files)
        .into_iter()
        .filter_map(|key| by_key.get(&key).cloned())
        .filter(|(_, block)| block.epics.iter().any(|s| s == slug))
        .collect()
}

/// Resolve one epic's cross-repo sequence — [`epic_members`], but implementing
/// `MV.13.D` Task 3's authored-vs-derived precedence rule instead of always reading
/// authored `block.epics`.
///
/// # The precedence rule (settled in `planning/MV.13.D/tasks.md`; not re-litigated here)
///
/// | `epic.kind` | Membership source | Why |
/// |---|---|---|
/// | `program` | **Derived lane membership wins where derivable** ([`crate::brain::lane_segments::derive_program_membership`]); authored `block.epics` is advisory and adds no members — *unless* derived membership is empty, in which case authored `block.epics` is used as a fallback. | A program's membership *is* its lane chain — that is the thing that executes. But a program that finished before lane tooling existed has no lane files to derive from, and rendering it as "no member blocks" misrepresents completed work; falling back to what was actually authored recovers that history without reintroducing ambiguity for live programs (see below). |
/// | `area` (or unset) | **Authored `block.epics` is the only source** — falls straight through to [`epic_members`]. | An area has no lanes to derive from; a `kind`-less epic gets today's behaviour unchanged. |
///
/// **Union was considered and rejected.** A block authored to epic X and also derived
/// into roadmap Y's lane would then render under both — exactly the double-counting
/// D57's two-axis rule (and `MV.13.A`'s `E_LANE_DOUBLE_CLAIM`) exists to prevent, and it
/// would misstate the remaining depth of both initiatives. So for `kind: program` the
/// derived set is used exclusively, never unioned with authored tags — the fallback below
/// does not change this: it only ever substitutes authored for derived, never adds to it.
///
/// For `kind: program`, the epic's `plan` document names the executing roadmap slug
/// (via [`crate::brain::lane_segments::roadmap_slug_from_plan_path`]); derived
/// positions are matched back to real [`TrackBlock`]s in `files` and returned in the
/// order [`crate::brain::lane_segments::derive_program_membership`] preserves.
///
/// **Amendment (chore `mev-chore-epic-membership-derived-fallback`): derived wins where
/// derivable, not unconditionally.** `MV.13.D`'s original rule made a program epic with no
/// lane files render an empty table even when it had genuine authored membership — correct
/// for `epic_members_resolved_program_kind_prefers_derived_lane_membership_over_authored_tags`'s
/// live-conflict shape, but wrong for a program like `bullet-proof-software` that completed
/// and was archived before lane files existed (31 rows → 0) or one like `demand-ready` where
/// only part of its membership ever got a lane file (44 rows → 29). So: whenever the derived
/// set for this program ends up empty *once a roadmap slug is known* — no lane files exist
/// for that roadmap, or lane files exist but derived no positions that matched a real
/// block — this falls back to authored `block.epics` via [`epic_members`] instead of
/// returning empty. **The fallback keys on "derived membership is empty," not on "no lane
/// files exist on disk."** Those two conditions produce the same observable set today, but
/// checking emptiness is one fewer thing to keep in sync with the filesystem and treats
/// "lane files that happen to derive nothing" the same as "no lane files at all" — both are
/// cases where derived membership has no signal to contribute, so authored is the better
/// answer either way. This does **not** extend to the case where `plan` never resolves to a
/// roadmap slug at all (a mis-tagged program epic) — that stays empty, since there is no
/// roadmap to have failed to derive from; see the comment at that check below. This cannot
/// reintroduce double-counting: the fallback only fires when the derived set is empty, so a
/// program table is built from derived rows exclusively or authored rows exclusively, never
/// both. `origin_roadmap`-adopted blocks are already folded into the correct (executing)
/// roadmap's derived set by `derive_program_membership`, so when derived is non-empty they
/// still render here exactly once.
pub fn epic_members_resolved<'a>(
    root: &Path,
    graph: &StateGraph,
    files: &'a [(StateSource, StateFile)],
    epic: &Epic,
) -> Vec<(String, &'a TrackBlock)> {
    let is_program = crate::brain::state::epic_kind_raw(epic)
        .and_then(|v| v.as_str())
        .is_some_and(|k| k == "program");
    if !is_program {
        return epic_members(graph, files, &epic.slug);
    }

    // A `plan` doc that does not resolve to a roadmap slug (e.g. a mis-tagged
    // `kind: program` epic pointing at an area's `.../epics/<slug>.md`) is not
    // "derived membership is empty" — it is "this epic is not actually lane-
    // derivable at all". That stays empty rather than falling back to
    // authored tags, which would silently paper over the mis-tagging instead
    // of surfacing it.
    let Some(roadmap_slug) = epic
        .plan
        .as_deref()
        .and_then(crate::brain::lane_segments::roadmap_slug_from_plan_path)
    else {
        return Vec::new();
    };

    let by_roadmap = crate::brain::lane_segments::derive_program_membership(root, files);
    let Some(positions) = by_roadmap.get(&roadmap_slug) else {
        // No lane files derived anything for this roadmap — fall back.
        return epic_members(graph, files, &epic.slug);
    };

    let mut by_key: HashMap<String, (String, &TrackBlock)> = HashMap::new();
    for (src, file) in files {
        for track in &file.tracks {
            for block in &track.blocks {
                by_key.insert(
                    format!("{}:{}", src.repo_slug, block.id),
                    (src.repo_slug.clone(), block),
                );
            }
        }
    }

    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let derived: Vec<(String, &TrackBlock)> = positions
        .iter()
        .filter_map(|p| by_key.get(&format!("{}:{}", p.repo, p.id)).cloned())
        .filter(|(_, block)| seen.insert(block.id.clone()))
        .collect();

    if derived.is_empty() {
        // Lane files exist for this roadmap but derived no positions (or none
        // matched a real block) — same "derived has no signal" case as the
        // branches above, so fall back rather than render an empty table.
        return epic_members(graph, files, &epic.slug);
    }

    derived
}

// ---------------------------------------------------------------------------
// render_wave_table — Markdown table for one repo's blocks
// ---------------------------------------------------------------------------

/// Render a Markdown table of `repo_slug`'s blocks in wave order.
///
/// Columns: `Wave | Block | Title | Status | Depends on`
///
/// - `Status` shows the **derived** state: an open block with at least one unmet
///   `depends_on` renders as `blocked`; otherwise the block's authored status is
///   used (defaulting to `open` when absent).
/// - `Depends on` lists `depends_on` targets as `repo:id` (for
///   `{type:"block"}`) or `external:<what>` (for `{type:"external"}`).
/// - `Wave` column shows the authored wave number, or `—` when absent.
///
/// The table is rendered without a trailing newline; callers that embed it inside
/// a document are responsible for any required surrounding blank lines.
pub fn render_wave_table(
    repo_slug: &str,
    file: &StateFile,
    graph: &StateGraph,
    global_status: &HashMap<String, Option<String>>,
) -> String {
    // Build a status map across all blocks in this file: id → authored status.
    // We need it to compute derived "blocked" status (unmet dep check).
    let mut all_status: HashMap<String, Option<String>> = HashMap::new();
    for track in &file.tracks {
        for block in &track.blocks {
            all_status.insert(block.id.clone(), block.status.clone());
        }
    }

    // Cross-repo deps are resolved against `global_status` (built by
    // `global_status_map` across every loaded state file): a cross-repo dep is
    // met when the target block's authored status is `closed`, unmet otherwise
    // (including when the target is absent from the map).
    //
    // Build the ordered list of (wave_key, iteration_idx, block_id) for this repo.
    let mut ordered: Vec<(i64, usize, &str)> = Vec::new();
    let mut idx: usize = 0;
    for track in &file.tracks {
        for block in &track.blocks {
            ordered.push((block.wave.unwrap_or(i64::MAX), idx, block.id.as_str()));
            idx += 1;
        }
    }
    ordered.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));

    // Build a lookup: block_id → &TrackBlock for cell rendering.
    let mut block_map: HashMap<&str, &crate::brain::state::TrackBlock> = HashMap::new();
    for track in &file.tracks {
        for block in &track.blocks {
            block_map.insert(block.id.as_str(), block);
        }
    }

    // `graph` is accepted for API symmetry with `wave_order` / forward-compat
    // (e.g. cycle-aware derivation); it is not used for the `blocked` derivation
    // today — cross-repo resolution goes through `global_status` instead.
    let _ = graph;

    // Header
    let header = "| Wave | Block | Title | Status | Depends on |";
    let sep = "|------|-------|-------|--------|------------|";

    let mut rows: Vec<String> = Vec::new();
    rows.push(header.to_string());
    rows.push(sep.to_string());

    for (wave_key, _, block_id) in &ordered {
        let Some(block) = block_map.get(block_id) else {
            continue;
        };

        // Wave column value
        let wave_col = if *wave_key == i64::MAX {
            "\u{2014}".to_string() // em-dash
        } else {
            wave_key.to_string()
        };

        // Derived status: check if this block is "blocked" (open + unmet dep).
        let authored_status = block.status.as_deref().unwrap_or("open");
        let derived_status = if authored_status == "open" {
            // Check for unmet deps — conservative: external deps always unmet;
            // block deps only resolved for same-repo.
            let has_unmet = block.depends_on.iter().any(|dep| match dep {
                BlockedBy::External(_) | BlockedBy::Operator(_) | BlockedBy::Approval(_) => true,
                BlockedBy::Block(BlockDep { repo, id, .. }) => {
                    if repo == repo_slug {
                        // Same-repo: check authored status.
                        all_status.get(id.as_str()).and_then(|s| s.as_deref()) != Some("closed")
                    } else {
                        // Cross-repo: resolve against the global status map — met
                        // only when the target block is authored `closed`; unmet
                        // when open or absent from the map.
                        let key = format!("{repo}:{id}");
                        global_status.get(&key).and_then(|s| s.as_deref()) != Some("closed")
                    }
                }
            });
            if has_unmet { "blocked" } else { "open" }
        } else {
            authored_status
        };

        // Depends-on column
        let deps_col = if block.depends_on.is_empty() {
            String::new()
        } else {
            block
                .depends_on
                .iter()
                .map(|dep| match dep {
                    BlockedBy::Block(BlockDep { repo, id, .. }) => format!("{repo}:{id}"),
                    BlockedBy::External(ExternalDep { what }) => format!("external:{what}"),
                    BlockedBy::Operator(OperatorDep { slug, .. }) => okf_core::op_id(slug),
                    BlockedBy::Approval(ApprovalDep { slug, .. }) => okf_core::op_id(slug),
                })
                .collect::<Vec<_>>()
                .join(", ")
        };

        rows.push(format!(
            "| {wave_col} | {block_id} | {} | {derived_status} | {deps_col} |",
            block.title
        ));
    }

    rows.join("\n")
}

// ---------------------------------------------------------------------------
// render_block_graph_reconcile_failed — human-readable reconcile_failed surfacing
// ---------------------------------------------------------------------------

/// Render one line per `export.nodes` entry (in the export's existing `topo_index`
/// order): `"{repo}:{id} — {title}"`, annotated `" (reconcile_failed)"` when
/// [`crate::brain::block_graph::BlockGraphNode::reconcile_failed`] is `Some(true)`.
///
/// This is the human-readable sibling of `BlockGraphNode.reconcile_failed`'s JSON
/// surfacing (`#[serde(skip_serializing_if)]`, task 2 of `ticket-reconcile-failed-consumer`).
/// Per base-template's `docs/data-contract.md` (`doc_id: sdlc-run-state-data-contract`,
/// the pinned authority for the terminal run-state vocabulary — not re-derived here), a
/// block whose most recent run ended `reconcile_failed` must not silently read the same
/// as any other open/unclosed block in a surface that displays it. Follows the same
/// terse `"{repo}:{id} — {title} (annotation)"` convention [`render_hq_board_line`] uses
/// for `(blocked by ...)` — a per-block annotation, not a new lane or a new section.
///
/// A node whose `reconcile_failed` is `Some(false)` or `None` renders with no annotation
/// at all, so a corpus with no `reconcile_failed` blocks renders byte-identical output to
/// what this function produced before the annotation existed. Rendered without a
/// trailing newline, matching the [`render_wave_table`] / [`render_hq_board`] convention;
/// callers that embed it inside a document own any surrounding blank lines.
pub fn render_block_graph_reconcile_failed(
    export: &crate::brain::block_graph::BlockGraphExport,
) -> String {
    export
        .nodes
        .iter()
        .map(|node| {
            let mut line = format!("{} — {}", node.key, node.title);
            if node.reconcile_failed == Some(true) {
                line.push_str(" (reconcile_failed)");
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// global_status_map — cross-file "repo:id" → authored status
// ---------------------------------------------------------------------------

/// Build a global `"{repo_slug}:{block_id}" → authored status` map across
/// **every** loaded state file, not just one repo.
///
/// This is the cross-file status lookup [`render_wave_table`] lacks today (it
/// only ever sees one repo's [`StateFile`]): callers that need to resolve a
/// cross-repo `depends_on` edge — e.g. "is `core:X` closed?" — can look it up
/// here regardless of which file `core:X` was declared in.
///
/// Keys are namespaced with `src.repo_slug` so blocks with the same `id` in
/// different repos never collide. The value is the block's authored
/// `status` field verbatim (`None` when absent — callers decide the default,
/// e.g. `open`).
pub fn global_status_map(files: &[(StateSource, StateFile)]) -> HashMap<String, Option<String>> {
    let mut map = HashMap::new();
    for (src, file) in files {
        for track in &file.tracks {
            for block in &track.blocks {
                let key = format!("{}:{}", src.repo_slug, block.id);
                map.insert(key, block.status.clone());
            }
        }
    }
    map
}

// ---------------------------------------------------------------------------
// group_blocked_by_gate — shared-identity dedup for operator/approval edges
// ---------------------------------------------------------------------------

/// One item in a rendered `BLOCKED` (or any other) section: either a plain
/// block, or a group of blocks that share one `operator`/`approval`
/// `depends_on` slug (Task 5, MV shared-identity dedup — `ticket-operator-edge-graph`).
///
/// A shared slug is a join key: several otherwise-unrelated blocks across any
/// number of repos can name the same operator session or approval decision.
/// Without collapsing them, one cleared gate produces N identical-looking
/// lines instead of one — the "new noise source" the ticket calls out as the
/// single highest-risk failure mode of this mechanism. `blocks.len() == 1`
/// covers both a plain block (no operator/approval entry at all) and a slug
/// that happens to gate only one block; both render identically to how a lone
/// block always rendered.
#[derive(Debug, Clone)]
pub struct BlockedGroup<'a> {
    /// The blocks sharing this slug, in first-appearance order within the
    /// input. Never empty.
    pub blocks: Vec<&'a Block>,
    /// The shared `operator`/`approval` edge this group is keyed on. `None`
    /// for a plain block whose `blocked_by` carries no such entry (e.g. only a
    /// `Block`/`External` dependency, or none at all) — dedup does not apply.
    pub gate: Option<&'a BlockedBy>,
}

impl<'a> BlockedGroup<'a> {
    /// The minimum effective priority across every block in this group — "the
    /// deduped item carries the minimum effective priority of the blocks it
    /// gates" (Task 5 AC). Delegates to [`effective_priority_for`] per block,
    /// so an absent `effective` entry falls back to the block's own raw
    /// `priority`, and a fully-absent priority sorts last (`u8::MAX`). Never
    /// panics on an empty group — `blocks` is always non-empty by
    /// construction — but returns `u8::MAX` defensively if it ever were.
    pub fn effective_priority(&self, effective: &HashMap<String, u8>) -> u8 {
        self.blocks
            .iter()
            .map(|b| {
                effective_priority_for(
                    b.repo.as_deref().unwrap_or(""),
                    &b.id,
                    b.priority,
                    effective,
                )
            })
            .min()
            .unwrap_or(u8::MAX)
    }
}

/// The `("operator"|"approval", slug)` grouping key for one block, taken from
/// the first `Operator`/`Approval` entry in its `blocked_by` (a block is not
/// expected to carry more than one gate; if it does, the first one present
/// wins deterministically). `None` when `blocked_by` carries no such entry.
fn gate_key(block: &Block) -> Option<(&'static str, &str)> {
    block.blocked_by.iter().find_map(|dep| match dep {
        BlockedBy::Operator(OperatorDep { slug, .. }) => Some(("operator", slug.as_str())),
        BlockedBy::Approval(ApprovalDep { slug, .. }) => Some(("approval", slug.as_str())),
        BlockedBy::Block(_) | BlockedBy::External(_) => None,
    })
}

/// Group `blocks` by shared `operator`/`approval` `depends_on` slug (Task 5,
/// `ticket-operator-edge-graph`) so a rendered section can emit one item per
/// slug instead of one per block.
///
/// Order-preserving: a group occupies the position of its first member's
/// first occurrence in `blocks`; later members sharing the same slug are
/// folded into that existing group rather than each claiming a new slot. A
/// block with no `operator`/`approval` entry in `blocked_by` — including one
/// with no `blocked_by` at all (`NOW`/`NEXT`/`DEFERRED` entries) — forms its
/// own singleton group with `gate: None`, so a section with no such edges
/// groups to exactly one group per block, identical to the pre-dedup
/// rendering.
pub fn group_blocked_by_gate(blocks: &[Block]) -> Vec<BlockedGroup<'_>> {
    let mut groups: Vec<BlockedGroup<'_>> = Vec::new();
    let mut index_by_key: HashMap<(&'static str, String), usize> = HashMap::new();

    for block in blocks {
        match gate_key(block) {
            Some((kind, slug)) => {
                let key = (kind, slug.to_string());
                if let Some(&idx) = index_by_key.get(&key) {
                    groups[idx].blocks.push(block);
                } else {
                    let gate = block.blocked_by.iter().find(|dep| match dep {
                        BlockedBy::Operator(OperatorDep { slug: s, .. }) => s == slug,
                        BlockedBy::Approval(ApprovalDep { slug: s, .. }) => s == slug,
                        BlockedBy::Block(_) | BlockedBy::External(_) => false,
                    });
                    index_by_key.insert(key, groups.len());
                    groups.push(BlockedGroup {
                        blocks: vec![block],
                        gate,
                    });
                }
            }
            None => groups.push(BlockedGroup {
                blocks: vec![block],
                gate: None,
            }),
        }
    }

    groups
}

// ---------------------------------------------------------------------------
// render_hq_board — pure NOW/NEXT/BLOCKED Operating Board renderer
// ---------------------------------------------------------------------------

/// Render the brain-derived [`Focus`] + `cross_repo[]` edges as the HQ
/// Operating Board Markdown (`## NOW` / `## NEXT` / `## BLOCKED`).
///
/// Each section lists its repo-tagged blocks as `repo:id — title`, in the
/// order they appear in `focus` (callers — [`crate::brain::state::derive_brain_focus`]
/// — already establish a stable, deterministic order; this renderer does not
/// re-sort). An empty section renders a single `_none_` line rather than being
/// omitted, so the section headings are always present and the output shape
/// never depends on which sections happen to be non-empty.
///
/// `BLOCKED` entries are annotated with what they're waiting on: each
/// `blocked_by` dependency renders as `repo:id` (a `Block` dependency) or
/// `external:<what>` (an `External` dependency). When a `Block` dependency
/// matches a `cross_repo[]` edge (`edge.from == {repo, id}` of the blocked
/// block and `edge.to == {repo, id}` of the dependency), the edge's `note` is
/// appended in parentheses; otherwise the dependency's own `what` gloss is
/// used if present. Multiple dependencies are joined with `, `.
///
/// Rendered without a trailing newline, matching the [`render_wave_table`]
/// convention; callers that embed it inside a document own any surrounding
/// blank lines.
///
/// **`focus.deferred` is deliberately not rendered here.** The Operating Board is
/// the terse three-lane triage view — "what is live right now". Back-burner work
/// is by definition not triage, so surfacing it would defeat the purpose of
/// deferring it. The unified board is the superset that does show a `DEFERRED`
/// section; see [`render_unified_board`].
pub fn render_hq_board(focus: &Focus, edges: &[CrossRepoEdge]) -> String {
    let sections = [
        render_hq_board_section("NOW", &focus.now, edges),
        render_hq_board_section("NEXT", &focus.next, edges),
        render_hq_board_section("BLOCKED", &focus.blocked, edges),
    ];
    sections.join("\n\n")
}

/// Render one `## {heading}` section of the Operating Board for `blocks`.
///
/// Blocks sharing an `operator`/`approval` `depends_on` slug are collapsed
/// into one line via [`group_blocked_by_gate`] (Task 5, MV shared-identity
/// dedup) — a heading with no such edges renders exactly as before, one line
/// per block.
fn render_hq_board_section(heading: &str, blocks: &[Block], edges: &[CrossRepoEdge]) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("## {heading}"));

    let groups = group_blocked_by_gate(blocks);
    if groups.is_empty() {
        lines.push("_none_".to_string());
    } else {
        for group in &groups {
            lines.push(format!("- {}", render_hq_board_group_line(group, edges)));
        }
    }

    lines.join("\n")
}

/// Render one [`BlockedGroup`] as its Operating Board line.
///
/// A singleton group (no shared operator/approval slug, or a slug gating only
/// this one block) renders exactly like [`render_hq_board_line`] always did.
/// A real group (`> 1` member) renders every member's `repo:id — title` joined
/// with `; `, followed by a single `(blocked by ...)` annotation for the
/// shared gate — not one annotation per member, which is the noise this dedup
/// exists to remove.
fn render_hq_board_group_line(group: &BlockedGroup<'_>, edges: &[CrossRepoEdge]) -> String {
    if group.blocks.len() == 1 {
        return render_hq_board_line(group.blocks[0], edges);
    }

    let names: Vec<String> = group
        .blocks
        .iter()
        .map(|b| {
            let repo = b.repo.as_deref().unwrap_or("");
            format!("{repo}:{} — {}", b.id, b.title)
        })
        .collect();

    match group.gate {
        Some(dep) => format!(
            "{} (blocked by {})",
            names.join("; "),
            render_hq_board_blocker("", "", dep, edges)
        ),
        None => names.join("; "),
    }
}

/// Render a single Operating Board line for `block`: `repo:id — title`,
/// annotated with `(blocked by ...)` when `block.blocked_by` is non-empty.
fn render_hq_board_line(block: &Block, edges: &[CrossRepoEdge]) -> String {
    let repo = block.repo.as_deref().unwrap_or("");
    let mut line = format!("{repo}:{} — {}", block.id, block.title);

    if !block.blocked_by.is_empty() {
        let annotations: Vec<String> = block
            .blocked_by
            .iter()
            .map(|dep| render_hq_board_blocker(repo, &block.id, dep, edges))
            .collect();
        line.push_str(&format!(" (blocked by {})", annotations.join(", ")));
    }

    line
}

/// Render one `blocked_by` dependency of the block `{from_repo}:{from_id}` as
/// its Operating Board annotation.
///
/// `Operator` and `Approval` entries render in full rather than as a bare
/// `OP.<slug>` gloss (Task 6, `ticket-operator-edge-graph`):
/// an operator gate shows its `exit` condition and paste-ready `start` command —
/// the two things a reader needs to actually clear it — and an approval shows
/// its `what` explicitly labeled `decision:` so it reads as a one-decision item
/// rather than a described task. Blocks with no operator/approval edge are
/// untouched by this change; only these two match arms grew.
fn render_hq_board_blocker(
    from_repo: &str,
    from_id: &str,
    dep: &BlockedBy,
    edges: &[CrossRepoEdge],
) -> String {
    match dep {
        BlockedBy::External(ExternalDep { what }) => format!("external:{what}"),
        BlockedBy::Operator(OperatorDep {
            slug, exit, start, ..
        }) => format!("{} — exit: {exit}; start: `{start}`", okf_core::op_id(slug)),
        BlockedBy::Approval(ApprovalDep { slug, what, .. }) => {
            format!("{} — decision: {what}", okf_core::op_id(slug))
        }
        BlockedBy::Block(BlockDep {
            repo: dep_repo,
            id: dep_id,
            what,
        }) => {
            let target = format!("{dep_repo}:{dep_id}");

            // Prefer the matching cross_repo[] edge's note (the resolved,
            // brain-level gloss); fall back to the dependency's own `what`.
            let note = edges
                .iter()
                .find(|e| {
                    e.from.repo == from_repo
                        && e.from.id == from_id
                        && e.to.repo == *dep_repo
                        && e.to.id == *dep_id
                })
                .and_then(|e| e.note.clone())
                .or_else(|| what.clone());

            match note {
                Some(note) => format!("{target} ({note})"),
                None => target,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// render_unified_board — pure NOW/NEXT/BLOCKED/DUE-SOON unified renderer (MV.6.B)
// ---------------------------------------------------------------------------

/// Number of days ahead of `today` a block's `due` must fall within to be
/// listed in the `DUE-SOON` section (overdue blocks are always included).
const DUE_SOON_WINDOW_DAYS: i64 = 14;

/// Render the unified, priority-ranked HQ board: `## NOW` / `## NEXT` /
/// `## BLOCKED` / `## DUE-SOON` over the brain-wide union `focus` (produced by
/// [`crate::brain::state::derive_brain_focus`] at
/// [`crate::brain::state::TierScope::All`]), unioning every registered repo
/// — including the business tier — tagged `[BIZ]`/`[ENG]`.
///
/// Tag derivation: a block's `repo` slug is looked up in `config.repos`; a
/// match whose `tier == "business"` renders `[BIZ]`, everything else
/// (including an unrecognised repo slug) renders `[ENG]`.
///
/// `NEXT` is stably re-sorted by `(priority asc, due asc)`, both with an
/// absent value sorted last — since [`Focus::next`] is already wave-ordered,
/// the stable sort keeps wave as the implicit tertiary key. `NOW`/`BLOCKED`
/// preserve the caller-supplied order, matching [`render_hq_board`].
///
/// `DUE-SOON` lists every block from the now+next+blocked union whose `due`
/// parses as `%Y-%m-%d` and is no later than `today + 14 days`, sorted by due
/// date ascending (soonest/most-overdue first); a block whose `due` is before
/// `today` is annotated `(overdue)`. Blocks with an absent or unparseable
/// `due` are excluded.
///
/// Rendered without a trailing newline, matching the [`render_wave_table`] /
/// [`render_hq_board`] convention; callers that embed it inside a document own
/// any surrounding blank lines.
pub fn render_unified_board(
    focus: &Focus,
    edges: &[CrossRepoEdge],
    effective: &HashMap<String, u8>,
    config: &BrainConfig,
    today: chrono::NaiveDate,
) -> String {
    let next_sorted = sort_unified_board_next(&focus.next, effective);

    let sections = [
        render_unified_board_section(BOARD_LANE_LEVEL, "NOW", &focus.now, edges, config),
        render_unified_board_section(BOARD_LANE_LEVEL, "NEXT", &next_sorted, edges, config),
        render_unified_board_section(BOARD_LANE_LEVEL, "BLOCKED", &focus.blocked, edges, config),
        // DEFERRED is deliberately NOT priority-sorted: back-burner work has no
        // queue position, and sorting it would imply one.
        render_unified_board_section(BOARD_LANE_LEVEL, "DEFERRED", &focus.deferred, edges, config),
        render_due_soon_section(focus, edges, config, today),
    ];
    sections.join("\n\n")
}

/// Parse a `Block::due` string (`%Y-%m-%d`) into a [`chrono::NaiveDate`];
/// `None` for an absent or unparseable value.
fn parse_due(due: &Option<String>) -> Option<chrono::NaiveDate> {
    due.as_deref()
        .and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
}

/// Look up a block's `[BIZ]`/`[ENG]` tag from its `repo` slug: `"business"`
/// tier renders `[BIZ]`, any other tier (or an unrecognised slug) renders
/// `[ENG]`. The `business` tier ROOT itself (`tier = "_root"`, slug
/// `"business"`) also renders `[BIZ]` — its own authored `tracks[]` (e.g.
/// `BZ.*`) are business blocks too, not just its `tier = "business"` children.
fn unified_board_tag(block: &Block, config: &BrainConfig) -> &'static str {
    let repo = block.repo.as_deref().unwrap_or("");
    let is_business = repo == "business"
        || config
            .repos
            .iter()
            .any(|r| r.slug == repo && r.tier == "business");
    if is_business { "[BIZ]" } else { "[ENG]" }
}

/// Look up the effective priority (MV.7.A) for a `"repo:id"` node: the
/// `effective_priorities` map wins when present — it reflects reverse-topo
/// `min`-propagation, so a block gating a hotter dependent floats up — and
/// falls back to the node's own raw `priority` (absent → `u8::MAX`, sorts
/// last) when the map has no entry for it.
///
/// Takes `repo`/`id`/`priority` rather than a `&Block` so both `Block`
/// (`focus.*` entries) and `TrackBlock` (`tracks[]` entries — MV.13.B Task 2's
/// `gate_rank` derivation) can share this one lookup instead of each growing
/// its own copy. `pub(crate)` so [`crate::brain::frontier::gate_ranks`] can
/// reuse it directly instead of re-deriving the map semantics.
pub(crate) fn effective_priority_for(
    repo: &str,
    id: &str,
    priority: Option<u8>,
    effective: &HashMap<String, u8>,
) -> u8 {
    let key = format!("{repo}:{id}");
    effective.get(&key).copied().or(priority).unwrap_or(u8::MAX)
}

/// Stably sort `next` by `(effective priority asc, due asc)`, both with an
/// absent value sorted last. The input is already wave-ordered, so the
/// stable sort keeps wave as the implicit tertiary key. `effective` is the
/// [`effective_priorities`] map (MV.7.A) — an engineering block that gates a
/// hotter dependent (via `depends_on`) sorts by that inherited hotness
/// rather than its own raw priority.
fn sort_unified_board_next(next: &[Block], effective: &HashMap<String, u8>) -> Vec<Block> {
    let mut sorted = next.to_vec();
    sorted.sort_by(|a, b| {
        let pa = effective_priority_for(
            a.repo.as_deref().unwrap_or(""),
            &a.id,
            a.priority,
            effective,
        );
        let pb = effective_priority_for(
            b.repo.as_deref().unwrap_or(""),
            &b.id,
            b.priority,
            effective,
        );
        pa.cmp(&pb)
            .then_with(|| match (parse_due(&a.due), parse_due(&b.due)) {
                (Some(da), Some(db)) => da.cmp(&db),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            })
    });
    sorted
}

/// Heading prefix for the top-level boards' lanes (`## NOW`, …).
const BOARD_LANE_LEVEL: &str = "##";

/// Heading prefix for the epic board's lanes. One level deeper than
/// [`BOARD_LANE_LEVEL`] because each epic already owns an `###` heading — lanes
/// must nest *under* their epic, not outrank it.
const EPIC_LANE_LEVEL: &str = "####";

/// Render one `{level} {heading}` section of a board for `blocks`.
///
/// Blocks sharing an `operator`/`approval` `depends_on` slug are collapsed
/// into one line via [`group_blocked_by_gate`] (Task 5, MV shared-identity
/// dedup) — a heading with no such edges renders exactly as before, one line
/// per block.
fn render_unified_board_section(
    level: &str,
    heading: &str,
    blocks: &[Block],
    edges: &[CrossRepoEdge],
    config: &BrainConfig,
) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("{level} {heading}"));

    let groups = group_blocked_by_gate(blocks);
    if groups.is_empty() {
        lines.push("_none_".to_string());
    } else {
        for group in &groups {
            lines.push(format!(
                "- {}",
                render_unified_board_group_line(group, edges, config)
            ));
        }
    }

    lines.join("\n")
}

/// Render one [`BlockedGroup`] as its unified-board line.
///
/// A singleton group renders exactly like [`render_unified_board_line`]
/// always did. A real group (`> 1` member) renders every member's
/// `[BIZ]|[ENG] repo:id — title` joined with `; `, followed by a single
/// `(blocked by ...)` annotation for the shared gate.
fn render_unified_board_group_line(
    group: &BlockedGroup<'_>,
    edges: &[CrossRepoEdge],
    config: &BrainConfig,
) -> String {
    if group.blocks.len() == 1 {
        return render_unified_board_line(group.blocks[0], edges, config);
    }

    let names: Vec<String> = group
        .blocks
        .iter()
        .map(|b| {
            let tag = unified_board_tag(b, config);
            let repo = b.repo.as_deref().unwrap_or("");
            format!("{tag} {repo}:{} — {}", b.id, b.title)
        })
        .collect();

    match group.gate {
        Some(dep) => format!(
            "{} (blocked by {})",
            names.join("; "),
            render_hq_board_blocker("", "", dep, edges)
        ),
        None => names.join("; "),
    }
}

/// Render a single unified board line for `block`: `[BIZ]|[ENG] repo:id — title`,
/// annotated with `(blocked by ...)` when `block.blocked_by` is non-empty
/// (reusing [`render_hq_board_blocker`]).
fn render_unified_board_line(
    block: &Block,
    edges: &[CrossRepoEdge],
    config: &BrainConfig,
) -> String {
    let tag = unified_board_tag(block, config);
    let repo = block.repo.as_deref().unwrap_or("");
    let mut line = format!("{tag} {repo}:{} — {}", block.id, block.title);

    if !block.blocked_by.is_empty() {
        let annotations: Vec<String> = block
            .blocked_by
            .iter()
            .map(|dep| render_hq_board_blocker(repo, &block.id, dep, edges))
            .collect();
        line.push_str(&format!(" (blocked by {})", annotations.join(", ")));
    }

    line
}

/// Render the `## DUE-SOON` section: every block from the now+next+blocked
/// union whose `due` parses and is `<= today + 14 days`, sorted by due date
/// ascending (soonest/most-overdue first) and annotated `(overdue)` when
/// `due < today`.
///
/// **`focus.deferred` is deliberately excluded from the union.** A deferred
/// block's `due` is no longer a commitment — deferring it *is* the decision to
/// let that date pass. Consequence to be aware of: deferring a block silently
/// removes it from DUE-SOON even when it is already overdue. That is intended;
/// un-defer it (back to `open`) to put the date back in play.
fn render_due_soon_section(
    focus: &Focus,
    edges: &[CrossRepoEdge],
    config: &BrainConfig,
    today: chrono::NaiveDate,
) -> String {
    let window_end = today + chrono::Duration::days(DUE_SOON_WINDOW_DAYS);

    let mut due_soon: Vec<(chrono::NaiveDate, &Block)> = focus
        .now
        .iter()
        .chain(focus.next.iter())
        .chain(focus.blocked.iter())
        .filter_map(|block| parse_due(&block.due).map(|date| (date, block)))
        .filter(|(date, _)| *date <= window_end)
        .collect();
    due_soon.sort_by_key(|(date, _)| *date);

    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("{BOARD_LANE_LEVEL} DUE-SOON"));

    if due_soon.is_empty() {
        lines.push("_none_".to_string());
    } else {
        for (date, block) in due_soon {
            let mut line = render_unified_board_line(block, edges, config);
            if date < today {
                line.push_str(" (overdue)");
            }
            lines.push(format!("- {line}"));
        }
    }

    lines.join("\n")
}

// ---------------------------------------------------------------------------
// render_epic_board — per-initiative NOW/NEXT/BLOCKED + progress + relationships
// ---------------------------------------------------------------------------

/// The epic `status` values whose boards render in FULL (all lanes + relationships).
///
/// A `complete` epic drops off the board entirely — a finished initiative should
/// stop competing for attention. A `paused` epic does **not** drop off: it renders
/// collapsed to a single marked line (see [`render_epic_board`]), because parked
/// work still needs to be visible enough that you remember it exists.
/// `focused` is included because it is active-equivalent everywhere else (see
/// [`EPIC_STATUS_FOCUSED`]); omitting it here would make the current-priority
/// epic the one epic that renders nowhere at all.
const RENDERED_EPIC_STATUSES: [&str; 2] = ["active", "focused"];

/// Epic `status` values that render collapsed to a one-line summary instead of a
/// full board section.
const COLLAPSED_EPIC_STATUSES: [&str; 1] = ["paused"];

/// The authored epic status meaning "parked" — the epic-level counterpart of a
/// block's `deferred` status. Paired with block deferral by `mev defer-epic`.
pub const EPIC_STATUS_PAUSED: &str = "paused";

/// The authored epic status meaning "live".
pub const EPIC_STATUS_ACTIVE: &str = "active";

/// The authored epic status meaning "the current priority" — bastion-web's
/// default filter.
///
/// It is a **refinement of `active`, not an alternative to it**: everywhere the
/// reconciler asks "is this epic live?" (`plan_sync_epics`' pause rule, the
/// epic-board render filter) `focused` must answer the same as `active`, or
/// `focused` becomes a hole. Only the consumer-side "what should I look at
/// first?" question distinguishes them. Note that resuming a paused epic sets
/// `active`, never `focused`: un-parking is not a promotion to priority.
pub const EPIC_STATUS_FOCUSED: &str = "focused";

/// The authored epic status meaning "finished" — terminal.
///
/// A `complete` epic drops off the board entirely ([`RENDERED_EPIC_STATUSES`],
/// state-schema.md:266) and is never inferred: `W_STATE_EPIC_ALL_CLOSED` is
/// warn-only by design (state-schema.md:290) precisely because the last block
/// closing is not the same as the initiative's goal being met. Only an operator
/// declaring completion by name — `mev complete-epic <slug>` — may set this;
/// no reconciler path may set it automatically.
pub const EPIC_STATUS_COMPLETE: &str = "complete";

/// Counted progress for one epic: how many member blocks are in each state.
pub struct EpicProgress {
    /// Members with authored `status == "closed"`.
    pub closed: usize,
    /// Members with authored `status == "in_progress"`.
    pub in_progress: usize,
    /// Members that are open (authored `open`, or status absent).
    pub open: usize,
    /// Members with authored `status == "deferred"`.
    pub deferred: usize,
    /// Members with authored `status == "wontfix"`.
    ///
    /// Terminal like `closed` for readiness — a dependent is not blocked on a
    /// `wontfix` member — but tallied in its own field so `closed` never
    /// silently absorbs it. Folding `wontfix` into `closed` would inflate the
    /// `N/M closed` progress line with work that was declared abandoned, not done.
    pub wontfix: usize,
    /// Members with authored `status == "superseded"`.
    ///
    /// Terminal like `closed`/`wontfix` for readiness — a dependent is not
    /// blocked on a `superseded` member — but tallied in its own field so
    /// neither `closed` nor `open` (the `_ =>` catch-all's target) ever
    /// silently absorbs it. Folding `superseded` into `open` would inflate
    /// every epic rollup's open count with work that already moved elsewhere.
    pub superseded: usize,
}

impl EpicProgress {
    /// Every member block, in any state.
    pub fn total(&self) -> usize {
        self.closed + self.in_progress + self.open + self.deferred + self.wontfix + self.superseded
    }

    /// Is this epic's remaining work entirely parked?
    ///
    /// True iff it has at least one deferred member and **no** unfinished
    /// non-deferred work (nothing open, nothing in progress). An epic whose
    /// members are all `closed` is *complete*, not deferred, so the
    /// `deferred > 0` clause is load-bearing.
    ///
    /// This is the **single** predicate behind the collapsed board rendering,
    /// the `fully_deferred` flag on the serve API, and `mev sync-epics`'s
    /// "all blocks deferred → pause the epic" direction — so the three can
    /// never disagree about what "a deferred epic" means.
    pub fn is_fully_deferred(&self) -> bool {
        self.deferred > 0 && self.open == 0 && self.in_progress == 0
    }
}

/// Tally one epic's member blocks by authored status.
pub fn epic_progress(members: &[(String, &TrackBlock)]) -> EpicProgress {
    let mut p = EpicProgress {
        closed: 0,
        in_progress: 0,
        open: 0,
        deferred: 0,
        wontfix: 0,
        superseded: 0,
    };
    for (_, block) in members {
        match block.status.as_deref() {
            Some("closed") => p.closed += 1,
            Some("in_progress") => p.in_progress += 1,
            Some("deferred") => p.deferred += 1,
            Some("wontfix") => p.wontfix += 1,
            Some("superseded") => p.superseded += 1,
            _ => p.open += 1,
        }
    }
    p
}

/// One epic's already-derived inputs to [`render_epic_board`].
///
/// Assembling these is the planner's job (it owns the corpus); the renderer
/// stays a pure data-in/string-out function.
pub struct EpicBoardInput<'a> {
    /// The registry entry being rendered.
    pub epic: &'a Epic,
    /// This epic's member blocks in cross-repo wave order ([`epic_members`]).
    pub members: Vec<(String, &'a TrackBlock)>,
    /// This epic's boundary edges ([`derive_epic_edges`]).
    pub edges: EpicEdges,
}

/// Render the per-epic board: one `### {title}` section per active epic, each
/// with a progress line, `NOW` / `NEXT` / `BLOCKED` lanes, and the derived
/// cross-epic relationships.
///
/// `focus` is the brain-level union (from `derive_brain_focus`) — each epic's
/// lanes are [`derive_epic_focus`] slices of it, so an epic board can never
/// disagree with the unified board about what is now, next, or blocked.
/// `NEXT` is sorted by effective priority exactly as the unified board sorts it.
///
/// The relationship lines list only the **blocking** edges
/// ([`EpicEdge::blocking`]) — satisfied history would bury the live signal. A
/// counterpart in no epic renders as `(no epic)`, the readable face of
/// `W_STATE_EPIC_UNREACHABLE_DEP`.
///
/// Rendered without a trailing newline, matching [`render_unified_board`] /
/// [`render_wave_table`]; callers own any surrounding blank lines.
pub fn render_epic_board(
    inputs: &[EpicBoardInput<'_>],
    focus: &Focus,
    effective: &HashMap<String, u8>,
    config: &BrainConfig,
) -> String {
    // Three-way split by authored status, not the old two-way filter:
    //   active (or absent)  → full board section
    //   focused             → full board section (active-equivalent)
    //   paused              → ONE collapsed line, still visible
    //   complete            → dropped entirely
    // Parked work must stay visible enough that you remember it exists; a
    // finished initiative should genuinely stop competing for attention.
    let status_of = |i: &EpicBoardInput<'_>| {
        i.epic
            .status
            .as_deref()
            .unwrap_or(EPIC_STATUS_ACTIVE)
            .to_string()
    };
    let rendered: Vec<&EpicBoardInput<'_>> = inputs
        .iter()
        .filter(|i| RENDERED_EPIC_STATUSES.contains(&status_of(i).as_str()))
        .collect();
    let collapsed: Vec<&EpicBoardInput<'_>> = inputs
        .iter()
        .filter(|i| COLLAPSED_EPIC_STATUSES.contains(&status_of(i).as_str()))
        .collect();

    if rendered.is_empty() && collapsed.is_empty() {
        return "_no active epics_".to_string();
    }

    let mut sections: Vec<String> = Vec::new();
    for input in rendered {
        let epic = input.epic;
        let progress = epic_progress(&input.members);
        let scoped = derive_epic_focus(focus, &epic.slug);
        let next_sorted = sort_unified_board_next(&scoped.next, effective);

        let mut lines = vec![format!("### {}", epic.title)];
        if let Some(ref description) = epic.description {
            lines.push(String::new());
            lines.push(format!("_{description}_"));
        }
        lines.push(String::new());
        lines.push(render_epic_progress_line(&progress));
        lines.push(String::new());
        // No `edges` for the `(blocked by ...)` annotation: cross_repo notes are
        // an HQ-board concern, and the epic board's own relationship lines below
        // already name what gates this initiative.
        lines.push(render_unified_board_section(
            EPIC_LANE_LEVEL,
            "NOW",
            &scoped.now,
            &[],
            config,
        ));
        lines.push(String::new());
        lines.push(render_unified_board_section(
            EPIC_LANE_LEVEL,
            "NEXT",
            &next_sorted,
            &[],
            config,
        ));
        lines.push(String::new());
        lines.push(render_unified_board_section(
            EPIC_LANE_LEVEL,
            "BLOCKED",
            &scoped.blocked,
            &[],
            config,
        ));
        lines.push(String::new());
        lines.push(render_unified_board_section(
            EPIC_LANE_LEVEL,
            "DEFERRED",
            &scoped.deferred,
            &[],
            config,
        ));

        let relationships = render_epic_relationships(&input.edges);
        if !relationships.is_empty() {
            lines.push(String::new());
            lines.push(relationships);
        }

        sections.push(lines.join("\n"));
    }

    // Collapsed (paused) epics render last, as one line each — present, but
    // visibly parked and costing a line instead of a dozen.
    for input in collapsed {
        sections.push(render_collapsed_epic_line(
            input.epic,
            &epic_progress(&input.members),
        ));
    }

    sections.join("\n\n")
}

/// Render one paused epic as a single line: heading, progress, parked marker.
///
/// This is what a deferred initiative looks like on the board — enough to know
/// it exists and how much is parked, without the dozen lines of empty
/// `NOW`/`NEXT`/`BLOCKED` cells a full section would spend saying "nothing here".
fn render_collapsed_epic_line(epic: &Epic, progress: &EpicProgress) -> String {
    let marker = if progress.is_fully_deferred() {
        "_deferred — all remaining work parked_"
    } else {
        "_paused_"
    };
    format!(
        "### {} — {} · {marker}",
        epic.title,
        render_epic_progress_line(progress)
    )
}

/// Render an epic's one-line progress summary, e.g.
/// `**7/23 closed** · 2 in progress · 14 open`.
fn render_epic_progress_line(p: &EpicProgress) -> String {
    if p.total() == 0 {
        return "**no member blocks yet**".to_string();
    }
    // `deferred` is reported as its own term rather than folded into `open`.
    // Folding made a fully-parked epic read "0 in progress · 2 open", which is
    // indistinguishable from two blocks that are ready to pick up. The clause is
    // omitted entirely when nothing is deferred, so active epics' lines are
    // byte-identical to before this change.
    let mut line = format!(
        "**{}/{} closed** · {} in progress · {} open",
        p.closed,
        p.total(),
        p.in_progress,
        p.open
    );
    if p.deferred > 0 {
        line.push_str(&format!(" · {} deferred", p.deferred));
    }
    // Same never-fold-into-closed rule as `deferred`: omitted when zero so
    // epics with no wontfix members render byte-identical to before this field
    // existed.
    if p.wontfix > 0 {
        line.push_str(&format!(" · {} wontfix", p.wontfix));
    }
    // Same never-fold rule as `deferred`/`wontfix`: omitted when zero so
    // epics with no superseded members render byte-identical to before this
    // field existed.
    if p.superseded > 0 {
        line.push_str(&format!(" · {} superseded", p.superseded));
    }
    line
}

/// Render the `Waiting on` / `Holding up` relationship lines for one epic,
/// listing only edges that still gate. Returns an empty string when nothing is
/// live, so the caller can omit the block entirely.
fn render_epic_relationships(edges: &EpicEdges) -> String {
    let describe = |e: &&EpicEdge, counterpart: &str| {
        let owner = if e.other_epics.is_empty() {
            "no epic".to_string()
        } else {
            e.other_epics.join(", ")
        };
        format!("- {counterpart} ({owner})")
    };

    let mut lines: Vec<String> = Vec::new();

    let waiting: Vec<&EpicEdge> = edges.outbound.iter().filter(|e| e.blocking).collect();
    if !waiting.is_empty() {
        lines.push("**Waiting on**".to_string());
        let mut seen: Vec<&str> = Vec::new();
        for edge in &waiting {
            if seen.contains(&edge.to.as_str()) {
                continue; // several members can gate on the same block
            }
            seen.push(&edge.to);
            lines.push(describe(edge, &edge.to));
        }
    }

    let holding: Vec<&EpicEdge> = edges.inbound.iter().filter(|e| e.blocking).collect();
    if !holding.is_empty() {
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.push("**Holding up**".to_string());
        let mut seen: Vec<&str> = Vec::new();
        for edge in &holding {
            if seen.contains(&edge.from.as_str()) {
                continue;
            }
            seen.push(&edge.from);
            lines.push(describe(edge, &edge.from));
        }
    }

    lines.join("\n")
}

// ---------------------------------------------------------------------------
// render_epic_sequence_table — one epic's cross-repo work order
// ---------------------------------------------------------------------------

/// Render one epic's members as a Markdown table in cross-repo wave order.
///
/// Columns: `Wave | Repo | Block | Title | Status | Depends on`. This is
/// [`render_wave_table`]'s cross-repo sibling — same derived-status rule (an
/// open block with an unmet `depends_on` renders `blocked`) and the same
/// `global_status` lookup, but the row set is one epic across every repo rather
/// than one repo across every epic.
///
/// Rendered without a trailing newline.
pub fn render_epic_sequence_table(
    members: &[(String, &TrackBlock)],
    global_status: &HashMap<String, Option<String>>,
) -> String {
    let mut lines = vec![
        "| Wave | Repo | Block | Title | Status | Depends on |".to_string(),
        "|---|---|---|---|---|---|".to_string(),
    ];

    if members.is_empty() {
        lines.push("| — | — | — | _no member blocks_ | — | — |".to_string());
        return lines.join("\n");
    }

    for (repo, block) in members {
        let wave = block
            .wave
            .map(|w| w.to_string())
            .unwrap_or_else(|| "—".to_string());

        let deps: Vec<String> = block
            .depends_on
            .iter()
            .map(|dep| match dep {
                BlockedBy::Block(BlockDep { repo, id, .. }) => format!("{repo}:{id}"),
                BlockedBy::External(ExternalDep { what }) => format!("external:{what}"),
                // Full exit/start/decision rendering (Task 6, `ticket-operator-edge-graph`) —
                // matches render_hq_board_blocker's annotation form so the epic sequence
                // table and the NOW/NEXT/BLOCKED boards read consistently.
                BlockedBy::Operator(OperatorDep {
                    slug, exit, start, ..
                }) => format!("{} — exit: {exit}; start: `{start}`", okf_core::op_id(slug)),
                BlockedBy::Approval(ApprovalDep { slug, what, .. }) => {
                    format!("{} — decision: {what}", okf_core::op_id(slug))
                }
            })
            .collect();
        let deps_cell = if deps.is_empty() {
            "—".to_string()
        } else {
            deps.join(", ")
        };

        let authored = block.status.as_deref().unwrap_or("open");
        let status = if authored == "open" && has_unmet_dep(block, global_status) {
            "blocked"
        } else {
            authored
        };

        lines.push(format!(
            "| {wave} | {repo} | {} | {} | {status} | {deps_cell} |",
            block.id, block.title
        ));
    }

    lines.join("\n")
}

/// Whether `block` has at least one `depends_on` entry that is not yet met — any
/// `external`/`operator`/`approval` entry (all three are targetless and unmet for
/// as long as they are present), or a `block` entry whose target's authored status
/// in `global_status` is not `closed` (an unresolvable target counts as unmet).
fn has_unmet_dep(block: &TrackBlock, global_status: &HashMap<String, Option<String>>) -> bool {
    block.depends_on.iter().any(|dep| match dep {
        BlockedBy::External(_) | BlockedBy::Operator(_) | BlockedBy::Approval(_) => true,
        BlockedBy::Block(BlockDep { repo, id, .. }) => {
            global_status
                .get(&format!("{repo}:{id}"))
                .and_then(|s| s.as_deref())
                != Some("closed")
        }
    })
}

// ---------------------------------------------------------------------------
// render_attention_section — stale carryover / aging backlog / orphaned captures
// ---------------------------------------------------------------------------

/// One row on the Attention board: structured fields only — **no
/// pre-flattened display string**. Flattening to a rendered line happens at
/// render time (in [`render_triage_lane`]/[`render_attention_lane_capped`]),
/// never before sorting/ranking, so a downstream consumer (or a future
/// re-rank) always has the real fields to work with instead of parsing text.
///
/// The carryover triage lanes (BLOCKING/HOT/AGING/STANDING) populate every
/// field from a [`crate::brain::carryover::CarryoverRanking`]. The backlog /
/// capture / distilled lanes are not this block's target — they populate
/// `repo`/`age`/`kind`/`slug`/`title_or_text` only and leave the
/// carryover-only fields (`priority`, `effective_priority`, `lane`,
/// `clears_when`) at their defaults (`None`), matching yesterday's rendered
/// output byte-for-byte (see `render_section_lanes_and_snooze_and_capture`
/// and friends in `tests/brain_emit.rs`).
pub(crate) struct AttentionRow {
    pub(crate) repo: String,
    /// `None` when the entry has no parseable anchor date (or, for
    /// carryover, is currently snoozed) — rendered as `—` rather than a
    /// fabricated `0d`.
    pub(crate) age: Option<i64>,
    pub(crate) kind: String,
    pub(crate) slug: String,
    /// The free-text portion of the row: a carryover's authored `summary`
    /// when present (rendered verbatim, never re-snippeted), otherwise its
    /// snippeted `text`; a backlog node's `title`; a capture's `"{title} —
    /// notes: {notes}"`; or a distilled entry's snippeted claim. Every case
    /// but the present-summary one is already truncated at construction
    /// time by [`attention_snippet`].
    pub(crate) title_or_text: String,
    /// Carryover-only: the authored `priority`, absent for every other lane.
    pub(crate) priority: Option<u8>,
    /// Carryover-only: the effective priority after `blocks[]`
    /// min-propagation, absent for every other lane.
    pub(crate) effective_priority: Option<u8>,
    /// Carryover-only: which [`TriageLane`] this row landed in.
    pub(crate) lane: Option<TriageLane>,
    /// Carryover-only: the display form of `clears_when`, if any.
    pub(crate) clears_when: Option<String>,
}

/// Truncate `text` to a single tidy line of at most `max` chars for a board row.
pub(crate) fn attention_snippet(text: &str, max: usize) -> String {
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() > max {
        let truncated: String = one_line.chars().take(max).collect();
        format!("{}…", truncated.trim_end())
    } else {
        one_line
    }
}

/// Render one `## {heading}` Attention sub-lane from `rows` (already filtered to
/// stale items), flattening each row to a display line via `detail` at RENDER
/// TIME only. Rows are sorted oldest-first (largest age). Empty → `_none_`.
///
/// Used by the backlog / capture / distilled lanes — the carryover triage
/// lanes use [`render_triage_lane`] instead, since their rows arrive
/// pre-ordered by [`rank_carryover`] and must not be re-sorted by age.
fn render_attention_lane(
    heading: &str,
    rows: Vec<AttentionRow>,
    detail: impl Fn(&AttentionRow) -> String,
) -> String {
    render_attention_lane_capped(heading, rows, usize::MAX, detail)
}

/// Same as [`render_attention_lane`], but shows at most `cap` rows (oldest-first)
/// and — when more than `cap` rows are stale — appends an explicit
/// `…and N more` line stating the true count of hidden rows. Never truncates
/// silently: the hidden count is always printed when rows are dropped.
fn render_attention_lane_capped(
    heading: &str,
    mut rows: Vec<AttentionRow>,
    cap: usize,
    detail: impl Fn(&AttentionRow) -> String,
) -> String {
    rows.sort_by_key(|r| std::cmp::Reverse(r.age.unwrap_or(i64::MIN)));
    let mut lines = vec![format!("## {heading}")];
    if rows.is_empty() {
        lines.push("_none_".to_string());
    } else {
        let total = rows.len();
        let shown = rows.into_iter().take(cap);
        for row in shown {
            let age = row.age.unwrap_or_default();
            lines.push(format!("- [{}] {} — {}d", row.repo, detail(&row), age));
        }
        if total > cap {
            lines.push(format!("- …and {} more", total - cap));
        }
    }
    lines.join("\n")
}

/// Render one `## {heading}` carryover triage sub-lane (BLOCKING / HOT / AGING
/// / STANDING) from `rows`, which arrive **already ordered** by
/// [`rank_carryover`] — this function does not re-sort, unlike
/// [`render_attention_lane_capped`]. Shows at most `cap` rows and — when more
/// than `cap` rows exist — appends an explicit `…and N more` line stating the
/// true hidden count (same never-truncate-silently principle as
/// [`DISTILL_LANE_CAP`]).
fn render_triage_lane(heading: &str, rows: &[AttentionRow], cap: usize) -> String {
    let mut lines = vec![format!("## {heading}")];
    if rows.is_empty() {
        lines.push("_none_".to_string());
    } else {
        let total = rows.len();
        for row in rows.iter().take(cap) {
            let age = row
                .age
                .map(|a| format!("{a}d"))
                .unwrap_or_else(|| "—".to_string());
            lines.push(format!(
                "- [{}] {} — {age}",
                row.repo,
                render_triage_detail(row)
            ));
        }
        if total > cap {
            lines.push(format!("- …and {} more", total - cap));
        }
    }
    lines.join("\n")
}

/// Flatten one triage row to a display line at RENDER TIME — the only place
/// carryover fields are ever joined into a string. Surfaces the authored and
/// effective priority so a reader can see *why* a row ranks where it does
/// (the pain point this block exists to fix), not just that it does.
fn render_triage_detail(row: &AttentionRow) -> String {
    let mut detail = format!("{} {} — {}", row.kind, row.slug, row.title_or_text);
    match (row.priority, row.effective_priority) {
        (Some(p), Some(ep)) if p == ep => detail.push_str(&format!(" [P{p}]")),
        (Some(p), Some(ep)) => detail.push_str(&format!(" [P{p} -> effective P{ep}]")),
        (Some(p), None) => detail.push_str(&format!(" [P{p}]")),
        (None, Some(ep)) => detail.push_str(&format!(" [effective P{ep}]")),
        (None, None) => {}
    }
    if let Some(c) = &row.clears_when {
        detail.push_str(&format!(" (clears when: {c})"));
    }
    // `row.lane` is redundant with the enclosing `## HEADING` for the four
    // triage lanes rendered today, but the field stays real data on the row
    // (not re-derived from which `Vec` it landed in) so a future single-list
    // export — grepping across lanes rather than by section — still carries
    // it, matching the discipline the rest of this row's fields already
    // follow: nothing gets flattened away before render time.
    if let Some(lane) = row.lane {
        detail.push_str(&format!(" ({lane:?})"));
    }
    detail
}

/// Cap applied to the "Stale distilled knowledge" lane — the 10 oldest entries
/// render, with an explicit `…and N more` line for the rest. Required, not
/// cosmetic: the June D35 fan-out cohort is large enough (hundreds of entries
/// at some tiers) that an uncapped lane is a wall, not a triage surface.
const DISTILL_LANE_CAP: usize = 10;

/// Cap applied to each carryover triage lane (BLOCKING/HOT/AGING/STANDING),
/// with the same explicit `…and N more` treatment as [`DISTILL_LANE_CAP`] —
/// never truncate silently. 20 mirrors the distilled cap's order of
/// magnitude: a triage lane is a glanceable board, not a full export (`mev
/// carryover --json` is the uncapped surface), and BLOCKING/HOT are the
/// lanes most likely to threaten it, since board membership no longer gates
/// on staleness alone.
const CARRYOVER_LANE_CAP: usize = 20;

/// Render the Attention board: the four carryover triage sub-lanes (BLOCKING
/// / HOT / AGING / STANDING, `MV.ticket.carryover-triage-ranking`) plus Aging
/// backlog / Orphaned captures / Stale distilled knowledge, built from the
/// pre-scoped, repo-tagged inputs.
///
/// Carryover lane **membership no longer gates on staleness alone** — every
/// non-snoozed `carryover[]` entry is ranked via [`rank_carryover`] and lands
/// in exactly one of the four triage lanes; `carryover_stale_age` still
/// supplies the single `stale` predicate (feeding AGING membership and every
/// row's displayed age) and is never reimplemented. The other three lanes
/// are unchanged: only items past their staleness threshold appear (the
/// visible twin of the `W_STATE_*_STALE` / `W_DISTILL_STALE` warnings — same
/// predicates). The `[<repo>]` tag is a separate axis from the unified
/// board's `[BIZ]/[ENG]` tag.
pub fn render_attention_section(
    carryover: &[(String, &Carryover)],
    backlog: &[(String, &Backlog)],
    today: chrono::NaiveDate,
    thresholds: &crate::brain::config::AttentionThresholds,
    block_priorities: &HashMap<String, u8>,
    block_status: &HashMap<String, Option<String>>,
) -> String {
    render_attention_section_with_distilled(
        carryover,
        backlog,
        &[],
        today,
        thresholds,
        block_priorities,
        block_status,
    )
}

/// Full form of [`render_attention_section`] that also takes the pre-scoped,
/// repo-tagged D35-distilled entries (`(repo, stem, &DistilledEntry)`) for the
/// "Stale distilled knowledge" lane. `stem` is `"knowledge"` or `"memory"`,
/// used to look up the entry's own threshold via
/// [`crate::brain::config::AttentionThresholds::distill_threshold`].
///
/// `block_priorities` (keyed `"repo:id"`) and `block_status` (keyed
/// `"repo:id"`) are the same maps [`plan_attention_board`] already computes
/// for the whole corpus ([`effective_priorities`] / [`global_status_map`]) —
/// passed straight through to [`rank_carryover`], never recomputed here.
/// Gather every row across the four carryover triage lanes (BLOCKING / HOT /
/// AGING / STANDING, `MV.ticket.carryover-triage-ranking`) as a flat,
/// unrendered `Vec<AttentionRow>` — the gather half of
/// [`render_attention_section_with_distilled`], split out so a downstream
/// consumer (e.g. `mev attention-queue`) can build payloads from the same
/// structured rows the board renders from, instead of re-deriving them or
/// parsing rendered Markdown.
///
/// Every returned row has `lane` populated (`Some(TriageLane::..)`) — callers
/// split back out by that field, never by which internal `Vec` a row landed
/// in. This performs no rendering and produces no Markdown; flattening a row
/// to a display line happens only in [`render_triage_detail`] /
/// [`render_triage_lane`], at render time.
pub(crate) fn collect_attention_rows(
    carryover: &[(String, &Carryover)],
    today: chrono::NaiveDate,
    thresholds: &crate::brain::config::AttentionThresholds,
    block_priorities: &HashMap<String, u8>,
    block_status: &HashMap<String, Option<String>>,
) -> Vec<AttentionRow> {
    // Build a `CarryoverVerdict` per non-snoozed entry — just enough for
    // `rank_carryover` (age/stale/priority/blocks), never re-running the
    // `clears_when` predicate evaluation (`MV.ticket.clears-when-evaluation`'s
    // job, not this gather path's) — `lane` stays `NotEvaluable` and
    // `clears_when_satisfied` is therefore always `false` here; that field is
    // deliberately not surfaced on the board today. A snoozed entry is
    // excluded from every triage lane, same as before this block.
    let mut verdicts: Vec<CarryoverVerdict> = Vec::with_capacity(carryover.len());
    let mut item_by_key: HashMap<String, &Carryover> = HashMap::with_capacity(carryover.len());
    for (repo, item) in carryover {
        if is_snoozed(item.snoozed_until.as_deref(), today) {
            continue;
        }
        let age_days = staleness_anchor(Some(item.created.as_str()), item.reviewed.as_deref())
            .map(|anchor| (today - anchor).num_days());
        let stale = carryover_stale_age(item, today, thresholds).is_some();
        let key = format!("{repo}:{}", item.slug);
        verdicts.push(CarryoverVerdict {
            repo: repo.clone(),
            slug: item.slug.clone(),
            kind: carryover_kind_str(&item.kind).into_owned(),
            text: item.text.clone(),
            clears_when: item.clears_when.as_ref().and_then(clears_when_display),
            created: item.created.clone(),
            age_days,
            stale,
            lane: CarryoverLane::NotEvaluable,
            refs: Vec::new(),
            reason: None,
            priority: item.priority,
            finding_id: item.finding_id.clone(),
            blocks: item.blocks.clone(),
            enforce: item.enforce,
            needs: item.needs.clone(),
        });
        item_by_key.insert(key, item);
    }

    let ranked = rank_carryover(&verdicts, block_priorities, block_status);

    ranked
        .iter()
        .map(|r| {
            let key = format!("{}:{}", r.repo, r.slug);
            let source = item_by_key.get(&key).copied();
            // Prefer the authored `summary` verbatim over the snippeted `text`.
            // Never re-snippet a present summary: it exists specifically to
            // already be short, so truncating it would cut an authored label
            // at a boundary its author did not choose. The over-long case is
            // caught at write time instead (W_STATE_CARRYOVER_SUMMARY_UNRENDERABLE),
            // where it is fixable.
            let title_or_text = source
                .map(|item| {
                    item.summary
                        .clone()
                        .unwrap_or_else(|| attention_snippet(&item.text, 80))
                })
                .unwrap_or_default();
            let clears_when = source
                .and_then(|item| item.clears_when.as_ref())
                .and_then(clears_when_display)
                .map(|c| attention_snippet(&c, 60));
            AttentionRow {
                repo: r.repo.clone(),
                age: r.age_days,
                kind: r.kind.clone(),
                slug: r.slug.clone(),
                title_or_text,
                priority: r.priority,
                effective_priority: r.effective_priority,
                lane: Some(r.lane),
                clears_when,
            }
        })
        .collect()
}

pub fn render_attention_section_with_distilled(
    carryover: &[(String, &Carryover)],
    backlog: &[(String, &Backlog)],
    distilled: &[(String, &str, &DistilledEntry)],
    today: chrono::NaiveDate,
    thresholds: &crate::brain::config::AttentionThresholds,
    block_priorities: &HashMap<String, u8>,
    block_status: &HashMap<String, Option<String>>,
) -> String {
    let carryover_rows =
        collect_attention_rows(carryover, today, thresholds, block_priorities, block_status);

    let mut blocking_rows: Vec<AttentionRow> = Vec::new();
    let mut hot_rows: Vec<AttentionRow> = Vec::new();
    let mut aging_rows: Vec<AttentionRow> = Vec::new();
    let mut standing_rows: Vec<AttentionRow> = Vec::new();
    for row in carryover_rows {
        // Split back out by `row.lane` — real data on the row, never
        // re-derived from which `Vec` it landed in (see `AttentionRow::lane`'s
        // doc comment). `collect_attention_rows` always populates `lane`.
        let lane = row
            .lane
            .expect("collect_attention_rows always populates `lane`");
        match lane {
            TriageLane::Blocking => blocking_rows.push(row),
            TriageLane::Hot => hot_rows.push(row),
            TriageLane::Aging => aging_rows.push(row),
            TriageLane::Standing => standing_rows.push(row),
        }
    }

    let mut backlog_rows: Vec<AttentionRow> = Vec::new();
    let mut capture_rows: Vec<AttentionRow> = Vec::new();
    for (repo, item) in backlog {
        if let Some(age) = backlog_stale_age(item, today, thresholds) {
            let is_capture = item.origin.as_ref().is_some_and(|o| o.kind == "capture");
            if is_capture {
                let notes = item
                    .origin
                    .as_ref()
                    .and_then(|o| o.notes.as_deref())
                    .or(item.notes.as_deref())
                    .unwrap_or("(no notes path)");
                capture_rows.push(AttentionRow {
                    repo: repo.clone(),
                    age: Some(age),
                    kind: String::new(),
                    slug: item.slug.clone(),
                    title_or_text: format!("{} — notes: {}", item.title, notes),
                    priority: None,
                    effective_priority: None,
                    lane: None,
                    clears_when: None,
                });
            } else {
                backlog_rows.push(AttentionRow {
                    repo: repo.clone(),
                    age: Some(age),
                    kind: item.status.clone(),
                    slug: item.slug.clone(),
                    title_or_text: item.title.clone(),
                    priority: None,
                    effective_priority: None,
                    lane: None,
                    clears_when: None,
                });
            }
        }
    }

    let mut distill_rows: Vec<AttentionRow> = Vec::new();
    for (repo, stem, entry) in distilled {
        if let Some(age) = distill_stale_age(entry, today, thresholds, stem) {
            let claim = if entry.claim.is_empty() {
                "(no claim text found)".to_string()
            } else {
                entry.claim.clone()
            };
            distill_rows.push(AttentionRow {
                repo: repo.clone(),
                age: Some(age),
                kind: (*stem).to_string(),
                slug: String::new(),
                title_or_text: attention_snippet(&claim, 80),
                priority: None,
                effective_priority: None,
                lane: None,
                clears_when: None,
            });
        }
    }

    [
        render_triage_lane("BLOCKING", &blocking_rows, CARRYOVER_LANE_CAP),
        render_triage_lane("HOT", &hot_rows, CARRYOVER_LANE_CAP),
        render_triage_lane("AGING", &aging_rows, CARRYOVER_LANE_CAP),
        render_triage_lane("STANDING", &standing_rows, CARRYOVER_LANE_CAP),
        render_attention_lane("Aging backlog", backlog_rows, |r| {
            format!("{} ({}) — {}", r.slug, r.kind, r.title_or_text)
        }),
        render_attention_lane("Orphaned captures", capture_rows, |r| {
            format!("{} — {}", r.slug, r.title_or_text)
        }),
        render_attention_lane_capped(
            "Stale distilled knowledge",
            distill_rows,
            DISTILL_LANE_CAP,
            |r| format!("{} — {}", r.kind, r.title_or_text),
        ),
    ]
    .join("\n\n")
}

// ---------------------------------------------------------------------------
// splice_generated — sentinel-aware idempotent splice
// ---------------------------------------------------------------------------

/// Replace the text between `<!-- BEGIN generated:{marker} -->` and
/// `<!-- END generated:{marker} -->` with `generated`.
///
/// Every line outside the sentinels is preserved verbatim.  The splice is
/// **idempotent**: re-splicing the result yields identical output.
///
/// Returns [`EmitError::MissingSentinel`] when:
/// - the `BEGIN` sentinel is absent from `original`, or
/// - the `BEGIN` sentinel appears but the `END` sentinel does not follow it.
pub fn splice_generated(
    original: &str,
    marker: &str,
    generated: &str,
) -> Result<String, EmitError> {
    let begin_tag = format!("<!-- BEGIN generated:{marker} -->");
    let end_tag = format!("<!-- END generated:{marker} -->");

    // Find the BEGIN sentinel line index.
    let lines: Vec<&str> = original.lines().collect();
    let begin_idx = lines.iter().position(|l| l.trim() == begin_tag.as_str());
    let Some(begin_idx) = begin_idx else {
        return Err(EmitError::MissingSentinel {
            marker: marker.to_string(),
        });
    };

    // Find the END sentinel after BEGIN.
    let end_idx = lines[begin_idx + 1..]
        .iter()
        .position(|l| l.trim() == end_tag.as_str())
        .map(|rel| begin_idx + 1 + rel);
    let Some(end_idx) = end_idx else {
        return Err(EmitError::MissingSentinel {
            marker: marker.to_string(),
        });
    };

    // Reconstruct: everything up to and including BEGIN, then generated, then END onwards.
    let before: Vec<&str> = lines[..=begin_idx].to_vec();
    let after: Vec<&str> = lines[end_idx..].to_vec();

    let mut result_parts: Vec<&str> = before;
    // Push generated lines (may be empty).
    let generated_lines: Vec<&str> = if generated.is_empty() {
        vec![]
    } else {
        generated.lines().collect()
    };
    result_parts.extend(generated_lines);
    result_parts.extend(after);

    // Preserve original trailing newline behaviour: if original ended with a newline, add one.
    let trailing_newline = original.ends_with('\n');
    let mut result = result_parts.join("\n");
    if trailing_newline {
        result.push('\n');
    }

    Ok(result)
}

// ---------------------------------------------------------------------------
// Task 3 — EmitAction, EmitPlan, planners, apply_plan
// ---------------------------------------------------------------------------

/// A single proposed file write produced by a planner.
///
/// Pure data — no IO is performed until [`apply_plan`] is called.
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

/// Restrict a planner's proposed writes to a single repo's derived surfaces
/// (`mev emit-state --scope <repo>`, ticket `ticket-emit-state-scope-and-lock`).
///
/// `scope == None` is the unscoped, full-corpus default — every action passes
/// through unfiltered, byte-for-byte identical to calling the planner alone.
/// `scope == Some(set)` drops any [`EmitAction`] whose `path` is not one of
/// `set`'s four target surfaces (own `state.json`, `cache_doc`, tier rollup
/// `status.md`, HQ board `status.md`) — see
/// [`crate::brain::config::ScopeDependencySet::allows`]. Diagnostics (e.g.
/// `W_EMIT_NO_SENTINEL`) are always passed through unfiltered: they report on
/// planning-time conditions, not writes, and scoping is about *what gets
/// written*, not what gets diagnosed.
///
/// Filtering out-of-scope actions here — rather than never planning them —
/// keeps every planner computing from the full, unfiltered corpus (`loaded`/
/// `graph` stay whole), so a scoped run's rollup tables still reflect every
/// repo's current state; only the *write* is narrowed. This is what makes a
/// scoped run never blank a repo it did not visit.
pub fn filter_plan_by_scope(
    plan: EmitPlan,
    root: &std::path::Path,
    scope: Option<&crate::brain::config::ScopeDependencySet>,
) -> EmitPlan {
    let Some(scope) = scope else {
        return plan;
    };
    EmitPlan {
        actions: plan
            .actions
            .into_iter()
            .filter(|a| scope.allows(root, &a.path))
            .collect(),
        diagnostics: plan.diagnostics,
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// A `tracks[].blocks[]` entry's title, authored status, priority, and due date.
type BlockIndexEntry = (String, Option<String>, Option<u8>, Option<String>);

/// Map every `tracks[].blocks[]` id in one file to its [`BlockIndexEntry`].
fn id_index(file: &StateFile) -> HashMap<String, BlockIndexEntry> {
    let mut map = HashMap::new();
    for track in &file.tracks {
        for block in &track.blocks {
            map.insert(
                block.id.clone(),
                (
                    block.title.clone(),
                    block.status.clone(),
                    block.priority,
                    block.due.clone(),
                ),
            );
        }
    }
    map
}

/// Call [`derive_focus`] and rehydrate the returned id lists into a [`Focus`] struct,
/// filling titles (and `priority`/`due`, when present on the source block) from this
/// file's `tracks[]`.
fn derived_focus_for(
    src: &StateSource,
    file: &StateFile,
    graph: &StateGraph,
    files: &[(StateSource, StateFile)],
) -> Focus {
    let idx = id_index(file);
    let d = derive_focus(src, file, graph, files, None);
    let title_of = |id: &str| idx.get(id).map(|(t, ..)| t.clone()).unwrap_or_default();
    let priority_of = |id: &str| idx.get(id).and_then(|(_, _, p, _)| *p);
    let due_of = |id: &str| idx.get(id).and_then(|(_, _, _, d)| d.clone());

    // `epics` stays empty in a leaf `focus`: membership is authored on
    // `tracks[].blocks[]`, and the epic board filters `derive_brain_focus`'s
    // union (which does carry it). Duplicating it here would make every leaf
    // `state.json` churn whenever a block's membership changes, on top of the
    // authored edit itself — and the empty vec is skipped on serialization, so
    // tagging blocks leaves leaf files byte-identical.
    let now = d
        .now
        .iter()
        .map(|id| Block {
            due: due_of(id),
            priority: priority_of(id),
            id: id.clone(),
            title: title_of(id),
            status: Some("in_progress".to_string()),
            note: None,
            repo: None,
            blocked_by: Vec::new(),
            epics: Vec::new(),
        })
        .collect();

    let next = d
        .next
        .iter()
        .map(|id| Block {
            due: due_of(id),
            priority: priority_of(id),
            id: id.clone(),
            title: title_of(id),
            status: None,
            note: None,
            repo: None,
            blocked_by: Vec::new(),
            epics: Vec::new(),
        })
        .collect();

    let blocked = d
        .blocked
        .iter()
        .map(|(id, unmet)| Block {
            due: due_of(id),
            priority: priority_of(id),
            id: id.clone(),
            title: title_of(id),
            status: None,
            note: None,
            repo: None,
            blocked_by: unmet.clone(),
            epics: Vec::new(),
        })
        .collect();

    let deferred = d
        .deferred
        .iter()
        .map(|id| Block {
            due: due_of(id),
            priority: priority_of(id),
            id: id.clone(),
            title: title_of(id),
            status: Some("deferred".to_string()),
            note: None,
            repo: None,
            blocked_by: Vec::new(),
            epics: Vec::new(),
        })
        .collect();

    Focus {
        now,
        next,
        blocked,
        deferred,
    }
}

// ---------------------------------------------------------------------------
// plan_state_json
// ---------------------------------------------------------------------------

/// Plan the derived-section rewrites for every loaded `state.json`.
///
/// - Leaf (`kind == "project"`): regenerate `focus` from [`derive_focus`].
/// - Brain (`kind == "brain"`): regenerate `repos[]` (tier-scoped, non-destructive —
///   see [`tier_scope_for`] / [`derive_rollup`]), `cross_repo[]`, and `focus`
///   (the repo-tagged union of in-scope children's derived focus — see
///   [`derive_brain_focus`]).
///
/// An [`EmitAction`] is added only when the re-serialised derived file differs from
/// the re-serialised original (fixed-point property — no action when already correct).
pub fn plan_state_json(
    files: &[(StateSource, StateFile)],
    graph: &StateGraph,
    config: &BrainConfig,
) -> EmitPlan {
    let mut plan = EmitPlan::default();

    for (src, file) in files {
        let mut derived = file.clone();

        match file.kind.as_str() {
            "project" => {
                derived.focus = derived_focus_for(src, file, graph, files);
            }
            "brain" => {
                let scope = tier_scope_for(file, config);
                derived.repos = derive_rollup(&scope, config, &file.repos, graph, files);
                derived.cross_repo = derive_cross_repo(files);
                derived.focus = derive_brain_focus(src, file, &scope, config, graph, files);
            }
            _ => continue, // unknown kind already flagged by check_schema
        }

        // Fixed-point check: compare canonical serialisations (both newline-free).
        let original = match serde_json::to_string_pretty(file) {
            Ok(s) => s,
            Err(e) => {
                plan.diagnostics.push(crate::Diagnostic::warning(
                    &src.abs_path,
                    "W_EMIT_SERIALIZE_FAILED",
                    format!(
                        "could not serialize original state for '{}': {e}",
                        src.repo_slug
                    ),
                ));
                continue;
            }
        };
        let new_serialised = match serde_json::to_string_pretty(&derived) {
            Ok(s) => s,
            Err(e) => {
                plan.diagnostics.push(crate::Diagnostic::warning(
                    &src.abs_path,
                    "W_EMIT_SERIALIZE_FAILED",
                    format!(
                        "could not serialize derived state for '{}': {e}",
                        src.repo_slug
                    ),
                ));
                continue;
            }
        };

        if new_serialised != original {
            let note = if file.kind == "project" {
                format!("regenerate focus for '{}'", src.repo_slug)
            } else {
                format!("regenerate repos[]/cross_repo[] for '{}'", src.repo_slug)
            };
            plan.actions.push(EmitAction {
                path: src.abs_path.clone(),
                // Add a trailing newline so the file is a POSIX text file.
                new_content: format!("{new_serialised}\n"),
                note,
            });
        }
    }

    plan
}

// ---------------------------------------------------------------------------
// plan_master_plan_tables
// ---------------------------------------------------------------------------

/// Plan the wave-table splice into each state file's sibling `master-plan.md`.
///
/// For each loaded state file, locates `<state.json parent>/master-plan.md`.  If
/// it exists and carries the `wave-table` sentinels, splices the rendered table
/// and adds an [`EmitAction`].  A missing file or missing sentinels produces a
/// [`W_EMIT_NO_SENTINEL`] warning diagnostic — never invents sentinels into
/// arbitrary prose.
///
/// `portfolio`-kind files are skipped entirely: they are terminal repos
/// (published to GitHub, no further planning state) and never carry a
/// `master-plan.md`, so flagging one would just be noise.
pub fn plan_master_plan_tables(files: &[(StateSource, StateFile)], graph: &StateGraph) -> EmitPlan {
    let mut plan = EmitPlan::default();
    let global_status = global_status_map(files);

    for (src, file) in files {
        if file.kind == "portfolio" {
            continue;
        }
        let Some(planning_dir) = src.abs_path.parent() else {
            continue;
        };
        let mp_path = planning_dir.join("master-plan.md");

        if !mp_path.exists() {
            plan.diagnostics.push(crate::Diagnostic::warning(
                &mp_path,
                "W_EMIT_NO_SENTINEL",
                format!(
                    "no master-plan.md beside '{}' state.json; skipping table emit",
                    src.repo_slug
                ),
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

        let table = render_wave_table(&src.repo_slug, file, graph, &global_status);

        match splice_generated(&original, markers::WAVE_TABLE, &table) {
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
                // Missing or unbalanced sentinels → warning, no write.
                plan.diagnostics.push(crate::Diagnostic::warning(
                    &mp_path,
                    "W_EMIT_NO_SENTINEL",
                    format!(
                        "master-plan.md for '{}' has no <!-- BEGIN generated:wave-table --> \
                         sentinels; skipping",
                        src.repo_slug
                    ),
                ));
            }
        }
    }

    plan
}

// ---------------------------------------------------------------------------
// plan_project_caches
// ---------------------------------------------------------------------------

/// Render the one-line derived focus headline for a project-kind repo.
///
/// Format: `**Current focus:** <now>. Next: <next>. Blocked: <blocked>.` where each
/// section joins its blocks as `` `id` — title `` (comma-separated), or the literal
/// `none` when the section is empty. This is the line [`plan_project_caches`]
/// splices into a project cache doc's [`markers::PROJECT_CACHE`] sentinel.
///
/// A fourth `` Deferred: <deferred>.`` clause is appended **only when the lane is
/// non-empty**. Emitting it unconditionally (as `Deferred: none.`) would rewrite
/// every project cache doc in the portfolio on the first `emit-state --write`
/// after this change, for repos that have deferred nothing.
fn render_focus_line(
    src: &StateSource,
    file: &StateFile,
    graph: &StateGraph,
    files: &[(StateSource, StateFile)],
) -> String {
    let focus = derived_focus_for(src, file, graph, files);

    let summarize = |blocks: &[Block]| -> String {
        if blocks.is_empty() {
            "none".to_string()
        } else {
            blocks
                .iter()
                .map(|b| format!("`{}` — {}", b.id, b.title))
                .collect::<Vec<_>>()
                .join(", ")
        }
    };

    let mut line = format!(
        "**Current focus:** {}. Next: {}. Blocked: {}.",
        summarize(&focus.now),
        summarize(&focus.next),
        summarize(&focus.blocked)
    );
    if !focus.deferred.is_empty() {
        line.push_str(&format!(" Deferred: {}.", summarize(&focus.deferred)));
    }
    line
}

/// Reconcile the `synced_from` scalar in `original`'s OKF frontmatter to `new_value`.
///
/// Locates the leading `---`-fenced frontmatter block (the opening fence must be the
/// document's first line). If a `synced_from:` line already exists inside the block,
/// its value is replaced in place; otherwise a new `synced_from:` line is appended at
/// the end of the block (before the closing fence). The value is always emitted
/// double-quoted (matching this codebase's existing hand-authored `synced_from`
/// docs), with `\` and `"` escaped.
///
/// When `original` has no leading frontmatter fence, or the fence is never closed,
/// `original` is returned unchanged — this function never invents a frontmatter
/// block into a document that lacks one.
fn reconcile_synced_from(original: &str, new_value: &str) -> String {
    let lines: Vec<&str> = original.lines().collect();
    if lines.first().map(|l| l.trim()) != Some("---") {
        return original.to_string();
    }
    let Some(end_idx) = lines[1..]
        .iter()
        .position(|l| l.trim() == "---")
        .map(|i| i + 1)
    else {
        return original.to_string();
    };

    let escaped = new_value.replace('\\', "\\\\").replace('"', "\\\"");
    let new_line = format!("synced_from: \"{escaped}\"");

    let mut fm_lines: Vec<String> = lines[1..end_idx].iter().map(|s| s.to_string()).collect();
    match fm_lines
        .iter()
        .position(|l| l.trim_start().starts_with("synced_from:"))
    {
        Some(pos) => fm_lines[pos] = new_line,
        None => fm_lines.push(new_line),
    }

    let mut result_lines: Vec<String> = Vec::with_capacity(lines.len() + 1);
    result_lines.push(lines[0].to_string());
    result_lines.extend(fm_lines);
    result_lines.extend(lines[end_idx..].iter().map(|s| s.to_string()));

    let trailing_newline = original.ends_with('\n');
    let mut result = result_lines.join("\n");
    if trailing_newline {
        result.push('\n');
    }
    result
}

/// Reconcile the `now`, `next`, `blocked`, and `deferred` scalars in `original`'s
/// OKF frontmatter to match the provided `focus`.
///
/// Locates the leading `---`-fenced frontmatter block. For each scalar:
/// - if the queue is empty, the value is the literal `[]`.
/// - otherwise, the value is a double-quoted string joining the blocks in that queue
///   (e.g., `"repo:id — title"`), with `\` and `"` escaped.
///
/// If a line for the key exists, it is always replaced. A **missing** line is
/// appended before the closing fence only for the three original scalars —
/// `deferred:` is appended only when the lane is non-empty. Appending it
/// unconditionally would inject a `deferred: []` line into the frontmatter of
/// every `status.md` in the portfolio on the first `emit-state --write` after
/// this change. An existing `deferred:` line is still updated (so a lane that
/// empties out correctly becomes `[]`).
///
/// Returns `original` unchanged if no frontmatter block is found.
pub fn reconcile_status_scalars(original: &str, focus: &Focus) -> String {
    let lines: Vec<&str> = original.lines().collect();
    if lines.first().map(|l| l.trim()) != Some("---") {
        return original.to_string();
    }
    let Some(end_idx) = lines[1..]
        .iter()
        .position(|l| l.trim() == "---")
        .map(|i| i + 1)
    else {
        return original.to_string();
    };

    let mut fm_lines: Vec<String> = lines[1..end_idx].iter().map(|s| s.to_string()).collect();

    let format_queue = |blocks: &[Block]| -> String {
        if blocks.is_empty() {
            "[]".to_string()
        } else {
            let joined = blocks
                .iter()
                .map(|b| {
                    if let Some(repo) = &b.repo {
                        format!("{repo}:{} — {}", b.id, b.title)
                    } else {
                        format!("{} — {}", b.id, b.title)
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            let escaped = joined.replace('\\', "\\\\").replace('"', "\\\"");
            format!("\"{escaped}\"")
        }
    };

    let updates = [
        ("now:", format_queue(&focus.now), true),
        ("next:", format_queue(&focus.next), true),
        ("blocked:", format_queue(&focus.blocked), true),
        (
            "deferred:",
            format_queue(&focus.deferred),
            !focus.deferred.is_empty(),
        ),
    ];

    for (key, val, append_if_missing) in updates {
        let new_line = format!("{key} {val}");
        match fm_lines
            .iter()
            .position(|l| l.trim_start().starts_with(key))
        {
            Some(pos) => fm_lines[pos] = new_line,
            None => {
                if append_if_missing {
                    fm_lines.push(new_line);
                }
            }
        }
    }

    let mut result_lines: Vec<String> = Vec::with_capacity(lines.len() + 1);
    result_lines.push(lines[0].to_string());
    result_lines.extend(fm_lines);
    result_lines.extend(lines[end_idx..].iter().map(|s| s.to_string()));

    let trailing_newline = original.ends_with('\n');
    let mut result = result_lines.join("\n");
    if trailing_newline {
        result.push('\n');
    }
    result
}

/// Plan the project-cache splice for each project-kind repo's `docs/projects/<slug>.md`
/// (or whatever path its `brain.toml` `[[repos]]` entry names as `cache_doc`).
///
/// For each loaded file with `kind == "project"`, looks up its `brain.toml`
/// `[[repos]]` entry by `repo_slug` and resolves `root.join(&entry.cache_doc)`
/// (the same resolution `check_sync` uses). If the target doc exists and carries
/// the [`markers::PROJECT_CACHE`] sentinels, splices in the rendered
/// [`render_focus_line`] headline and reconciles the doc's OKF frontmatter
/// `synced_from` field to the child repo's own `status_file` `timestamp`
/// watermark — the same field `check_sync` validates against (see
/// [`reconcile_synced_from`]). An [`EmitAction`] is added only when the resulting
/// content differs from the original (fixed-point property).
///
/// A missing target doc, or one lacking the sentinels, produces a
/// `W_EMIT_NO_SENTINEL` warning diagnostic and no write — this planner never
/// splices into arbitrary prose. A repo with no matching `[[repos]]` entry, or
/// whose entry has a blank `cache_doc`, is silently skipped (nothing to target).
/// If `status_file` can't be read or has no `timestamp` field, the same warning
/// is emitted and the cache write is skipped — `synced_from` is never reconciled
/// to a value that `check_sync` couldn't validate anyway.
pub fn plan_project_caches(
    root: &std::path::Path,
    files: &[(StateSource, StateFile)],
    graph: &StateGraph,
    config: &BrainConfig,
) -> EmitPlan {
    let mut plan = EmitPlan::default();

    for (src, file) in files {
        if file.kind != "project" {
            continue;
        }
        let Some(entry) = config.repos.iter().find(|r| r.slug == src.repo_slug) else {
            continue;
        };
        if entry.cache_doc.trim().is_empty() {
            continue;
        }
        let cache_path = root.join(&entry.cache_doc);

        let original = match std::fs::read_to_string(&cache_path) {
            Ok(s) => s,
            Err(_) => {
                plan.diagnostics.push(crate::Diagnostic::warning(
                    &cache_path,
                    "W_EMIT_NO_SENTINEL",
                    format!(
                        "no project-cache doc for '{}' at '{}'; skipping cache emit",
                        src.repo_slug, entry.cache_doc
                    ),
                ));
                continue;
            }
        };

        let focus_line = render_focus_line(src, file, graph, files);

        let spliced = match splice_generated(&original, markers::PROJECT_CACHE, &focus_line) {
            Ok(c) => c,
            Err(_) => {
                plan.diagnostics.push(crate::Diagnostic::warning(
                    &cache_path,
                    "W_EMIT_NO_SENTINEL",
                    format!(
                        "project-cache doc for '{}' has no <!-- BEGIN generated:project-cache \
                         --> sentinels; skipping",
                        src.repo_slug
                    ),
                ));
                continue;
            }
        };

        let status_path = root.join(&entry.status_file);
        let timestamp = match crate::brain::sync::read_watermark(&status_path)
            .ok()
            .and_then(|fm| fm.timestamp)
        {
            Some(t) => t,
            None => {
                plan.diagnostics.push(crate::Diagnostic::warning(
                    &status_path,
                    "W_EMIT_NO_SENTINEL",
                    format!(
                        "repo '{}': status_file '{}' has no readable 'timestamp' field; \
                         skipping cache emit",
                        src.repo_slug, entry.status_file
                    ),
                ));
                continue;
            }
        };

        let new_content = reconcile_synced_from(&spliced, &timestamp);

        if new_content != original {
            plan.actions.push(EmitAction {
                path: cache_path,
                new_content,
                note: format!("update project cache for '{}'", src.repo_slug),
            });
        }
    }

    plan
}

// ---------------------------------------------------------------------------
// plan_brain_cache_watermarks
// ---------------------------------------------------------------------------

/// Reconciles `synced_from` in the `cache_doc` for `kind == "brain"` files.
///
/// Brain-kind repos (tier sub-brains, and dual-role nodes like `business`) have
/// their own `docs/projects/<slug>.md`-style cache doc but no project-cache
/// sentinel to splice a focus-line into (that's [`plan_project_caches`]'s
/// job for `kind == "project"` repos) — this planner only reconciles the
/// OKF frontmatter `synced_from` watermark to the repo's own `status_file`
/// `timestamp`, the same field [`check_sync`](crate::brain::sync::check_sync)
/// validates against (see [`reconcile_synced_from`]). An [`EmitAction`] is
/// added only when the resulting content differs from the original
/// (fixed-point property).
///
/// A repo with no matching `[[repos]]` entry, or whose entry has a blank
/// `cache_doc`, is silently skipped (nothing to target). A missing/unreadable
/// cache doc produces a `W_EMIT_IO_ERROR` warning and no write. If
/// `status_file` can't be read or has no `timestamp` field, a
/// `W_EMIT_NO_SENTINEL` warning is emitted and the write is skipped —
/// `synced_from` is never reconciled to a value `check_sync` couldn't
/// validate anyway.
pub fn plan_brain_cache_watermarks(
    root: &std::path::Path,
    files: &[(StateSource, StateFile)],
    config: &BrainConfig,
) -> EmitPlan {
    let mut plan = EmitPlan::default();

    for (src, file) in files {
        if file.kind != "brain" {
            continue;
        }
        let Some(entry) = config.repos.iter().find(|r| r.slug == src.repo_slug) else {
            continue;
        };
        if entry.cache_doc.trim().is_empty() {
            continue;
        }
        let cache_path = root.join(&entry.cache_doc);

        let original = match std::fs::read_to_string(&cache_path) {
            Ok(s) => s,
            Err(_) => {
                plan.diagnostics.push(crate::Diagnostic::warning(
                    &cache_path,
                    "W_EMIT_IO_ERROR",
                    format!(
                        "could not read brain-cache doc for '{}' at '{}'; skipping cache emit",
                        src.repo_slug, entry.cache_doc
                    ),
                ));
                continue;
            }
        };

        let status_path = root.join(&entry.status_file);
        let timestamp = match crate::brain::sync::read_watermark(&status_path)
            .ok()
            .and_then(|fm| fm.timestamp)
        {
            Some(t) => t,
            None => {
                plan.diagnostics.push(crate::Diagnostic::warning(
                    &status_path,
                    "W_EMIT_NO_SENTINEL",
                    format!(
                        "repo '{}': status_file '{}' has no readable 'timestamp' field; \
                         skipping cache emit",
                        src.repo_slug, entry.status_file
                    ),
                ));
                continue;
            }
        };

        let new_content = reconcile_synced_from(&original, &timestamp);

        if new_content != original {
            plan.actions.push(EmitAction {
                path: cache_path,
                new_content,
                note: format!("update brain cache watermark for '{}'", src.repo_slug),
            });
        }
    }

    plan
}

// ---------------------------------------------------------------------------
// plan_tier_rollups
// ---------------------------------------------------------------------------

/// Render the tier-rollup table body for a tier's rolled-up repo rows.
///
/// Columns: `Repo | Now | Next | Blocked`. Each cell joins its blocks as
/// `` `id` — title `` (comma-separated), or the literal `none` when the
/// section is empty. This is the table body [`plan_tier_rollups`] splices
/// into a tier's `status.md`'s [`markers::TIER_ROLLUP`] sentinel.
fn render_tier_rollup_table(rollups: &[RepoRollup]) -> String {
    let summarize = |blocks: &[Block]| -> String {
        if blocks.is_empty() {
            "none".to_string()
        } else {
            blocks
                .iter()
                .map(|b| format!("`{}` — {}", b.id, b.title))
                .collect::<Vec<_>>()
                .join(", ")
        }
    };

    let mut rows: Vec<String> = Vec::new();
    rows.push("| Repo | Now | Next | Blocked |".to_string());
    rows.push("|------|-----|------|---------|".to_string());

    for rollup in rollups {
        rows.push(format!(
            "| {} | {} | {} | {} |",
            rollup.repo,
            summarize(&rollup.now),
            summarize(&rollup.next),
            summarize(&rollup.blocked)
        ));
    }

    rows.join("\n")
}

/// Plan the tier-rollup splice into each tier sub-brain's sibling `status.md`.
///
/// For every loaded `kind == "brain"` file whose [`tier_scope_for`] resolves to
/// [`TierScope::Tier`] (i.e. every tier sub-brain — the HQ root, whose `repo`
/// matches no declared tier and resolves to [`TierScope::All`], is out of
/// scope here; its cross-repo view is `MV.4.C`'s `plan_hq_board`), derives the
/// tier-scoped rollup rows via [`derive_rollup`] (fed the tier's own current
/// `repos[]` as the non-destructive `existing` baseline) and renders them via
/// [`render_tier_rollup_table`].
///
/// The target doc is `<tier's state.json parent>/status.md` — the same
/// state-file-relative resolution [`plan_master_plan_tables`] uses for
/// `master-plan.md`. If it exists and carries the [`markers::TIER_ROLLUP`]
/// sentinels, the rendered table is spliced in and an [`EmitAction`] is added
/// only when the resulting content differs from the original (fixed-point
/// property). A missing `status.md`, or one lacking the sentinels, produces a
/// `W_EMIT_NO_SENTINEL` warning diagnostic and no write — this planner never
/// splices into arbitrary prose.
pub fn plan_tier_rollups(
    files: &[(StateSource, StateFile)],
    graph: &StateGraph,
    config: &BrainConfig,
) -> EmitPlan {
    let mut plan = EmitPlan::default();

    for (src, file) in files {
        if file.kind != "brain" {
            continue;
        }
        let scope = tier_scope_for(file, config);
        let tier_name = match &scope {
            TierScope::Tier(t) => t.clone(),
            TierScope::All => continue,
        };

        let Some(planning_dir) = src.abs_path.parent() else {
            continue;
        };
        let status_path = planning_dir.join("status.md");

        let original = match std::fs::read_to_string(&status_path) {
            Ok(s) => s,
            Err(_) => {
                plan.diagnostics.push(crate::Diagnostic::warning(
                    &status_path,
                    "W_EMIT_NO_SENTINEL",
                    format!(
                        "no status.md beside tier '{tier_name}' state.json; skipping \
                         tier-rollup emit"
                    ),
                ));
                continue;
            }
        };

        let rollups = derive_rollup(&scope, config, &file.repos, graph, files);
        let table = render_tier_rollup_table(&rollups);

        match splice_generated(&original, markers::TIER_ROLLUP, &table) {
            Ok(new_content) => {
                if new_content != original {
                    plan.actions.push(EmitAction {
                        path: status_path,
                        new_content,
                        note: format!("update tier rollup for '{tier_name}'"),
                    });
                }
            }
            Err(_) => {
                plan.diagnostics.push(crate::Diagnostic::warning(
                    &status_path,
                    "W_EMIT_NO_SENTINEL",
                    format!(
                        "status.md for tier '{tier_name}' has no <!-- BEGIN \
                         generated:tier-rollup --> sentinels; skipping"
                    ),
                ));
            }
        }
    }

    plan
}

// ---------------------------------------------------------------------------
// plan_hq_board
// ---------------------------------------------------------------------------

/// Plan the HQ Operating Board splice into the HQ brain's sibling `status.md`.
///
/// For every loaded `kind == "brain"` file whose [`tier_scope_for`] resolves to
/// [`TierScope::All`] (the HQ root — a tier sub-brain resolves to
/// [`TierScope::Tier`] and is out of scope here; its rolled-up view is
/// [`plan_tier_rollups`]'s responsibility), derives the brain-wide [`Focus`] via
/// [`derive_brain_focus`] and the `cross_repo[]` edges via [`derive_cross_repo`],
/// then renders them via [`render_hq_board`].
///
/// The target doc is `<HQ state.json parent>/status.md` — the same
/// state-file-relative resolution [`plan_master_plan_tables`] and
/// [`plan_tier_rollups`] use for their sibling Markdown files. If it exists and
/// carries the [`markers::HQ_BOARD`] sentinels, the rendered board is spliced in
/// and an [`EmitAction`] is added only when the resulting content differs from
/// the original (fixed-point property). A missing `status.md`, or one lacking
/// the sentinels, produces a `W_EMIT_NO_SENTINEL` warning diagnostic and no
/// write — this planner never splices into arbitrary prose.
pub fn plan_hq_board(
    files: &[(StateSource, StateFile)],
    graph: &StateGraph,
    config: &BrainConfig,
) -> EmitPlan {
    let mut plan = EmitPlan::default();

    for (src, file) in files {
        if file.kind != "brain" {
            continue;
        }
        let scope = tier_scope_for(file, config);
        if !matches!(scope, TierScope::All) {
            continue; // tier sub-brains are plan_tier_rollups's responsibility
        }

        let Some(planning_dir) = src.abs_path.parent() else {
            continue;
        };
        let board_path = planning_dir.join("status.md");

        let original = match std::fs::read_to_string(&board_path) {
            Ok(s) => s,
            Err(_) => {
                plan.diagnostics.push(crate::Diagnostic::warning(
                    &board_path,
                    "W_EMIT_NO_SENTINEL",
                    format!(
                        "no status.md beside HQ '{}' state.json; skipping hq-board emit",
                        src.repo_slug
                    ),
                ));
                continue;
            }
        };

        let focus = derive_brain_focus(src, file, &scope, config, graph, files);
        let edges = derive_cross_repo(files);
        let board = render_hq_board(&focus, &edges);

        match splice_generated(&original, markers::HQ_BOARD, &board) {
            Ok(new_content) => {
                if new_content != original {
                    plan.actions.push(EmitAction {
                        path: board_path,
                        new_content,
                        note: format!("update HQ operating board for '{}'", src.repo_slug),
                    });
                }
            }
            Err(_) => {
                plan.diagnostics.push(crate::Diagnostic::warning(
                    &board_path,
                    "W_EMIT_NO_SENTINEL",
                    format!(
                        "status.md for HQ '{}' has no <!-- BEGIN generated:hq-board --> \
                         sentinels; skipping",
                        src.repo_slug
                    ),
                ));
            }
        }
    }

    plan
}

// ---------------------------------------------------------------------------
// plan_unified_board
// ---------------------------------------------------------------------------

/// Plan the unified priority-ranked NOW/NEXT/BLOCKED/DUE-SOON board splice
/// into the HQ brain's sibling `status.md` (`MV.6.B`).
///
/// Mirrors [`plan_hq_board`] exactly, but targets the separate
/// [`markers::UNIFIED_BOARD`] sentinel and renders via
/// [`render_unified_board`], which additionally tags each row `[BIZ]`/`[ENG]`
/// by source-repo tier and adds the DUE-SOON section (evaluated against
/// `today`). The [`markers::HQ_BOARD`] region and every other planner are
/// untouched — this is an independent sentinel in the same document.
pub fn plan_unified_board(
    files: &[(StateSource, StateFile)],
    graph: &StateGraph,
    config: &BrainConfig,
    today: chrono::NaiveDate,
) -> EmitPlan {
    let mut plan = EmitPlan::default();

    for (src, file) in files {
        if file.kind != "brain" {
            continue;
        }
        let scope = tier_scope_for(file, config);
        if !matches!(scope, TierScope::All) {
            continue; // tier sub-brains have no unified board; HQ root only
        }

        let Some(planning_dir) = src.abs_path.parent() else {
            continue;
        };
        let board_path = planning_dir.join("status.md");

        let original = match std::fs::read_to_string(&board_path) {
            Ok(s) => s,
            Err(_) => {
                plan.diagnostics.push(crate::Diagnostic::warning(
                    &board_path,
                    "W_EMIT_NO_SENTINEL",
                    format!(
                        "no status.md beside HQ '{}' state.json; skipping unified-board emit",
                        src.repo_slug
                    ),
                ));
                continue;
            }
        };

        let focus = derive_brain_focus(src, file, &scope, config, graph, files);
        let edges = derive_cross_repo(files);
        let effective = effective_priorities(graph, files);
        let board = render_unified_board(&focus, &edges, &effective, config, today);

        match splice_generated(&original, markers::UNIFIED_BOARD, &board) {
            Ok(new_content) => {
                if new_content != original {
                    plan.actions.push(EmitAction {
                        path: board_path,
                        new_content,
                        note: format!("update unified priority board for '{}'", src.repo_slug),
                    });
                }
            }
            Err(_) => {
                plan.diagnostics.push(crate::Diagnostic::warning(
                    &board_path,
                    "W_EMIT_NO_SENTINEL",
                    format!(
                        "status.md for HQ '{}' has no <!-- BEGIN generated:unified-board --> \
                         sentinels; skipping",
                        src.repo_slug
                    ),
                ));
            }
        }
    }

    plan
}

// ---------------------------------------------------------------------------
// plan_attention_board
// ---------------------------------------------------------------------------

/// The `brain.toml` tier a repo slug belongs to (its `[[repos]]` `tier`), if any.
fn tier_of_repo<'a>(slug: &str, config: &'a BrainConfig) -> Option<&'a str> {
    config
        .repos
        .iter()
        .find(|r| r.slug == slug)
        .map(|r| r.tier.as_str())
}

// ---------------------------------------------------------------------------
// plan_epic_boards
// ---------------------------------------------------------------------------

/// Plan the per-epic board splice into every brain-level `status.md` that
/// carries the [`markers::EPIC_BOARD`] sentinel pair.
///
/// **Epic lanes are always global**, derived once from the HQ file at
/// `TierScope::All`, even on a tier sub-brain's board. An epic is a cross-repo
/// initiative by definition — showing a tier-truncated slice of one would hide
/// exactly the cross-boundary sequence the board exists to make visible. What
/// *is* tier-scoped is **which epics appear**: the HQ board shows every
/// registered epic, and a tier sub-brain shows only those with at least one
/// member block in its own tier. (This is the one board that deliberately
/// departs from `plan_attention_board`'s tier-scoped-content rule.)
///
/// A missing `status.md`, or one lacking the sentinels, yields
/// `W_EMIT_NO_SENTINEL` and no write — sentinels are never invented.
pub fn plan_epic_boards(
    root: &Path,
    files: &[(StateSource, StateFile)],
    graph: &StateGraph,
    config: &BrainConfig,
) -> EmitPlan {
    let mut plan = EmitPlan::default();

    // The registry and the global focus both come from the single All-scoped
    // brain file. With no HQ file there is nothing to render anywhere.
    let Some((hq_src, hq_file)) = files
        .iter()
        .find(|(_, f)| f.kind == "brain" && matches!(tier_scope_for(f, config), TierScope::All))
    else {
        return plan;
    };
    if hq_file.epics.is_empty() {
        return plan; // no registry authored yet — nothing to emit
    }

    let global_focus = derive_brain_focus(hq_src, hq_file, &TierScope::All, config, graph, files);
    let effective = effective_priorities(graph, files);

    // Derive each epic's members + boundary edges once, not per brain file.
    let all_inputs: Vec<EpicBoardInput<'_>> = hq_file
        .epics
        .iter()
        .map(|epic| EpicBoardInput {
            epic,
            members: epic_members_resolved(root, graph, files, epic),
            edges: derive_epic_edges(files, &epic.slug),
        })
        .collect();

    for (src, file) in files {
        if file.kind != "brain" {
            continue;
        }
        let scope = tier_scope_for(file, config);

        // Which epics this board shows (see the tier note above).
        let inputs: Vec<EpicBoardInput<'_>> = all_inputs
            .iter()
            .filter(|i| match &scope {
                TierScope::All => true,
                TierScope::Tier(tier) => i
                    .members
                    .iter()
                    .any(|(repo, _)| tier_of_repo(repo, config) == Some(tier.as_str())),
            })
            .map(|i| EpicBoardInput {
                epic: i.epic,
                members: i.members.clone(),
                edges: i.edges.clone(),
            })
            .collect();
        if inputs.is_empty() {
            continue; // this tier owns no epic work; leave its sentinel alone
        }

        let Some(planning_dir) = src.abs_path.parent() else {
            continue;
        };
        let board_path = planning_dir.join("status.md");

        let original = match std::fs::read_to_string(&board_path) {
            Ok(s) => s,
            Err(_) => {
                plan.diagnostics.push(crate::Diagnostic::warning(
                    &board_path,
                    "W_EMIT_NO_SENTINEL",
                    format!(
                        "no status.md beside '{}' state.json; skipping epic-board emit",
                        src.repo_slug
                    ),
                ));
                continue;
            }
        };

        let board = render_epic_board(&inputs, &global_focus, &effective, config);

        match splice_generated(&original, markers::EPIC_BOARD, &board) {
            Ok(new_content) => {
                if new_content != original {
                    plan.actions.push(EmitAction {
                        path: board_path,
                        new_content,
                        note: format!("update epic board for '{}'", src.repo_slug),
                    });
                }
            }
            Err(_) => {
                plan.diagnostics.push(crate::Diagnostic::warning(
                    &board_path,
                    "W_EMIT_NO_SENTINEL",
                    format!(
                        "status.md for '{}' has no <!-- BEGIN generated:{} --> sentinels; skipping",
                        src.repo_slug,
                        markers::EPIC_BOARD
                    ),
                ));
            }
        }
    }

    plan
}

// ---------------------------------------------------------------------------
// plan_epic_sequences
// ---------------------------------------------------------------------------

/// Plan the per-epic cross-repo sequence table splice into each registered
/// epic's own `plan` document.
///
/// For every registry entry carrying a `plan` path, resolves it relative to
/// `root` and splices [`render_epic_sequence_table`] into its
/// [`markers::EPIC_SEQUENCE`] sentinel pair. An entry with no `plan`, a path
/// that does not resolve, or a document without the sentinels is skipped
/// (`W_EMIT_NO_SENTINEL` for the latter two) — never invents sentinels, and
/// never creates the document.
pub fn plan_epic_sequences(
    root: &std::path::Path,
    files: &[(StateSource, StateFile)],
    graph: &StateGraph,
    config: &BrainConfig,
) -> EmitPlan {
    let mut plan = EmitPlan::default();

    let Some((_, hq_file)) = files
        .iter()
        .find(|(_, f)| f.kind == "brain" && matches!(tier_scope_for(f, config), TierScope::All))
    else {
        return plan;
    };

    let global_status = global_status_map(files);
    let mut claimed: HashMap<&str, &str> = HashMap::new();

    for epic in &hq_file.epics {
        let Some(ref rel) = epic.plan else {
            continue; // an epic with no plan doc is fine — nothing to splice
        };
        let doc_path = root.join(rel);

        // Two epics sharing one plan doc would each produce a full-document
        // write carrying only their own table, and the later apply would drop
        // the earlier one's. Refuse the second claim rather than lose it.
        if let Some(first) = claimed.insert(rel.as_str(), epic.slug.as_str()) {
            plan.diagnostics.push(crate::Diagnostic::warning(
                &doc_path,
                "W_EMIT_EPIC_PLAN_CONFLICT",
                format!(
                    "epics '{first}' and '{}' both point at plan doc '{rel}'; only one \
                     epic-sequence table fits per document, so '{}' is skipped — give it \
                     its own plan doc",
                    epic.slug, epic.slug
                ),
            ));
            continue;
        }

        let original = match std::fs::read_to_string(&doc_path) {
            Ok(s) => s,
            Err(_) => {
                plan.diagnostics.push(crate::Diagnostic::warning(
                    &doc_path,
                    "W_EMIT_NO_SENTINEL",
                    format!(
                        "epic '{}' points at plan doc '{rel}', which does not exist; \
                         skipping sequence emit",
                        epic.slug
                    ),
                ));
                continue;
            }
        };

        let table = render_epic_sequence_table(
            &epic_members_resolved(root, graph, files, epic),
            &global_status,
        );

        match splice_generated(&original, markers::EPIC_SEQUENCE, &table) {
            Ok(new_content) => {
                if new_content != original {
                    plan.actions.push(EmitAction {
                        path: doc_path,
                        new_content,
                        note: format!("update sequence table for epic '{}'", epic.slug),
                    });
                }
            }
            Err(_) => {
                plan.diagnostics.push(crate::Diagnostic::warning(
                    &doc_path,
                    "W_EMIT_NO_SENTINEL",
                    format!(
                        "plan doc for epic '{}' has no <!-- BEGIN generated:{} --> sentinels; \
                         skipping",
                        epic.slug,
                        markers::EPIC_SEQUENCE
                    ),
                ));
            }
        }
    }

    plan
}

// ---------------------------------------------------------------------------
// plan_attention_board
// ---------------------------------------------------------------------------

/// Plan the Attention board splice into **every brain-level `status.md`,
/// tier-scoped** (`markers::ATTENTION`).
///
/// Unlike [`plan_unified_board`] (HQ root only), this emits for both scopes:
/// - **HQ root** ([`TierScope::All`]) — unions `carryover[]` from *every* loaded
///   repo/tier + the whole HQ `backlog[]`.
/// - **Tier sub-brain** ([`TierScope::Tier`]) — unions `carryover[]` from that
///   tier's leaf repos plus the tier brain file itself, and the HQ `backlog[]`
///   nodes whose `repo` belongs to that tier (so a capture made in a core
///   project surfaces on the core board).
///
/// So `/prime`, `/session-recap`, and `/attention` run inside a tier show that
/// tier's stale items, while HQ shows the whole corpus. Missing `status.md`, or
/// one lacking the sentinels, yields a `W_EMIT_NO_SENTINEL` warning and no write.
///
/// `graph` drives the block-side [`effective_priorities`] pass
/// (`MV.ticket.carryover-triage-ranking`, task 2/3) — its output is the
/// `block_priorities` map every board's carryover union is ranked against
/// via [`render_attention_section_with_distilled`]/[`rank_carryover`]; a
/// carryover's own priority never changes any block's effective priority
/// (that pass treats block targets as terminal).
pub fn plan_attention_board(
    files: &[(StateSource, StateFile)],
    graph: &StateGraph,
    config: &BrainConfig,
    today: chrono::NaiveDate,
) -> EmitPlan {
    let mut plan = EmitPlan::default();

    // The HQ backlog lives on the single All-scoped brain file.
    let hq_backlog: &[Backlog] = files
        .iter()
        .find(|(_, f)| f.kind == "brain" && matches!(tier_scope_for(f, config), TierScope::All))
        .map(|(_, f)| f.backlog.as_slice())
        .unwrap_or(&[]);

    // Shared across every tier/HQ board this function emits — `rank_carryover`
    // never recomputes either map itself.
    let block_priorities = effective_priorities(graph, files);
    let block_status = global_status_map(files);

    // Read each repo's sibling `knowledge.md` / `memory.md` at most once, cached by
    // `repo_slug` — the loop below is already O(files²) over the carryover union, and this
    // cache keeps the distilled-entry gather from repeating those reads per board (perf
    // note in the spec).
    let mut distilled_cache: HashMap<String, Vec<(&'static str, DistilledEntry)>> = HashMap::new();
    for (src, _) in files {
        if distilled_cache.contains_key(&src.repo_slug) {
            continue;
        }
        let Some(planning_dir) = src.abs_path.parent() else {
            continue;
        };
        let mut entries = Vec::new();
        for stem in ["knowledge", "memory"] {
            let path = planning_dir.join(format!("{stem}.md"));
            if let Ok(contents) = std::fs::read_to_string(&path) {
                entries.extend(parse_distilled(&contents).into_iter().map(|e| (stem, e)));
            }
        }
        distilled_cache.insert(src.repo_slug.clone(), entries);
    }

    for (src, file) in files {
        if file.kind != "brain" {
            continue;
        }
        let scope = tier_scope_for(file, config);

        // Scope the carryover union (repo-tagged) to this board. Deliberately
        // does NOT union `f2.reference` — `reference[]` (D72) is permanently-
        // true material with no clock and no triage lane by design; it must
        // never reach the Attention board. Pinned by
        // `tests/reference_container.rs::triage_surface_exclusion`
        // (`MV.ticket.reference-container-validation` task 3).
        let mut carryover: Vec<(String, &Carryover)> = Vec::new();
        for (s2, f2) in files {
            let include = match &scope {
                TierScope::All => true,
                TierScope::Tier(t) => {
                    s2.repo_slug == src.repo_slug // the tier brain file's own carryover
                        || tier_of_repo(&s2.repo_slug, config) == Some(t.as_str())
                }
            };
            if include {
                carryover.extend(f2.carryover.iter().map(|c| (s2.repo_slug.clone(), c)));
            }
        }

        // Scope the HQ backlog subset to this board.
        let backlog: Vec<(String, &Backlog)> = hq_backlog
            .iter()
            .filter(|b| match &scope {
                TierScope::All => true,
                TierScope::Tier(t) => tier_of_repo(&b.repo, config) == Some(t.as_str()),
            })
            .map(|b| (b.repo.clone(), b))
            .collect();

        // Scope the distilled-entry union (repo-tagged) to this board, using the IDENTICAL
        // `include` predicate the carryover union above uses — tier scoping is free.
        let mut distilled: Vec<(String, &str, &DistilledEntry)> = Vec::new();
        for (s2, _f2) in files {
            let include = match &scope {
                TierScope::All => true,
                TierScope::Tier(t) => {
                    s2.repo_slug == src.repo_slug
                        || tier_of_repo(&s2.repo_slug, config) == Some(t.as_str())
                }
            };
            if include && let Some(entries) = distilled_cache.get(&s2.repo_slug) {
                distilled.extend(
                    entries
                        .iter()
                        .map(|(stem, entry)| (s2.repo_slug.clone(), *stem, entry)),
                );
            }
        }

        let Some(planning_dir) = src.abs_path.parent() else {
            continue;
        };
        let board_path = planning_dir.join("status.md");

        let original = match std::fs::read_to_string(&board_path) {
            Ok(s) => s,
            Err(_) => {
                plan.diagnostics.push(crate::Diagnostic::warning(
                    &board_path,
                    "W_EMIT_NO_SENTINEL",
                    format!(
                        "no status.md beside '{}' state.json; skipping attention emit",
                        src.repo_slug
                    ),
                ));
                continue;
            }
        };

        let board = render_attention_section_with_distilled(
            &carryover,
            &backlog,
            &distilled,
            today,
            &config.attention,
            &block_priorities,
            &block_status,
        );

        match splice_generated(&original, markers::ATTENTION, &board) {
            Ok(new_content) => {
                if new_content != original {
                    plan.actions.push(EmitAction {
                        path: board_path,
                        new_content,
                        note: format!("update attention board for '{}'", src.repo_slug),
                    });
                }
            }
            Err(_) => {
                plan.diagnostics.push(crate::Diagnostic::warning(
                    &board_path,
                    "W_EMIT_NO_SENTINEL",
                    format!(
                        "status.md for '{}' has no <!-- BEGIN generated:attention --> \
                         sentinels; skipping",
                        src.repo_slug
                    ),
                ));
            }
        }
    }

    plan
}

// ---------------------------------------------------------------------------
// plan_status_frontmatter
// ---------------------------------------------------------------------------

/// Plan the YAML frontmatter status scalars splice (`now`, `next`, `blocked`).
///
/// For each loaded state file, derives its focus. Then resolves its `status.md`
/// path: checks `status_file` in the corresponding `brain.toml` `[[repos]]` entry
/// (resolved relative to `root`), or falls back to `status.md` in the same
/// directory as the `state.json`. If the file exists, applies `reconcile_status_scalars`.
///
/// If the output differs from the original, adds an [`EmitAction`] to write it.
pub fn plan_status_frontmatter(
    root: &std::path::Path,
    files: &[(StateSource, StateFile)],
    graph: &StateGraph,
    config: &BrainConfig,
) -> EmitPlan {
    let mut plan = EmitPlan::default();

    for (src, file) in files {
        let focus = if file.kind == "project" {
            derived_focus_for(src, file, graph, files)
        } else if file.kind == "brain" {
            let scope = tier_scope_for(file, config);
            derive_brain_focus(src, file, &scope, config, graph, files)
        } else {
            continue;
        };

        let status_path = if let Some(entry) = config.repos.iter().find(|r| r.slug == src.repo_slug)
        {
            if !entry.status_file.trim().is_empty() {
                root.join(entry.status_file.trim())
            } else {
                src.abs_path.parent().unwrap().join("status.md")
            }
        } else {
            src.abs_path.parent().unwrap().join("status.md")
        };

        if !status_path.exists() {
            continue;
        }

        let original = match std::fs::read_to_string(&status_path) {
            Ok(s) => s,
            Err(_) => {
                plan.diagnostics.push(crate::Diagnostic::warning(
                    &status_path,
                    "W_EMIT_IO_ERROR",
                    format!("could not read status file for '{}'", src.repo_slug),
                ));
                continue;
            }
        };

        let new_content = reconcile_status_scalars(&original, &focus);
        if new_content != original {
            plan.actions.push(EmitAction {
                path: status_path,
                new_content,
                note: format!("update status frontmatter for '{}'", src.repo_slug),
            });
        }
    }

    plan
}

// ---------------------------------------------------------------------------
// apply_plan
// ---------------------------------------------------------------------------

/// Execute a plan.
///
/// When `write` is `true`, each action is applied atomically and (for an
/// existing file) append-only-snapshotted first:
///
/// 1. **Snapshot.** If `action.path` already exists, its prior bytes are read
///    and — unless `[history].enabled = false` in the nearest `brain.toml`
///    (walked up from `action.path` via [`crate::brain::config::find_brain_root`])
///    — recorded as a new revision via [`crate::brain::history::record_revision`],
///    then pruned to `[history].keep` (default 10) via
///    [`crate::brain::history::prune`]. A brand-new file has no prior content
///    to lose, so no revision is recorded for it. The `[history]` resolution is
///    cached per brain root for the lifetime of one `apply_plan` call — a plan
///    can carry dozens of actions across a handful of roots and `brain.toml` is
///    not re-read per action. A snapshot or prune failure never aborts the
///    write: it emits a `W_HISTORY_FAILED` (Warning) diagnostic and the primary
///    write still proceeds — history is a safety net, never a new failure mode.
/// 2. **Atomic write.** The new content is written to a temp file in the
///    destination's own directory (never the system temp dir — a cross-device
///    `rename` fails) and then renamed over `action.path`, so a partially
///    written file is never observable at the destination path. The temp file
///    is cleaned up on any error path so a failed write leaves no litter. On
///    success this emits `I_EMIT_WROTE`; on a real IO failure on the primary
///    write it emits `E_EMIT_WRITE_FAILED` (Error).
///
/// When `write` is `false` (dry-run), nothing is read or written — no history
/// dir is created, no temp file, no prune — and a `W_EMIT_DRY_RUN` diagnostic
/// is emitted per planned action instead. Always passes through the plan's own
/// diagnostics.
///
/// `I_EMIT_WROTE` and `W_EMIT_DRY_RUN` use Warning severity (no info level
/// exists in [`crate::Diagnostic`]) so they surface in the reporter without
/// failing the exit code.  Only `E_EMIT_WRITE_FAILED` is Error-severity (a real
/// IO failure that should abort the run).
pub fn apply_plan(plan: &EmitPlan, write: bool) -> Vec<crate::Diagnostic> {
    let mut diags = plan.diagnostics.clone();

    if !write {
        for action in &plan.actions {
            diags.push(crate::Diagnostic::warning(
                &action.path,
                "W_EMIT_DRY_RUN",
                format!("would write (dry-run): {}", action.note),
            ));
        }
        return diags;
    }

    // Cache resolved [history] config per brain root so a plan spanning many
    // actions across a handful of roots doesn't re-read brain.toml per action.
    let mut history_config_cache: HashMap<PathBuf, crate::brain::config::HistoryConfig> =
        HashMap::new();

    for action in &plan.actions {
        // Step 1: snapshot the prior content of an existing file before it is
        // overwritten. A brand-new file has nothing prior to lose.
        if let Ok(prior) = std::fs::read(&action.path) {
            let history_cfg = resolve_history_config(&action.path, &mut history_config_cache);
            if history_cfg.enabled {
                match crate::brain::history::record_revision(&action.path, &prior) {
                    Ok(_) => {
                        if let Err(e) = crate::brain::history::prune(&action.path, history_cfg.keep)
                        {
                            diags.push(crate::Diagnostic::warning(
                                &action.path,
                                "W_HISTORY_FAILED",
                                format!(
                                    "failed to prune history for {}: {e}",
                                    action.path.display()
                                ),
                            ));
                        }
                    }
                    Err(e) => {
                        diags.push(crate::Diagnostic::warning(
                            &action.path,
                            "W_HISTORY_FAILED",
                            format!(
                                "failed to record history for {}: {e}",
                                action.path.display()
                            ),
                        ));
                    }
                }
            }
        }

        // Step 2: write atomically — temp file in the destination's own
        // directory, then rename over the destination.
        match write_atomic(&action.path, action.new_content.as_bytes()) {
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
    }

    diags
}

/// Resolve the `[history]` config for `path` by walking up from it looking for
/// `brain.toml`, caching the result per resolved brain root in `cache`.
///
/// No `brain.toml` found (or it fails to parse) resolves to
/// [`crate::brain::config::HistoryConfig::default`] — the same "absent table
/// means defaults, never an error" contract `brain.toml` itself carries for an
/// absent `[history]` section.
fn resolve_history_config(
    path: &Path,
    cache: &mut HashMap<PathBuf, crate::brain::config::HistoryConfig>,
) -> crate::brain::config::HistoryConfig {
    // Empty PathBuf is the cache key for "no brain root found from this path" —
    // never a real brain root, since find_brain_root always returns an
    // absolute, non-empty directory when it succeeds.
    let root = crate::brain::config::find_brain_root(path).unwrap_or_else(|_| PathBuf::new());

    if let Some(cfg) = cache.get(&root) {
        return cfg.clone();
    }

    let cfg = if root.as_os_str().is_empty() {
        crate::brain::config::HistoryConfig::default()
    } else {
        crate::brain::config::load_brain_config(&root.join("brain.toml"))
            .map(|c| c.history)
            .unwrap_or_default()
    };

    cache.insert(root, cfg.clone());
    cfg
}

/// Write `content` to `path` atomically: a temp file in `path`'s own directory
/// followed by `std::fs::rename` over `path`.
///
/// The temp file lives beside the destination (never the system temp dir) so
/// the final `rename` is same-filesystem and therefore atomic. On any error —
/// creating the temp file, writing it, or renaming it — the temp file is
/// removed (best-effort) so a failed write never leaves litter behind.
///
/// `pub` (not module-private) so `mev state-history --restore` (`src/main.rs`)
/// can reuse the exact same atomic-write helper `apply_plan` uses rather than
/// duplicating it — see `planning/ticket-append-only-emit-state-writer/tasks.md`
/// Task 4.
pub fn write_atomic(path: &Path, content: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("emit-output");
    let temp_path = dir.join(format!(".{file_name}.mev-tmp-{}", std::process::id()));

    // Create the parent subtree before the temp write. Without this, the FIRST
    // write into a corpus subtree that does not exist yet fails ENOENT and the
    // caller sees a write error instead of a created file — `std::fs::write`
    // does not create directories. engine-rs carried a downstream workaround
    // for exactly this (`doc_materializer.rs::ensure_plan_parents`, whose own
    // comment documents the gap), and mev's `doc_materialize` fixture only ever
    // passed because it hand-created the target directory; the moment okf-core
    // repointed LEARNING_CORPUS_INDEX to a subtree the fixture did not
    // pre-create, the write silently produced nothing and the idempotency
    // assertion failed. See carryover `write-atomic-does-not-create-missing-parents`.
    std::fs::create_dir_all(dir)?;

    let write_result = std::fs::write(&temp_path, content);
    if let Err(e) = write_result {
        let _ = std::fs::remove_file(&temp_path);
        return Err(e);
    }

    if let Err(e) = std::fs::rename(&temp_path, path) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(e);
    }

    Ok(())
}

#[cfg(test)]
mod apply_plan_history_tests {
    use super::*;

    fn fixture_dir(tag: &str) -> PathBuf {
        let dir = crate::testsupport::unique_temp_dir(&format!("mev-apply-plan-{tag}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn plan_for(path: &Path, new_content: &str) -> EmitPlan {
        EmitPlan {
            actions: vec![EmitAction {
                path: path.to_path_buf(),
                new_content: new_content.to_string(),
                note: "test".to_string(),
            }],
            diagnostics: vec![],
        }
    }

    #[test]
    fn overwriting_existing_file_records_exactly_one_revision_of_prior_content() {
        let dir = fixture_dir("overwrite-one-rev");
        let target = dir.join("state.json");
        std::fs::write(&target, b"prior content").unwrap();

        let plan = plan_for(&target, "new content");
        let diags = apply_plan(&plan, true);

        assert!(diags.iter().any(|d| d.locator == "I_EMIT_WROTE"));
        assert!(!diags.iter().any(|d| d.locator == "W_HISTORY_FAILED"));

        let revisions = crate::brain::history::list_revisions(&target).unwrap();
        assert_eq!(revisions.len(), 1, "expected exactly one recorded revision");
        assert_eq!(revisions[0].seq, 1);

        let snapshot = crate::brain::history::read_revision(&target, 1).unwrap();
        assert_eq!(
            snapshot, b"prior content",
            "revision must hold PRIOR content"
        );

        let after = std::fs::read_to_string(&target).unwrap();
        assert_eq!(after, "new content");
    }

    #[test]
    fn creating_new_file_records_zero_revisions() {
        let dir = fixture_dir("new-file-zero-rev");
        let target = dir.join("brand-new.json");
        assert!(!target.exists());

        let plan = plan_for(&target, "fresh content");
        let diags = apply_plan(&plan, true);

        assert!(diags.iter().any(|d| d.locator == "I_EMIT_WROTE"));
        let revisions = crate::brain::history::list_revisions(&target).unwrap();
        assert_eq!(
            revisions.len(),
            0,
            "a brand-new file has no prior content to snapshot"
        );
    }

    #[test]
    fn dry_run_creates_no_history_dir_and_no_temp_files() {
        let dir = fixture_dir("dry-run-side-effect-free");
        let target = dir.join("state.json");
        std::fs::write(&target, b"prior content").unwrap();

        let plan = plan_for(&target, "new content");
        let diags = apply_plan(&plan, false);

        assert!(diags.iter().any(|d| d.locator == "W_EMIT_DRY_RUN"));

        let history_dir = crate::brain::history::history_dir(&target);
        assert!(
            !history_dir.exists(),
            "dry-run must not create a history dir"
        );

        let entries: Vec<_> = std::fs::read_dir(&dir).unwrap().collect();
        assert_eq!(entries.len(), 1, "dry-run must leave no temp files behind");

        let after = std::fs::read(&target).unwrap();
        assert_eq!(after, b"prior content", "dry-run must not touch the file");
    }

    #[test]
    fn second_overwrite_yields_seq_two_while_seq_one_is_untouched() {
        let dir = fixture_dir("second-overwrite-seq-two");
        let target = dir.join("state.json");
        std::fs::write(&target, b"version one").unwrap();

        apply_plan(&plan_for(&target, "version two"), true);
        apply_plan(&plan_for(&target, "version three"), true);

        let revisions = crate::brain::history::list_revisions(&target).unwrap();
        assert_eq!(revisions.len(), 2);
        assert_eq!(revisions[0].seq, 1);
        assert_eq!(revisions[1].seq, 2);

        let rev1 = crate::brain::history::read_revision(&target, 1).unwrap();
        assert_eq!(rev1, b"version one");
        let rev2 = crate::brain::history::read_revision(&target, 2).unwrap();
        assert_eq!(rev2, b"version two");

        let current = std::fs::read_to_string(&target).unwrap();
        assert_eq!(current, "version three");
    }

    #[test]
    fn successful_write_leaves_no_leftover_temp_files() {
        let dir = fixture_dir("no-leftover-temp");
        let target = dir.join("state.json");
        std::fs::write(&target, b"prior content").unwrap();

        apply_plan(&plan_for(&target, "new content"), true);

        for entry in std::fs::read_dir(&dir).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().to_string_lossy().to_string();
            assert!(
                name == "state.json" || name == ".mev-history",
                "unexpected leftover entry: {name}"
            );
        }
    }

    #[test]
    fn history_disabled_writes_file_but_records_nothing() {
        let dir = fixture_dir("history-disabled");
        std::fs::write(dir.join("brain.toml"), "[history]\nenabled = false\n").unwrap();
        let target = dir.join("state.json");
        std::fs::write(&target, b"prior content").unwrap();

        let plan = plan_for(&target, "new content");
        let diags = apply_plan(&plan, true);

        assert!(diags.iter().any(|d| d.locator == "I_EMIT_WROTE"));
        let revisions = crate::brain::history::list_revisions(&target).unwrap();
        assert_eq!(
            revisions.len(),
            0,
            "history disabled must record no revisions"
        );

        let after = std::fs::read_to_string(&target).unwrap();
        assert_eq!(after, "new content", "the write itself must still proceed");
    }
}

#[cfg(test)]
mod epic_progress_superseded_tests {
    use super::*;

    fn block_with_status(id: &str, status: &str) -> TrackBlock {
        TrackBlock {
            id: id.to_string(),
            title: id.to_string(),
            status: Some(status.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn epic_progress_counts_superseded_in_its_own_field_not_open() {
        // Assertion 4. Before the "superseded" match arm existed, this fixture
        // fell through `_ => p.open += 1`, so `open` read 1 instead of 0 —
        // observed RED for exactly that reason. A superseded member must be
        // tallied in EpicProgress::superseded and NOT move the open count.
        let a = block_with_status("AL.1.A", "superseded");
        let b = block_with_status("AL.1.B", "closed");
        let members: Vec<(String, &TrackBlock)> =
            vec![("alpha".to_string(), &a), ("alpha".to_string(), &b)];

        let p = epic_progress(&members);

        assert_eq!(
            p.superseded, 1,
            "the superseded member must be counted in its own field"
        );
        assert_eq!(
            p.open, 0,
            "a superseded member must NOT be folded into open, got open={}",
            p.open
        );
        assert_eq!(p.closed, 1);
        assert_eq!(p.total(), 2);
    }

    #[test]
    fn render_epic_progress_line_reports_superseded_when_present() {
        let a = block_with_status("AL.1.A", "superseded");
        let members: Vec<(String, &TrackBlock)> = vec![("alpha".to_string(), &a)];
        let p = epic_progress(&members);

        let line = render_epic_progress_line(&p);
        assert!(
            line.contains("1 superseded"),
            "expected the progress line to name the superseded member, got: {line}"
        );
    }

    #[test]
    fn render_epic_progress_line_omits_superseded_clause_when_zero() {
        // Same never-fold rule as `deferred`/`wontfix`: epics with no
        // superseded members render byte-identical to before this field
        // existed.
        let a = block_with_status("AL.1.A", "closed");
        let members: Vec<(String, &TrackBlock)> = vec![("alpha".to_string(), &a)];
        let p = epic_progress(&members);

        let line = render_epic_progress_line(&p);
        assert!(
            !line.contains("superseded"),
            "expected no superseded clause when zero, got: {line}"
        );
    }
}
