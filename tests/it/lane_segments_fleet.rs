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

use mev::brain::availability::{
    SegmentAvailability, SegmentStatus, apply_unregistered_overrides, lane_registration_issues,
};
use mev::brain::config::find_brain_root;
use mev::brain::lane_segments::{
    LaneBlockRef, LaneFile, discover_lane_files, lane_registration_diagnostics, segment_lane_blocks,
};
use mev::brain::state::{StateSource, load_state};

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

// ---------------------------------------------------------------------------
// Both registration clauses — MV.ticket.lane-file-registration-two-clauses Task 1
// ---------------------------------------------------------------------------
//
// A "lane registration" join fixture: one throwaway corpus root under which each
// repo gets its own `<repo>/planning/state.json`, and (for the both-clauses-hold
// case) a `<repo>/planning/blocks/<id>.json` record. `lane_registration_diagnostics`
// derives each repo's root from its loaded `StateSource::abs_path` directly, so
// writing a real `planning/state.json` on disk (rather than hand-building a
// `StateFile` in memory) is what exercises that derivation honestly.

/// Write `<dir>/<repo>/planning/state.json` carrying one track with the given raw
/// `tracks[].blocks[]` JSON array, then load it back via [`load_state`] — mirroring
/// how the real corpus loader produces a `(StateSource, StateFile)` pair, so
/// `abs_path` is a genuine on-disk path `lane_registration_diagnostics` can walk
/// `.parent().parent()` on.
fn state_source_and_file(
    dir: &Path,
    repo: &str,
    blocks_json: &str,
) -> (StateSource, mev::brain::state::StateFile) {
    let state_path = dir.join(repo).join("planning/state.json");
    std::fs::create_dir_all(state_path.parent().unwrap()).unwrap();
    std::fs::write(
        &state_path,
        format!(
            r#"{{"repo":"{repo}","kind":"project","updated":"2026-09-02","tracks":[
                {{"title":"t","blocks":{blocks_json}}}
            ]}}"#
        ),
    )
    .unwrap();
    let file = load_state(&state_path).expect("fixture state.json must load");
    let src = StateSource {
        repo_slug: repo.to_string(),
        abs_path: state_path,
        expected_kind: "project",
    };
    (src, file)
}

