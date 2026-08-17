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

use std::collections::HashMap;

use crate::Diagnostic;
use crate::brain::block_graph::BlockGraphExport;
use crate::brain::emit::global_status_map;
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
    /// `"operator:<slug>"` / `"approval:<slug>"` / `"external:<what>"`.
    pub unmet_gates: Vec<String>,
    /// `true` iff both `unmet_blocks` and `unmet_gates` are empty.
    pub startable: bool,
}

/// The corpus-wide frontier: one [`FrontierEntry`] per active `(roadmap, lane,
/// segment)`, plus the `gate_rank` derivation `MV.13.B` Task 2 folds in here.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
pub struct Frontier {
    pub entries: Vec<FrontierEntry>,
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
pub fn compute_frontier(
    lane_positions: &[DerivedBlockPosition],
    graph: &StateGraph,
    files: &[(StateSource, StateFile)],
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
                        unmet_gates.push(format!("operator:{slug}"));
                    }
                    BlockedBy::Approval(ApprovalDep { slug, .. }) => {
                        unmet_gates.push(format!("approval:{slug}"));
                    }
                    BlockedBy::External(ExternalDep { what }) => {
                        unmet_gates.push(format!("external:{what}"));
                    }
                }
            }
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

    Frontier { entries }
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
        let frontier = compute_frontier(&lane_positions, &empty_graph(), &files);
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
        let frontier = compute_frontier(&lane_positions, &empty_graph(), &files);
        assert!(frontier.entries.is_empty());
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
        let frontier = compute_frontier(&lane_positions, &empty_graph(), &files);
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
        let frontier = compute_frontier(&lane_positions, &empty_graph(), &files);
        assert_eq!(frontier.entries.len(), 1);
        let entry = &frontier.entries[0];
        assert!(!entry.startable);
        assert!(entry.unmet_blocks.is_empty());
        assert_eq!(entry.unmet_gates, vec!["operator:review-plan".to_string()]);
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
}
