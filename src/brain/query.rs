//! `mev blocks` — filtered block queries, the transitive leverage cone, and the
//! longest same-repo run reachable from a head.
//!
//! See `MV.ticket.query-verb-leverage-chain-and-filters` for the full rationale.
//! Deliberately self-contained: [`BlockInfo`], [`BlockCone`], and [`QueryReport`]
//! are this module's own types — no field is added to
//! [`crate::brain::block_graph::BlockGraphNode`] or `BlockGraphExport` (see the
//! block record's `out_of_scope`).
//!
//! `block_cone` reuses `crate::brain::availability::transitive_closure`
//! (widened to `pub(crate)`) to seed the transitive downstream closure.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;

/// One block as seen by this module's queries — a minimal, self-contained view.
/// Callers (the `mev blocks` verb, task 3) build these from the real corpus;
/// nothing here reads `state.json` directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockInfo {
    /// Canonical `"repo:id"` key.
    pub key: String,
    /// Owning repo slug, parsed from `key`'s `repo:` prefix.
    pub repo: String,
    /// Authored lifecycle status string (`"open"`, `"in_progress"`, `"blocked"`,
    /// `"deferred"`, `"wontfix"`, `"closed"`, ...).
    pub status: String,
    /// Roadmap this block belongs to, per D57 attribution (`origin_roadmap` by
    /// default — see the block record's `why` #5 and `notes`).
    pub roadmap: Option<String>,
    /// Whether this block currently has every prerequisite met (mirrors
    /// `DerivedFocus`'s startable classification upstream).
    pub startable: bool,
    /// Effective priority (`0..=3`), if resolvable.
    pub priority: Option<u8>,
}

impl BlockInfo {
    /// `true` unless the status is one of the "parked" statuses that must never
    /// count toward a leverage cone's size (`deferred`, `wontfix`, `closed`) —
    /// see the block record's regression case in `why` #1 / `testing_strategy`.
    pub fn is_live(&self) -> bool {
        !matches!(self.status.as_str(), "deferred" | "wontfix" | "closed")
    }
}

/// A directed `depends_on` edge for the cone/chain graphs: `from` depends on
/// (is blocked by) `to`. `blocking` mirrors `StateEdgeKind::BlockedBy` — a
/// non-blocking edge (e.g. an informational `reference` edge) must never extend
/// a cone or a chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DepEdge<'a> {
    pub from: &'a str,
    pub to: &'a str,
    pub blocking: bool,
}

/// Filter predicate for `mev blocks`. Every field is independent and skipped
/// when `None` — filters compose by AND, never by an implicit dependency
/// between fields (e.g. `max_priority` never implies a status filter).
#[derive(Debug, Clone, Default)]
pub struct BlockQuery {
    pub repo: Option<String>,
    pub roadmap: Option<String>,
    pub status: Option<BTreeSet<String>>,
    pub startable: Option<bool>,
    pub max_priority: Option<u8>,
}

/// The live/parked split of one block's transitive downstream cone — everything
/// that (directly or indirectly) depends on the seed block. Ranking consumers
/// (`--leverage`) use `live` only; `parked` is reported, never counted, per the
/// block record's regression case.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct BlockCone {
    pub live: BTreeSet<String>,
    pub parked: BTreeSet<String>,
}

impl BlockCone {
    /// The count ranking consumers must sort on — live members only.
    pub fn live_count(&self) -> usize {
        self.live.len()
    }
}

/// Whether a block's spec files exist on disk — a property of the filesystem,
/// distinct from [`BlockInfo::startable`] (a property of the dependency
/// graph). Three states are distinguishable, never collapsed into one flag:
/// both present, record only, or neither — the difference between one
/// `/generate-tasks` and a whole `/ticket`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct Readiness {
    /// `planning/blocks/<id>.json` exists under the owning repo's root.
    pub record: bool,
    /// `planning/<id>/tasks.json` exists under the owning repo's root.
    pub tasks: bool,
}

