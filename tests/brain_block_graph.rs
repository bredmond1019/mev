//! Integration tests for `mev::block_graph_brain` — the enriched block-graph exporter
//! driver (Phase 10, Block MV.10.B, Task 4).
//!
//! Mirrors the structure and fixture style of `tests/brain_graph_emit.rs`: a synthetic
//! multi-repo brain fixture on a temp dir, driven entirely through the public
//! `mev::block_graph_brain` API (never the internal `build_block_graph_export` builder),
//! so the corpus-discovery + load pipeline is exercised too, not just the enrichment.
//!
//! Fixture shape (`brain.toml`):
//! - HQ root (`"hq"`, `repo_path = "."`) with an `epics[]` registry (`"epic-x"`) and
//!   `tiers[]` pointing at two tier sub-brains: `"core"` (`alpha`, `beta`) and
//!   `"portfolio"` (`gamma`).
//! - `core/planning/state.json` is a **dual-role** brain file: `kind: "brain"` (a tier
//!   rollup) that *also* carries its own `tracks[]` (block `CORE.1`).
//! - `alpha`: `A1` (open, `epics: ["epic-x"]`, one `{type:"external"}` dep), `A2`
//!   (closed), `A3` (open, cross-repo `depends_on` on `gamma:G1`), `A4` (open, a
//!   **dangling** `depends_on` on `alpha:GHOST`, which does not exist anywhere).
//! - `beta`: `B1` (in_progress), `B2` (deferred).
//! - `gamma` (tier `portfolio`): `G1` (closed, `epics: ["epic-x"]`), `G2` (open,
//!   `depends_on` on the closed `gamma:G1`).

use std::fs;
use std::path::Path;

use mev::brain::state::TierScope;
use mev::{BlockGraphScope, BlockLane, block_graph_brain};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn write_file(root: &Path, rel: &str, content: &str) {
    let full = root.join(rel);
    fs::create_dir_all(full.parent().unwrap()).unwrap();
    fs::write(&full, content.as_bytes()).unwrap();
}

fn write_json(root: &Path, rel: &str, value: &serde_json::Value) {
    write_file(root, rel, &serde_json::to_string_pretty(value).unwrap());
}

fn temp_dir(suffix: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("mev-brain-block-graph-{suffix}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// `default_scope()` — unscoped (`TierScope::All`), closed included, no boundary,
/// unlimited `max_nodes` — the baseline every test starts from and tweaks.
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

fn write_brain_toml(root: &Path) {
    let toml = r#"[vocab]
layer = ["brain", "engine", "factory", "console", "surface", "infra", "business", "content", "meta"]
status = ["active", "draft", "deprecated", "superseded", "archived"]

[crawl]
skip_dirs = ["target", "node_modules", ".git"]

[[repos]]
slug = "hq"
tier = "_root"
repo_path = "."
status_file = "planning/status.md"
cache_doc = "docs/projects/hq.md"
heading = "HQ"

[[repos]]
slug = "core"
tier = "_root"
repo_path = "core"
status_file = "core/planning/status.md"
cache_doc = "docs/projects/core.md"
heading = "Core Tier"

[[repos]]
slug = "alpha"
tier = "core"
repo_path = "alpha"
status_file = "alpha/planning/status.md"
cache_doc = "docs/projects/alpha.md"
heading = "Alpha"

[[repos]]
slug = "beta"
tier = "core"
repo_path = "beta"
status_file = "beta/planning/status.md"
cache_doc = "docs/projects/beta.md"
heading = "Beta"

[[repos]]
slug = "portfolio"
tier = "_root"
repo_path = "portfolio"
status_file = "portfolio/planning/status.md"
cache_doc = "docs/projects/portfolio.md"
heading = "Portfolio Tier"

[[repos]]
slug = "gamma"
tier = "portfolio"
repo_path = "gamma"
status_file = "gamma/planning/status.md"
cache_doc = "docs/projects/gamma.md"
heading = "Gamma"
"#;
    fs::write(root.join("brain.toml"), toml.as_bytes()).unwrap();
}

/// HQ root `planning/state.json` — `tiers[]` pointing at both sub-brains, plus the
/// `epics[]` registry (`"epic-x"`, spanning `alpha` and `gamma`).
fn write_hq_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "hq",
        "kind": "brain",
        "updated": "2026-07-28",
        "focus": { "now": [], "next": [], "blocked": [] },
        "repos": [],
        "cross_repo": [],
        "tiers": [
            { "tier": "core", "rollup": "core/planning/state.json", "summary": null },
            { "tier": "portfolio", "rollup": "portfolio/planning/state.json", "summary": null }
        ],
        "epics": [
            {
                "slug": "epic-x",
                "title": "Epic X",
                "description": "Cross-repo initiative spanning alpha and gamma.",
                "status": "active",
                "plan": null,
                "repos": ["alpha", "gamma"]
            }
        ]
    });
    write_json(root, "planning/state.json", &state);
}

