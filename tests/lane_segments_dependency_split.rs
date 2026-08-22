//! Fixture-evidence integration test — `MV.ticket.lane-segmentation-ignores-dependencies`
//! Task 2.
//!
//! Task 1 added [`mev::brain::lane_segments::split_segment_on_unmet_dependencies`], which
//! further splits a repo-grouped [`mev::brain::lane_segments::LaneSegment`] at any mid-run
//! block carrying a real, unmet dependency. This test is the fixture-evidence GATE the
//! block's own spec names: a same-repo lane whose middle block has an open, unmet,
//! cross-repo `depends_on` edge must report as **2** segments (the real barrier surfaced),
//! and the identical fixture with that dependency closed must report as **1** segment
//! (lane order stays load-bearing once the blocker is gone).
//!
//! **This is the shown-failing case the block's spec names as its GATE.** Before Task 1's
//! split logic existed, `derive_lane_positions` segmented purely on repo-change
//! (`segment_lane_blocks`) — a 3-block, single-repo lane always produced exactly 1
//! segment, cross-repo dependency or not, because nothing downstream of
//! `segment_lane_blocks` ever inspected `depends_on`. Both fixtures below (dependency
//! open, dependency closed) would have reported 1 segment under that old behaviour; only
//! Task 1's split makes the open-dependency fixture report 2.
//!
//! Follows the fixture-directory pattern `tests/lane_segments_fleet.rs` and
//! `tests/lanes_driver.rs` already use: a throwaway temp dir with a minimal `brain.toml`
//! registering two leaf repos, each repo's `planning/state.json`, and one
//! `planning/roadmaps/<slug>/lane-<name>.json` record. Discovery and derivation run
//! through the same public seam `mev::frontier_brain`/`mev::lanes_brain` use
//! (`discover_state_files` + `load_state` + `discover_lane_files` +
//! `derive_lane_positions`), not a hand-rolled parse.

use std::fs;
use std::path::Path;

use mev::brain::config::find_brain_config;
use mev::brain::lane_segments::{derive_lane_positions, discover_lane_files};
use mev::brain::state::{discover_state_files, load_state};

/// Minimal `brain.toml` registering the two leaf repos the fixture lane spans.
fn write_brain_toml(root: &Path) {
    let toml = r#"[vocab]
layer = ["brain", "engine", "factory", "console", "surface", "infra", "business", "content", "meta"]
status = ["active", "draft", "deprecated", "superseded", "archived"]

[crawl]
skip_dirs = ["target", "node_modules", ".git"]

[[repos]]
slug = "alpha"
tier = "primary"
repo_path = "repos/alpha"
status_file = "repos/alpha/planning/status.md"
cache_doc = "docs/projects/alpha.md"
heading = "Alpha"

[[repos]]
slug = "beta"
tier = "primary"
repo_path = "repos/beta"
status_file = "repos/beta/planning/status.md"
cache_doc = "docs/projects/beta.md"
heading = "Beta"
"#;
    fs::write(root.join("brain.toml"), toml.as_bytes()).unwrap();
}

fn write_json(root: &Path, rel: &str, value: &serde_json::Value) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        path,
        serde_json::to_string_pretty(value).unwrap().as_bytes(),
    )
    .unwrap();
}

/// Write the full fixture corpus: a three-block, all-`alpha` lane whose middle block
/// (`AL.1.B`) carries a `depends_on` block edge to `beta:BE.1.A`, plus both repos'
/// `planning/state.json`. `beta_target_status` lets the caller flip the dependency
/// target between `"open"` (unmet — must split) and `"closed"` (satisfied — must not).
fn write_corpus(root: &Path, beta_target_status: &str) {
    write_brain_toml(root);

    write_json(
        root,
        "planning/state.json",
        &serde_json::json!({
            "repo": "hq",
            "kind": "brain",
            "updated": "2026-08-22",
            "focus": { "now": [], "next": [], "blocked": [] },
            "repos": [],
            "cross_repo": []
        }),
    );

    write_json(
        root,
        "repos/alpha/planning/state.json",
        &serde_json::json!({
            "repo": "alpha",
            "kind": "project",
            "updated": "2026-08-22",
            "focus": { "now": [], "next": [], "blocked": [] },
            "repos": [],
            "cross_repo": [],
            "tracks": [{
                "title": "Phase 1",
                "blocks": [
                    { "id": "AL.1.A", "title": "First", "status": "open" },
                    {
                        "id": "AL.1.B",
                        "title": "Mid-run, waits on beta",
                        "status": "open",
                        "depends_on": [
                            { "type": "block", "repo": "beta", "id": "BE.1.A" }
                        ]
                    },
                    { "id": "AL.1.C", "title": "Third", "status": "open" }
                ]
            }]
        }),
    );

    write_json(
        root,
        "repos/beta/planning/state.json",
        &serde_json::json!({
            "repo": "beta",
            "kind": "project",
            "updated": "2026-08-22",
            "focus": { "now": [], "next": [], "blocked": [] },
            "repos": [],
            "cross_repo": [],
            "tracks": [{
                "title": "Phase 1",
                "blocks": [
                    { "id": "BE.1.A", "title": "Dependency target", "status": beta_target_status }
                ]
            }]
        }),
    );

    // All three blocks are authored in one all-alpha lane — no ownership change, so
    // segment_lane_blocks alone would report exactly one segment regardless of the
    // depends_on edge above.
    write_json(
        root,
        "planning/roadmaps/fixture/lane-only.json",
        &serde_json::json!({
            "lane": "only",
            "roadmap": "fixture",
            "blocks": [
                { "id": "AL.1.A", "origin_roadmap": "fixture", "repo": "alpha" },
                { "id": "AL.1.B", "origin_roadmap": "fixture", "repo": "alpha" },
                { "id": "AL.1.C", "origin_roadmap": "fixture", "repo": "alpha" }
            ]
        }),
    );
}

