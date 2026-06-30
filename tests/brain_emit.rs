//! Integration tests for the `brain::emit` module — Task 2.
//!
//! Tests:
//!   1. `wave_order` produces keys in wave-ascending order with None-wave blocks last.
//!   2. `wave_order` tiebreaks by iteration order (track order then block index).
//!   3. `render_wave_table` renders an open block with an unmet dep as `blocked`.
//!   4. `render_wave_table` renders a closed block as `closed` and an open-ready block as `open`.
//!   5. `render_wave_table` emits `—` for blocks with no wave.
//!   6. `splice_generated` replaces content between sentinels and is idempotent.
//!   7. `splice_generated` preserves all non-sentinel lines verbatim.
//!   8. `splice_generated` returns `MissingSentinel` when the BEGIN sentinel is absent.
//!   9. `splice_generated` returns `MissingSentinel` when BEGIN is present but END is absent.

use mev::brain::emit::{render_wave_table, splice_generated, wave_order};
use mev::brain::state::{BlockedBy, StateFile, StateGraph, StateSource, Track, TrackBlock};
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a minimal `StateSource` for testing.
fn make_src(repo_slug: &str) -> StateSource {
    StateSource {
        repo_slug: repo_slug.to_string(),
        abs_path: PathBuf::from(format!("/fake/{repo_slug}/planning/state.json")),
        expected_kind: "project",
    }
}

/// Build a minimal `StateFile` with `kind:"project"` and the given tracks.
fn make_leaf(repo: &str, tracks: Vec<Track>) -> StateFile {
    StateFile {
        repo: repo.to_string(),
        kind: "project".to_string(),
        updated: "2026-06-30".to_string(),
        focus: Default::default(),
        tracks,
        repos: vec![],
        cross_repo: vec![],
        tiers: vec![],
        note: None,
        backlog: vec![],
    }
}

/// Build a `TrackBlock` with a given wave and no deps.
fn block(id: &str, title: &str, status: Option<&str>, wave: Option<i64>) -> TrackBlock {
    TrackBlock {
        id: id.to_string(),
        title: title.to_string(),
        status: status.map(|s| s.to_string()),
        depends_on: vec![],
        wave,
        origin: None,
    }
}

/// Build a `TrackBlock` with a same-repo block dep.
fn block_with_dep(
    id: &str,
    title: &str,
    status: Option<&str>,
    wave: Option<i64>,
    dep_repo: &str,
    dep_id: &str,
) -> TrackBlock {
    TrackBlock {
        id: id.to_string(),
        title: title.to_string(),
        status: status.map(|s| s.to_string()),
        depends_on: vec![BlockedBy::Block {
            repo: dep_repo.to_string(),
            id: dep_id.to_string(),
            what: None,
        }],
        wave,
        origin: None,
    }
}

/// Build an empty `StateGraph` (sufficient for `wave_order` and `render_wave_table`).
fn empty_graph() -> StateGraph {
    mev::brain::state::build_state_graph(&[])
}

// ---------------------------------------------------------------------------
// wave_order tests
// ---------------------------------------------------------------------------

#[test]
fn wave_order_ascending_wave_numbers() {
    let src = make_src("myrepo");
    let file = make_leaf(
        "myrepo",
        vec![Track {
            title: "Phase 1".to_string(),
            blocks: vec![
                block("B", "Block B", None, Some(3)),
                block("A", "Block A", None, Some(1)),
                block("C", "Block C", None, Some(2)),
            ],
        }],
    );
    let graph = empty_graph();
    let files = vec![(src, file)];

    let order = wave_order(&graph, &files);
    assert_eq!(order, vec!["myrepo:A", "myrepo:C", "myrepo:B"]);
}

#[test]
fn wave_order_none_wave_last() {
    let src = make_src("repo");
    let file = make_leaf(
        "repo",
        vec![Track {
            title: "Phase".to_string(),
            blocks: vec![
                block("X", "No wave", None, None), // no wave → last
                block("Y", "Wave 1", None, Some(1)),
                block("Z", "Wave 2", None, Some(2)),
            ],
        }],
    );
    let graph = empty_graph();
    let files = vec![(src, file)];

    let order = wave_order(&graph, &files);
    // Y (wave 1), Z (wave 2), X (no wave → i64::MAX)
    assert_eq!(order, vec!["repo:Y", "repo:Z", "repo:X"]);
}

#[test]
fn wave_order_tiebreak_by_iteration_order() {
    let src = make_src("repo");
    let file = make_leaf(
        "repo",
        vec![Track {
            title: "Phase".to_string(),
            blocks: vec![
                block("A", "First", None, Some(1)),
                block("B", "Second", None, Some(1)),
                block("C", "Third", None, Some(1)),
            ],
        }],
    );
    let graph = empty_graph();
    let files = vec![(src, file)];

    let order = wave_order(&graph, &files);
    // Same wave → track iteration order preserved.
    assert_eq!(order, vec!["repo:A", "repo:B", "repo:C"]);
}