/// The `"core"` tier sub-brain — DUAL-ROLE: `kind: "brain"` (a tier rollup file) that
/// also carries its own `tracks[]` with one block (`CORE.1`).
fn write_core_tier_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "core",
        "kind": "brain",
        "updated": "2026-07-28",
        "focus": { "now": [], "next": [], "blocked": [] },
        "repos": [],
        "cross_repo": [],
        "tracks": [
            {
                "title": "Core Tier Track",
                "blocks": [
                    { "id": "CORE.1", "title": "Core dual-role block", "status": "in_progress" }
                ]
            }
        ]
    });
    write_json(root, "core/planning/state.json", &state);
}

/// The `"portfolio"` tier sub-brain — a plain rollup, no `tracks[]` of its own.
fn write_portfolio_tier_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "portfolio",
        "kind": "brain",
        "updated": "2026-07-28",
        "focus": { "now": [], "next": [], "blocked": [] },
        "repos": [],
        "cross_repo": []
    });
    write_json(root, "portfolio/planning/state.json", &state);
}

/// `alpha` leaf — four blocks: `A1` (open, epic member, one external dep), `A2`
/// (closed), `A3` (open, cross-repo dep on `gamma:G1`), `A4` (open, a DANGLING dep on
/// `alpha:GHOST`, which is never defined anywhere in the fixture).
fn write_alpha_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "alpha",
        "kind": "project",
        "updated": "2026-07-28",
        "focus": { "now": [], "next": [], "blocked": [] },
        "tracks": [
            {
                "title": "Phase 1",
                "blocks": [
                    {
                        "id": "A1",
                        "title": "Alpha 1",
                        "status": "open",
                        "epics": ["epic-x"],
                        "depends_on": [
                            { "type": "external", "what": "waiting on vendor API" }
                        ]
                    },
                    { "id": "A2", "title": "Alpha 2", "status": "closed" },
                    {
                        "id": "A3",
                        "title": "Alpha 3",
                        "status": "open",
                        "depends_on": [
                            { "type": "block", "repo": "gamma", "id": "G1" }
                        ]
                    },
                    {
                        "id": "A4",
                        "title": "Alpha 4",
                        "status": "open",
                        "depends_on": [
                            { "type": "block", "repo": "alpha", "id": "GHOST" }
                        ]
                    }
                ]
            }
        ]
    });
    write_json(root, "alpha/planning/state.json", &state);
}

/// `beta` leaf — `B1` (in_progress), `B2` (deferred).
fn write_beta_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "beta",
        "kind": "project",
        "updated": "2026-07-28",
        "focus": { "now": [], "next": [], "blocked": [] },
        "tracks": [
            {
                "title": "Phase 1",
                "blocks": [
                    { "id": "B1", "title": "Beta 1", "status": "in_progress" },
                    { "id": "B2", "title": "Beta 2", "status": "deferred" }
                ]
            }
        ]
    });
    write_json(root, "beta/planning/state.json", &state);
}

/// `gamma` leaf (tier `portfolio`) — `G1` (closed, epic member), `G2` (open, depends
/// on the closed `G1`).
fn write_gamma_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "gamma",
        "kind": "project",
        "updated": "2026-07-28",
        "focus": { "now": [], "next": [], "blocked": [] },
        "tracks": [
            {
                "title": "Phase 1",
                "blocks": [
                    { "id": "G1", "title": "Gamma 1", "status": "closed", "epics": ["epic-x"] },
                    {
                        "id": "G2",
                        "title": "Gamma 2",
                        "status": "open",
                        "depends_on": [
                            { "type": "block", "repo": "gamma", "id": "G1" }
                        ]
                    }
                ]
            }
        ]
    });
    write_json(root, "gamma/planning/state.json", &state);
}