/// Discover + load state, discover lane files, and derive lane positions — the same
/// seam `mev::frontier_brain`/`mev::lanes_brain` use internally.
fn derive(root: &Path) -> Vec<mev::brain::lane_segments::DerivedBlockPosition> {
    let config = find_brain_config(root).expect("fixture brain.toml must parse");
    let (sources, discovery_diags) = discover_state_files(root, &config);
    assert!(
        discovery_diags.is_empty(),
        "expected no state-discovery diagnostics, got {discovery_diags:?}"
    );

    let mut loaded = Vec::new();
    for src in &sources {
        let file = load_state(&src.abs_path)
            .unwrap_or_else(|e| panic!("fixture state.json at {:?} must load: {e}", src.abs_path));
        loaded.push((src.clone(), file));
    }

    let (lane_files, discover_diags) = discover_lane_files(root);
    assert!(
        discover_diags.is_empty(),
        "expected no lane-discovery diagnostics, got {discover_diags:?}"
    );
    assert_eq!(
        lane_files.len(),
        1,
        "expected exactly one discovered lane file"
    );

    let (positions, derive_diags) = derive_lane_positions(&lane_files, &loaded);
    assert!(
        derive_diags.is_empty(),
        "expected no derivation diagnostics, got {derive_diags:?}"
    );
    positions
}

#[test]
fn mid_run_open_cross_repo_dependency_splits_into_two_segments() {
    let dir = mev::testsupport::unique_temp_dir("mev-lane-segments-dependency-split-open");
    fs::create_dir_all(&dir).expect("create fixture dir");
    write_corpus(&dir, "open");

    let positions = derive(&dir);
    assert_eq!(
        positions.len(),
        3,
        "all 3 blocks must render: {positions:?}"
    );

    let by_id: std::collections::HashMap<&str, &mev::brain::lane_segments::DerivedBlockPosition> =
        positions.iter().map(|p| (p.id.as_str(), p)).collect();

    let a = by_id["AL.1.A"];
    let b = by_id["AL.1.B"];
    let c = by_id["AL.1.C"];

    let segment_count = positions.iter().map(|p| p.segment).max().unwrap() + 1;
    assert_eq!(
        segment_count, 2,
        "an open, unmet cross-repo dependency on the mid-run block must split the lane \
         into exactly 2 segments, got positions: {positions:?}"
    );

    assert_eq!(a.segment, 0, "AL.1.A stays alone in the first segment");
    assert_eq!(a.position, 0);
    assert_eq!(
        b.segment, 1,
        "AL.1.B starts the new segment its unmet dependency forces"
    );
    assert_eq!(
        b.position, 0,
        "position renumbers 0-based within the new segment"
    );
    assert_eq!(
        c.segment, 1,
        "AL.1.C stays with AL.1.B in the second segment"
    );
    assert_eq!(c.position, 1);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn identical_fixture_with_dependency_closed_yields_exactly_one_segment() {
    let dir = mev::testsupport::unique_temp_dir("mev-lane-segments-dependency-split-closed");
    fs::create_dir_all(&dir).expect("create fixture dir");
    write_corpus(&dir, "closed");

    let positions = derive(&dir);
    assert_eq!(
        positions.len(),
        3,
        "all 3 blocks must render: {positions:?}"
    );

    let segment_count = positions.iter().map(|p| p.segment).max().unwrap() + 1;
    assert_eq!(
        segment_count, 1,
        "a closed dependency target must not split the lane — lane order stays \
         load-bearing once the blocker is gone, got positions: {positions:?}"
    );

    for (i, p) in positions.iter().enumerate() {
        assert_eq!(p.segment, 0, "single segment: {p:?}");
        assert_eq!(
            p.position, i,
            "positions stay 0-based and contiguous: {p:?}"
        );
    }

    let _ = fs::remove_dir_all(&dir);
}
