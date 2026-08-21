//! Real-worst-case segmentation gate — `MV.13.A` Task 2, re-pointed at the `lane.json`
//! fixture by `MV.17.A` Task 2.
//!
//! The spec is explicit that segmentation must be built and tested against the
//! **worst-case** multi-repo lane shape actually seen in the fleet, not only small
//! synthetic fixtures — `close_the_loop_substrate.json` (a hand-converted copy of the
//! live `planning/close-the-loop/lane-substrate.txt`, MV.17.A Task 1) spans seven repos
//! (base-template, mev, okf-core, claude-code-rs, engine-rs, bastion, orchestrator),
//! all contiguous by repo. This pins the exact segment count and boundaries.
//!
//! Deliberately **not** run against the live corpus: as of this block, the real
//! `agentic-portfolio` fleet's lane files are still `.txt` — `HQ.8.A` converts them.
//! Asserting against `.txt` here would either fail immediately (glob mismatch) or,
//! worse, silently pass on zero files. The fixture is the worst-case shape frozen at a
//! point in time; asserting against it is exactly as strong a regression gate as the
//! live-corpus sweep this test replaces, without depending on file conversion work
//! that hasn't landed yet.

use std::path::PathBuf;

use mev::brain::lane_segments::{discover_lane_files, segment_lane_blocks};

#[test]
fn close_the_loop_lane_substrate_segments_into_seven_contiguous_repo_runs() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lane_json");
    let fixture_path = fixture_dir.join("close_the_loop_substrate.json");
    assert!(
        fixture_path.is_file(),
        "lane_segments_fleet: fixture not found at {}",
        fixture_path.display()
    );

    // Copy the fixture into a throwaway `planning/roadmaps/<slug>/lane-<name>.json`
    // layout so `discover_lane_files` exercises the real discovery-plus-deserialize
    // path, not a hand-rolled parse.
    let dir = mev::testsupport::unique_temp_dir("mev-lane-segments-fleet-close-the-loop");
    let lane_dir = dir.join("planning/roadmaps/close-the-loop");
    std::fs::create_dir_all(&lane_dir).expect("create fixture roadmap dir");
    std::fs::copy(&fixture_path, lane_dir.join("lane-substrate.json"))
        .expect("copy close_the_loop_substrate.json fixture");

    let (lane_files, diags) = discover_lane_files(&dir);
    assert!(
        diags.is_empty(),
        "lane_segments_fleet: expected no diagnostics reading the fixture, got {diags:?}"
    );
    assert_eq!(
        lane_files.len(),
        1,
        "lane_segments_fleet: expected exactly one discovered lane file, got {lane_files:?}"
    );
    let lane_file = &lane_files[0];

    let segments = segment_lane_blocks(&lane_file.blocks);

    // Hand-checked against the live file's `# C<n> · <repo>` section headers this
    // fixture was converted from (MV.17.A Task 1): nine — now twenty — blocks across
    // seven repos, each repo's blocks contiguous, in this order.
    let expected_repos = [
        "base-template",
        "mev",
        "okf-core",
        "claude-code-rs",
        "engine-rs",
        "bastion",
        "orchestrator",
    ];
    let got_repos: Vec<&str> = segments.iter().map(|s| s.repo.as_str()).collect();

    let resolved_block_count: usize = segments.iter().map(|s| s.blocks.len()).sum();
    assert_eq!(
        resolved_block_count,
        lane_file.blocks.len(),
        "lane_segments_fleet: {} of {} blocks in the fixture did not render — segments: \
         {segments:?}",
        lane_file.blocks.len() - resolved_block_count,
        lane_file.blocks.len(),
    );

    assert_eq!(
        got_repos, expected_repos,
        "lane_segments_fleet: expected 7 contiguous segments in this repo order, got {got_repos:?}"
    );
    for (i, seg) in segments.iter().enumerate() {
        assert_eq!(
            seg.segment, i,
            "segment index must equal its position in the lane"
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}
