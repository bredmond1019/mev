//! Block-graph exporter — the single enriched block-graph derivation (Phase 10, Block MV.10.B).
//!
//! [`build_block_graph_export`] is *the* derivation consumed by `MV.10.C`'s CLI and
//! bastion's `BA.17.A` endpoint — neither of those ever re-derives a field; they project
//! this module's output. Mirrors [`crate::brain::graph_emit`]'s envelope style: a
//! `version`/`root` header, a `nodes`/`edges` body, a resolved-target field on edges,
//! deterministic ordering, and a pure value returned (nothing written to disk).
//!
//! Every derivation runs over the **full corpus** first; the seven-stage scope pipeline
//! (tier → repo → epic → closed → boundary → edges → truncate, with truncate re-filtering
//! edges against the final node set) is layered on top,
//! strictly *after* enrichment, which is what guarantees a scoped export can never
//! report a different `lane`, `effective_priority`, `layer`, or `topo_index` for a node
//! than an unscoped export does. Epic scope overrides tier rather than intersecting
//! with it.
//!
//! Enrichment sources are consumed, never re-derived: [`crate::brain::emit::topo_order`],
//! [`crate::brain::state::cycle_paths`], [`crate::brain::state::effective_priorities`],
//! [`crate::brain::state::ready_order`], and [`crate::brain::state::derive_focus`].

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::Serialize;