impl Readiness {
    /// `true` only when both the block record and its `tasks.json` exist.
    pub fn runnable(&self) -> bool {
        self.record && self.tasks
    }
}

/// Compute [`Readiness`] for block `id` under `repo_root` — the resolved
/// filesystem root of the block's OWNING repo (from `brain.toml`'s
/// `[[repos]]`, resolved by the caller). `repo_root: None` — an unresolvable
/// repo slug — degrades to [`Readiness::default`] (neither file present, so
/// [`Readiness::runnable`] is `false`) rather than erroring: an unknown repo
/// is a reporting gap, not a crash.
pub fn readiness_for(repo_root: Option<&Path>, id: &str) -> Readiness {
    let Some(root) = repo_root else {
        return Readiness::default();
    };
    let record = root
        .join("planning")
        .join("blocks")
        .join(format!("{id}.json"))
        .is_file();
    let tasks = root.join("planning").join(id).join("tasks.json").is_file();
    Readiness { record, tasks }
}

/// One selected block's full report row: its identity, its graph-derived
/// [`BlockInfo::startable`], and its disk-derived [`Readiness`] — kept as
/// separate keys (never folded into one flag) so `--json` lets a consumer
/// distinguish "blocked" from "no spec yet".
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct BlockRow {
    /// Canonical `"repo:id"` key.
    pub key: String,
    pub startable: bool,
    pub record: bool,
    pub tasks: bool,
    pub runnable: bool,
}

/// `mev blocks`' own JSON/text report shape. Populated by the verb (task 3);
/// declared now so the module's public surface is stable across tasks.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct QueryReport {
    /// One row per block matching the query, in the verb's chosen order.
    pub blocks: Vec<BlockRow>,
    /// `--leverage`: per selected block, its cone.
    pub cones: HashMap<String, BlockCone>,
    /// `--chain`: per startable head, the longest same-repo run from it.
    pub chains: HashMap<String, Vec<String>>,
}

/// Filter `blocks` against `query`. Every set field on `query` narrows the
/// result independently; an unset field imposes no constraint. `max_priority`
/// is inclusive (`priority <= max_priority`) and does not require `priority` to
/// be `Some` on its own — a block with no resolvable priority never matches a
/// `max_priority` filter, since there's nothing to compare.
///
pub fn select<'a>(blocks: &'a [BlockInfo], query: &BlockQuery) -> Vec<&'a BlockInfo> {
    blocks
        .iter()
        .filter(|b| {
            if let Some(repo) = &query.repo
                && &b.repo != repo
            {
                return false;
            }
            if let Some(roadmap) = &query.roadmap {
                match &b.roadmap {
                    Some(r) if r == roadmap => {}
                    _ => return false,
                }
            }
            if let Some(status) = &query.status
                && !status.contains(&b.status)
            {
                return false;
            }
            if let Some(startable) = query.startable
                && b.startable != startable
            {
                return false;
            }
            if let Some(max_priority) = query.max_priority {
                match b.priority {
                    Some(p) if p <= max_priority => {}
                    _ => return false,
                }
            }
            true
        })
        .collect()
}