/// Write `<dir>/<repo>/planning/blocks/<id>.json` — the on-disk block record clause 2
/// checks for. Content is irrelevant to the check (only existence matters); a minimal
/// valid-looking JSON object is used so a future stricter check over this same fixture
/// would not need rewriting.
fn write_block_record(dir: &Path, repo: &str, id: &str) {
    let path = dir
        .join(repo)
        .join("planning/blocks")
        .join(format!("{id}.json"));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, format!(r#"{{"id":"{id}"}}"#)).unwrap();
}

fn one_block_lane(path: &Path, id: &str, repo: &str) -> LaneFile {
    LaneFile {
        roadmap: "close-the-loop".to_string(),
        lane: "substrate".to_string(),
        path: path.to_path_buf(),
        blocks: vec![LaneBlockRef {
            id: id.to_string(),
            line: 1,
            origin_roadmap: Some("close-the-loop".to_string()),
            repo: repo.to_string(),
        }],
        directives: None,
    }
}

#[test]
fn lane_registration_diagnostics_both_clauses_hold_reports_clean() {
    // The positive control: a block present in tracks[] AND carrying an on-disk
    // record must report nothing — an empty result must be distinguishable from a
    // check that never ran, which is why every other case below pairs with this one.
    let dir = mev::testsupport::unique_temp_dir("mev-lane-registration-both-clauses-hold");
    let (src, file) = state_source_and_file(
        &dir,
        "repoA",
        r#"[{"id":"A.1","title":"x","status":"open"}]"#,
    );
    write_block_record(&dir, "repoA", "A.1");
    let loaded = vec![(src, file)];

    let lane_file = one_block_lane(&dir.join("lane-substrate.json"), "A.1", "repoA");
    let diags = lane_registration_diagnostics(&lane_file, &loaded);
    assert!(
        diags.is_empty(),
        "both clauses hold: expected no diagnostics, got {diags:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lane_registration_diagnostics_row_only_fires_clause_2() {
    // In tracks[], open, no on-disk block record.
    let dir = mev::testsupport::unique_temp_dir("mev-lane-registration-row-only");
    let (src, file) = state_source_and_file(
        &dir,
        "repoA",
        r#"[{"id":"A.1","title":"x","status":"open"}]"#,
    );
    let loaded = vec![(src, file)];

    let lane_file = one_block_lane(&dir.join("lane-substrate.json"), "A.1", "repoA");
    let diags = lane_registration_diagnostics(&lane_file, &loaded);

    assert_eq!(
        diags.len(),
        1,
        "row-only: expected exactly one diagnostic, got {diags:?}"
    );
    assert_eq!(diags[0].severity, mev::Severity::Warning);
    assert_eq!(diags[0].file, lane_file.path);
    assert_eq!(diags[0].locator, "W_LANE_BLOCK_UNREGISTERED");
    assert!(diags[0].message.contains("A.1"));
    assert!(diags[0].message.contains("repoA"));
    assert!(diags[0].message.contains("clause 2"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lane_registration_diagnostics_record_only_fires_clause_1() {
    // A block record exists on disk, but the id is absent from tracks[].blocks[]
    // entirely — including the derived-container case: registering a block ONLY in
    // `focus.next[]` and never in `tracks[].blocks[]` is indistinguishable from this
    // at the `dependency_block_index` layer, so this fixture also stands in for
    // `base-template@9e6af3949`'s focus-only registration.
    let dir = mev::testsupport::unique_temp_dir("mev-lane-registration-record-only");
    let (src, file) = state_source_and_file(&dir, "repoA", r#"[]"#);
    write_block_record(&dir, "repoA", "A.1");
    let loaded = vec![(src, file)];

    let lane_file = one_block_lane(&dir.join("lane-substrate.json"), "A.1", "repoA");
    let diags = lane_registration_diagnostics(&lane_file, &loaded);

    assert_eq!(
        diags.len(),
        1,
        "record-only: expected exactly one diagnostic, got {diags:?}"
    );
    assert_eq!(diags[0].locator, "W_LANE_BLOCK_UNREGISTERED");
    assert!(diags[0].message.contains("A.1"));
    assert!(diags[0].message.contains("repoA"));
    assert!(diags[0].message.contains("clause 1"));
    assert!(diags[0].message.to_lowercase().contains("derived"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lane_registration_diagnostics_neither_clause_holds_fires_clause_1_only() {
    // Absent from tracks[] AND no record on disk — clause 1 subsumes clause 2 here
    // (see the function doc: clause 2 is only evaluated once clause 1 already
    // passed), so exactly one diagnostic, not two.
    let dir = mev::testsupport::unique_temp_dir("mev-lane-registration-neither");
    let (src, file) = state_source_and_file(&dir, "repoA", r#"[]"#);
    let loaded = vec![(src, file)];

    let lane_file = one_block_lane(&dir.join("lane-substrate.json"), "GHOST.1", "repoA");
    let diags = lane_registration_diagnostics(&lane_file, &loaded);

    assert_eq!(
        diags.len(),
        1,
        "neither clause holds: expected exactly one diagnostic, got {diags:?}"
    );
    assert!(diags[0].message.contains("clause 1"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lane_registration_diagnostics_terminal_statuses_are_exempt_from_clause_2() {
    // Blocks with status closed/wontfix/superseded/archived, in tracks[], with no
    // on-disk record — every one of the four must be exempt from clause 2 and report
    // nothing, pinning `LANE_REGISTRATION_TERMINAL_STATUSES` exactly.
    let dir = mev::testsupport::unique_temp_dir("mev-lane-registration-terminal-exempt");
    let (src, file) = state_source_and_file(
        &dir,
        "repoA",
        r#"[
            {"id":"A.closed","title":"x","status":"closed"},
            {"id":"A.wontfix","title":"x","status":"wontfix"},
            {"id":"A.superseded","title":"x","status":"superseded"},
            {"id":"A.archived","title":"x","status":"archived"}
        ]"#,
    );
    let loaded = vec![(src, file)];

    let lane_file = LaneFile {
        roadmap: "close-the-loop".to_string(),
        lane: "substrate".to_string(),
        path: dir.join("lane-substrate.json"),
        blocks: vec![
            LaneBlockRef {
                id: "A.closed".to_string(),
                line: 1,
                origin_roadmap: Some("close-the-loop".to_string()),
                repo: "repoA".to_string(),
            },
            LaneBlockRef {
                id: "A.wontfix".to_string(),
                line: 2,
                origin_roadmap: Some("close-the-loop".to_string()),
                repo: "repoA".to_string(),
            },
            LaneBlockRef {
                id: "A.superseded".to_string(),
                line: 3,
                origin_roadmap: Some("close-the-loop".to_string()),
                repo: "repoA".to_string(),
            },
            LaneBlockRef {
                id: "A.archived".to_string(),
                line: 4,
                origin_roadmap: Some("close-the-loop".to_string()),
                repo: "repoA".to_string(),
            },
        ],
        directives: None,
    };
    let diags = lane_registration_diagnostics(&lane_file, &loaded);
    assert!(
        diags.is_empty(),
        "all four terminal statuses must be exempt from clause 2, got {diags:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// mev lanes surfacing — MV.ticket.lane-file-registration-two-clauses Task 2
// ---------------------------------------------------------------------------

/// A synthetic `SegmentStatus` whose head is `<repo>:<id>`, matching the
/// `"repo:id"` key [`SegmentStatus::head`] documents.
fn head_status(repo: &str, id: &str, availability: SegmentAvailability) -> SegmentStatus {
    SegmentStatus {
        roadmap: "close-the-loop".to_string(),
        lane: "substrate".to_string(),
        segment: 0,
        repo: repo.to_string(),
        head: Some(format!("{repo}:{id}")),
        availability,
        reason: None,
    }
}

#[test]
fn availability_overrides_startable_with_held_unregistered_for_all_three_failing_cases() {
    // The same four-case matrix `lane_registration_diagnostics` is tested against
    // above, now run through `lane_registration_issues` +
    // `apply_unregistered_overrides` — the path `mev lanes --json` actually takes
    // (`MV.ticket.lane-file-registration-two-clauses` Task 2, record AC1).
    let cases: &[(&str, &str, bool, bool)] = &[
        // (fixture label, block id, register-in-tracks, write-on-disk-record)
        ("both clauses hold", "A.1", true, true),
        ("row-only (clause 2)", "A.2", true, false),
        ("record-only (clause 1)", "A.3", false, true),
        ("neither clause holds", "A.4", false, false),
    ];

    for (label, id, in_tracks, has_record) in cases {
        let dir = mev::testsupport::unique_temp_dir(&format!(
            "mev-availability-overrides-{}",
            id.to_lowercase()
        ));
        let blocks_json = if *in_tracks {
            format!(r#"[{{"id":"{id}","title":"x","status":"open"}}]"#)
        } else {
            "[]".to_string()
        };
        let (src, file) = state_source_and_file(&dir, "repoA", &blocks_json);
        if *has_record {
            write_block_record(&dir, "repoA", id);
        }
        let loaded = vec![(src, file)];
        let lane_file = one_block_lane(&dir.join("lane-substrate.json"), id, "repoA");
        let issues = lane_registration_issues(std::slice::from_ref(&lane_file), &loaded);

        let mut statuses = vec![head_status("repoA", id, SegmentAvailability::Startable)];
        apply_unregistered_overrides(&mut statuses, &issues);

        let both_clauses_hold = *in_tracks && *has_record;
        if both_clauses_hold {
            assert_eq!(
                statuses[0].availability,
                SegmentAvailability::Startable,
                "{label}: registered head must stay startable, the positive control \
                 distinguishing an empty result from a check that never ran"
            );
            assert_eq!(
                statuses[0].reason, None,
                "{label}: startable carries no reason"
            );
        } else {
            assert_eq!(
                statuses[0].availability,
                SegmentAvailability::HeldUnregistered,
                "{label}: an unregistered head must not report startable"
            );
            let reason = statuses[0].reason.as_deref().unwrap_or_default();
            assert!(
                reason.contains(*id),
                "{label}: reason must name the block, got {reason:?}"
            );
            let expected_clause = if *in_tracks { "clause 2" } else { "clause 1" };
            assert!(
                reason.contains(expected_clause),
                "{label}: expected reason to name {expected_clause}, got {reason:?}"
            );
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn lane_registration_diagnostics_all_are_warnings_never_errors() {
    let dir = mev::testsupport::unique_temp_dir("mev-lane-registration-warning-severity");
    let (src, file) = state_source_and_file(&dir, "repoA", r#"[]"#);
    let loaded = vec![(src, file)];

    let lane_file = one_block_lane(&dir.join("lane-substrate.json"), "GHOST.1", "repoA");
    let diags = lane_registration_diagnostics(&lane_file, &loaded);
    assert!(!diags.is_empty());
    for d in &diags {
        assert_eq!(
            d.severity,
            mev::Severity::Warning,
            "lane registration diagnostics must never be errors (ships warning-first, \
             per the block record's out_of_scope), got {d:?}"
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}