#[test]
fn wave_order_multiple_repos_stable_ordering() {
    let src_a = make_src("alpha");
    let file_a = make_leaf(
        "alpha",
        vec![Track {
            title: "P".to_string(),
            blocks: vec![block("A1", "Alpha 1", None, Some(2))],
        }],
    );
    let src_b = make_src("beta");
    let file_b = make_leaf(
        "beta",
        vec![Track {
            title: "P".to_string(),
            blocks: vec![block("B1", "Beta 1", None, Some(1))],
        }],
    );
    let graph = empty_graph();
    let files = vec![(src_a, file_a), (src_b, file_b)];

    let order = wave_order(&graph, &files);
    // beta:B1 (wave 1) before alpha:A1 (wave 2).
    assert_eq!(order, vec!["beta:B1", "alpha:A1"]);
}

// ---------------------------------------------------------------------------
// render_wave_table tests
// ---------------------------------------------------------------------------

#[test]
fn render_wave_table_includes_header_and_sep() {
    let src = make_src("repo");
    let file = make_leaf(
        "repo",
        vec![Track {
            title: "P".to_string(),
            blocks: vec![block("B1", "My Block", Some("open"), Some(1))],
        }],
    );
    let graph = empty_graph();
    // We only need the file and repo_slug for render_wave_table.
    let _ = src;

    let table = render_wave_table("repo", &file, &graph);
    assert!(
        table.contains("| Wave | Block | Title | Status | Depends on |"),
        "missing header row; got:\n{table}"
    );
    assert!(
        table.contains("|------|-------|-------|--------|------------|"),
        "missing separator row; got:\n{table}"
    );
}

#[test]
fn render_wave_table_open_block_with_unmet_dep_shows_blocked() {
    // Block B depends on block A which is also `open` (not closed) → derived status = blocked.
    let file = make_leaf(
        "repo",
        vec![Track {
            title: "P".to_string(),
            blocks: vec![
                block("A", "Block A", Some("open"), Some(1)),
                block_with_dep("B", "Block B", Some("open"), Some(2), "repo", "A"),
            ],
        }],
    );
    let graph = empty_graph();
    let table = render_wave_table("repo", &file, &graph);

    // B's row should show "blocked".
    let b_row = table
        .lines()
        .find(|l| l.contains("| B |"))
        .unwrap_or_else(|| panic!("no row for B in:\n{table}"));
    assert!(
        b_row.contains("blocked"),
        "expected 'blocked' in B's row; got: {b_row}"
    );
}

#[test]
fn render_wave_table_open_ready_block_shows_open() {
    // Block A has no deps → derived status = open (ready).
    let file = make_leaf(
        "repo",
        vec![Track {
            title: "P".to_string(),
            blocks: vec![block("A", "Block A", Some("open"), Some(1))],
        }],
    );
    let graph = empty_graph();
    let table = render_wave_table("repo", &file, &graph);

    let a_row = table
        .lines()
        .find(|l| l.contains("| A |"))
        .unwrap_or_else(|| panic!("no row for A in:\n{table}"));
    assert!(
        a_row.contains("open"),
        "expected 'open' in A's row; got: {a_row}"
    );
}

#[test]
fn render_wave_table_closed_block_shows_closed() {
    let file = make_leaf(
        "repo",
        vec![Track {
            title: "P".to_string(),
            blocks: vec![block("X", "Done block", Some("closed"), Some(1))],
        }],
    );
    let graph = empty_graph();
    let table = render_wave_table("repo", &file, &graph);

    let x_row = table
        .lines()
        .find(|l| l.contains("| X |"))
        .unwrap_or_else(|| panic!("no row for X in:\n{table}"));
    assert!(
        x_row.contains("closed"),
        "expected 'closed' in X's row; got: {x_row}"
    );
}

#[test]
fn render_wave_table_no_wave_shows_em_dash() {
    let file = make_leaf(
        "repo",
        vec![Track {
            title: "P".to_string(),
            blocks: vec![block("Y", "No wave block", None, None)],
        }],
    );
    let graph = empty_graph();
    let table = render_wave_table("repo", &file, &graph);

    let y_row = table
        .lines()
        .find(|l| l.contains("| Y |"))
        .unwrap_or_else(|| panic!("no row for Y in:\n{table}"));
    // Em-dash character (U+2014).
    assert!(
        y_row.contains('\u{2014}'),
        "expected em-dash for missing wave in Y's row; got: {y_row}"
    );
}

#[test]
fn render_wave_table_depends_on_column_lists_deps() {
    let mut b = block_with_dep("B", "B", Some("open"), Some(2), "other", "X");
    // Also add an external dep.
    b.depends_on.push(BlockedBy::External {
        what: "deploy-gate".to_string(),
    });
    let file = make_leaf(
        "repo",
        vec![Track {
            title: "P".to_string(),
            blocks: vec![block("A", "A", Some("closed"), Some(1)), b],
        }],
    );
    let graph = empty_graph();
    let table = render_wave_table("repo", &file, &graph);

    let b_row = table
        .lines()
        .find(|l| l.contains("| B |"))
        .unwrap_or_else(|| panic!("no row for B in:\n{table}"));
    assert!(
        b_row.contains("other:X"),
        "expected 'other:X' dep in B's row; got: {b_row}"
    );
    assert!(
        b_row.contains("external:deploy-gate"),
        "expected 'external:deploy-gate' dep in B's row; got: {b_row}"
    );
}

