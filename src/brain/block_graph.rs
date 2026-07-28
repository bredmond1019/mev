//! Block-graph exporter — the single enriched block-graph derivation (Phase 10, Block MV.10.B).
//!
//! [`build_block_graph_export`] is *the* derivation consumed by `MV.10.C`'s CLI and
//! bastion's `BA.17.A` endpoint — neither of those ever re-derives a field; they project
//! this module's output. Mirrors [`crate::brain::graph_emit`]'s envelope style: a
//! `version`/`root` header, a `nodes`/`edges` body, a resolved-target field on edges,
//! deterministic ordering, and a pure value returned (nothing written to disk).
//!
//! **Task 1** (this file's initial cut) computes full-corpus enrichment only — every
//! node is `in_scope: true` and no scope filtering is applied yet. **Task 2** layers the
//! seven-stage scope pipeline on top, strictly *after* enrichment, which is what
//! guarantees a scoped export can never report a different `lane`, `effective_priority`,
//! `layer`, or `topo_index` for a node than an unscoped export does.
//!
//! Enrichment sources are consumed, never re-derived: [`crate::brain::emit::topo_order`],
//! [`crate::brain::state::cycle_paths`], [`crate::brain::state::effective_priorities`],
//! [`crate::brain::state::ready_order`], and [`crate::brain::state::derive_focus`].

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::Serialize;

use crate::brain::config::BrainConfig;
use crate::brain::emit::topo_order;
use crate::brain::state::{
    BlockedBy, StateEdgeKind, StateFile, StateGraph, StateSource, TierScope, TrackBlock,
    cycle_paths, derive_focus, effective_priorities, ready_order,
};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A node's derived attention lane.
///
/// Mirrors [`crate::brain::state::DerivedFocus`]'s four lanes (`now`/`next`/`blocked`/
/// `deferred`) plus two lanes `DerivedFocus` does not carry: `Closed` (from the authored
/// `TrackBlock.status == "closed"`) and `Other` (the fallback for an authored status that
/// matches none of the recognised values).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockLane {
    /// Authored `status == "in_progress"`.
    Now,
    /// Ready — open, no external deps, all block deps closed.
    Next,
    /// Open with at least one unmet dependency.
    Blocked,
    /// Authored `status == "deferred"`.
    Deferred,
    /// Authored `status == "closed"`.
    Closed,
    /// An authored status that matches none of the above (should not occur in a
    /// validated corpus, but the enrichment must not panic on one).
    Other,
}

/// One block, enriched with every full-corpus derivation the graph views need.
#[derive(Debug, Clone, Serialize)]
pub struct BlockGraphNode {
    /// Canonical `"repo:id"` key.
    pub key: String,
    /// Owning repo slug.
    pub repo: String,
    /// Canonical block ID.
    pub id: String,
    /// Brief human description.
    pub title: String,
    /// Authored lifecycle status (`open`/`in_progress`/`deferred`/`closed`), if any.
    pub status: Option<String>,
    /// Derived attention lane — see [`BlockLane`].
    pub lane: BlockLane,
    /// Title of the containing `tracks[]` phase/wave, if resolvable.
    pub track: Option<String>,
    /// Authored execution-order rank.
    pub wave: Option<i64>,
    /// Authored own priority.
    pub priority: Option<u8>,
    /// Effective priority from [`effective_priorities`] — absent when it never lands
    /// in the real `0..=3` range.
    pub effective_priority: Option<u8>,
    /// Authored due date/timing string.
    pub due: Option<String>,
    /// Cross-repo epic membership.
    pub epics: Vec<String>,
    /// Longest path over resolved `depends_on` edges (`0` = no resolved prerequisites).
    pub layer: u32,
    /// Position in the full-corpus [`topo_order`].
    pub topo_index: u32,
    /// Membership in [`ready_order`].
    pub ready: bool,
    /// Whether this node participates in a `depends_on` cycle.
    pub in_cycle: bool,
    /// Whether this node survives the scope pipeline (task 2). Always `true` in the
    /// task-1 full-corpus enrichment.
    pub in_scope: bool,
    /// `what` strings from this block's `{type:"external"}` `depends_on` entries. No
    /// synthetic node is ever created for an external dependency.
    pub external_deps: Vec<String>,
    /// Count of unmet dependencies for a `Blocked` node (from `DerivedFocus.blocked`'s
    /// unmet subset) — `0` for every other lane.
    pub unmet_count: u32,
}