use crate::brain::config::BrainConfig;
use crate::brain::emit::{epic_members, topo_order};
use crate::brain::last_touched::derive_last_touched;
use crate::brain::state::{
    BlockedBy, ExternalDep, StateEdgeKind, StateFile, StateGraph, StateSource, TierScope,
    TrackBlock, cycle_paths, derive_focus, derive_rollup, effective_priorities, ready_order,
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
    /// Whether this node survives the scope pipeline's tier/repo/epic/closed stages.
    /// `true` for a survivor, `false` for a node re-added only as a boundary neighbour.
    pub in_scope: bool,
    /// `what` strings from this block's `{type:"external"}` `depends_on` entries. No
    /// synthetic node is ever created for an external dependency.
    pub external_deps: Vec<String>,
    /// Count of unmet dependencies for a `Blocked` node (from `DerivedFocus.blocked`'s
    /// unmet subset) — `0` for every other lane.
    pub unmet_count: u32,
    /// Corpus-wide count of in-corpus blocks whose `BlockedBy` edges point at this node
    /// (`CrossRepo` edges excluded, mirroring `layer`'s edge-kind filter). Computed over
    /// the full corpus before any scope filtering, so it is identical for a given node
    /// key across a scoped and an unscoped export. `0` for a node nothing depends on —
    /// never absent, never a sentinel. Distinct `(from, to_ref)` pairs are deduped, so a
    /// duplicated authored `depends_on` entry cannot inflate the count.
    pub dependent_count: u32,
    /// Timestamp of this block's most recent SDLC run artifact on disk, derived by
    /// [`crate::brain::last_touched::derive_last_touched`]. Computed over the full
    /// corpus before any scope filtering, so it is identical for a given node key
    /// across a scoped and an unscoped export — mirrors `dependent_count`. `None`
    /// means the block has **never** been worked, not that it was worked long ago;
    /// never a sentinel date, never `state.json.updated`. Always serialized (no
    /// `skip_serializing_if`) so a consumer can distinguish "no run" from "field not
    /// understood".
    pub last_touched: Option<String>,
    /// Whether this block's most-recent SDLC run artifact (the same winning file
    /// `last_touched` was derived from — see [`crate::brain::last_touched::LastTouched`])
    /// ended with run-state `status == "reconcile_failed"`. Per base-template's
    /// `docs/data-contract.md` (`doc_id: sdlc-run-state-data-contract`, the pinned
    /// authority for this vocabulary — not re-derived here): such a block must be
    /// reported as **not closed**, but distinguishable from an ordinary in-progress or
    /// `"blocked"` run. Three states, not two: `Some(true)` (most recent run ended
    /// `reconcile_failed`), `Some(false)` (a run was found and it did not), `None` (no
    /// run found at all — absence means "never worked", same honesty rule as
    /// `last_touched`). Skipped from the emitted JSON when `None` so a block that has
    /// never run does not grow a spurious `false`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reconcile_failed: Option<bool>,
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
/// Computes every full-corpus derivation first, then applies the seven-stage scope
/// pipeline (tier → repo → epic → closed → boundary → edges → truncate) strictly after
/// enrichment, so a scoped export reports the same `lane`/`effective_priority`/`layer`/
/// `topo_index` for a node as an unscoped export does.
pub fn build_block_graph_export(
    root: &Path,
    config: &BrainConfig,
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
    let ready_set: HashSet<String> = ready_order(graph, files, None).into_iter().collect();

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
        let focus = derive_focus(src, file, graph, files, None);
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
        let l = layer_of(
            key.as_str(),
            &resolved_deps,
            &mut layer_memo,
            &mut layer_stack,
        );
        layer_of_key.insert(key.clone(), l);
    }

    // --- dependent_count: corpus-wide fan-in over resolved BlockedBy edges only
    // (CrossRepo excluded, same edge-kind filter `layer` uses above). Distinct `from`
    // keys per `to_ref` are deduped so a duplicated authored `depends_on` entry can't
    // inflate the count. ---
    let mut dependents_of: HashMap<&str, HashSet<&str>> = HashMap::new();
    for edge in &graph.edges {
        if edge.kind == StateEdgeKind::BlockedBy {
            dependents_of
                .entry(edge.to_ref.as_str())
                .or_default()
                .insert(edge.from.as_str());
        }
    }

    // --- last_touched: corpus-wide SDLC-artifact recency, derived once here (before
    // scope filtering) so it is identical for a given node key across a scoped and an
    // unscoped export — same placement as dependent_count above. ---
    let last_touched_map = derive_last_touched(root, config, files);

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
                BlockedBy::External(ExternalDep { what }) => Some(what.clone()),
                BlockedBy::Block(_) | BlockedBy::Operator(_) | BlockedBy::Approval(_) => None,
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
            dependent_count: dependents_of
                .get(key.as_str())
                .map(|s| s.len() as u32)
                .unwrap_or(0),
            last_touched: last_touched_map.get(key).map(|lt| lt.updated_at.clone()),
            reconcile_failed: last_touched_map
                .get(key)
                .map(|lt| lt.status.as_deref() == Some("reconcile_failed")),
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
            let source_closed =
                status_map.get(edge.from.as_str()).copied().flatten() == Some("closed");
            let target_closed =
                status_map.get(edge.to_ref.as_str()).copied().flatten() == Some("closed");
            BlockGraphEdge {
                from: edge.from.clone(),
                to_ref: edge.to_ref.clone(),
                kind: edge.kind.clone(),
                target_node_id,
                blocking: !source_closed && !target_closed,
            }
        })
        .collect();

    // -----------------------------------------------------------------
    // Seven-stage scope filter pipeline (task 2). Runs strictly AFTER every
    // derivation above — a scoped node keeps the exact same lane, priority,
    // layer, and topo_index it has in an unscoped export.
    // -----------------------------------------------------------------

    let node_meta: HashMap<String, (String, BlockLane)> = nodes
        .iter()
        .map(|n| (n.key.clone(), (n.repo.clone(), n.lane)))
        .collect();

    // Stage 1 — TIER: resolve in-scope repo slugs the same way bastion's
    // `assemble_board` does, via `derive_rollup`.
    let tier_repos: HashSet<String> = derive_rollup(&scope.tier, config, &[], graph, files)
        .into_iter()
        .map(|r| r.repo)
        .collect();
    let mut in_scope_keys: HashSet<String> = node_meta
        .iter()
        .filter(|(_, (repo, _))| tier_repos.contains(repo))
        .map(|(k, _)| k.clone())
        .collect();

    // Stage 2 — REPO: intersect down to a single slug, if requested.
    if let Some(repo) = &scope.repo {
        in_scope_keys.retain(|k| node_meta.get(k).map(|(r, _)| r == repo).unwrap_or(false));
    }

    // Stage 3 — EPIC: overrides tier/repo entirely (a cross-repo projection),
    // never intersects with them.
    if let Some(epic_slug) = &scope.epic {
        in_scope_keys = epic_members(graph, files, epic_slug)
            .into_iter()
            .map(|(repo, block)| format!("{repo}:{}", block.id))
            .collect();
    }

    // Stage 4 — CLOSED: drop Closed-lane nodes unless explicitly included.
    if !scope.include_closed {
        in_scope_keys.retain(|k| {
            node_meta
                .get(k)
                .map(|(_, lane)| *lane != BlockLane::Closed)
                .unwrap_or(true)
        });
    }

    // Stage 5 — BOUNDARY: re-add direct dependencies/dependents of the
    // surviving set, flagged `in_scope: false`. Survivors keep `in_scope: true`.
    let mut boundary_keys: HashSet<String> = HashSet::new();
    if scope.include_boundary {
        for edge in &edges {
            let from_in = in_scope_keys.contains(&edge.from);
            let to_in = edge
                .target_node_id
                .as_ref()
                .is_some_and(|t| in_scope_keys.contains(t));
            if from_in && !to_in {
                if let Some(t) = &edge.target_node_id {
                    boundary_keys.insert(t.clone());
                }
            } else if to_in && !from_in {
                boundary_keys.insert(edge.from.clone());
            }
        }
    }

    let final_keys: HashSet<String> = in_scope_keys.union(&boundary_keys).cloned().collect();

    let mut scoped_nodes: Vec<BlockGraphNode> = nodes
        .into_iter()
        .filter(|n| final_keys.contains(&n.key))
        .map(|mut n| {
            n.in_scope = in_scope_keys.contains(&n.key);
            n
        })
        .collect();

    // Stage 6 — EDGES: keep an edge when its `from` survives (in-scope or
    // boundary) AND its `to_ref` either survives too or is dangling.
    let mut scoped_edges: Vec<BlockGraphEdge> = edges
        .into_iter()
        .filter(|e| {
            let from_ok = final_keys.contains(&e.from);
            let to_ok = match &e.target_node_id {
                Some(t) => final_keys.contains(t),
                None => true, // dangling edges are always retained
            };
            from_ok && to_ok
        })
        .collect();

    // Stage 7 — TRUNCATE: `total_nodes` holds the pre-truncation count; the
    // (already topo-ordered) node list is then capped at `max_nodes`. Edges are
    // re-filtered against the truncated node set so the envelope stays
    // internally consistent — an edge surviving Stage 6 but naming a node that
    // truncation just dropped would otherwise dangle for a reason a consumer
    // can't distinguish from a genuinely unresolved reference.
    let total_nodes = scoped_nodes.len() as u32;
    let truncated = scoped_nodes.len() > scope.max_nodes;
    if truncated {
        scoped_nodes.truncate(scope.max_nodes);
        let truncated_keys: HashSet<&str> = scoped_nodes.iter().map(|n| n.key.as_str()).collect();
        scoped_edges.retain(|e| {
            let from_ok = truncated_keys.contains(e.from.as_str());
            let to_ok = match &e.target_node_id {
                Some(t) => truncated_keys.contains(t.as_str()),
                None => true,
            };
            from_ok && to_ok
        });
    }

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

    BlockGraphExport {
        version: "1".to_string(),
        root: root.display().to_string(),
        scope: scope_echo,
        nodes: scoped_nodes,
        edges: scoped_edges,
        cycles,
        total_nodes,
        truncated,
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::config::BrainConfig;
    use crate::brain::state::BlockDep;
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
            note: None,
            description: None,
            ..Default::default()
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
                ..Default::default()
            }],
            repos: Vec::new(),
            cross_repo: Vec::new(),
            tiers: Vec::new(),
            epics: Vec::new(),
            note: None,
            backlog: Vec::new(),
            carryover: Vec::new(),
            ..Default::default()
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

    /// Config carrying a single `"repo"` entry, matching `src("repo")` — needed so
    /// `TierScope::All` (via `derive_rollup`) resolves that repo as in-scope. All
    /// single-repo unscoped-derivation tests use this rather than
    /// `BrainConfig::default()`, whose empty `repos[]` would filter every node out.
    fn single_repo_config() -> BrainConfig {
        BrainConfig {
            repos: vec![crate::brain::config::RepoEntry {
                slug: "repo".to_string(),
                tier: "core".to_string(),
                repo_path: "repo".to_string(),
                status_file: String::new(),
                cache_doc: String::new(),
                heading: String::new(),
                prefix: None,
            }],
            ..BrainConfig::default()
        }
    }

    fn dep(repo: &str, id: &str) -> BlockedBy {
        BlockedBy::Block(BlockDep {
            repo: repo.to_string(),
            id: id.to_string(),
            what: None,
        })
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
            &single_repo_config(),
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
        b.depends_on = vec![BlockedBy::External(ExternalDep {
            what: "waiting on vendor API".to_string(),
        })];
        let file = project_file(vec![b]);
        let files = vec![(src("repo"), file)];
        let graph = build_state_graph(&files);
        let export = build_block_graph_export(
            Path::new("/hq"),
            &single_repo_config(),
            &graph,
            &files,
            &default_scope(),
        );

        assert_eq!(
            export.nodes.len(),
            1,
            "no synthetic node for the external dep"
        );
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
            &single_repo_config(),
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
            &single_repo_config(),
            &graph,
            &files,
            &default_scope(),
        );

        let edge = export
            .edges
            .iter()
            .find(|e| e.from == "repo:A" && e.to_ref == "repo:B")
            .unwrap();
        assert!(
            !edge.blocking,
            "target is closed, so the edge is not blocking"
        );
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
            &single_repo_config(),
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
            &single_repo_config(),
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
            &single_repo_config(),
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
            &single_repo_config(),
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

    // -----------------------------------------------------------------
    // Task 2 — seven-stage scope filter pipeline
    // -----------------------------------------------------------------

    fn repo_entry(slug: &str, tier: &str) -> crate::brain::config::RepoEntry {
        crate::brain::config::RepoEntry {
            slug: slug.to_string(),
            tier: tier.to_string(),
            repo_path: slug.to_string(),
            status_file: String::new(),
            cache_doc: String::new(),
            heading: String::new(),
            prefix: None,
        }
    }

    fn two_repo_config() -> BrainConfig {
        BrainConfig {
            repos: vec![repo_entry("alpha", "core"), repo_entry("beta", "portfolio")],
            ..BrainConfig::default()
        }
    }

    /// Two repos (`alpha` in tier `core`, `beta` in tier `portfolio`), one block
    /// each, no cross-repo edges — the shared fixture for the scoping tests below.
    fn two_repo_files() -> Vec<(StateSource, StateFile)> {
        let a = block("A1", Some("open"));
        let b = block("B1", Some("open"));
        vec![
            (src("alpha"), project_file(vec![a])),
            (src("beta"), project_file(vec![b])),
        ]
    }

    #[test]
    fn tier_scope_limits_to_repos_in_that_tier() {
        let files = two_repo_files();
        let graph = build_state_graph(&files);
        let config = two_repo_config();
        let mut scope = default_scope();
        scope.tier = TierScope::Tier("core".to_string());

        let export = build_block_graph_export(Path::new("/hq"), &config, &graph, &files, &scope);

        assert_eq!(export.nodes.len(), 1);
        assert_eq!(export.nodes[0].key, "alpha:A1");
        assert_eq!(export.scope.tier, Some("core".to_string()));
    }

    #[test]
    fn repo_scope_narrows_to_one_slug() {
        let files = two_repo_files();
        let graph = build_state_graph(&files);
        let config = two_repo_config();
        let mut scope = default_scope();
        scope.repo = Some("beta".to_string());

        let export = build_block_graph_export(Path::new("/hq"), &config, &graph, &files, &scope);

        assert_eq!(export.nodes.len(), 1);
        assert_eq!(export.nodes[0].key, "beta:B1");
        assert_eq!(export.scope.repo, Some("beta".to_string()));
    }

    #[test]
    fn epic_scope_overrides_tier() {
        let mut a = block("A1", Some("open"));
        a.epics = vec!["epic-x".to_string()];
        let mut b = block("B1", Some("open"));
        b.epics = vec!["epic-x".to_string()];
        let files = vec![
            (src("alpha"), project_file(vec![a])),
            (src("beta"), project_file(vec![b])),
        ];
        let graph = build_state_graph(&files);
        let config = two_repo_config();
        let mut scope = default_scope();
        // Tier "core" alone would exclude beta entirely; epic must override it.
        scope.tier = TierScope::Tier("core".to_string());
        scope.epic = Some("epic-x".to_string());

        let export = build_block_graph_export(Path::new("/hq"), &config, &graph, &files, &scope);

        let mut keys: Vec<&str> = export.nodes.iter().map(|n| n.key.as_str()).collect();
        keys.sort();
        assert_eq!(keys, vec!["alpha:A1", "beta:B1"]);
    }

    #[test]
    fn include_closed_toggle_changes_node_count() {
        let open_block = block("A", Some("open"));
        let closed_block = block("B", Some("closed"));
        let file = project_file(vec![open_block, closed_block]);
        let files = vec![(src("repo"), file)];
        let graph = build_state_graph(&files);

        let mut with_closed = default_scope();
        with_closed.include_closed = true;
        let export_with = build_block_graph_export(
            Path::new("/hq"),
            &single_repo_config(),
            &graph,
            &files,
            &with_closed,
        );
        assert_eq!(export_with.nodes.len(), 2);

        let mut without_closed = default_scope();
        without_closed.include_closed = false;
        let export_without = build_block_graph_export(
            Path::new("/hq"),
            &single_repo_config(),
            &graph,
            &files,
            &without_closed,
        );
        assert_eq!(export_without.nodes.len(), 1);
        assert_eq!(export_without.nodes[0].id, "A");
    }

    #[test]
    fn include_boundary_adds_out_of_scope_nodes_and_retains_edges() {
        let mut a = block("A", Some("open"));
        a.depends_on = vec![dep("beta", "B1")];
        let b = block("B1", Some("open"));
        let files = vec![
            (src("alpha"), project_file(vec![a])),
            (src("beta"), project_file(vec![b])),
        ];
        let graph = build_state_graph(&files);
        let config = two_repo_config();

        // Scope to just "alpha" — "beta:B1" is a dependency but out of scope.
        let mut scope = default_scope();
        scope.repo = Some("alpha".to_string());
        scope.include_boundary = false;
        let export_no_boundary =
            build_block_graph_export(Path::new("/hq"), &config, &graph, &files, &scope);
        assert_eq!(export_no_boundary.nodes.len(), 1);
        assert!(
            export_no_boundary
                .edges
                .iter()
                .all(|e| e.to_ref != "beta:B1")
        );

        scope.include_boundary = true;
        let export_boundary =
            build_block_graph_export(Path::new("/hq"), &config, &graph, &files, &scope);
        assert_eq!(export_boundary.nodes.len(), 2);
        let boundary_node = export_boundary
            .nodes
            .iter()
            .find(|n| n.key == "beta:B1")
            .expect("boundary node re-added");
        assert!(!boundary_node.in_scope);
        let scoped_node = export_boundary
            .nodes
            .iter()
            .find(|n| n.key == "alpha:A")
            .unwrap();
        assert!(scoped_node.in_scope);
        assert!(
            export_boundary
                .edges
                .iter()
                .any(|e| e.from == "alpha:A" && e.to_ref == "beta:B1"),
            "boundary edge retained"
        );
    }

    #[test]
    fn max_nodes_sets_truncated_with_pre_truncation_total() {
        let mut c = block("C", Some("open"));
        c.depends_on = vec![dep("repo", "B")];
        let mut b = block("B", Some("open"));
        b.depends_on = vec![dep("repo", "A")];
        let a = block("A", Some("open"));
        let file = project_file(vec![a, b, c]);
        let files = vec![(src("repo"), file)];
        let graph = build_state_graph(&files);

        let mut scope = default_scope();
        scope.max_nodes = 1;
        let export = build_block_graph_export(
            Path::new("/hq"),
            &single_repo_config(),
            &graph,
            &files,
            &scope,
        );

        assert_eq!(export.total_nodes, 3);
        assert!(export.truncated);
        assert_eq!(export.nodes.len(), 1);

        let mut untruncated_scope = default_scope();
        untruncated_scope.max_nodes = usize::MAX;
        let export_full = build_block_graph_export(
            Path::new("/hq"),
            &single_repo_config(),
            &graph,
            &files,
            &untruncated_scope,
        );
        assert!(!export_full.truncated);
        assert_eq!(export_full.total_nodes, 3);
        assert_eq!(export_full.nodes.len(), 3);
    }

    #[test]
    fn scoped_node_matches_unscoped_enrichment() {
        let mut c = block("C", Some("open"));
        c.depends_on = vec![dep("alpha", "A"), dep("beta", "B1")];
        let a = block("A", Some("open"));
        let b = block("B1", Some("open"));
        let files = vec![
            (src("alpha"), project_file(vec![a, c])),
            (src("beta"), project_file(vec![b])),
        ];
        let graph = build_state_graph(&files);
        let config = two_repo_config();

        let unscoped =
            build_block_graph_export(Path::new("/hq"), &config, &graph, &files, &default_scope());
        let unscoped_node = unscoped
            .nodes
            .iter()
            .find(|n| n.key == "alpha:A")
            .unwrap()
            .clone();

        let mut scope = default_scope();
        scope.repo = Some("alpha".to_string());
        let scoped = build_block_graph_export(Path::new("/hq"), &config, &graph, &files, &scope);
        let scoped_node = scoped
            .nodes
            .iter()
            .find(|n| n.key == "alpha:A")
            .unwrap()
            .clone();

        assert_eq!(scoped_node.lane, unscoped_node.lane);
        assert_eq!(
            scoped_node.effective_priority,
            unscoped_node.effective_priority
        );
        assert_eq!(scoped_node.layer, unscoped_node.layer);
        assert_eq!(scoped_node.topo_index, unscoped_node.topo_index);
    }

    // -----------------------------------------------------------------
    // Task 1 — dependent_count
    // -----------------------------------------------------------------

    #[test]
    fn dependent_count_fan_in_of_three() {
        // B, C, D all depend on A -> dependent_count(A) == 3.
        let a = block("A", Some("open"));
        let mut b = block("B", Some("open"));
        b.depends_on = vec![dep("repo", "A")];
        let mut c = block("C", Some("open"));
        c.depends_on = vec![dep("repo", "A")];
        let mut d = block("D", Some("open"));
        d.depends_on = vec![dep("repo", "A")];

        let file = project_file(vec![a, b, c, d]);
        let files = vec![(src("repo"), file)];
        let graph = build_state_graph(&files);
        let export = build_block_graph_export(
            Path::new("/hq"),
            &single_repo_config(),
            &graph,
            &files,
            &default_scope(),
        );

        let dependent_count_of = |id: &str| {
            export
                .nodes
                .iter()
                .find(|n| n.id == id)
                .unwrap()
                .dependent_count
        };
        assert_eq!(dependent_count_of("A"), 3);
        assert_eq!(dependent_count_of("B"), 0);
    }

    #[test]
    fn dependent_count_zero_for_leaf_no_dependents() {
        let a = block("A", Some("open"));
        let file = project_file(vec![a]);
        let files = vec![(src("repo"), file)];
        let graph = build_state_graph(&files);
        let export = build_block_graph_export(
            Path::new("/hq"),
            &single_repo_config(),
            &graph,
            &files,
            &default_scope(),
        );

        assert_eq!(export.nodes[0].dependent_count, 0);
    }

    #[test]
    fn dependent_count_ignores_dangling_edge() {
        // A depends on a nonexistent block "GHOST" — the dangling edge must not
        // increment any node's dependent_count (there is no node "GHOST" to increment,
        // and "A" itself gets no dependent from this edge).
        let mut a = block("A", Some("open"));
        a.depends_on = vec![dep("repo", "GHOST")];
        let file = project_file(vec![a]);
        let files = vec![(src("repo"), file)];
        let graph = build_state_graph(&files);
        let export = build_block_graph_export(
            Path::new("/hq"),
            &single_repo_config(),
            &graph,
            &files,
            &default_scope(),
        );

        assert_eq!(export.nodes.len(), 1);
        assert_eq!(export.nodes[0].dependent_count, 0);
    }

    #[test]
    fn dependent_count_excludes_cross_repo_edges() {
        // A brain-level CrossRepo edge from "alpha:A" to "beta:B1" must not count
        // toward beta:B1's dependent_count — only BlockedBy edges count.
        let a = block("A", Some("open"));
        let b = block("B1", Some("open"));
        let mut alpha_file = project_file(vec![a]);
        alpha_file.cross_repo = vec![crate::brain::state::CrossRepoEdge {
            from: crate::brain::state::Endpoint {
                repo: "alpha".to_string(),
                id: "A".to_string(),
            },
            to: crate::brain::state::Endpoint {
                repo: "beta".to_string(),
                id: "B1".to_string(),
            },
            note: None,
        }];
        let files = vec![
            (src("alpha"), alpha_file),
            (src("beta"), project_file(vec![b])),
        ];
        let graph = build_state_graph(&files);
        let config = two_repo_config();
        let export =
            build_block_graph_export(Path::new("/hq"), &config, &graph, &files, &default_scope());

        let b1 = export.nodes.iter().find(|n| n.key == "beta:B1").unwrap();
        assert_eq!(
            b1.dependent_count, 0,
            "a CrossRepo edge must not count toward dependent_count"
        );
    }

    #[test]
    fn dependent_count_deduped_for_duplicate_depends_on_entries() {
        // B lists a dependency on A twice — dependent_count(A) must still be 1, not 2.
        let a = block("A", Some("open"));
        let mut b = block("B", Some("open"));
        b.depends_on = vec![dep("repo", "A"), dep("repo", "A")];
        let file = project_file(vec![a, b]);
        let files = vec![(src("repo"), file)];
        let graph = build_state_graph(&files);
        let export = build_block_graph_export(
            Path::new("/hq"),
            &single_repo_config(),
            &graph,
            &files,
            &default_scope(),
        );

        let dependent_count_of = |id: &str| {
            export
                .nodes
                .iter()
                .find(|n| n.id == id)
                .unwrap()
                .dependent_count
        };
        assert_eq!(dependent_count_of("A"), 1);
    }

    #[test]
    fn dependent_count_identical_scoped_and_unscoped() {
        // "alpha:A" has two dependents: "alpha:C" (same repo) and "beta:B1" (cross-repo
        // BlockedBy, not CrossRepo). dependent_count(alpha:A) must match whether the
        // export is scoped down to just "alpha" or left unscoped.
        let mut c = block("C", Some("open"));
        c.depends_on = vec![dep("alpha", "A")];
        let a = block("A", Some("open"));
        let mut b1 = block("B1", Some("open"));
        b1.depends_on = vec![dep("alpha", "A")];
        let files = vec![
            (src("alpha"), project_file(vec![a, c])),
            (src("beta"), project_file(vec![b1])),
        ];
        let graph = build_state_graph(&files);
        let config = two_repo_config();

        let unscoped =
            build_block_graph_export(Path::new("/hq"), &config, &graph, &files, &default_scope());
        let unscoped_count = unscoped
            .nodes
            .iter()
            .find(|n| n.key == "alpha:A")
            .unwrap()
            .dependent_count;
        assert_eq!(unscoped_count, 2);

        let mut scope = default_scope();
        scope.repo = Some("alpha".to_string());
        let scoped = build_block_graph_export(Path::new("/hq"), &config, &graph, &files, &scope);
        let scoped_count = scoped
            .nodes
            .iter()
            .find(|n| n.key == "alpha:A")
            .unwrap()
            .dependent_count;

        assert_eq!(
            scoped_count, unscoped_count,
            "dependent_count must be scope-stable"
        );
    }

    // -----------------------------------------------------------------
    // Task 4 — last_touched
    // -----------------------------------------------------------------

    /// A `src()` fixture whose `abs_path` points at a real on-disk `planning/`
    /// directory (task 4's `src(repo)` helper above uses a fake `/repo/planning/...`
    /// path, which never resolves any folder — these tests need a real one for
    /// `derive_last_touched` to find candidate spec folders under).
    fn src_at(repo: &str, planning_dir: &std::path::Path) -> StateSource {
        StateSource {
            repo_slug: repo.to_string(),
            abs_path: planning_dir.join("state.json"),
            expected_kind: "project",
        }
    }

    fn write_run_state(spec_folder: &std::path::Path, updated_at: &str) {
        let sdlc_dir = spec_folder.join("sdlc");
        std::fs::create_dir_all(&sdlc_dir).unwrap();
        std::fs::write(
            sdlc_dir.join("sdlc-task-state.json"),
            format!(r#"{{"updated_at": "{updated_at}"}}"#),
        )
        .unwrap();
    }

    #[test]
    fn last_touched_reports_the_run_timestamp_when_a_folder_exists() {
        let dir = tempfile::tempdir().unwrap();
        let planning = dir.path().join("planning");
        let spec = planning.join("A");
        std::fs::create_dir_all(&spec).unwrap();
        write_run_state(&spec, "2026-07-01T10:00:00Z");

        let a = block("A", Some("open"));
        let files = vec![(src_at("repo", &planning), project_file(vec![a]))];
        let graph = build_state_graph(&files);
        let export = build_block_graph_export(
            Path::new("/hq"),
            &single_repo_config(),
            &graph,
            &files,
            &default_scope(),
        );

        assert_eq!(
            export.nodes[0].last_touched.as_deref(),
            Some("2026-07-01T10:00:00Z")
        );
    }

    #[test]
    fn last_touched_is_none_when_no_folder_exists() {
        let dir = tempfile::tempdir().unwrap();
        let planning = dir.path().join("planning");
        std::fs::create_dir_all(&planning).unwrap();

        let a = block("A", Some("open"));
        let files = vec![(src_at("repo", &planning), project_file(vec![a]))];
        let graph = build_state_graph(&files);
        let export = build_block_graph_export(
            Path::new("/hq"),
            &single_repo_config(),
            &graph,
            &files,
            &default_scope(),
        );

        assert_eq!(export.nodes[0].last_touched, None);
    }

    #[test]
    fn scope_echo_reflects_request() {
        let files = two_repo_files();
        let graph = build_state_graph(&files);
        let config = two_repo_config();
        let scope = BlockGraphScope {
            tier: TierScope::Tier("core".to_string()),
            epic: Some("epic-x".to_string()),
            repo: Some("alpha".to_string()),
            include_closed: false,
            include_boundary: true,
            max_nodes: 5,
        };

        let export = build_block_graph_export(Path::new("/hq"), &config, &graph, &files, &scope);

        assert_eq!(export.scope.tier, Some("core".to_string()));
        assert_eq!(export.scope.epic, Some("epic-x".to_string()));
        assert_eq!(export.scope.repo, Some("alpha".to_string()));
        assert!(!export.scope.include_closed);
        assert!(export.scope.include_boundary);
    }
}