#[test]
fn render_wave_table_wave_order_respected_in_rows() {
    // Wave 2 then wave 1 in authored order — table must emit wave-1 block first.
    let file = make_leaf(
        "repo",
        vec![Track {
            title: "P".to_string(),
            blocks: vec![
                block("Late", "Late block", None, Some(2)),
                block("Early", "Early block", None, Some(1)),
            ],
        }],
    );
    let graph = empty_graph();
    let table = render_wave_table("repo", &file, &graph);

    let lines: Vec<&str> = table.lines().collect();
    let early_idx = lines
        .iter()
        .position(|l| l.contains("| Early |"))
        .expect("Early row missing");
    let late_idx = lines
        .iter()
        .position(|l| l.contains("| Late |"))
        .expect("Late row missing");
    assert!(
        early_idx < late_idx,
        "Early (wave 1) should appear before Late (wave 2)"
    );
}

// ---------------------------------------------------------------------------
// splice_generated tests
// ---------------------------------------------------------------------------

#[test]
fn splice_generated_replaces_content_between_sentinels() {
    let original = "# Doc\n\nNarrative.\n<!-- BEGIN generated:wave-table -->\nOLD CONTENT\n<!-- END generated:wave-table -->\n\nMore narrative.\n";
    let result = splice_generated(original, "wave-table", "NEW CONTENT").unwrap();

    assert!(
        result.contains("NEW CONTENT"),
        "new content missing; got:\n{result}"
    );
    assert!(
        !result.contains("OLD CONTENT"),
        "old content still present; got:\n{result}"
    );
    assert!(
        result.contains("Narrative."),
        "narrative before sentinel lost; got:\n{result}"
    );
    assert!(
        result.contains("More narrative."),
        "narrative after sentinel lost; got:\n{result}"
    );
    assert!(
        result.contains("<!-- BEGIN generated:wave-table -->"),
        "BEGIN sentinel missing; got:\n{result}"
    );
    assert!(
        result.contains("<!-- END generated:wave-table -->"),
        "END sentinel missing; got:\n{result}"
    );
}

#[test]
fn splice_generated_is_idempotent() {
    let original = "# Doc\n\nStuff before.\n<!-- BEGIN generated:wave-table -->\n<!-- END generated:wave-table -->\nStuff after.\n";
    let generated = "| Wave | Block |\n|------|-------|\n| 1 | X |";

    let first = splice_generated(original, "wave-table", generated).unwrap();
    let second = splice_generated(&first, "wave-table", generated).unwrap();

    assert_eq!(
        first, second,
        "splice is not idempotent:\nfirst:\n{first}\nsecond:\n{second}"
    );
}

#[test]
fn splice_generated_preserves_all_non_sentinel_lines_verbatim() {
    let original = "line1\nline2\n<!-- BEGIN generated:my-marker -->\noldcontent\n<!-- END generated:my-marker -->\nline3\nline4\n";
    let result = splice_generated(original, "my-marker", "newcontent").unwrap();

    assert!(result.starts_with("line1\n"), "line1 not preserved");
    assert!(result.contains("line2\n"), "line2 not preserved");
    assert!(result.contains("line3\n"), "line3 not preserved");
    assert!(result.ends_with("line4\n"), "line4 not preserved");
}

#[test]
fn splice_generated_trailing_newline_preserved_when_original_has_one() {
    let original = "# Doc\n<!-- BEGIN generated:x -->\nold\n<!-- END generated:x -->\n";
    let result = splice_generated(original, "x", "new").unwrap();
    assert!(
        result.ends_with('\n'),
        "trailing newline not preserved; result ends with: {:?}",
        result.chars().last()
    );
}

#[test]
fn splice_generated_missing_begin_sentinel_returns_error() {
    let original = "No sentinels here.\nJust some text.\n";
    let result = splice_generated(original, "wave-table", "content");
    assert!(
        result.is_err(),
        "expected MissingSentinel error for missing BEGIN sentinel"
    );
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("wave-table"),
        "error message should mention the marker; got: {err}"
    );
}

#[test]
fn splice_generated_missing_end_sentinel_returns_error() {
    // BEGIN present but END absent.
    let original = "# Doc\n<!-- BEGIN generated:wave-table -->\ncontent\nno end sentinel\n";
    let result = splice_generated(original, "wave-table", "new content");
    assert!(
        result.is_err(),
        "expected MissingSentinel error for missing END sentinel"
    );
}

#[test]
fn splice_generated_empty_generated_clears_between_sentinels() {
    let original = "before\n<!-- BEGIN generated:mark -->\nsome old content\n<!-- END generated:mark -->\nafter\n";
    let result = splice_generated(original, "mark", "").unwrap();

    // The old content should be gone; sentinels and surrounding text should remain.
    assert!(
        !result.contains("some old content"),
        "old content still present; got:\n{result}"
    );
    assert!(result.contains("before"), "before-line lost");
    assert!(result.contains("after"), "after-line lost");
    assert!(
        result.contains("<!-- BEGIN generated:mark -->"),
        "BEGIN sentinel missing"
    );
    assert!(
        result.contains("<!-- END generated:mark -->"),
        "END sentinel missing"
    );
}

// ---------------------------------------------------------------------------
// Task 3: plan_state_json, plan_master_plan_tables, apply_plan
// ---------------------------------------------------------------------------