/// One directed edge in the exported block graph.
#[derive(Debug, Clone, Serialize)]
pub struct BlockGraphEdge {
    /// `"repo:id"` key of the source (dependent) block.
    pub from: String,
    /// Raw, as-authored `"repo:id"` reference.
    pub to_ref: String,
    /// Edge discriminant.
    pub kind: StateEdgeKind,
    /// `Some(to_ref)` when it resolves to a node in this export; `None` when dangling.
    /// A dangling edge is retained, never dropped.
    pub target_node_id: Option<String>,
    /// `false` when either endpoint is `closed`.
    pub blocking: bool,
}

/// The scope request driving the seven-stage filter pipeline (task 2).
#[derive(Debug, Clone)]
pub struct BlockGraphScope {
    /// Tier restriction, resolved the same way [`crate::brain::state::derive_rollup`]
    /// resolves a brain file's `repos[]` scope.
    pub tier: TierScope,
    /// Epic slug restriction — overrides `tier` rather than intersecting with it.
    pub epic: Option<String>,
    /// Single-repo restriction.
    pub repo: Option<String>,
    /// Whether `Closed`-lane nodes are retained.
    pub include_closed: bool,
    /// Whether direct neighbours of the in-scope set are re-added as boundary nodes.
    pub include_boundary: bool,
    /// Truncate the (topo-ordered) node list to at most this many entries.
    pub max_nodes: usize,
}

/// The request echoed back on [`BlockGraphExport`] for the caller's own bookkeeping.
#[derive(Debug, Clone, Serialize)]
pub struct BlockGraphScopeEcho {
    /// `None` for [`TierScope::All`], `Some(name)` for [`TierScope::Tier`].
    pub tier: Option<String>,
    pub epic: Option<String>,
    pub repo: Option<String>,
    pub include_closed: bool,
    pub include_boundary: bool,
}