/// The transitive downstream cone of `seed`: every block reachable by
/// following blocking `DepEdge`s outward (`from` depends on `to`, so a block
/// depending on `seed` is one hop out), split into `live` and `parked` by
/// [`BlockInfo::is_live`]. `seed` itself is never included. Terminates on a
/// cycle (a block already visited is never re-queued).
///
/// Seeds `crate::brain::availability::transitive_closure` with `{seed}` over
/// the blocking-only `dependents_of` adjacency derived from `edges` (a
/// `DepEdge { from, to, blocking: true }` means `from` depends on `to`, so
/// `to`'s dependents include `from`), then splits the resulting closure into
/// `live`/`parked` by [`BlockInfo::is_live`].
pub fn block_cone(
    seed: &str,
    edges: &[DepEdge<'_>],
    blocks: &HashMap<&str, &BlockInfo>,
) -> BlockCone {
    let mut dependents_of: HashMap<&str, HashSet<&str>> = HashMap::new();
    for edge in edges {
        if !edge.blocking {
            continue;
        }
        dependents_of.entry(edge.to).or_default().insert(edge.from);
    }

    let seed_set: HashSet<String> = std::iter::once(seed.to_string()).collect();
    let closure = crate::brain::availability::transitive_closure(&seed_set, &dependents_of);

    let mut cone = BlockCone::default();
    for key in closure {
        if key == seed {
            continue;
        }
        let is_live = blocks.get(key.as_str()).is_none_or(|b| b.is_live());
        if is_live {
            cone.live.insert(key);
        } else {
            cone.parked.insert(key);
        }
    }
    cone
}

/// The longest run of blocks reachable from `head` by following blocking
/// `DepEdge`s outward, refusing to step into a different repo (per `blocks`'
/// `repo` field) or through a parked block, and guarding against revisiting a
/// node so a cycle terminates. `head` is included as the first element when the
/// chain has any length at all.
///
pub fn same_repo_chain(
    head: &str,
    edges: &[DepEdge<'_>],
    blocks: &HashMap<&str, &BlockInfo>,
) -> Vec<String> {
    let Some(head_block) = blocks.get(head) else {
        return Vec::new();
    };
    let head_repo = head_block.repo.clone();

    let mut dependents_of: HashMap<&str, Vec<&str>> = HashMap::new();
    for edge in edges {
        if !edge.blocking {
            continue;
        }
        dependents_of.entry(edge.to).or_default().push(edge.from);
    }

    let mut visited: HashSet<&str> = HashSet::new();
    visited.insert(head);
    longest_same_repo_run(head, &dependents_of, blocks, &head_repo, &visited)
}

/// DFS helper for [`same_repo_chain`]: the longest run starting at `node`,
/// exploring every dependent branch and keeping the longest. `visited` guards
/// against revisiting a node on the current path so a cycle terminates rather
/// than looping.
fn longest_same_repo_run<'a>(
    node: &'a str,
    dependents_of: &HashMap<&'a str, Vec<&'a str>>,
    blocks: &HashMap<&str, &BlockInfo>,
    head_repo: &str,
    visited: &HashSet<&'a str>,
) -> Vec<String> {
    let mut best = vec![node.to_string()];
    let Some(deps) = dependents_of.get(node) else {
        return best;
    };
    for &dep in deps {
        if visited.contains(dep) {
            continue;
        }
        let Some(dep_block) = blocks.get(dep) else {
            continue;
        };
        if dep_block.repo != head_repo || !dep_block.is_live() {
            continue;
        }
        let mut visited_next = visited.clone();
        visited_next.insert(dep);
        let mut candidate = vec![node.to_string()];
        candidate.extend(longest_same_repo_run(
            dep,
            dependents_of,
            blocks,
            head_repo,
            &visited_next,
        ));
        if candidate.len() > best.len() {
            best = candidate;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn block(key: &str, status: &str, startable: bool, priority: Option<u8>) -> BlockInfo {
        let repo = key.split(':').next().unwrap_or(key).to_string();
        BlockInfo {
            key: key.to_string(),
            repo,
            status: status.to_string(),
            roadmap: None,
            startable,
            priority,
        }
    }

    fn block_with_roadmap(
        key: &str,
        status: &str,
        startable: bool,
        priority: Option<u8>,
        roadmap: &str,
    ) -> BlockInfo {
        let mut b = block(key, status, startable, priority);
        b.roadmap = Some(roadmap.to_string());
        b
    }

    fn as_map(blocks: &[BlockInfo]) -> HashMap<&str, &BlockInfo> {
        blocks.iter().map(|b| (b.key.as_str(), b)).collect()
    }

    // -----------------------------------------------------------------
    // block_cone
    // -----------------------------------------------------------------

    #[test]
    fn cone_is_transitive() {
        // A blocks B blocks C: A -> {from: B, to: A}, B -> {from: C, to: B}
        let edges = vec![
            DepEdge {
                from: "mev:B",
                to: "mev:A",
                blocking: true,
            },
            DepEdge {
                from: "mev:C",
                to: "mev:B",
                blocking: true,
            },
        ];
        let blocks = vec![
            block("mev:A", "open", true, None),
            block("mev:B", "open", false, None),
            block("mev:C", "open", false, None),
        ];
        let map = as_map(&blocks);

        let cone = block_cone("mev:A", &edges, &map);

        assert_eq!(
            cone.live_count(),
            2,
            "A's cone must be transitive (B AND C), not just the direct hop B; got {cone:?}"
        );
        assert!(cone.live.contains("mev:B"));
        assert!(cone.live.contains("mev:C"));
        assert!(!cone.live.contains("mev:A"), "seed must not include itself");
    }

    #[test]
    fn cone_terminates_on_cycle() {
        // A <-> B cycle: A depends on B, B depends on A.
        let edges = vec![
            DepEdge {
                from: "mev:A",
                to: "mev:B",
                blocking: true,
            },
            DepEdge {
                from: "mev:B",
                to: "mev:A",
                blocking: true,
            },
        ];
        let blocks = vec![
            block("mev:A", "open", true, None),
            block("mev:B", "open", false, None),
        ];
        let map = as_map(&blocks);

        // Must return, not hang, and must not include the seed itself.
        let cone = block_cone("mev:A", &edges, &map);
        assert!(
            !cone.live.contains("mev:A") && !cone.parked.contains("mev:A"),
            "cone must not include its own seed even inside a cycle; got {cone:?}"
        );
        assert_eq!(cone.live_count(), 1, "cycle cone must contain exactly B");
    }

    #[test]
    fn cone_ignores_non_blocking_edges() {
        let edges = vec![DepEdge {
            from: "mev:B",
            to: "mev:A",
            blocking: false, // e.g. an informational/reference edge
        }];
        let blocks = vec![
            block("mev:A", "open", true, None),
            block("mev:B", "open", false, None),
        ];
        let map = as_map(&blocks);

        let cone = block_cone("mev:A", &edges, &map);
        assert!(
            cone.live.is_empty() && cone.parked.is_empty(),
            "a non-blocking edge must contribute nothing to the cone; got {cone:?}"
        );
    }

    #[test]
    fn cone_all_parked_does_not_outrank_smaller_live_cone() {
        // Regression: a cone of 11 all-`deferred`/`wontfix` blocks must not
        // outrank a cone of 8 all-`open` blocks in a `--leverage` ordering,
        // because ranking sorts on `live_count()`, not on raw cone size.
        let mut edges = Vec::new();
        let mut blocks = vec![
            block("business:BIG", "open", true, None),
            block("jynx:SMALL", "open", true, None),
        ];
        for i in 0..11 {
            let key = format!("business:parked-{i}");
            edges.push(DepEdge {
                from: Box::leak(key.clone().into_boxed_str()),
                to: "business:BIG",
                blocking: true,
            });
            blocks.push(block(&key, "deferred", false, None));
        }
        for i in 0..8 {
            let key = format!("jynx:live-{i}");
            edges.push(DepEdge {
                from: Box::leak(key.clone().into_boxed_str()),
                to: "jynx:SMALL",
                blocking: true,
            });
            blocks.push(block(&key, "open", false, None));
        }
        let map = as_map(&blocks);

        let big_cone = block_cone("business:BIG", &edges, &map);
        let small_cone = block_cone("jynx:SMALL", &edges, &map);

        assert_eq!(big_cone.live_count(), 0, "all 11 dependents are parked");
        assert_eq!(big_cone.parked.len(), 11);
        assert_eq!(small_cone.live_count(), 8, "all 8 dependents are live");

        assert!(
            small_cone.live_count() > big_cone.live_count(),
            "the smaller all-live cone must outrank the larger all-parked cone \
             when ranking by live_count(): small={} big={}",
            small_cone.live_count(),
            big_cone.live_count()
        );
    }

    // -----------------------------------------------------------------
    // same_repo_chain
    // -----------------------------------------------------------------

    #[test]
    fn chain_follows_same_repo_dependents_only() {
        // mev:A -> mev:B -> mev:C, all same repo.
        let edges = vec![
            DepEdge {
                from: "mev:B",
                to: "mev:A",
                blocking: true,
            },
            DepEdge {
                from: "mev:C",
                to: "mev:B",
                blocking: true,
            },
        ];
        let blocks = vec![
            block("mev:A", "open", true, None),
            block("mev:B", "open", false, None),
            block("mev:C", "open", false, None),
        ];
        let map = as_map(&blocks);

        let chain = same_repo_chain("mev:A", &edges, &map);
        assert_eq!(
            chain,
            vec![
                "mev:A".to_string(),
                "mev:B".to_string(),
                "mev:C".to_string()
            ],
            "chain must follow the same-repo dependent run in order; got {chain:?}"
        );
    }

    #[test]
    fn chain_stops_at_cross_repo_dependent() {
        // mev:A -> mev:B -> other:C; the step into `other` must not extend the chain.
        let edges = vec![
            DepEdge {
                from: "mev:B",
                to: "mev:A",
                blocking: true,
            },
            DepEdge {
                from: "other:C",
                to: "mev:B",
                blocking: true,
            },
        ];
        let blocks = vec![
            block("mev:A", "open", true, None),
            block("mev:B", "open", false, None),
            block("other:C", "open", false, None),
        ];
        let map = as_map(&blocks);

        let chain = same_repo_chain("mev:A", &edges, &map);
        assert_eq!(
            chain,
            vec!["mev:A".to_string(), "mev:B".to_string()],
            "a cross-repo dependent must not extend the chain; got {chain:?}"
        );
    }

    #[test]
    fn chain_does_not_traverse_a_parked_block() {
        // mev:A -> mev:B (deferred) -> mev:C: the chain must not reach C through
        // parked B.
        let edges = vec![
            DepEdge {
                from: "mev:B",
                to: "mev:A",
                blocking: true,
            },
            DepEdge {
                from: "mev:C",
                to: "mev:B",
                blocking: true,
            },
        ];
        let blocks = vec![
            block("mev:A", "open", true, None),
            block("mev:B", "deferred", false, None),
            block("mev:C", "open", false, None),
        ];
        let map = as_map(&blocks);

        let chain = same_repo_chain("mev:A", &edges, &map);
        assert!(
            !chain.contains(&"mev:C".to_string()),
            "chain must not traverse through a parked block to reach C; got {chain:?}"
        );
    }

    #[test]
    fn chain_terminates_on_cycle() {
        let edges = vec![
            DepEdge {
                from: "mev:A",
                to: "mev:B",
                blocking: true,
            },
            DepEdge {
                from: "mev:B",
                to: "mev:A",
                blocking: true,
            },
        ];
        let blocks = vec![
            block("mev:A", "open", true, None),
            block("mev:B", "open", false, None),
        ];
        let map = as_map(&blocks);

        // Must return rather than hang; must not contain duplicates.
        let chain = same_repo_chain("mev:A", &edges, &map);
        let unique: HashSet<&String> = chain.iter().collect();
        assert_eq!(
            unique.len(),
            chain.len(),
            "a cycle must not produce a chain with repeated members; got {chain:?}"
        );
    }

    // -----------------------------------------------------------------
    // select / BlockQuery
    // -----------------------------------------------------------------

    #[test]
    fn filters_compose_repo_and_startable_and_status() {
        let blocks = vec![
            block("mev:A", "open", true, None),
            block("mev:B", "open", false, None),
            block("other:C", "open", true, None),
            block("mev:D", "closed", true, None),
        ];
        let mut status = BTreeSet::new();
        status.insert("open".to_string());
        let query = BlockQuery {
            repo: Some("mev".to_string()),
            startable: Some(true),
            status: Some(status),
            ..Default::default()
        };

        let result = select(&blocks, &query);
        let keys: BTreeSet<&str> = result.iter().map(|b| b.key.as_str()).collect();

        assert_eq!(
            keys,
            BTreeSet::from(["mev:A"]),
            "repo AND startable AND status must all hold at once; got {keys:?}"
        );
    }

    #[test]
    fn max_priority_is_inclusive_and_does_not_imply_a_status_filter() {
        let blocks = vec![
            block("mev:A", "open", false, Some(1)),
            block("mev:B", "closed", false, Some(1)), // matches priority, not status-filtered
            block("mev:C", "open", false, Some(2)),   // exceeds max_priority
            block("mev:D", "open", false, None),      // unresolvable priority never matches
        ];
        let query = BlockQuery {
            max_priority: Some(1),
            ..Default::default()
        };

        let result = select(&blocks, &query);
        let keys: BTreeSet<&str> = result.iter().map(|b| b.key.as_str()).collect();

        assert_eq!(
            keys,
            BTreeSet::from(["mev:A", "mev:B"]),
            "max_priority=1 must be inclusive (<=1) and independent of status; \
             D (no priority) must never match; got {keys:?}"
        );
    }

    #[test]
    fn roadmap_filter_with_no_membership_index_matches_nothing() {
        // No block in this fixture carries `roadmap`, simulating a corpus with no
        // roadmap membership index available for the requested roadmap.
        let blocks = vec![
            block("mev:A", "open", true, None),
            block("mev:B", "open", true, None),
        ];
        let query = BlockQuery {
            roadmap: Some("nonexistent-roadmap".to_string()),
            ..Default::default()
        };

        let result = select(&blocks, &query);
        assert!(
            result.is_empty(),
            "an unresolvable roadmap filter must match nothing, never fall back \
             to the unfiltered set; got {:?}",
            result.iter().map(|b| &b.key).collect::<Vec<_>>()
        );
    }

    #[test]
    fn roadmap_filter_matches_only_members_of_that_roadmap() {
        let blocks = vec![
            block_with_roadmap("mev:A", "open", true, None, "fleet-integrity"),
            block_with_roadmap("mev:B", "open", true, None, "other-roadmap"),
        ];
        let query = BlockQuery {
            roadmap: Some("fleet-integrity".to_string()),
            ..Default::default()
        };

        let result = select(&blocks, &query);
        let keys: BTreeSet<&str> = result.iter().map(|b| b.key.as_str()).collect();
        assert_eq!(keys, BTreeSet::from(["mev:A"]));
    }

    // -----------------------------------------------------------------
    // readiness_for — MV.ticket.query-verb-leverage-chain-and-filters AC 14-16
    // -----------------------------------------------------------------

    #[test]
    fn readiness_both_present_is_runnable() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("planning/blocks")).unwrap();
        std::fs::write(dir.path().join("planning/blocks/A.1.json"), "{}").unwrap();
        std::fs::create_dir_all(dir.path().join("planning/A.1")).unwrap();
        std::fs::write(dir.path().join("planning/A.1/tasks.json"), "[]").unwrap();

        let r = readiness_for(Some(dir.path()), "A.1");
        assert!(r.record);
        assert!(r.tasks);
        assert!(r.runnable());
    }

    #[test]
    fn readiness_record_only_is_not_runnable() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("planning/blocks")).unwrap();
        std::fs::write(dir.path().join("planning/blocks/A.1.json"), "{}").unwrap();

        let r = readiness_for(Some(dir.path()), "A.1");
        assert!(r.record);
        assert!(!r.tasks);
        assert!(!r.runnable());
    }

    #[test]
    fn readiness_neither_present_is_not_runnable() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("planning")).unwrap();

        let r = readiness_for(Some(dir.path()), "A.1");
        assert!(!r.record);
        assert!(!r.tasks);
        assert!(!r.runnable());
    }

    #[test]
    fn readiness_unresolvable_repo_degrades_to_not_runnable_rather_than_erroring() {
        let r = readiness_for(None, "A.1");
        assert!(!r.record);
        assert!(!r.tasks);
        assert!(!r.runnable());
    }
}
