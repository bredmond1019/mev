//! Frontier computation over the untruncated in-process block graph — `MV.13.B` Task 1.
//!
//! Closure over the block graph MUST run in mev itself, over the untruncated corpus:
//! `block_graph.rs` builds with `usize::MAX` in-process, but the HTTP export defaults to
//! `max_nodes=400` against 756 blocks, so any client-side frontier would silently drop
//! gates. [`ensure_untruncated`] is the refusal that guarantees a frontier is never
//! computed over a truncated node set.
//!
//! [`compute_frontier`] walks every lane's derived block positions
//! ([`crate::brain::lane_segments::derive_lane_positions`]) and picks, per
//! `(roadmap, lane, segment)`, the segment head: the first block in file order whose
//! authored status is not `closed`. A segment whose blocks are all closed contributes
//! no entry — the lane has nothing left to do there. `unmet_blocks`/`unmet_gates`
//! record exactly why a head is not yet startable, so downstream consumers (bastion's
//! `/lanes` endpoint, the cockpit board) never re-derive the closure themselves.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use crate::Diagnostic;
use crate::brain::block_graph::BlockGraphExport;
use crate::brain::carryover::RepoGatingReport;
use crate::brain::emit::{EmitAction, EmitPlan, effective_priority_for, global_status_map};
use crate::brain::lane_segments::DerivedBlockPosition;
use crate::brain::state::{
    ApprovalDep, BlockDep, BlockedBy, ExternalDep, OperatorDep, StateFile, StateGraph, StateSource,
    TrackBlock,
};

/// One lane's current head: the first not-yet-closed block in its active segment,
/// plus exactly what is blocking it (if anything) from being startable right now.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct FrontierEntry {
    pub roadmap: String,
    pub lane: String,
    pub segment: usize,
    pub repo: String,
    /// Canonical `"repo:id"` key, matching [`crate::brain::state::StateNode::key`]'s
    /// convention.
    pub key: String,
    pub id: String,
    pub title: String,
    /// Authored status, defaulted to `"open"` when absent — matches the
    /// `authored_status`/`unwrap_or("open")` convention used throughout
    /// [`crate::brain::emit`].
    pub status: String,
    /// Every unmet `BlockedBy::Block` dependency, rendered `"repo:id"`.
    pub unmet_blocks: Vec<String>,
    /// Every unmet `BlockedBy::Operator`/`Approval`/`External` dependency, rendered
    /// `"OP.<slug>"` (D76, both operator and approval edges — see
    /// [`okf_core::op_id`]) / `"external:<what>"`, plus (`MV.16.C` task 4) a
    /// `"carryover:{repo}:{slug}"` entry when this head is itself held by a
    /// `carryover[].blocks[]` enforcement gate.
    pub unmet_gates: Vec<String>,
    /// `true` iff both `unmet_blocks` and `unmet_gates` are empty.
    pub startable: bool,
}

/// The corpus-wide frontier: one [`FrontierEntry`] per active `(roadmap, lane,
/// segment)`, plus the `gate_rank` derivation `MV.13.B` Task 2 folds in here.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
pub struct Frontier {
    pub entries: Vec<FrontierEntry>,
    pub gate_ranks: Vec<GateRank>,
}

/// One operator/approval gate's rank, derived over every block it gates —
/// `MV.13.B` Task 2.
///
/// Operator and approval gates are targetless: they gate a block but are not
/// themselves graph nodes with dependents, so
/// [`crate::brain::state::effective_priorities`] never assigns them a
/// priority directly. This is the derived substitute — the minimum effective
/// priority across every block the gate blocks, via
/// [`crate::brain::emit::effective_priority_for`] (the same lookup `emit`'s
/// `BlockedGroup::effective_priority` uses, widened to take `repo`/`id`/
/// `priority` directly so it works for `TrackBlock` too — no second min
/// implementation).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct GateRank {
    /// `"operator"` or `"approval"`.
    pub kind: &'static str,
    pub slug: String,
    /// The minimum effective priority across every block in `gates`. Absent
    /// priority throughout sorts last (`u8::MAX`), matching
    /// [`crate::brain::emit::effective_priority_for`]'s own fallback.
    pub rank: u8,
    /// Every block this gate blocks, rendered `"repo:id"`, in the order
    /// first encountered.
    pub gates: Vec<String>,
}