mod task3_planners {
    use mev::brain::emit::{apply_plan, plan_master_plan_tables, plan_state_json};
    use mev::brain::state::{
        Block, BlockedBy, Focus, RepoRollup, StateFile, StateSource, Track, TrackBlock,
        build_state_graph,
    };
    use std::path::PathBuf;

    // -----------------------------------------------------------------------
    // Fixture helpers
    // -----------------------------------------------------------------------

    fn track_block(
        id: &str,
        title: &str,
        status: Option<&str>,
        wave: Option<i64>,
        deps: Vec<BlockedBy>,
    ) -> TrackBlock {
        TrackBlock {
            id: id.to_string(),
            title: title.to_string(),
            status: status.map(|s| s.to_string()),
            depends_on: deps,
            wave,
            origin: None,
        }
    }

    fn make_leaf_file(repo: &str, tracks: Vec<Track>, focus: Focus) -> StateFile {
        StateFile {
            repo: repo.to_string(),
            kind: "project".to_string(),
            updated: "2026-06-30".to_string(),
            focus,
            tracks,
            repos: vec![],
            cross_repo: vec![],
            tiers: vec![],
            note: None,
            backlog: vec![],
        }
    }

    fn make_brain_file(
        repos: Vec<RepoRollup>,
        cross_repo: Vec<mev::brain::state::CrossRepoEdge>,
        focus: Focus,
    ) -> StateFile {
        StateFile {
            repo: "brain".to_string(),
            kind: "brain".to_string(),
            updated: "2026-06-30".to_string(),
            focus,
            tracks: vec![],
            repos,
            cross_repo,
            tiers: vec![],
            note: None,
            backlog: vec![],
        }
    }

    fn focus_with_now(block_id: &str, title: &str) -> Focus {
        Focus {
            now: vec![Block {
                id: block_id.to_string(),
                title: title.to_string(),
                status: Some("in_progress".to_string()),
                note: None,
                repo: None,
                blocked_by: vec![],
            }],
            next: vec![],
            blocked: vec![],
        }
    }

    // -----------------------------------------------------------------------
    // plan_state_json — leaf focus regeneration
    // -----------------------------------------------------------------------

    #[test]
    fn leaf_focus_regenerated_from_tracks() {
        // Leaf has a stale focus.now (a closed block) while "B" is in_progress.
        let stale_focus = focus_with_now("A", "Block A"); // stale: A is now closed
        let tracks = vec![Track {
            title: "Phase 1".to_string(),
            blocks: vec![
                track_block("A", "Block A", Some("closed"), Some(1), vec![]),
                track_block("B", "Block B", Some("in_progress"), Some(2), vec![]),
            ],
        }];
        let file = make_leaf_file("myrepo", tracks, stale_focus);
        let src = StateSource {
            repo_slug: "myrepo".to_string(),
            abs_path: PathBuf::from("/fake/myrepo/planning/state.json"),
            expected_kind: "project",
        };
        let files = vec![(src.clone(), file)];
        let graph = build_state_graph(&files);

        let plan = plan_state_json(&files, &graph);

        // Exactly one action for this file.
        assert_eq!(
            plan.actions.len(),
            1,
            "expected one action; got {}",
            plan.actions.len()
        );
        let action = &plan.actions[0];
        assert_eq!(action.path, src.abs_path);

        // Parse the new content and verify focus.now = ["B"], not ["A"].
        let derived: StateFile =
            serde_json::from_str(&action.new_content).expect("new_content must be valid JSON");
        let now_ids: Vec<&str> = derived.focus.now.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(
            now_ids,
            vec!["B"],
            "focus.now should be [B]; got {now_ids:?}"
        );
        assert!(
            derived.focus.now.iter().all(|b| b.id != "A"),
            "stale block A should not appear in focus.now"
        );

        // tracks[] must survive unchanged.
        assert_eq!(derived.tracks.len(), 1);
        assert_eq!(derived.tracks[0].blocks.len(), 2);
    }

    #[test]
    fn fixed_point_no_action() {
        // A leaf whose stored focus already matches the derivation → no action.
        let tracks = vec![Track {
            title: "P".to_string(),
            blocks: vec![
                track_block("A", "Block A", Some("closed"), Some(1), vec![]),
                track_block("B", "Block B", Some("in_progress"), Some(2), vec![]),
            ],
        }];
        // Pre-derive the correct focus.
        let correct_focus = Focus {
            now: vec![Block {
                id: "B".to_string(),
                title: "Block B".to_string(),
                status: Some("in_progress".to_string()),
                note: None,
                repo: None,
                blocked_by: vec![],
            }],
            next: vec![],
            blocked: vec![],
        };
        let file = make_leaf_file("myrepo", tracks, correct_focus);
        let src = StateSource {
            repo_slug: "myrepo".to_string(),
            abs_path: PathBuf::from("/fake/myrepo/planning/state.json"),
            expected_kind: "project",
        };
        let files = vec![(src, file)];
        let graph = build_state_graph(&files);

        let plan = plan_state_json(&files, &graph);

        assert_eq!(
            plan.actions.len(),
            0,
            "no action expected when focus is already at fixed point; got {} actions",
            plan.actions.len()
        );
    }

