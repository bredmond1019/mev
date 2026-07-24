//! Emit module — derived-view generator for `mev emit-state`.
//!
//! This module is the **single derivation engine** for every generated view the
//! v2 state schema declares.  The public surface for Task 2:
//!
//! - [`EmitError`] — error type for sentinel-related failures.
//! - [`wave_order`] — all block keys (`"repo:id"`) sorted by `wave` ascending.
//! - [`render_wave_table`] — Markdown table of a repo's blocks in wave order.
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

use thiserror::Error;

use crate::brain::config::BrainConfig;
use crate::brain::state::{
    Backlog, Block, BlockedBy, Carryover, CrossRepoEdge, Epic, EpicEdge, EpicEdges, Focus,
    RepoRollup, StateFile, StateGraph, StateSource, TierScope, TrackBlock, backlog_stale_age,
    carryover_stale_age, derive_brain_focus, derive_cross_repo, derive_epic_edges,
    derive_epic_focus, derive_focus, derive_rollup, effective_priorities, tier_scope_for,
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
/// This instead does a DFS-based topological sort over the full `depends_on`
/// graph (every block, every repo — not just this epic's members), then
/// filters the result down to `slug`'s members. Walking the *full* graph
/// (rather than only edges between two members) means a member gated by a
/// same-repo, out-of-epic prerequisite still sorts after it correctly.
///
/// DFS visitation order defaults to [`wave_order`] (global wave, then
/// iteration index) so that within a component with no dependency
/// constraint — most same-epic block pairs, which just aren't linked by an
/// edge — the table still reads in the same stable order it always has.
/// Only `{type:"block"}` deps that resolve to a real node participate;
/// `external` deps have no target node and cannot constrain ordering.
///
/// Cycle-safe: a node already on the current DFS stack short-circuits
/// instead of recursing again, mirroring the guard in
/// [`crate::brain::state::effective_priorities`] — this does not assume
/// `MV.3.P2`'s cycle check already rejected the corpus.
pub fn epic_members<'a>(
    graph: &StateGraph,
    files: &'a [(StateSource, StateFile)],
    slug: &str,
) -> Vec<(String, &'a TrackBlock)> {
    use std::collections::HashSet;

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

    // Forward deps: key -> [dep keys it depends_on], block-type only, and
    // only where the target actually resolves to a node in this corpus.
    let mut deps: HashMap<&str, Vec<String>> = HashMap::new();
    for (key, (_, block)) in &by_key {
        let ds = block
            .depends_on
            .iter()
            .filter_map(|dep| match dep {
                BlockedBy::Block { repo, id, .. } => {
                    let dep_key = format!("{repo}:{id}");
                    by_key.contains_key(&dep_key).then_some(dep_key)
                }
                BlockedBy::External { .. } => None,
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
        .into_iter()
        .filter_map(|key| by_key.get(&key).cloned())
        .filter(|(_, block)| block.epics.iter().any(|s| s == slug))
        .collect()
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
                BlockedBy::External { .. } => true,
                BlockedBy::Block { repo, id, .. } => {
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
                    BlockedBy::Block { repo, id, .. } => format!("{repo}:{id}"),
                    BlockedBy::External { what } => format!("external:{what}"),
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
pub fn render_hq_board(focus: &Focus, edges: &[CrossRepoEdge]) -> String {
    let sections = [
        render_hq_board_section("NOW", &focus.now, edges),
        render_hq_board_section("NEXT", &focus.next, edges),
        render_hq_board_section("BLOCKED", &focus.blocked, edges),
    ];
    sections.join("\n\n")
}

/// Render one `## {heading}` section of the Operating Board for `blocks`.
fn render_hq_board_section(heading: &str, blocks: &[Block], edges: &[CrossRepoEdge]) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("## {heading}"));

    if blocks.is_empty() {
        lines.push("_none_".to_string());
    } else {
        for block in blocks {
            lines.push(format!("- {}", render_hq_board_line(block, edges)));
        }
    }

    lines.join("\n")
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
fn render_hq_board_blocker(
    from_repo: &str,
    from_id: &str,
    dep: &BlockedBy,
    edges: &[CrossRepoEdge],
) -> String {
    match dep {
        BlockedBy::External { what } => format!("external:{what}"),
        BlockedBy::Block {
            repo: dep_repo,
            id: dep_id,
            what,
        } => {
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

/// Look up `block`'s effective priority (MV.7.A): the `effective_priorities`
/// map (keyed `"repo:id"`) wins when present — it reflects reverse-topo
/// `min`-propagation, so a block gating a hotter dependent floats up — and
/// falls back to the block's own raw `priority` (absent → `u8::MAX`, sorts
/// last) when the block has no entry in the map.
fn effective_priority_for(block: &Block, effective: &HashMap<String, u8>) -> u8 {
    let key = format!("{}:{}", block.repo.as_deref().unwrap_or(""), block.id);
    effective
        .get(&key)
        .copied()
        .or(block.priority)
        .unwrap_or(u8::MAX)
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
        let pa = effective_priority_for(a, effective);
        let pb = effective_priority_for(b, effective);
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
fn render_unified_board_section(
    level: &str,
    heading: &str,
    blocks: &[Block],
    edges: &[CrossRepoEdge],
    config: &BrainConfig,
) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("{level} {heading}"));

    if blocks.is_empty() {
        lines.push("_none_".to_string());
    } else {
        for block in blocks {
            lines.push(format!(
                "- {}",
                render_unified_board_line(block, edges, config)
            ));
        }
    }

    lines.join("\n")
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

/// The epic `status` values whose boards are rendered. A `complete` or `paused`
/// epic keeps its registry entry (and its blocks keep their membership) but
/// drops off the board, so a finished initiative stops competing for attention.
const RENDERED_EPIC_STATUSES: [&str; 1] = ["active"];

/// Counted progress for one epic: how many member blocks are in each state.
struct EpicProgress {
    closed: usize,
    in_progress: usize,
    open: usize,
}

impl EpicProgress {
    fn total(&self) -> usize {
        self.closed + self.in_progress + self.open
    }
}

/// Tally one epic's member blocks by authored status.
fn epic_progress(members: &[(String, &TrackBlock)]) -> EpicProgress {
    let mut p = EpicProgress {
        closed: 0,
        in_progress: 0,
        open: 0,
    };
    for (_, block) in members {
        match block.status.as_deref() {
            Some("closed") => p.closed += 1,
            Some("in_progress") => p.in_progress += 1,
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
    let rendered: Vec<&EpicBoardInput<'_>> = inputs
        .iter()
        .filter(|i| {
            // An absent status defaults to active — a registry entry with no
            // lifecycle set is still work someone declared.
            i.epic
                .status
                .as_deref()
                .map(|s| RENDERED_EPIC_STATUSES.contains(&s))
                .unwrap_or(true)
        })
        .collect();

    if rendered.is_empty() {
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

        let relationships = render_epic_relationships(&input.edges);
        if !relationships.is_empty() {
            lines.push(String::new());
            lines.push(relationships);
        }

        sections.push(lines.join("\n"));
    }

    sections.join("\n\n")
}

/// Render an epic's one-line progress summary, e.g.
/// `**7/23 closed** · 2 in progress · 14 open`.
fn render_epic_progress_line(p: &EpicProgress) -> String {
    if p.total() == 0 {
        return "**no member blocks yet**".to_string();
    }
    format!(
        "**{}/{} closed** · {} in progress · {} open",
        p.closed,
        p.total(),
        p.in_progress,
        p.open
    )
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
                BlockedBy::Block { repo, id, .. } => format!("{repo}:{id}"),
                BlockedBy::External { what } => format!("external:{what}"),
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
/// `external` entry, or a `block` entry whose target's authored status in
/// `global_status` is not `closed` (an unresolvable target counts as unmet).
fn has_unmet_dep(block: &TrackBlock, global_status: &HashMap<String, Option<String>>) -> bool {
    block.depends_on.iter().any(|dep| match dep {
        BlockedBy::External { .. } => true,
        BlockedBy::Block { repo, id, .. } => {
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

/// One row on the Attention board: its source repo tag, computed age (days),
/// and the rendered detail line.
struct AttentionRow {
    repo: String,
    age: i64,
    detail: String,
}

/// Truncate `text` to a single tidy line of at most `max` chars for a board row.
fn attention_snippet(text: &str, max: usize) -> String {
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() > max {
        let truncated: String = one_line.chars().take(max).collect();
        format!("{}…", truncated.trim_end())
    } else {
        one_line
    }
}

/// Render one `## {heading}` Attention sub-lane from `rows` (already filtered to
/// stale items). Rows are sorted oldest-first (largest age). Empty → `_none_`.
fn render_attention_lane(heading: &str, mut rows: Vec<AttentionRow>) -> String {
    rows.sort_by_key(|r| std::cmp::Reverse(r.age));
    let mut lines = vec![format!("## {heading}")];
    if rows.is_empty() {
        lines.push("_none_".to_string());
    } else {
        for row in rows {
            lines.push(format!("- [{}] {} — {}d", row.repo, row.detail, row.age));
        }
    }
    lines.join("\n")
}

/// Render the Attention board: three sub-lanes (Stale carryover / Aging backlog
/// / Orphaned captures) built from the pre-scoped, repo-tagged inputs. Only
/// items past their staleness threshold appear (the visible twin of the
/// `W_STATE_*_STALE` warnings — same predicate). The `[<repo>]` tag is a
/// separate axis from the unified board's `[BIZ]/[ENG]` tag.
pub fn render_attention_section(
    carryover: &[(String, &Carryover)],
    backlog: &[(String, &Backlog)],
    today: chrono::NaiveDate,
    thresholds: &crate::brain::config::AttentionThresholds,
) -> String {
    let mut carry_rows: Vec<AttentionRow> = Vec::new();
    for (repo, item) in carryover {
        if let Some(age) = carryover_stale_age(item, today, thresholds) {
            let clears = item
                .clears_when
                .as_deref()
                .map(|c| format!(" (clears when: {})", attention_snippet(c, 60)))
                .unwrap_or_default();
            carry_rows.push(AttentionRow {
                repo: repo.clone(),
                age,
                detail: format!(
                    "{} {} — {}{}",
                    item.kind,
                    item.slug,
                    attention_snippet(&item.text, 80),
                    clears
                ),
            });
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
                    age,
                    detail: format!("{} — {} — notes: {}", item.slug, item.title, notes),
                });
            } else {
                backlog_rows.push(AttentionRow {
                    repo: repo.clone(),
                    age,
                    detail: format!("{} ({}) — {}", item.slug, item.status, item.title),
                });
            }
        }
    }

    [
        render_attention_lane("Stale carryover", carry_rows),
        render_attention_lane("Aging backlog", backlog_rows),
        render_attention_lane("Orphaned captures", capture_rows),
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
    let d = derive_focus(src, file, graph, files);
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

    Focus { now, next, blocked }
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

    format!(
        "**Current focus:** {}. Next: {}. Blocked: {}.",
        summarize(&focus.now),
        summarize(&focus.next),
        summarize(&focus.blocked)
    )
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

/// Reconcile the `now`, `next`, and `blocked` scalars in `original`'s OKF frontmatter
/// to match the provided `focus`.
///
/// Locates the leading `---`-fenced frontmatter block. For each scalar:
/// - if the queue is empty, the value is the literal `[]`.
/// - otherwise, the value is a double-quoted string joining the blocks in that queue
///   (e.g., `"repo:id — title"`), with `\` and `"` escaped.
///
/// If a line for the key exists, it is replaced; if not, it is appended before the
/// closing fence.
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
        ("now:", format_queue(&focus.now)),
        ("next:", format_queue(&focus.next)),
        ("blocked:", format_queue(&focus.blocked)),
    ];

    for (key, val) in updates {
        let new_line = format!("{key} {val}");
        match fm_lines
            .iter()
            .position(|l| l.trim_start().starts_with(key))
        {
            Some(pos) => fm_lines[pos] = new_line,
            None => fm_lines.push(new_line),
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
            members: epic_members(graph, files, &epic.slug),
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

        let table =
            render_epic_sequence_table(&epic_members(graph, files, &epic.slug), &global_status);

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
pub fn plan_attention_board(
    files: &[(StateSource, StateFile)],
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

    for (src, file) in files {
        if file.kind != "brain" {
            continue;
        }
        let scope = tier_scope_for(file, config);

        // Scope the carryover union (repo-tagged) to this board.
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

        let board = render_attention_section(&carryover, &backlog, today, &config.attention);

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
/// When `write` is `true`, writes each action's `new_content` to its `path` and
/// emits a `I_EMIT_WROTE` (Warning severity) diagnostic per file.  When `false`
/// (dry-run), writes nothing and emits a `W_EMIT_DRY_RUN` diagnostic per planned
/// action.  Always passes through the plan's own diagnostics.
///
/// `I_EMIT_WROTE` and `W_EMIT_DRY_RUN` use Warning severity (no info level
/// exists in [`crate::Diagnostic`]) so they surface in the reporter without
/// failing the exit code.  Only `E_EMIT_WRITE_FAILED` is Error-severity (a real
/// IO failure that should abort the run).
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