/// Derive [`GateRank`]s for every operator/approval gate named in `files`'
/// `depends_on` edges.
///
/// Groups every `TrackBlock` carrying an `Operator`/`Approval` dependency by
/// `(kind, slug)` — a shared slug is a join key: several otherwise-unrelated
/// blocks across any number of repos can name the same operator session or
/// approval decision, mirroring [`crate::brain::emit::group_blocked_by_gate`]'s
/// shared-identity dedup. Each group's `rank` is the minimum
/// [`crate::brain::emit::effective_priority_for`] across its `gates`.
///
/// Output is sorted `(rank asc, slug asc)` so it is deterministic regardless
/// of corpus file-walk order.
pub fn gate_ranks(
    files: &[(StateSource, StateFile)],
    effective: &HashMap<String, u8>,
) -> Vec<GateRank> {
    // (kind, slug) -> index into `groups`, preserving first-seen order.
    let mut order: Vec<(&'static str, String)> = Vec::new();
    let mut groups: HashMap<(&'static str, String), (u8, Vec<String>)> = HashMap::new();

    for (src, file) in files {
        for track in &file.tracks {
            for block in &track.blocks {
                let block_key = format!("{}:{}", src.repo_slug, block.id);
                let block_rank =
                    effective_priority_for(&src.repo_slug, &block.id, block.priority, effective);
                for dep in &block.depends_on {
                    let key = match dep {
                        BlockedBy::Operator(OperatorDep { slug, .. }) => ("operator", slug.clone()),
                        BlockedBy::Approval(ApprovalDep { slug, .. }) => ("approval", slug.clone()),
                        BlockedBy::Block(_) | BlockedBy::External(_) => continue,
                    };
                    if !groups.contains_key(&key) {
                        order.push(key.clone());
                    }
                    let entry = groups.entry(key).or_insert_with(|| (u8::MAX, Vec::new()));
                    entry.0 = entry.0.min(block_rank);
                    if !entry.1.contains(&block_key) {
                        entry.1.push(block_key.clone());
                    }
                }
            }
        }
    }

    let mut ranks: Vec<GateRank> = order
        .into_iter()
        .map(|(kind, slug)| {
            let (rank, gates) = groups
                .remove(&(kind, slug.clone()))
                .unwrap_or((u8::MAX, Vec::new()));
            GateRank {
                kind,
                slug,
                rank,
                gates,
            }
        })
        .collect();
    ranks.sort_by(|a, b| a.rank.cmp(&b.rank).then_with(|| a.slug.cmp(&b.slug)));
    ranks
}

/// Look up a `TrackBlock` by its canonical `"repo:id"` key across every loaded state
/// file — the sibling lookup to [`global_status_map`], but keeping the whole block
/// (for `title` and `depends_on`) rather than only `status`.
fn track_block_index(files: &[(StateSource, StateFile)]) -> HashMap<String, &TrackBlock> {
    let mut index = HashMap::new();
    for (src, file) in files {
        for track in &file.tracks {
            for block in &track.blocks {
                let key = format!("{}:{}", src.repo_slug, block.id);
                index.insert(key, block);
            }
        }
    }
    index
}

/// Compute the corpus-wide frontier from `lane_positions` (already segmented by
/// [`crate::brain::lane_segments::derive_lane_positions`]), the untruncated in-process
/// `graph` (titles), and `files` (authored status + `depends_on`).
///
/// `graph` MUST be built with `max_nodes: usize::MAX` — this function trusts its
/// caller already refused a truncated node set via [`ensure_untruncated`]; it does not
/// re-check truncation itself since [`StateGraph`] carries no such flag (that lives on
/// [`BlockGraphExport`], the HTTP-side shape).
/// `effective` is the [`crate::brain::state::effective_priorities`] map
/// (keyed `"repo:id"`) — threaded through so [`gate_ranks`] can derive a rank
/// for the targetless operator/approval gates that map itself never reaches.
///
/// `gating` (`MV.16.C` task 4) is the same per-repo carryover-enforcement gating
/// set [`crate::brain::state::derive_focus`]/[`crate::brain::state::ready_order`]
/// consume — built once by
/// [`crate::brain::carryover::build_carryover_gating_sets`] and passed in here
/// rather than recomputed, so the frontier can never disagree with the
/// block-level derivation about which blocks are held. `None` (today's every
/// production call site, matching [`crate::brain::block_graph`]'s and
/// [`crate::brain::emit`]'s own `derive_focus`/`ready_order` calls) means "no
/// carryover gate applies here" — exactly today's behaviour. A segment head
/// directly named by a gate (looked up by the head's own `"repo:id"` key,
/// under its own repo) gets an `unmet_gates` entry of `"carryover:{owner}"`
/// naming the owning carryover slug, and is therefore never `startable`.
pub fn compute_frontier(
    lane_positions: &[DerivedBlockPosition],
    graph: &StateGraph,
    files: &[(StateSource, StateFile)],
    effective: &HashMap<String, u8>,
    gating: Option<&BTreeMap<String, RepoGatingReport>>,
) -> Frontier {
    let node_titles: HashMap<&str, &str> = graph
        .nodes
        .iter()
        .map(|n| (n.key.as_str(), n.title.as_str()))
        .collect();
    let block_index = track_block_index(files);
    let global_status = global_status_map(files);

    // Group lane_positions by (roadmap, lane, segment), preserving first-seen order —
    // derive_lane_positions already yields file order, and every position within one
    // segment shares that file's contiguous run, so grouping is a stable partition.
    let mut order: Vec<(String, String, usize)> = Vec::new();
    let mut groups: HashMap<(String, String, usize), Vec<&DerivedBlockPosition>> = HashMap::new();
    for p in lane_positions {
        let key = (p.roadmap.clone(), p.lane.clone(), p.segment);
        groups.entry(key.clone()).or_insert_with(|| {
            order.push(key.clone());
            Vec::new()
        });
        groups.get_mut(&key).expect("just inserted").push(p);
    }

    let mut entries = Vec::new();
    for key in order {
        let mut members = groups.remove(&key).unwrap_or_default();
        members.sort_by_key(|p| p.position);

        let head = members.into_iter().find(|p| {
            let block_key = format!("{}:{}", p.repo, p.id);
            let status = global_status
                .get(&block_key)
                .cloned()
                .flatten()
                .unwrap_or_else(|| "open".to_string());
            status != "closed"
        });

        let Some(head) = head else {
            continue; // every block in this segment is closed — nothing to surface
        };

        let block_key = format!("{}:{}", head.repo, head.id);
        let status = global_status
            .get(&block_key)
            .cloned()
            .flatten()
            .unwrap_or_else(|| "open".to_string());
        let title = block_index
            .get(&block_key)
            .map(|b| b.title.clone())
            .or_else(|| node_titles.get(block_key.as_str()).map(|t| t.to_string()))
            .unwrap_or_default();

        let mut unmet_blocks = Vec::new();
        let mut unmet_gates = Vec::new();
        if let Some(track_block) = block_index.get(&block_key) {
            for dep in &track_block.depends_on {
                match dep {
                    BlockedBy::Block(BlockDep { repo, id, .. }) => {
                        let target_key = format!("{repo}:{id}");
                        let target_status = global_status
                            .get(&target_key)
                            .cloned()
                            .flatten()
                            .unwrap_or_else(|| "open".to_string());
                        if target_status != "closed" {
                            unmet_blocks.push(target_key);
                        }
                    }
                    BlockedBy::Operator(OperatorDep { slug, .. }) => {
                        unmet_gates.push(okf_core::op_id(slug));
                    }
                    BlockedBy::Approval(ApprovalDep { slug, .. }) => {
                        unmet_gates.push(okf_core::op_id(slug));
                    }
                    BlockedBy::External(ExternalDep { what }) => {
                        unmet_gates.push(format!("external:{what}"));
                    }
                }
            }
        }

        // Carryover-enforcement gate (`MV.16.C` task 4) — a segment head can be
        // held directly by a `carryover[].blocks[]` edge even when every
        // authored `depends_on` entry is met, exactly as `derive_focus` treats
        // it. Looked up by the head's own repo, matching
        // `build_carryover_gating_sets`' grouping by target repo.
        if let Some(gate) = gating
            .and_then(|sets| sets.get(&head.repo))
            .and_then(|report| report.gates.get(&block_key))
        {
            unmet_gates.push(format!("carryover:{}", gate.owner));
        }

        let startable = unmet_blocks.is_empty() && unmet_gates.is_empty();

        entries.push(FrontierEntry {
            roadmap: head.roadmap.clone(),
            lane: head.lane.clone(),
            segment: head.segment,
            repo: head.repo.clone(),
            key: block_key,
            id: head.id.clone(),
            title,
            status,
            unmet_blocks,
            unmet_gates,
            startable,
        });
    }

    Frontier {
        entries,
        gate_ranks: gate_ranks(files, effective),
    }
}

/// Refuse to compute a frontier over a truncated node set.
///
/// The HTTP block-graph export defaults to `max_nodes=400` against 756 blocks; any
/// HTTP-side closure that ignores `truncated: true` and proceeds anyway silently drops
/// gates from the frontier. This is the hard-fail AC2 asks for — mev never computes a
/// frontier over a truncated export and never degrades to a partial one.
pub fn ensure_untruncated(export: &BlockGraphExport) -> Result<(), Diagnostic> {
    if export.truncated {
        return Err(Diagnostic::error(
            export.root.clone(),
            "block_graph.truncated",
            format!(
                "E_FRONTIER_TRUNCATED_GRAPH: block graph export truncated ({} of {} nodes) — \
                 refusing to compute a frontier over a partial node set; re-request with a \
                 larger max_nodes (mev's own in-process closure always uses usize::MAX)",
                export.nodes.len(),
                export.total_nodes
            ),
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// `mev frontier` CLI text renderer — MV.13.B Task 4
// ---------------------------------------------------------------------------

/// Render a [`Frontier`]'s entries as one line each: the read-only text shape for
/// `mev frontier` (without `--json`).
///
/// Format: `{roadmap}/{lane}#{segment} {repo}:{id} — startable` for a startable
/// entry, or `{roadmap}/{lane}#{segment} {repo}:{id} — blocked by <reasons>` where
/// `<reasons>` is every `unmet_blocks` then `unmet_gates` entry, comma-joined, for a
/// blocked one. `gate_ranks` are not rendered here — `--json` is the surface for
/// those. Returns an empty string (no trailing newline) when `entries` is empty.
pub fn render_frontier_text(frontier: &Frontier) -> String {
    frontier
        .entries
        .iter()
        .map(|e| {
            let reasons: Vec<&str> = e
                .unmet_blocks
                .iter()
                .chain(e.unmet_gates.iter())
                .map(String::as_str)
                .collect();
            let status = if e.startable {
                "startable".to_string()
            } else {
                format!("blocked by {}", reasons.join(", "))
            };
            format!(
                "{}/{}#{} {}:{} — {status}",
                e.roadmap, e.lane, e.segment, e.repo, e.id
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// Wired into `emit-state` — MV.13.B Task 3
// ---------------------------------------------------------------------------

/// Relative path (from the brain root) of the derived lane-frontier artifact
/// [`plan_frontier`] writes. Like [`crate::brain::lane_segments::LANE_SEGMENTS_ARTIFACT`],
/// this is a cross-repo, corpus-wide derivation, not one repo's scoped surface — it is
/// written unconditionally, never narrowed by `emit_state`'s `--scope <repo>`.
pub const LANE_FRONTIER_ARTIFACT: &str = "planning/lane-frontier.json";

/// The full [`Frontier`] derivation, serialized as-is — `mev emit-state`'s JSON
/// artifact at [`LANE_FRONTIER_ARTIFACT`], plus `derived_at`. `pub` so `mev frontier
/// --json` (Task 4) can emit this exact shape to stdout without a second definition.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FrontierArtifact {
    /// RFC 3339 timestamp of the derivation run (`chrono::Local::now()`).
    ///
    /// `state.json` only changes at `/log-work` time, but lane progress (a block
    /// closing, a new open block landing) lands live between those commits — a
    /// consumer (bastion's `BA.19.C` `/lanes` endpoint) needs to be able to tell how
    /// stale this artifact is relative to the live corpus, since the artifact itself
    /// is only as fresh as the last `mev emit-state --write` run that produced it.
    pub derived_at: String,
    pub entries: Vec<FrontierEntry>,
    pub gate_ranks: Vec<GateRank>,
}

/// Wrap `frontier` with a fresh `derived_at` timestamp — the same construction
/// [`assemble_frontier_plan`] uses for the `emit-state` artifact, exposed for
/// `mev frontier --json` (Task 4), which prints this shape to stdout instead of
/// writing it to [`LANE_FRONTIER_ARTIFACT`].
pub fn build_frontier_artifact(frontier: Frontier) -> FrontierArtifact {
    FrontierArtifact {
        derived_at: chrono::Local::now().to_rfc3339(),
        entries: frontier.entries,
        gate_ranks: frontier.gate_ranks,
    }
}

/// Plan the [`LANE_FRONTIER_ARTIFACT`] write: derive lane positions the same way
/// [`crate::brain::lane_segments::plan_lane_segments`] does, build the untruncated
/// (`max_nodes: usize::MAX`) in-process block graph, refuse via [`ensure_untruncated`]
/// if it somehow reports `truncated: true`, then run [`compute_frontier`] and
/// [`gate_ranks`] over the result — modelled on `plan_lane_segments`, one [`EmitPlan`]
/// for `emit_state` to apply alongside its other planners.
///
/// No `EmitAction` is planned when zero lane files are discovered (an empty corpus, or
/// `root` has no `planning/` at all), nor when [`ensure_untruncated`] refuses the
/// export — a diagnostic is carried on the plan in the latter case, but never a
/// partial write.
pub fn plan_frontier(root: &Path, loaded: &[(StateSource, StateFile)]) -> EmitPlan {
    use crate::brain::block_graph::{BlockGraphScope, build_block_graph_export};
    use crate::brain::config::load_brain_config;
    use crate::brain::lane_segments::{
        build_owner_index, derive_lane_positions, discover_lane_files, unresolved_owner_diagnostics,
    };
    use crate::brain::state::{TierScope, build_state_graph};

    let mut plan = EmitPlan::default();

    let (lane_files, discover_diags) = discover_lane_files(root);
    plan.diagnostics.extend(discover_diags);

    if lane_files.is_empty() {
        return plan;
    }

    let owner_index = build_owner_index(loaded);
    for lf in &lane_files {
        plan.diagnostics
            .extend(unresolved_owner_diagnostics(lf, &owner_index));
    }

    let (lane_positions, derive_diags) = derive_lane_positions(&lane_files, loaded);
    plan.diagnostics.extend(derive_diags);

    // `config` only feeds the block-graph export's TIER stage, which is a no-op here
    // (`TierScope::All`, no `--repo`/`--epic` filter) — a missing/unreadable
    // `brain.toml` falls back to `BrainConfig::default()` rather than aborting the
    // frontier plan; `emit_state`'s own top-level `find_brain_config` call already
    // gates the whole run on a real config existing.
    let config = load_brain_config(&root.join("brain.toml")).unwrap_or_default();

    let graph = build_state_graph(loaded);
    let scope = BlockGraphScope {
        tier: TierScope::All,
        epic: None,
        repo: None,
        include_closed: true,
        include_boundary: false,
        max_nodes: usize::MAX,
    };
    let export = build_block_graph_export(root, &config, &graph, loaded, &scope);
    let lane_file_count = lane_files.len();

    assemble_frontier_plan(
        root,
        loaded,
        &lane_positions,
        &graph,
        &export,
        lane_file_count,
        plan,
    )
}

/// The part of [`plan_frontier`] that runs once the untruncated export is in hand:
/// refuse via [`ensure_untruncated`], else derive the frontier and append the
/// [`LANE_FRONTIER_ARTIFACT`] write to `plan`. Split out from [`plan_frontier`] so the
/// truncation-refusal branch is unit-testable against a fabricated `export` — the real
/// call site always builds `export` with `max_nodes: usize::MAX`, under which
/// `truncated` can never actually be `true`, so exercising that branch end-to-end
/// through `plan_frontier` itself is not possible.
fn assemble_frontier_plan(
    root: &Path,
    loaded: &[(StateSource, StateFile)],
    lane_positions: &[DerivedBlockPosition],
    graph: &StateGraph,
    export: &BlockGraphExport,
    lane_file_count: usize,
    mut plan: EmitPlan,
) -> EmitPlan {
    use crate::brain::state::effective_priorities;

    if let Err(diag) = ensure_untruncated(export) {
        plan.diagnostics.push(diag);
        return plan;
    }

    let effective = effective_priorities(graph, loaded);
    let frontier = compute_frontier(lane_positions, graph, loaded, &effective, None);

    let entry_count = frontier.entries.len();
    let gate_count = frontier.gate_ranks.len();
    let artifact = build_frontier_artifact(frontier);
    let new_content = match serde_json::to_string_pretty(&artifact) {
        Ok(mut s) => {
            s.push('\n');
            s
        }
        Err(e) => {
            plan.diagnostics.push(Diagnostic::error(
                root,
                "E_EMIT_FRONTIER_SERIALIZE",
                format!("failed to serialize lane-frontier artifact: {e}"),
            ));
            return plan;
        }
    };

    plan.actions.push(EmitAction {
        path: root.join(LANE_FRONTIER_ARTIFACT),
        new_content,
        note: format!(
            "derived lane frontier: {entry_count} entr{} across {lane_file_count} lane file(s), {gate_count} gate rank(s)",
            if entry_count == 1 { "y" } else { "ies" },
        ),
    });

    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::block_graph::BlockGraphScopeEcho;
    use crate::brain::state::{Focus, StateFile, StateSource, Track, TrackBlock};
    use std::path::PathBuf;

    fn pos(
        roadmap: &str,
        lane: &str,
        segment: usize,
        position: usize,
        repo: &str,
        id: &str,
    ) -> DerivedBlockPosition {
        DerivedBlockPosition {
            roadmap: roadmap.to_string(),
            lane: lane.to_string(),
            repo: repo.to_string(),
            id: id.to_string(),
            line: position + 1,
            segment,
            position,
            origin_roadmap: None,
            directives: None,
        }
    }

    fn track_block(id: &str, status: Option<&str>, depends_on: Vec<BlockedBy>) -> TrackBlock {
        TrackBlock {
            id: id.to_string(),
            title: format!("Title for {id}"),
            status: status.map(str::to_string),
            depends_on,
            ..Default::default()
        }
    }

    fn state_file(repo: &str, blocks: Vec<TrackBlock>) -> (StateSource, StateFile) {
        let src = StateSource {
            repo_slug: repo.to_string(),
            abs_path: PathBuf::from(format!("{repo}/planning/state.json")),
            expected_kind: "project",
        };
        let file = StateFile {
            repo: repo.to_string(),
            kind: "project".to_string(),
            updated: "2026-08-17".to_string(),
            focus: Focus::default(),
            tracks: vec![Track {
                title: "Phase 1".to_string(),
                blocks,
                extra: Default::default(),
            }],
            repos: Vec::new(),
            ..Default::default()
        };
        (src, file)
    }

    fn empty_graph() -> StateGraph {
        StateGraph {
            nodes: Vec::new(),
            edges: Vec::new(),
        }
    }

    fn export(total: u32, truncated: bool) -> BlockGraphExport {
        BlockGraphExport {
            version: "1".to_string(),
            root: "planning".to_string(),
            scope: BlockGraphScopeEcho {
                tier: None,
                epic: None,
                repo: None,
                include_closed: false,
                include_boundary: false,
            },
            nodes: Vec::new(),
            edges: Vec::new(),
            cycles: Vec::new(),
            total_nodes: total,
            truncated,
        }
    }

    #[test]
    fn segment_head_skips_closed_blocks() {
        let lane_positions = vec![
            pos("r1", "lane-a", 0, 0, "repo", "A.1"),
            pos("r1", "lane-a", 0, 1, "repo", "A.2"),
        ];
        let files = vec![state_file(
            "repo",
            vec![
                track_block("A.1", Some("closed"), vec![]),
                track_block("A.2", Some("open"), vec![]),
            ],
        )];
        let frontier = compute_frontier(
            &lane_positions,
            &empty_graph(),
            &files,
            &HashMap::new(),
            None,
        );
        assert_eq!(frontier.entries.len(), 1);
        assert_eq!(frontier.entries[0].id, "A.2");
        assert!(frontier.entries[0].startable);
    }

    #[test]
    fn all_closed_segment_yields_no_entry() {
        let lane_positions = vec![pos("r1", "lane-a", 0, 0, "repo", "A.1")];
        let files = vec![state_file(
            "repo",
            vec![track_block("A.1", Some("closed"), vec![])],
        )];
        let frontier = compute_frontier(
            &lane_positions,
            &empty_graph(),
            &files,
            &HashMap::new(),
            None,
        );
        assert!(frontier.entries.is_empty());
    }

    /// `MV.16.C` task 4: builds a one-gate `RepoGatingReport` map holding
    /// `target_key`, exactly the shape
    /// [`crate::brain::carryover::build_carryover_gating_sets`] returns.
    fn one_gate_map(target_key: &str, owner: &str) -> BTreeMap<String, RepoGatingReport> {
        let repo = target_key
            .split_once(':')
            .map(|(repo, _)| repo)
            .unwrap_or(target_key)
            .to_string();
        let mut gates = BTreeMap::new();
        gates.insert(
            target_key.to_string(),
            crate::brain::carryover::CarryoverGate {
                target_key: target_key.to_string(),
                owner: owner.to_string(),
            },
        );
        let mut sets = BTreeMap::new();
        sets.insert(
            repo,
            RepoGatingReport {
                gates,
                candidate_count: 1,
                applied_count: 1,
                cap: 10,
                cap_exceeded: false,
            },
        );
        sets
    }

    #[test]
    fn carryover_gate_holds_an_otherwise_startable_head_and_names_owner() {
        let lane_positions = vec![pos("r1", "lane-a", 0, 0, "repo", "A.1")];
        let files = vec![state_file(
            "repo",
            vec![track_block("A.1", Some("open"), vec![])],
        )];
        let gating = one_gate_map("repo:A.1", "repo:finding-1");

        let frontier = compute_frontier(
            &lane_positions,
            &empty_graph(),
            &files,
            &HashMap::new(),
            Some(&gating),
        );

        assert_eq!(frontier.entries.len(), 1);
        let entry = &frontier.entries[0];
        assert!(
            !entry.startable,
            "a carryover-gated head must not be startable: {entry:?}"
        );
        assert_eq!(entry.unmet_gates, vec!["carryover:repo:finding-1"]);
        assert!(
            entry.unmet_blocks.is_empty(),
            "the gate is not a depends_on entry, so unmet_blocks stays empty: {entry:?}"
        );
    }

    #[test]
    fn carryover_gate_absent_is_a_byte_identical_no_op() {
        let lane_positions = vec![pos("r1", "lane-a", 0, 0, "repo", "A.1")];
        let files = vec![state_file(
            "repo",
            vec![track_block("A.1", Some("open"), vec![])],
        )];

        let unenforced = compute_frontier(
            &lane_positions,
            &empty_graph(),
            &files,
            &HashMap::new(),
            None,
        );
        let empty_map: BTreeMap<String, RepoGatingReport> = BTreeMap::new();
        let enforced_but_empty = compute_frontier(
            &lane_positions,
            &empty_graph(),
            &files,
            &HashMap::new(),
            Some(&empty_map),
        );

        assert_eq!(
            unenforced, enforced_but_empty,
            "gating: None and gating: Some(empty map) must derive byte-identical frontiers"
        );
        assert!(unenforced.entries[0].startable);
    }

    #[test]
    fn unmet_cross_repo_block_dep_clears_startable() {
        let lane_positions = vec![pos("r1", "lane-a", 0, 0, "repoA", "A.1")];
        let dep = BlockedBy::Block(BlockDep {
            repo: "repoB".to_string(),
            id: "B.1".to_string(),
            what: None,
        });
        let files = vec![
            state_file("repoA", vec![track_block("A.1", Some("open"), vec![dep])]),
            state_file("repoB", vec![track_block("B.1", Some("open"), vec![])]),
        ];
        let frontier = compute_frontier(
            &lane_positions,
            &empty_graph(),
            &files,
            &HashMap::new(),
            None,
        );
        assert_eq!(frontier.entries.len(), 1);
        let entry = &frontier.entries[0];
        assert!(!entry.startable);
        assert_eq!(entry.unmet_blocks, vec!["repoB:B.1".to_string()]);
        assert!(entry.unmet_gates.is_empty());
    }

    #[test]
    fn operator_gate_clears_startable_and_lands_in_unmet_gates() {
        let lane_positions = vec![pos("r1", "lane-a", 0, 0, "repo", "A.1")];
        let dep = BlockedBy::Operator(OperatorDep {
            slug: "review-plan".to_string(),
            exit: "planning/decision.md".to_string(),
            start: "/begin-session review-plan".to_string(),
            what: None,
        });
        let files = vec![state_file(
            "repo",
            vec![track_block("A.1", Some("open"), vec![dep])],
        )];
        let frontier = compute_frontier(
            &lane_positions,
            &empty_graph(),
            &files,
            &HashMap::new(),
            None,
        );
        assert_eq!(frontier.entries.len(), 1);
        let entry = &frontier.entries[0];
        assert!(!entry.startable);
        assert!(entry.unmet_blocks.is_empty());
        assert_eq!(entry.unmet_gates, vec!["OP.review-plan".to_string()]);
    }

    fn track_block_with_priority(
        id: &str,
        priority: Option<u8>,
        depends_on: Vec<BlockedBy>,
    ) -> TrackBlock {
        TrackBlock {
            id: id.to_string(),
            title: format!("Title for {id}"),
            status: Some("open".to_string()),
            priority,
            depends_on,
            ..Default::default()
        }
    }

    fn operator_dep(slug: &str) -> BlockedBy {
        BlockedBy::Operator(OperatorDep {
            slug: slug.to_string(),
            exit: "planning/decision.md".to_string(),
            start: format!("/begin-session {slug}"),
            what: None,
        })
    }

    #[test]
    fn shared_slug_across_two_repos_collapses_to_one_gate_rank_carrying_the_minimum() {
        let files = vec![
            state_file(
                "repoA",
                vec![
                    track_block_with_priority("A.1", Some(2), vec![operator_dep("review-plan")]),
                    track_block_with_priority("A.2", Some(1), vec![operator_dep("review-plan")]),
                ],
            ),
            state_file(
                "repoB",
                vec![track_block_with_priority(
                    "B.1",
                    Some(3),
                    vec![operator_dep("review-plan")],
                )],
            ),
        ];
        let ranks = gate_ranks(&files, &HashMap::new());
        assert_eq!(ranks.len(), 1);
        let rank = &ranks[0];
        assert_eq!(rank.kind, "operator");
        assert_eq!(rank.slug, "review-plan");
        assert_eq!(rank.rank, 1);
        assert_eq!(
            rank.gates,
            vec![
                "repoA:A.1".to_string(),
                "repoA:A.2".to_string(),
                "repoB:B.1".to_string()
            ]
        );
    }

    #[test]
    fn absent_priority_throughout_ranks_u8_max_and_sorts_last() {
        let files = vec![
            state_file(
                "repoA",
                vec![track_block_with_priority(
                    "A.1",
                    Some(0),
                    vec![operator_dep("hot-gate")],
                )],
            ),
            state_file(
                "repoB",
                vec![track_block_with_priority(
                    "B.1",
                    None,
                    vec![operator_dep("cold-gate")],
                )],
            ),
        ];
        let ranks = gate_ranks(&files, &HashMap::new());
        assert_eq!(ranks.len(), 2);
        // sorted rank asc, so the absent-priority gate sorts last.
        assert_eq!(ranks[0].slug, "hot-gate");
        assert_eq!(ranks[0].rank, 0);
        assert_eq!(ranks[1].slug, "cold-gate");
        assert_eq!(ranks[1].rank, u8::MAX);
    }

    #[test]
    fn effective_priorities_map_wins_over_raw_priority_for_gate_rank() {
        let files = vec![state_file(
            "repo",
            vec![track_block_with_priority(
                "A.1",
                Some(3),
                vec![operator_dep("review-plan")],
            )],
        )];
        let mut effective = HashMap::new();
        effective.insert("repo:A.1".to_string(), 0u8);
        let ranks = gate_ranks(&files, &effective);
        assert_eq!(ranks.len(), 1);
        assert_eq!(ranks[0].rank, 0);
    }

    #[test]
    fn compute_frontier_attaches_gate_ranks() {
        let lane_positions = vec![pos("r1", "lane-a", 0, 0, "repo", "A.1")];
        let dep = operator_dep("review-plan");
        let files = vec![state_file(
            "repo",
            vec![track_block_with_priority("A.1", Some(1), vec![dep])],
        )];
        let frontier = compute_frontier(
            &lane_positions,
            &empty_graph(),
            &files,
            &HashMap::new(),
            None,
        );
        assert_eq!(frontier.gate_ranks.len(), 1);
        assert_eq!(frontier.gate_ranks[0].slug, "review-plan");
        assert_eq!(frontier.gate_ranks[0].rank, 1);
    }

    #[test]
    fn ensure_untruncated_errors_on_truncated_and_passes_otherwise() {
        let truncated = export(756, true);
        let err = ensure_untruncated(&truncated).expect_err("must refuse truncated export");
        assert_eq!(err.locator, "block_graph.truncated");
        assert!(err.message.contains("E_FRONTIER_TRUNCATED_GRAPH"));

        let full = export(756, false);
        assert!(ensure_untruncated(&full).is_ok());
    }

    // -----------------------------------------------------------------------
    // plan_frontier — MV.13.B Task 3
    // -----------------------------------------------------------------------

    fn write(dir: &std::path::Path, rel: &str, content: &str) {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    fn plan_state_file_fixture(repo: &str, id: &str) -> (StateSource, StateFile) {
        let src = StateSource {
            repo_slug: repo.to_string(),
            abs_path: PathBuf::from(format!("{repo}/planning/state.json")),
            expected_kind: "project",
        };
        let file: StateFile = serde_json::from_str(&format!(
            r#"{{"repo":"{repo}","kind":"project","updated":"2026-08-17","tracks":[
                {{"title":"t","blocks":[{{"id":"{id}","title":"x","status":"open"}}]}}
            ]}}"#
        ))
        .unwrap();
        (src, file)
    }

    #[test]
    fn plan_frontier_writes_artifact_with_parseable_derived_at_and_expected_entries() {
        let dir = crate::testsupport::unique_temp_dir("mev-plan-frontier-basic");
        write(
            &dir,
            "planning/roadmaps/alpha/lane-substrate.json",
            r#"{"lane":"substrate","roadmap":"alpha","blocks":[{"id":"MV.ticket.a","origin_roadmap":"alpha","repo":"mev"}]}"#,
        );
        let loaded = vec![plan_state_file_fixture("mev", "MV.ticket.a")];

        let plan = plan_frontier(&dir, &loaded);
        assert!(
            plan.diagnostics.is_empty(),
            "expected no diagnostics, got {:?}",
            plan.diagnostics
        );
        assert_eq!(plan.actions.len(), 1, "expected exactly one write action");

        let action = &plan.actions[0];
        assert_eq!(action.path, dir.join(LANE_FRONTIER_ARTIFACT));

        let artifact: serde_json::Value =
            serde_json::from_str(&action.new_content).expect("artifact must be valid JSON");
        let derived_at = artifact["derived_at"].as_str().expect("derived_at string");
        chrono::DateTime::parse_from_rfc3339(derived_at)
            .expect("derived_at must be a parseable RFC 3339 timestamp");

        let entries = artifact["entries"].as_array().expect("entries array");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["roadmap"], "alpha");
        assert_eq!(entries[0]["lane"], "substrate");
        assert_eq!(entries[0]["repo"], "mev");
        assert_eq!(entries[0]["id"], "MV.ticket.a");
        assert_eq!(entries[0]["startable"], true);

        assert!(artifact["gate_ranks"].as_array().unwrap().is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn plan_frontier_no_lane_files_plans_nothing() {
        let dir = crate::testsupport::unique_temp_dir("mev-plan-frontier-empty");
        std::fs::create_dir_all(dir.join("planning")).unwrap();

        let plan = plan_frontier(&dir, &[]);
        assert!(plan.actions.is_empty());
        assert!(plan.diagnostics.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------------
    // render_frontier_text — MV.13.B Task 4
    // -----------------------------------------------------------------------

    #[test]
    fn render_frontier_text_shape_for_startable_and_blocked_entries() {
        let frontier = Frontier {
            entries: vec![
                FrontierEntry {
                    roadmap: "alpha".to_string(),
                    lane: "derive".to_string(),
                    segment: 0,
                    repo: "mev".to_string(),
                    key: "mev:MV.13.A".to_string(),
                    id: "MV.13.A".to_string(),
                    title: "Frontier".to_string(),
                    status: "open".to_string(),
                    unmet_blocks: Vec::new(),
                    unmet_gates: Vec::new(),
                    startable: true,
                },
                FrontierEntry {
                    roadmap: "alpha".to_string(),
                    lane: "console".to_string(),
                    segment: 1,
                    repo: "bastion".to_string(),
                    key: "bastion:BA.19.C".to_string(),
                    id: "BA.19.C".to_string(),
                    title: "Lanes endpoint".to_string(),
                    status: "open".to_string(),
                    unmet_blocks: vec!["mev:MV.13.B".to_string()],
                    unmet_gates: vec!["OP.review-plan".to_string()],
                    startable: false,
                },
            ],
            gate_ranks: Vec::new(),
        };

        let text = render_frontier_text(&frontier);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "alpha/derive#0 mev:MV.13.A — startable");
        assert_eq!(
            lines[1],
            "alpha/console#1 bastion:BA.19.C — blocked by mev:MV.13.B, OP.review-plan"
        );
    }

    #[test]
    fn render_frontier_text_empty_entries_yields_empty_string() {
        let frontier = Frontier::default();
        assert_eq!(render_frontier_text(&frontier), "");
    }

    #[test]
    fn json_round_trips_to_the_same_struct() {
        let frontier = Frontier {
            entries: vec![FrontierEntry {
                roadmap: "alpha".to_string(),
                lane: "derive".to_string(),
                segment: 0,
                repo: "mev".to_string(),
                key: "mev:MV.13.A".to_string(),
                id: "MV.13.A".to_string(),
                title: "Frontier".to_string(),
                status: "open".to_string(),
                unmet_blocks: Vec::new(),
                unmet_gates: Vec::new(),
                startable: true,
            }],
            gate_ranks: vec![GateRank {
                kind: "operator",
                slug: "review-plan".to_string(),
                rank: 1,
                gates: vec!["mev:MV.13.A".to_string()],
            }],
        };

        let json = serde_json::to_string(&frontier).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(value["entries"][0]["id"], "MV.13.A");
        assert_eq!(value["entries"][0]["startable"], true);
        assert_eq!(value["gate_ranks"][0]["kind"], "operator");
        assert_eq!(value["gate_ranks"][0]["slug"], "review-plan");
        assert_eq!(value["gate_ranks"][0]["rank"], 1);
        assert_eq!(value["gate_ranks"][0]["gates"][0], "mev:MV.13.A");
    }

    #[test]
    fn assemble_frontier_plan_truncated_export_produces_diagnostic_and_zero_actions() {
        let root = PathBuf::from("/hq");
        let truncated_export = export(756, true);
        let plan = assemble_frontier_plan(
            &root,
            &[],
            &[],
            &empty_graph(),
            &truncated_export,
            1,
            EmitPlan::default(),
        );

        assert!(
            plan.actions.is_empty(),
            "a truncated export must never produce a write action"
        );
        assert_eq!(plan.diagnostics.len(), 1);
        assert!(
            plan.diagnostics[0]
                .message
                .contains("E_FRONTIER_TRUNCATED_GRAPH")
        );
    }
}