    #[test]
    fn brain_rollup_regenerated_preserves_authored() {
        use mev::brain::state::{Backlog, TierEntry};

        // Leaf repo with one in_progress block.
        let leaf_tracks = vec![Track {
            title: "P".to_string(),
            blocks: vec![track_block(
                "BA.1",
                "My block",
                Some("in_progress"),
                Some(1),
                vec![],
            )],
        }];
        let leaf_file = make_leaf_file("myrepo", leaf_tracks, Focus::default());
        let leaf_src = StateSource {
            repo_slug: "myrepo".to_string(),
            abs_path: PathBuf::from("/fake/myrepo/planning/state.json"),
            expected_kind: "project",
        };

        // Brain with stale repos[] (no children listed) and authored backlog/tiers.
        let brain_focus = focus_with_now("brain-block", "Brain Block");
        let mut brain_file = make_brain_file(vec![], vec![], brain_focus.clone());
        brain_file.backlog = vec![Backlog {
            slug: "bl-1".to_string(),
            title: "Backlog item".to_string(),
            repo: "myrepo".to_string(),
            kind: "feature".to_string(),
            status: "idea".to_string(),
            depends_on: vec![],
            block: None,
            notes: None,
        }];
        brain_file.tiers = vec![TierEntry {
            tier: "core".to_string(),
            rollup: None,
            summary: None,
        }];

        let brain_src = StateSource {
            repo_slug: "brain".to_string(),
            abs_path: PathBuf::from("/fake/brain/planning/state.json"),
            expected_kind: "brain",
        };

        let files = vec![
            (leaf_src.clone(), leaf_file.clone()),
            (brain_src.clone(), brain_file),
        ];
        let graph = build_state_graph(&files);

        let plan = plan_state_json(&files, &graph);

        // Brain file should have an action (its repos[] was stale).
        let brain_action = plan
            .actions
            .iter()
            .find(|a| a.path == brain_src.abs_path)
            .expect("expected an action for the brain state.json");

        let derived: StateFile = serde_json::from_str(&brain_action.new_content)
            .expect("new_content must be valid JSON");

        // repos[] should now contain myrepo with the in_progress block.
        assert_eq!(derived.repos.len(), 1, "expected one repo rollup");
        let rollup = &derived.repos[0];
        assert_eq!(rollup.repo, "myrepo");
        let now_ids: Vec<&str> = rollup.now.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(now_ids, vec!["BA.1"]);

        // Authored backlog must survive.
        assert_eq!(derived.backlog.len(), 1);
        assert_eq!(derived.backlog[0].slug, "bl-1");

        // Authored tiers must survive.
        assert_eq!(derived.tiers.len(), 1);
        assert_eq!(derived.tiers[0].tier, "core");
    }

    #[test]
    fn brain_focus_untouched() {
        // Brain file has a non-empty authored focus — it must not be touched.
        let authored_focus = focus_with_now("special-block", "Special");
        let brain_file = make_brain_file(vec![], vec![], authored_focus);
        let brain_src = StateSource {
            repo_slug: "brain".to_string(),
            abs_path: PathBuf::from("/fake/brain/planning/state.json"),
            expected_kind: "brain",
        };

        let files = vec![(brain_src.clone(), brain_file.clone())];
        let graph = build_state_graph(&files);

        let plan = plan_state_json(&files, &graph);

        // If there's an action, parse it and check the brain focus is identical.
        if let Some(action) = plan.actions.iter().find(|a| a.path == brain_src.abs_path) {
            let derived: StateFile = serde_json::from_str(&action.new_content).expect("valid JSON");
            assert_eq!(
                derived.focus.now.len(),
                brain_file.focus.now.len(),
                "brain focus.now length changed"
            );
            if !derived.focus.now.is_empty() {
                assert_eq!(
                    derived.focus.now[0].id, "special-block",
                    "brain focus.now id changed"
                );
            }
        }
        // (No action is also acceptable if repos[]/cross_repo[] are already empty/correct.)
    }

    // -----------------------------------------------------------------------
    // plan_master_plan_tables
    // -----------------------------------------------------------------------

    #[test]
    fn splices_table_inside_sentinels() {
        let tmp = tempfile::tempdir().unwrap();
        let mp_path = tmp.path().join("master-plan.md");
        let state_path = tmp.path().join("state.json");

        let original_mp = "# Plan\n\nNarrative before.\n\
            <!-- BEGIN generated:wave-table -->\n\
            <!-- END generated:wave-table -->\n\
            \nNarrative after.\n";
        std::fs::write(&mp_path, original_mp).unwrap();

        let tracks = vec![Track {
            title: "P".to_string(),
            blocks: vec![track_block(
                "B1",
                "Block One",
                Some("open"),
                Some(1),
                vec![],
            )],
        }];
        let file = make_leaf_file("myrepo", tracks, Focus::default());
        let src = StateSource {
            repo_slug: "myrepo".to_string(),
            abs_path: state_path,
            expected_kind: "project",
        };

        let files = vec![(src, file)];
        let graph = build_state_graph(&files);
        let plan = plan_master_plan_tables(&files, &graph);

        assert_eq!(
            plan.actions.len(),
            1,
            "expected one action; got {}",
            plan.actions.len()
        );
        let action = &plan.actions[0];
        assert_eq!(action.path, mp_path);

        // Narrative lines outside sentinels preserved.
        assert!(
            action.new_content.contains("Narrative before."),
            "narrative before sentinel was lost"
        );
        assert!(
            action.new_content.contains("Narrative after."),
            "narrative after sentinel was lost"
        );

        // Table rendered inside sentinels.
        assert!(
            action.new_content.contains("| Wave |"),
            "table header missing from new content"
        );
        assert!(
            action.new_content.contains("B1"),
            "block id B1 missing from rendered table"
        );
        assert!(
            action
                .new_content
                .contains("<!-- BEGIN generated:wave-table -->"),
            "BEGIN sentinel missing"
        );
        assert!(
            action
                .new_content
                .contains("<!-- END generated:wave-table -->"),
            "END sentinel missing"
        );
    }