/// The complete, enriched block-graph export envelope for a Brain corpus.
///
/// Serialises to JSON. A pure value — nothing is written to disk (D4).
#[derive(Debug, Clone, Serialize)]
pub struct BlockGraphExport {
    /// Schema version — currently `"1"`.
    pub version: String,
    /// Display path of the brain root used for the build.
    pub root: String,
    /// Echo of the scope request that produced this export.
    pub scope: BlockGraphScopeEcho,
    /// Nodes, emitted in `topo_index` order.
    pub nodes: Vec<BlockGraphNode>,
    /// Edges — one per `graph.edges` entry that survives the scope pipeline.
    pub edges: Vec<BlockGraphEdge>,
    /// Cycles found over the **full corpus** (never the scoped subgraph), from
    /// [`cycle_paths`].
    pub cycles: Vec<Vec<String>>,
    /// Node count before any `max_nodes` truncation.
    pub total_nodes: u32,
    /// Whether `max_nodes` truncated the node list.
    pub truncated: bool,
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Build the enriched [`BlockGraphExport`] for a Brain corpus.
///
/// Task 1: computes every full-corpus derivation and treats every node as `in_scope:
/// true`; no filtering is applied. Task 2 layers the seven-stage scope pipeline on top of
/// this function's output, strictly after enrichment.
pub fn build_block_graph_export(
    root: &Path,
    _config: &BrainConfig,
    graph: &StateGraph,
    files: &[(StateSource, StateFile)],
    scope: &BlockGraphScope,
) -> BlockGraphExport {
    // --- Index every block by "repo:id" (repo_slug, &TrackBlock, track title). ---
    let mut by_key: HashMap<String, (String, &TrackBlock, String)> = HashMap::new();
    for (src, file) in files {
        for track in &file.tracks {
            for block in &track.blocks {
                let key = format!("{}:{}", src.repo_slug, block.id);
                by_key.insert(key, (src.repo_slug.clone(), block, track.title.clone()));
            }
        }
    }

    // --- Full-corpus derivations (never re-derived downstream). ---
    let topo = topo_order(graph, files);
    let topo_index_of: HashMap<&str, u32> = topo
        .iter()
        .enumerate()
        .map(|(i, k)| (k.as_str(), i as u32))
        .collect();

    let eff_priorities = effective_priorities(graph, files);
    let ready_set: HashSet<String> = ready_order(graph, files).into_iter().collect();

    let cycle_paths_list = cycle_paths(graph);
    let cycles: Vec<Vec<String>> = cycle_paths_list.iter().map(|c| c.keys.clone()).collect();
    let in_cycle_set: HashSet<&str> = cycle_paths_list
        .iter()
        .flat_map(|c| c.keys.iter().map(|k| k.as_str()))
        .collect();

    // Lane derivation: merge derive_focus per (src, file), prefixing bare IDs with the
    // owning file's repo slug (derive_focus returns bare block IDs, everything else is
    // keyed "repo:id").
    let mut now_set: HashSet<String> = HashSet::new();
    let mut next_set: HashSet<String> = HashSet::new();
    let mut blocked_map: HashMap<String, u32> = HashMap::new();
    let mut deferred_set: HashSet<String> = HashSet::new();
    for (src, file) in files {
        let focus = derive_focus(src, file, graph, files);
        let prefix = &src.repo_slug;
        now_set.extend(focus.now.iter().map(|id| format!("{prefix}:{id}")));
        next_set.extend(focus.next.iter().map(|id| format!("{prefix}:{id}")));
        deferred_set.extend(focus.deferred.iter().map(|id| format!("{prefix}:{id}")));
        for (id, unmet) in &focus.blocked {
            blocked_map.insert(format!("{prefix}:{id}"), unmet.len() as u32);
        }
    }

    // Authored status per key, for the Closed lane, layer's resolved-edge filter, and
    // edge `blocking`.
    let status_map: HashMap<&str, Option<&str>> = by_key
        .iter()
        .map(|(k, (_, block, _))| (k.as_str(), block.status.as_deref()))
        .collect();

    // --- layer: longest path over resolved BlockedBy edges, memoized DFS with an
    // on-stack recursion guard (mirrors effective_priorities); a back-edge contributes
    // 0 to the max instead of being followed again, so a cycle terminates. ---
    let mut resolved_deps: HashMap<&str, Vec<String>> = HashMap::new();
    for node_key in by_key.keys() {
        resolved_deps.entry(node_key.as_str()).or_default();
    }
    for edge in &graph.edges {
        if edge.kind == StateEdgeKind::BlockedBy && by_key.contains_key(&edge.to_ref) {
            resolved_deps
                .entry(edge.from.as_str())
                .or_default()
                .push(edge.to_ref.clone());
        }
    }

    fn layer_of<'a>(
        key: &'a str,
        deps: &HashMap<&'a str, Vec<String>>,
        memo: &mut HashMap<String, u32>,
        on_stack: &mut HashSet<String>,
    ) -> u32 {
        if let Some(&v) = memo.get(key) {
            return v;
        }
        on_stack.insert(key.to_string());
        let mut best = 0u32;
        if let Some(ds) = deps.get(key) {
            for d in ds {
                let contrib = if on_stack.contains(d.as_str()) {
                    // Back-edge — `d` is already on the recursion stack. Contributes 0
                    // instead of being followed again, so a cycle terminates.
                    0
                } else {
                    layer_of(d.as_str(), deps, memo, on_stack) + 1
                };
                if contrib > best {
                    best = contrib;
                }
            }
        }
        on_stack.remove(key);
        memo.insert(key.to_string(), best);
        best
    }

    let mut layer_memo: HashMap<String, u32> = HashMap::new();
    let mut layer_stack: HashSet<String> = HashSet::new();
    let mut layer_of_key: HashMap<String, u32> = HashMap::new();
    for key in by_key.keys() {
        let l = layer_of(key.as_str(), &resolved_deps, &mut layer_memo, &mut layer_stack);
        layer_of_key.insert(key.clone(), l);
    }

