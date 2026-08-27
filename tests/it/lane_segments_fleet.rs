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

use std::path::{Path, PathBuf};

use mev::brain::config::find_brain_root;
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

/// MV.17.A Task 4: the honest post-flip state of the live-corpus half this test file
/// used to carry (see the module doc above — that half was replaced by the fixture
/// test because, pre-`HQ.8.A`, every real roadmap directory is still `.txt` and
/// asserting zero-files-zero-diagnostics against that would be worthless as a
/// regression gate). What *is* worth asserting against the live corpus right now is
/// the silent-miss diagnostic Task 4 adds: discovery must never emit an ERROR, and it
/// must still warn (`W_LANE_DIR_NO_RECORD`) for whichever real roadmap directories
/// remain lane-less at any given moment.
///
/// **Relaxed `MV.ticket.lane-segmentation-ignores-dependencies` (2026-08-22), per this
/// test's own original doc comment.** `HQ.8.A` has since converted 70 of the fleet's
/// legacy `.txt` lane substrates to `lane-*.json` records — `discover_lane_files`
/// against the live corpus now returns a non-empty `lane_files`, which is the expected,
/// desired post-conversion state, not drift. The original assertion
/// (`lane_files.is_empty()`) was written for the pre-conversion world and its own
/// message said to relax it once conversion began; this is that relaxation. The
/// no-ERROR guarantee is unconditional and still checked; the "at least one
/// `W_LANE_DIR_NO_RECORD` warning" guarantee is dropped as a hard assertion (kept as a
/// soft `eprintln!` observation instead) because full conversion could someday leave
/// zero roadmap directories lane-less, at which point that count should legitimately
/// reach zero without failing this test.
#[test]
fn live_corpus_discovery_warns_on_every_lane_less_roadmap_dir_and_errors_on_none() {
    // mev's own integration test binaries run with cwd == the mev crate root
    // (`core/mev`, or a worktree under it); the HQ brain root (where `brain.toml`
    // lives) is found by walking up — see `discover_lane_files_fleet_...` in the
    // module tests for the same pattern applied to `find_brain_config`. Walking up
    // rather than hard-coding a relative depth keeps this passing from a worktree
    // (one extra path segment deep) as well as a plain checkout, and lets it skip
    // cleanly on a standalone `mev` clone with no sibling HQ checkout at all.
    let start = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let live_root = match find_brain_root(&start) {
        Ok(root) => root,
        Err(_) => {
            eprintln!(
                "skipping live_corpus_discovery_warns_on_every_lane_less_roadmap_dir_and_errors_on_none: \
                 no brain.toml found walking up from {} (standalone mev checkout or CI runner \
                 without the sibling HQ checkout)",
                start.display()
            );
            return;
        }
    };

    let (lane_files, diags) = discover_lane_files(&live_root);

    eprintln!(
        "lane_segments_fleet: live corpus has {} converted lane record(s) (HQ.8.A in progress)",
        lane_files.len()
    );

    let errors: Vec<&mev::Diagnostic> = diags
        .iter()
        .filter(|d| d.severity == mev::Severity::Error)
        .collect();
    assert!(
        errors.is_empty(),
        "lane_segments_fleet: expected no ERROR diagnostics discovering the live corpus, \
         got {errors:?}"
    );

    let no_record_warnings: Vec<&mev::Diagnostic> = diags
        .iter()
        .filter(|d| d.severity == mev::Severity::Warning && d.locator == "W_LANE_DIR_NO_RECORD")
        .collect();
    eprintln!(
        "lane_segments_fleet: {} W_LANE_DIR_NO_RECORD warning(s) against the live corpus \
         (roadmap directories still lane-less)",
        no_record_warnings.len()
    );
}

/// Sanity check for the skip path above: a directory with no `brain.toml` anywhere in
/// its ancestry must not panic and must be reported as skippable, not treated as a
/// pass. This does not re-run the live-corpus assertions (that would defeat the
/// point) — it only pins that `find_brain_root` genuinely fails closed outside any
/// HQ checkout, which is what the skip branch above depends on.
#[test]
fn find_brain_root_fails_closed_outside_any_hq_checkout() {
    let dir = mev::testsupport::unique_temp_dir("mev-lane-segments-fleet-no-brain-toml");
    std::fs::create_dir_all(&dir).expect("create scratch dir");

    let result = find_brain_root(Path::new(&dir));
    assert!(
        result.is_err(),
        "find_brain_root must fail when no brain.toml exists anywhere up the tree \
         from a throwaway temp dir, got {result:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