    #[test]
    fn no_sentinels_warns_no_action() {
        let tmp = tempfile::tempdir().unwrap();
        let mp_path = tmp.path().join("master-plan.md");
        let state_path = tmp.path().join("state.json");

        // master-plan.md has no sentinels.
        std::fs::write(&mp_path, "# Plan\n\nNo sentinels here.\n").unwrap();

        let file = make_leaf_file("myrepo", vec![], Focus::default());
        let src = StateSource {
            repo_slug: "myrepo".to_string(),
            abs_path: state_path,
            expected_kind: "project",
        };

        let files = vec![(src, file)];
        let graph = build_state_graph(&files);
        let plan = plan_master_plan_tables(&files, &graph);

        assert_eq!(
            plan.actions.len(),
            0,
            "no action expected when sentinels are absent; got {}",
            plan.actions.len()
        );
        let warn = plan
            .diagnostics
            .iter()
            .find(|d| d.locator == "W_EMIT_NO_SENTINEL");
        assert!(
            warn.is_some(),
            "expected W_EMIT_NO_SENTINEL diagnostic; got none"
        );
    }

    #[test]
    fn missing_master_plan_warns() {
        let tmp = tempfile::tempdir().unwrap();
        let state_path = tmp.path().join("state.json");
        // No master-plan.md created.

        let file = make_leaf_file("myrepo", vec![], Focus::default());
        let src = StateSource {
            repo_slug: "myrepo".to_string(),
            abs_path: state_path,
            expected_kind: "project",
        };

        let files = vec![(src, file)];
        let graph = build_state_graph(&files);
        let plan = plan_master_plan_tables(&files, &graph);

        assert_eq!(
            plan.actions.len(),
            0,
            "no action expected; got {}",
            plan.actions.len()
        );
        let warn = plan
            .diagnostics
            .iter()
            .find(|d| d.locator == "W_EMIT_NO_SENTINEL");
        assert!(
            warn.is_some(),
            "expected W_EMIT_NO_SENTINEL for missing file; got none"
        );
    }

    #[test]
    fn master_plan_splice_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let mp_path = tmp.path().join("master-plan.md");
        let state_path = tmp.path().join("state.json");

        let original_mp = "# Plan\n\
            <!-- BEGIN generated:wave-table -->\n\
            <!-- END generated:wave-table -->\n";
        std::fs::write(&mp_path, original_mp).unwrap();

        let tracks = vec![Track {
            title: "P".to_string(),
            blocks: vec![track_block("X", "X block", Some("open"), Some(1), vec![])],
        }];
        let file = make_leaf_file("repo", tracks, Focus::default());
        let src = StateSource {
            repo_slug: "repo".to_string(),
            abs_path: state_path,
            expected_kind: "project",
        };

        let files = vec![(src.clone(), file.clone())];
        let graph = build_state_graph(&files);

        // First pass: get the action's new_content.
        let plan1 = plan_master_plan_tables(&files, &graph);
        assert!(
            !plan1.actions.is_empty(),
            "first pass should produce an action"
        );
        let first_content = plan1.actions[0].new_content.clone();

        // Write it to disk.
        std::fs::write(&mp_path, &first_content).unwrap();

        // Second pass: re-run the planner.
        let plan2 = plan_master_plan_tables(&files, &graph);
        // The content is now at fixed point, so no action should be generated.
        assert_eq!(
            plan2.actions.len(),
            0,
            "second pass (idempotent) should produce no action; got {}",
            plan2.actions.len()
        );
    }

    // -----------------------------------------------------------------------
    // apply_plan
    // -----------------------------------------------------------------------

    #[test]
    fn dry_run_writes_nothing() {
        use mev::brain::emit::EmitAction;

        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("output.json");
        let original_bytes = b"original content";
        std::fs::write(&target, original_bytes).unwrap();

        let plan = mev::brain::emit::EmitPlan {
            actions: vec![EmitAction {
                path: target.clone(),
                new_content: "new content".to_string(),
                note: "test".to_string(),
            }],
            diagnostics: vec![],
        };

        let diags = apply_plan(&plan, false);

        // File must be unchanged.
        let after = std::fs::read(&target).unwrap();
        assert_eq!(after, original_bytes, "dry-run must not write to the file");

        // Should have a W_EMIT_DRY_RUN diagnostic.
        let dry_diag = diags.iter().find(|d| d.locator == "W_EMIT_DRY_RUN");
        assert!(dry_diag.is_some(), "expected W_EMIT_DRY_RUN diagnostic");
    }

    #[test]
    fn write_true_persists() {
        use mev::brain::emit::EmitAction;

        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("output.json");
        std::fs::write(&target, b"original content").unwrap();

        let new_content = "new content after write".to_string();
        let plan = mev::brain::emit::EmitPlan {
            actions: vec![EmitAction {
                path: target.clone(),
                new_content: new_content.clone(),
                note: "test write".to_string(),
            }],
            diagnostics: vec![],
        };

        let diags = apply_plan(&plan, true);

        // File must now contain the new content.
        let after = std::fs::read_to_string(&target).unwrap();
        assert_eq!(after, new_content, "write=true must update the file");

        // Should have an I_EMIT_WROTE diagnostic.
        let wrote_diag = diags.iter().find(|d| d.locator == "I_EMIT_WROTE");
        assert!(wrote_diag.is_some(), "expected I_EMIT_WROTE diagnostic");
    }
}