    // --- Assemble nodes, in topo_index order. ---
    let mut nodes: Vec<BlockGraphNode> = Vec::with_capacity(topo.len());
    for key in &topo {
        let Some((repo, block, track_title)) = by_key.get(key.as_str()) else {
            // topo_order only ever emits keys it built from `files`, so this should be
            // unreachable, but skip defensively rather than panic.
            continue;
        };

        let authored_status = block.status.clone();
        let lane = if authored_status.as_deref() == Some("closed") {
            BlockLane::Closed
        } else if now_set.contains(key) {
            BlockLane::Now
        } else if next_set.contains(key) {
            BlockLane::Next
        } else if blocked_map.contains_key(key) {
            BlockLane::Blocked
        } else if deferred_set.contains(key) {
            BlockLane::Deferred
        } else {
            BlockLane::Other
        };
        let unmet_count = blocked_map.get(key).copied().unwrap_or(0);

        let external_deps: Vec<String> = block
            .depends_on
            .iter()
            .filter_map(|d| match d {
                BlockedBy::External { what } => Some(what.clone()),
                BlockedBy::Block { .. } => None,
            })
            .collect();

        nodes.push(BlockGraphNode {
            key: key.clone(),
            repo: repo.clone(),
            id: block.id.clone(),
            title: block.title.clone(),
            status: authored_status,
            lane,
            track: Some(track_title.clone()),
            wave: block.wave,
            priority: block.priority,
            effective_priority: eff_priorities.get(key).copied(),
            due: block.due.clone(),
            epics: block.epics.clone(),
            layer: layer_of_key.get(key).copied().unwrap_or(0),
            topo_index: topo_index_of.get(key.as_str()).copied().unwrap_or(0),
            ready: ready_set.contains(key),
            in_cycle: in_cycle_set.contains(key.as_str()),
            in_scope: true,
            external_deps,
            unmet_count,
        });
    }

    // --- Edges. ---
    let edges: Vec<BlockGraphEdge> = graph
        .edges
        .iter()
        .map(|edge| {
            let target_node_id = by_key
                .contains_key(&edge.to_ref)
                .then(|| edge.to_ref.clone());
            let source_closed = status_map
                .get(edge.from.as_str())
                .copied()
                .flatten()
                == Some("closed");
            let target_closed = status_map
                .get(edge.to_ref.as_str())
                .copied()
                .flatten()
                == Some("closed");
            BlockGraphEdge {
                from: edge.from.clone(),
                to_ref: edge.to_ref.clone(),
                kind: edge.kind.clone(),
                target_node_id,
                blocking: !source_closed && !target_closed,
            }
        })
        .collect();

    let scope_echo = BlockGraphScopeEcho {
        tier: match &scope.tier {
            TierScope::All => None,
            TierScope::Tier(name) => Some(name.clone()),
        },
        epic: scope.epic.clone(),
        repo: scope.repo.clone(),
        include_closed: scope.include_closed,
        include_boundary: scope.include_boundary,
    };

    let total_nodes = nodes.len() as u32;