/// Build the complete fixture and return its temp-dir root.
fn build_fixture(suffix: &str) -> std::path::PathBuf {
    let dir = temp_dir(suffix);
    write_brain_toml(&dir);
    write_hq_state(&dir);
    write_core_tier_state(&dir);
    write_portfolio_tier_state(&dir);
    write_alpha_state(&dir);
    write_beta_state(&dir);
    write_gamma_state(&dir);
    dir
}

fn node_keys(export: &mev::BlockGraphExport) -> Vec<String> {
    let mut keys: Vec<String> = export.nodes.iter().map(|n| n.key.clone()).collect();
    keys.sort();
    keys
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// HQ scope (`TierScope::All`, unfiltered) returns every repo's blocks — all nine:
/// `alpha` (4), `beta` (2), `gamma` (2), and the dual-role `core` tier's own `CORE.1`.
#[test]
fn hq_scope_returns_every_repos_blocks() {
    let dir = build_fixture("hq-scope");

    let export = block_graph_brain(&dir, &default_scope()).expect("block_graph_brain must succeed");

    assert_eq!(
        node_keys(&export),
        vec![
            "alpha:A1",
            "alpha:A2",
            "alpha:A3",
            "alpha:A4",
            "beta:B1",
            "beta:B2",
            "core:CORE.1",
            "gamma:G1",
            "gamma:G2",
        ]
    );
    assert_eq!(export.total_nodes, 9);
    assert!(!export.truncated);

    let _ = fs::remove_dir_all(&dir);
}

/// `TierScope::Tier("core")` limits the export to `alpha` and `beta` — the tier
/// *container*'s own dual-role block (`core:CORE.1`) is not itself tagged `tier =
/// "core"` in `brain.toml` (its `[[repos]]` entry carries `tier = "_root"`, matching
/// every other tier-container self-entry), so it is correctly excluded, and `gamma`
/// (tier `portfolio`) is excluded too.
#[test]
fn tier_scope_limits_to_repos_in_that_tier() {
    let dir = build_fixture("tier-scope");

    let mut scope = default_scope();
    scope.tier = TierScope::Tier("core".to_string());
    let export = block_graph_brain(&dir, &scope).expect("block_graph_brain must succeed");

    assert_eq!(
        node_keys(&export),
        vec![
            "alpha:A1", "alpha:A2", "alpha:A3", "alpha:A4", "beta:B1", "beta:B2"
        ]
    );
    assert_eq!(export.scope.tier, Some("core".to_string()));

    let _ = fs::remove_dir_all(&dir);
}

/// A single-repo scope narrows to exactly that slug's blocks.
#[test]
fn repo_scope_narrows_to_one_slug() {
    let dir = build_fixture("repo-scope");

    let mut scope = default_scope();
    scope.repo = Some("alpha".to_string());
    let export = block_graph_brain(&dir, &scope).expect("block_graph_brain must succeed");

    assert_eq!(
        node_keys(&export),
        vec!["alpha:A1", "alpha:A2", "alpha:A3", "alpha:A4"]
    );
    assert_eq!(export.scope.repo, Some("alpha".to_string()));

    let _ = fs::remove_dir_all(&dir);
}

/// Epic scope returns exactly that epic's members (`alpha:A1`, `gamma:G1`) and
/// OVERRIDES tier — `scope.tier` is pinned to `"core"` (which alone would exclude
/// `gamma` entirely), yet `gamma:G1` still appears because epic scope takes priority.
#[test]
fn epic_scope_overrides_tier() {
    let dir = build_fixture("epic-scope");

    let mut scope = default_scope();
    scope.tier = TierScope::Tier("core".to_string());
    scope.epic = Some("epic-x".to_string());
    let export = block_graph_brain(&dir, &scope).expect("block_graph_brain must succeed");

    assert_eq!(node_keys(&export), vec!["alpha:A1", "gamma:G1"]);
    assert_eq!(export.scope.epic, Some("epic-x".to_string()));

    let _ = fs::remove_dir_all(&dir);
}

/// `include_closed` toggles whether `Closed`-lane nodes (`alpha:A2`, `gamma:G1`) are
/// retained — a difference of exactly two nodes across the full corpus.
#[test]
fn include_closed_toggle_changes_node_count() {
    let dir = build_fixture("include-closed");

    let mut with_closed = default_scope();
    with_closed.include_closed = true;
    let export_with =
        block_graph_brain(&dir, &with_closed).expect("block_graph_brain must succeed");
    assert_eq!(export_with.nodes.len(), 9);

    let mut without_closed = default_scope();
    without_closed.include_closed = false;
    let export_without =
        block_graph_brain(&dir, &without_closed).expect("block_graph_brain must succeed");
    assert_eq!(export_without.nodes.len(), 7);
    assert!(
        export_without
            .nodes
            .iter()
            .all(|n| n.lane != BlockLane::Closed)
    );

    let _ = fs::remove_dir_all(&dir);
}

/// `include_boundary` re-adds a direct neighbour of the in-scope set, flagged
/// `in_scope: false` — scoping to `alpha` alone drops `gamma:G1` (the target of
/// `alpha:A3`'s cross-repo dep); boundary re-adds it and retains the edge.
#[test]
fn include_boundary_adds_out_of_scope_neighbour_and_retains_edge() {
    let dir = build_fixture("include-boundary");

    let mut scope = default_scope();
    scope.repo = Some("alpha".to_string());
    scope.include_boundary = false;
    let export_no_boundary =
        block_graph_brain(&dir, &scope).expect("block_graph_brain must succeed");
    assert_eq!(export_no_boundary.nodes.len(), 4);
    assert!(
        export_no_boundary
            .edges
            .iter()
            .all(|e| e.to_ref != "gamma:G1")
    );

    scope.include_boundary = true;
    let export_boundary = block_graph_brain(&dir, &scope).expect("block_graph_brain must succeed");
    assert_eq!(export_boundary.nodes.len(), 5);
    let boundary_node = export_boundary
        .nodes
        .iter()
        .find(|n| n.key == "gamma:G1")
        .expect("boundary node re-added");
    assert!(!boundary_node.in_scope);
    assert!(
        export_boundary
            .nodes
            .iter()
            .filter(|n| n.key == "alpha:A3")
            .all(|n| n.in_scope),
        "surviving alpha nodes keep in_scope: true"
    );
    assert!(
        export_boundary
            .edges
            .iter()
            .any(|e| e.from == "alpha:A3" && e.to_ref == "gamma:G1"),
        "boundary edge retained"
    );

    let _ = fs::remove_dir_all(&dir);
}

/// `max_nodes` truncates the (topo-ordered) node list and sets `truncated`, while
/// `total_nodes` holds the PRE-truncation count.
#[test]
fn max_nodes_sets_truncated_with_pre_truncation_total() {
    let dir = build_fixture("max-nodes");

    let mut scope = default_scope();
    scope.max_nodes = 3;
    let export = block_graph_brain(&dir, &scope).expect("block_graph_brain must succeed");

    assert_eq!(export.total_nodes, 9);
    assert!(export.truncated);
    assert_eq!(export.nodes.len(), 3);

    let mut untruncated = default_scope();
    untruncated.max_nodes = usize::MAX;
    let export_full =
        block_graph_brain(&dir, &untruncated).expect("block_graph_brain must succeed");
    assert!(!export_full.truncated);
    assert_eq!(export_full.total_nodes, 9);
    assert_eq!(export_full.nodes.len(), 9);

    let _ = fs::remove_dir_all(&dir);
}

/// `max_nodes` truncation re-filters `edges` against the truncated node set: an edge
/// surviving Stage 6 (both endpoints in scope) must still be dropped at Stage 7 if
/// truncation removed one of its endpoints from the returned `nodes` list. Otherwise a
/// consumer would receive an edge naming a node it was never sent.
#[test]
fn max_nodes_truncation_drops_edges_whose_endpoints_were_truncated_out() {
    let dir = build_fixture("max-nodes-edges");

    let mut scope = default_scope();
    scope.max_nodes = 1;
    let export = block_graph_brain(&dir, &scope).expect("block_graph_brain must succeed");

    assert_eq!(export.nodes.len(), 1);
    let kept_key = export.nodes[0].key.clone();

    // With only one node kept, every surviving edge must have BOTH endpoints equal to
    // that single node — impossible in this fixture (no self-loops) — so edges must be
    // empty. Every edge in this fixture (A3->G1, A4->alpha:GHOST, G2->G1) has distinct
    // from/to, so this also proves the fix actually filters rather than no-op'ing.
    assert!(
        export.edges.is_empty(),
        "expected no edges to survive a single-node truncation, kept node was {kept_key}, got: {:?}",
        export.edges
    );

    let _ = fs::remove_dir_all(&dir);
}

/// DETERMINISM: two consecutive `block_graph_brain` builds over an unchanged fixture
/// serialize BYTE-IDENTICAL. This relies on `discover_state_files`' stable ordering
/// (HQ root, then tier sub-brains in `tiers[]` order, then `[[repos]]` leaves in
/// `brain.toml` order) — the same ordering `emit_state` already depends on for its own
/// byte-identical output.
#[test]
fn two_consecutive_builds_are_byte_identical() {
    let dir = build_fixture("determinism");

    let first = block_graph_brain(&dir, &default_scope()).expect("first build must succeed");
    let second = block_graph_brain(&dir, &default_scope()).expect("second build must succeed");

    let first_json = serde_json::to_string(&first).expect("first export must serialize");
    let second_json = serde_json::to_string(&second).expect("second export must serialize");

    assert_eq!(
        first_json, second_json,
        "two builds over an unchanged corpus must be byte-identical"
    );

    let _ = fs::remove_dir_all(&dir);
}

/// DUAL-ROLE BRAIN: `core/planning/state.json` is `kind: "brain"` (a tier rollup) that
/// also carries its own `tracks[]`. Its block (`core:CORE.1`, authored
/// `status: "in_progress"`) must appear exactly once, in lane `Now` — the corpus-wide
/// enrichment merge must not drop it (because the file is `kind: "brain"`) nor
/// double-count it (because it is also registered as a tier rollup).
#[test]
fn dual_role_brain_fixture_produces_correct_lane() {
    let dir = build_fixture("dual-role");

    let export = block_graph_brain(&dir, &default_scope()).expect("block_graph_brain must succeed");

    let core_nodes: Vec<_> = export
        .nodes
        .iter()
        .filter(|n| n.key == "core:CORE.1")
        .collect();
    assert_eq!(
        core_nodes.len(),
        1,
        "the dual-role brain's own block must appear exactly once"
    );
    assert_eq!(core_nodes[0].lane, BlockLane::Now);

    let _ = fs::remove_dir_all(&dir);
}

/// `dependent_count` is corpus-wide and scope-stable: `gamma:G1` has two dependents
/// (`alpha:A3`, a cross-repo `BlockedBy`, and `gamma:G2`, same-repo) — that count must
/// be identical whether the export is unscoped (`hq`) or scoped down to the `portfolio`
/// tier (which still contains `gamma`), the load-bearing scope-stability assertion for
/// MV.10.C task 1.
#[test]
fn dependent_count_identical_across_scoped_and_unscoped_export() {
    let dir = build_fixture("dependent-count-scope-stability");

    let unscoped = block_graph_brain(&dir, &default_scope()).expect("unscoped build must succeed");
    let unscoped_g1 = unscoped
        .nodes
        .iter()
        .find(|n| n.key == "gamma:G1")
        .expect("gamma:G1 present in unscoped export");
    assert_eq!(
        unscoped_g1.dependent_count, 2,
        "gamma:G1 has two BlockedBy dependents: alpha:A3 and gamma:G2"
    );

    let mut scope = default_scope();
    scope.tier = TierScope::Tier("portfolio".to_string());
    let scoped = block_graph_brain(&dir, &scope).expect("portfolio-tier-scoped build must succeed");
    let scoped_g1 = scoped
        .nodes
        .iter()
        .find(|n| n.key == "gamma:G1")
        .expect("gamma:G1 present in portfolio-tier-scoped export");

    assert_eq!(
        scoped_g1.dependent_count, unscoped_g1.dependent_count,
        "dependent_count must be identical across scoped and unscoped exports"
    );

    let _ = fs::remove_dir_all(&dir);
}

/// DANGLING EDGE: `alpha:A4`'s `depends_on` targets `alpha:GHOST`, which is never
/// defined anywhere in the fixture. The edge is present in `edges` with
/// `target_node_id: None` — retained, never dropped, and no node is synthesized for it.
#[test]
fn dangling_edge_present_with_none_target() {
    let dir = build_fixture("dangling-edge");

    let export = block_graph_brain(&dir, &default_scope()).expect("block_graph_brain must succeed");

    let dangling = export
        .edges
        .iter()
        .find(|e| e.from == "alpha:A4" && e.to_ref == "alpha:GHOST")
        .expect("dangling edge must be present");
    assert_eq!(dangling.target_node_id, None);
    assert!(
        export.nodes.iter().all(|n| n.key != "alpha:GHOST"),
        "no synthetic node for the dangling target"
    );

    let _ = fs::remove_dir_all(&dir);
}