// ---------------------------------------------------------------------------
// Task 4 — emit_state library driver + CLI surface
// ---------------------------------------------------------------------------

mod task4_emit_state {
    use std::fs;
    use std::path::Path;

    // -----------------------------------------------------------------------
    // Fixture helpers (mirrored from brain_state.rs)
    // -----------------------------------------------------------------------

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("mev-emit-state-{tag}"));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn write_file(root: &Path, rel: &str, content: &str) {
        let target = root.join(rel);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&target, content.as_bytes()).unwrap();
    }

    fn write_json(root: &Path, rel: &str, value: &serde_json::Value) {
        write_file(root, rel, &serde_json::to_string_pretty(value).unwrap());
    }

    /// Write a minimal `brain.toml` that registers two leaf repos (alpha, beta).
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

    /// Write a stale HQ brain state.json (repos[] is wrong — it caches alpha with empty now).
    fn write_stale_brain_state(root: &Path) {
        let state = serde_json::json!({
            "repo": "hq",
            "kind": "brain",
            "updated": "2026-06-29",
            "focus": { "now": [], "next": [], "blocked": [] },
            "repos": [
                {
                    "repo": "alpha",
                    "now": [],   // stale: alpha actually has in_progress
                    "next": [],
                    "blocked": []
                }
            ],
            "cross_repo": []
        });
        write_json(root, "planning/state.json", &state);
    }

    /// Write an alpha leaf state.json with one in_progress block.
    fn write_alpha_state(root: &Path) {
        let state = serde_json::json!({
            "repo": "alpha",
            "kind": "project",
            "updated": "2026-06-29",
            "focus": {
                "now": [{ "id": "AL.1.A", "title": "Alpha block A", "status": "in_progress" }],
                "next": [],
                "blocked": []
            },
            "tracks": [
                {
                    "title": "Phase 1",
                    "blocks": [
                        { "id": "AL.1.A", "title": "Alpha block A", "status": "in_progress" },
                        { "id": "AL.1.B", "title": "Alpha block B", "status": "open" }
                    ]
                }
            ]
        });
        write_json(root, "repos/alpha/planning/state.json", &state);
    }

    /// Write a beta leaf state.json with a stale focus (now should be empty, but is set to "BE.1.A").
    fn write_stale_beta_state(root: &Path) {
        let state = serde_json::json!({
            "repo": "beta",
            "kind": "project",
            "updated": "2026-06-29",
            "focus": {
                "now": [{ "id": "BE.1.A", "title": "Beta block A", "status": "in_progress" }],  // stale
                "next": [],
                "blocked": []
            },
            "tracks": [
                {
                    "title": "Phase 1",
                    "blocks": [
                        { "id": "BE.1.A", "title": "Beta block A", "status": "open" }  // open, not in_progress
                    ]
                }
            ]
        });
        write_json(root, "repos/beta/planning/state.json", &state);
    }

    /// Build the full stale fixture (stale brain + alpha + stale beta).
    fn write_stale_fixture(root: &Path) {
        write_brain_toml(root);
        write_stale_brain_state(root);
        write_alpha_state(root);
        write_stale_beta_state(root);
    }

    // -----------------------------------------------------------------------
    // Test 1 — dry-run leaves files unchanged and reports W_EMIT_DRY_RUN
    // -----------------------------------------------------------------------

    #[test]
    fn dry_run_leaves_files_unchanged() {
        let dir = temp_dir("dry-run");
        write_stale_fixture(&dir);

        let alpha_state_path = dir.join("repos/alpha/planning/state.json");
        let beta_state_path = dir.join("repos/beta/planning/state.json");
        let brain_state_path = dir.join("planning/state.json");

        let alpha_before = fs::read(&alpha_state_path).unwrap();
        let beta_before = fs::read(&beta_state_path).unwrap();
        let brain_before = fs::read(&brain_state_path).unwrap();

        let report = mev::emit_state(&dir, false).expect("emit_state should not error");

        // No files must have changed.
        assert_eq!(
            fs::read(&alpha_state_path).unwrap(),
            alpha_before,
            "alpha state.json must be unchanged in dry-run"
        );
        assert_eq!(
            fs::read(&beta_state_path).unwrap(),
            beta_before,
            "beta state.json must be unchanged in dry-run"
        );
        assert_eq!(
            fs::read(&brain_state_path).unwrap(),
            brain_before,
            "brain state.json must be unchanged in dry-run"
        );

        // No errors.
        assert_eq!(
            report.error_count(),
            0,
            "dry-run on valid fixture should produce no errors; got: {:#?}",
            report.diagnostics
        );

        // At least one W_EMIT_DRY_RUN diagnostic (the stale files should produce planned actions).
        let dry_run_diags: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.locator == "W_EMIT_DRY_RUN")
            .collect();
        assert!(
            !dry_run_diags.is_empty(),
            "expected at least one W_EMIT_DRY_RUN diagnostic; got none. All diagnostics: {:#?}",
            report.diagnostics
        );

        let _ = fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------------
    // Test 2 — write=true updates derived views in place
    // -----------------------------------------------------------------------

    #[test]
    fn write_mode_updates_derived_views() {
        let dir = temp_dir("write-mode");
        write_stale_fixture(&dir);

        let beta_state_path = dir.join("repos/beta/planning/state.json");
        let beta_before: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&beta_state_path).unwrap()).unwrap();

        // Confirm stale: beta focus.now should show "in_progress" (stale) when beta's block is "open".
        let before_now = beta_before["focus"]["now"].as_array().unwrap();
        assert!(
            !before_now.is_empty(),
            "fixture must be stale (now should be non-empty before emit)"
        );

        let report = mev::emit_state(&dir, true).expect("emit_state should not error");

        // No errors.
        assert_eq!(
            report.error_count(),
            0,
            "write-mode on valid fixture should produce no errors; got: {:#?}",
            report.diagnostics
        );

        // At least one I_EMIT_WROTE diagnostic.
        let wrote_diags: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.locator == "I_EMIT_WROTE")
            .collect();
        assert!(
            !wrote_diags.is_empty(),
            "expected at least one I_EMIT_WROTE diagnostic; got none. All: {:#?}",
            report.diagnostics
        );

        // Beta's focus.now must now be empty (block is "open", not "in_progress").
        let beta_after: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&beta_state_path).unwrap()).unwrap();
        let after_now = beta_after["focus"]["now"].as_array().unwrap();
        assert!(
            after_now.is_empty(),
            "after emit, beta focus.now must be empty (block is open); got: {after_now:?}"
        );

        // Beta's tracks[] must be preserved — check the block id and status survive.
        let beta_tracks = &beta_after["tracks"];
        let first_block = &beta_tracks[0]["blocks"][0];
        assert_eq!(
            first_block["id"].as_str().unwrap(),
            "BE.1.A",
            "beta tracks[0].blocks[0].id must survive round-trip"
        );
        assert_eq!(
            first_block["status"].as_str().unwrap(),
            "open",
            "beta tracks[0].blocks[0].status must survive round-trip"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------------
    // Test 3 — write=true then emit_state again produces no further changes (fixed-point)
    // -----------------------------------------------------------------------

    #[test]
    fn second_run_is_idempotent() {
        let dir = temp_dir("idempotent");
        write_stale_fixture(&dir);

        // First write.
        mev::emit_state(&dir, true).expect("first emit should not error");

        // Snapshot file contents after first write.
        let alpha_after1 = fs::read(dir.join("repos/alpha/planning/state.json")).unwrap();
        let beta_after1 = fs::read(dir.join("repos/beta/planning/state.json")).unwrap();
        let brain_after1 = fs::read(dir.join("planning/state.json")).unwrap();

        // Second write.
        let report2 = mev::emit_state(&dir, true).expect("second emit should not error");

        // No I_EMIT_WROTE diagnostics on the second run (nothing changed).
        let wrote2: Vec<_> = report2
            .diagnostics
            .iter()
            .filter(|d| d.locator == "I_EMIT_WROTE")
            .collect();
        assert!(
            wrote2.is_empty(),
            "second run should produce no writes (fixed-point); got: {wrote2:#?}"
        );

        // Files must be identical.
        assert_eq!(
            fs::read(dir.join("repos/alpha/planning/state.json")).unwrap(),
            alpha_after1,
            "alpha state must be stable after second emit"
        );
        assert_eq!(
            fs::read(dir.join("repos/beta/planning/state.json")).unwrap(),
            beta_after1,
            "beta state must be stable after second emit"
        );
        assert_eq!(
            fs::read(dir.join("planning/state.json")).unwrap(),
            brain_after1,
            "brain state must be stable after second emit"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------------
    // Test 4 — emit_state on a dir without brain.toml returns E_CONFIG_NOT_FOUND
    // -----------------------------------------------------------------------

    #[test]
    fn missing_brain_toml_returns_config_error() {
        let dir = temp_dir("no-config");
        // No brain.toml — emit_state must return E_CONFIG_NOT_FOUND.
        // (walk-up may find a brain.toml in an ancestor — guard against that.)
        let has_ancestor = mev::brain::config::find_brain_config(&dir).is_ok();
        if has_ancestor {
            // Running on a machine where an ancestor has brain.toml — skip assertion.
            let _ = fs::remove_dir_all(&dir);
            return;
        }

        let report = mev::emit_state(&dir, false).expect("emit_state should not panic");
        let config_err = report
            .diagnostics
            .iter()
            .find(|d| d.locator == "E_CONFIG_NOT_FOUND");
        assert!(
            config_err.is_some(),
            "expected E_CONFIG_NOT_FOUND; got: {:#?}",
            report.diagnostics
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