    BlockGraphExport {
        version: "1".to_string(),
        root: root.display().to_string(),
        scope: scope_echo,
        nodes,
        edges,
        cycles,
        total_nodes,
        truncated: false,
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::config::BrainConfig;
    use crate::brain::state::{Focus, StateFile, StateSource, Track, build_state_graph};
    use std::path::PathBuf;

    fn src(repo: &str) -> StateSource {
        StateSource {
            repo_slug: repo.to_string(),
            abs_path: PathBuf::from(format!("/{repo}/planning/state.json")),
            expected_kind: "project",
        }
    }

    fn block(id: &str, status: Option<&str>) -> TrackBlock {
        TrackBlock {
            id: id.to_string(),
            title: format!("Block {id}"),
            status: status.map(|s| s.to_string()),
            depends_on: Vec::new(),
            wave: None,
            origin: None,
            priority: None,
            due: None,
            sdlc_workflow: None,
            model: None,
            epics: Vec::new(),
        }
    }

    fn project_file(blocks: Vec<TrackBlock>) -> StateFile {
        StateFile {
            repo: "test".to_string(),
            kind: "project".to_string(),
            updated: "2026-01-01".to_string(),
            focus: Focus::default(),
            tracks: vec![Track {
                title: "Phase 1".to_string(),
                blocks,
            }],
            repos: Vec::new(),
            cross_repo: Vec::new(),
            tiers: Vec::new(),
            epics: Vec::new(),
            note: None,
            backlog: Vec::new(),
            carryover: Vec::new(),
        }
    }

    fn default_scope() -> BlockGraphScope {
        BlockGraphScope {
            tier: TierScope::All,
            epic: None,
            repo: None,
            include_closed: true,
            include_boundary: false,
            max_nodes: usize::MAX,
        }
    }

    fn dep(repo: &str, id: &str) -> BlockedBy {
        BlockedBy::Block {
            repo: repo.to_string(),
            id: id.to_string(),
            what: None,
        }
    }

    #[test]
    fn lane_covers_all_six_variants_with_repo_prefix() {
        let mut a = block("A", Some("in_progress"));
        a.id = "A".to_string();
        let mut b = block("B", Some("open"));
        b.depends_on = vec![dep("repo", "MISSING")];
        let c = block("C", Some("open")); // ready -> next
        let d = block("D", Some("deferred"));
        let e = block("E", Some("closed"));
        let f = block("F", Some("bogus_status"));

        let file = project_file(vec![a, b, c, d, e, f]);
        let files = vec![(src("repo"), file)];
        let graph = build_state_graph(&files);
        let export = build_block_graph_export(
            Path::new("/hq"),
            &BrainConfig::default(),
            &graph,
            &files,
            &default_scope(),
        );

        let lane_of = |id: &str| {
            export
                .nodes
                .iter()
                .find(|n| n.id == id)
                .unwrap_or_else(|| panic!("node {id} present"))
                .lane
        };
        assert_eq!(lane_of("A"), BlockLane::Now);
        assert_eq!(lane_of("B"), BlockLane::Blocked);
        assert_eq!(lane_of("C"), BlockLane::Next);
        assert_eq!(lane_of("D"), BlockLane::Deferred);
        assert_eq!(lane_of("E"), BlockLane::Closed);
        assert_eq!(lane_of("F"), BlockLane::Other);

        // Repo-slug prefix applied: every node's key is "repo:<id>".
        for node in &export.nodes {
            assert_eq!(node.key, format!("repo:{}", node.id));
        }

        let blocked_node = export.nodes.iter().find(|n| n.id == "B").unwrap();
        assert_eq!(blocked_node.unmet_count, 1);
    }

    #[test]
    fn external_deps_populate_with_no_synthetic_node() {
        let mut b = block("A", Some("open"));
        b.depends_on = vec![BlockedBy::External {
            what: "waiting on vendor API".to_string(),
        }];
        let file = project_file(vec![b]);
        let files = vec![(src("repo"), file)];
        let graph = build_state_graph(&files);
        let export = build_block_graph_export(
            Path::new("/hq"),
            &BrainConfig::default(),
            &graph,
            &files,
            &default_scope(),
        );

        assert_eq!(export.nodes.len(), 1, "no synthetic node for the external dep");
        assert_eq!(
            export.nodes[0].external_deps,
            vec!["waiting on vendor API".to_string()]
        );
        assert_eq!(export.nodes[0].lane, BlockLane::Blocked);
    }

    #[test]
    fn dangling_edge_is_retained_with_none_target() {
        let mut a = block("A", Some("open"));
        a.depends_on = vec![dep("repo", "GHOST")];
        let file = project_file(vec![a]);
        let files = vec![(src("repo"), file)];
        let graph = build_state_graph(&files);
        let export = build_block_graph_export(
            Path::new("/hq"),
            &BrainConfig::default(),
            &graph,
            &files,
            &default_scope(),
        );

        assert_eq!(export.edges.len(), 1);
        assert_eq!(export.edges[0].to_ref, "repo:GHOST");
        assert_eq!(export.edges[0].target_node_id, None);
        // Only one real node — the dangling target is never synthesized.
        assert_eq!(export.nodes.len(), 1);
    }

    #[test]
    fn blocking_is_false_when_either_endpoint_closed() {
        let mut a = block("A", Some("open"));
        a.depends_on = vec![dep("repo", "B")];
        let b = block("B", Some("closed"));
        let file = project_file(vec![a, b]);
        let files = vec![(src("repo"), file)];
        let graph = build_state_graph(&files);
        let export = build_block_graph_export(
            Path::new("/hq"),
            &BrainConfig::default(),
            &graph,
            &files,
            &default_scope(),
        );

        let edge = export
            .edges
            .iter()
            .find(|e| e.from == "repo:A" && e.to_ref == "repo:B")
            .unwrap();
        assert!(!edge.blocking, "target is closed, so the edge is not blocking");
    }

    #[test]
    fn layer_is_correct_on_a_diamond() {
        // D depends on B and C; B and C both depend on A. layer(A)=0, layer(B)=layer(C)=1,
        // layer(D)=2.
        let mut d = block("D", Some("open"));
        d.depends_on = vec![dep("repo", "B"), dep("repo", "C")];
        let mut b = block("B", Some("open"));
        b.depends_on = vec![dep("repo", "A")];
        let mut c = block("C", Some("open"));
        c.depends_on = vec![dep("repo", "A")];
        let a = block("A", Some("closed"));

        let file = project_file(vec![a, b, c, d]);
        let files = vec![(src("repo"), file)];
        let graph = build_state_graph(&files);
        let export = build_block_graph_export(
            Path::new("/hq"),
            &BrainConfig::default(),
            &graph,
            &files,
            &default_scope(),
        );

        let layer_of = |id: &str| export.nodes.iter().find(|n| n.id == id).unwrap().layer;
        assert_eq!(layer_of("A"), 0);
        assert_eq!(layer_of("B"), 1);
        assert_eq!(layer_of("C"), 1);
        assert_eq!(layer_of("D"), 2);
    }

    #[test]
    fn layer_terminates_on_a_cycle() {
        // A depends on B, B depends on A: a two-node cycle. Must terminate, not hang.
        let mut a = block("A", Some("open"));
        a.depends_on = vec![dep("repo", "B")];
        let mut b = block("B", Some("open"));
        b.depends_on = vec![dep("repo", "A")];
        let file = project_file(vec![a, b]);
        let files = vec![(src("repo"), file)];
        let graph = build_state_graph(&files);
        let export = build_block_graph_export(
            Path::new("/hq"),
            &BrainConfig::default(),
            &graph,
            &files,
            &default_scope(),
        );

        // Both nodes are flagged in_cycle, and layer computation terminated (test itself
        // not hanging is the primary assertion).
        assert!(export.nodes.iter().all(|n| n.in_cycle));
        assert_eq!(export.cycles.len(), 1);
    }

    #[test]
    fn cycles_populated_from_cycle_paths() {
        let mut a = block("A", Some("open"));
        a.depends_on = vec![dep("repo", "B")];
        let mut b = block("B", Some("open"));
        b.depends_on = vec![dep("repo", "A")];
        let file = project_file(vec![a, b]);
        let files = vec![(src("repo"), file)];
        let graph = build_state_graph(&files);
        let export = build_block_graph_export(
            Path::new("/hq"),
            &BrainConfig::default(),
            &graph,
            &files,
            &default_scope(),
        );

        assert_eq!(export.cycles.len(), 1);
        let mut keys = export.cycles[0].clone();
        keys.sort();
        assert_eq!(keys, vec!["repo:A".to_string(), "repo:B".to_string()]);
    }

    #[test]
    fn nodes_emitted_in_topo_index_order() {
        let mut a = block("A", Some("open"));
        a.depends_on = vec![dep("repo", "B")];
        let b = block("B", Some("closed"));
        let file = project_file(vec![a, b]);
        let files = vec![(src("repo"), file)];
        let graph = build_state_graph(&files);
        let export = build_block_graph_export(
            Path::new("/hq"),
            &BrainConfig::default(),
            &graph,
            &files,
            &default_scope(),
        );

        for (i, node) in export.nodes.iter().enumerate() {
            assert_eq!(node.topo_index, i as u32);
        }
        assert_eq!(export.version, "1");
        assert_eq!(export.root, "/hq");
    }
}
