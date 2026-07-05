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

use mev::brain::emit::{
    global_status_map, markers, render_wave_table, splice_generated, wave_order,
};
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
        carryover: vec![],
    }
}

/// Build a `TrackBlock` with a given wave and no deps.
fn block(id: &str, title: &str, status: Option<&str>, wave: Option<i64>) -> TrackBlock {
    TrackBlock {
        due: None,
        priority: None,
        sdlc_workflow: None,
        model: None,
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
        due: None,
        priority: None,
        sdlc_workflow: None,
        model: None,
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

    let global_status = std::collections::HashMap::new();
    let table = render_wave_table("repo", &file, &graph, &global_status);
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
    let global_status = std::collections::HashMap::new();
    let table = render_wave_table("repo", &file, &graph, &global_status);

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
    let global_status = std::collections::HashMap::new();
    let table = render_wave_table("repo", &file, &graph, &global_status);

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
    let global_status = std::collections::HashMap::new();
    let table = render_wave_table("repo", &file, &graph, &global_status);

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
    let global_status = std::collections::HashMap::new();
    let table = render_wave_table("repo", &file, &graph, &global_status);

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
    let global_status = std::collections::HashMap::new();
    let table = render_wave_table("repo", &file, &graph, &global_status);

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
    let global_status = std::collections::HashMap::new();
    let table = render_wave_table("repo", &file, &graph, &global_status);

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

#[test]
fn render_wave_table_cross_repo_closed_dep_shows_open() {
    // Block B (in "repo") depends on "other:X", which is CLOSED in the global map.
    let file = make_leaf(
        "repo",
        vec![Track {
            title: "P".to_string(),
            blocks: vec![block_with_dep(
                "B",
                "Block B",
                Some("open"),
                Some(1),
                "other",
                "X",
            )],
        }],
    );
    let graph = empty_graph();
    let mut global_status = std::collections::HashMap::new();
    global_status.insert("other:X".to_string(), Some("closed".to_string()));

    let table = render_wave_table("repo", &file, &graph, &global_status);

    let b_row = table
        .lines()
        .find(|l| l.contains("| B |"))
        .unwrap_or_else(|| panic!("no row for B in:\n{table}"));
    assert!(
        b_row.contains("| open |"),
        "expected 'open' (dep resolved as met via closed cross-repo status) in B's row; got: {b_row}"
    );
}

#[test]
fn render_wave_table_cross_repo_open_dep_shows_blocked() {
    // Block B (in "repo") depends on "other:X", which is OPEN (not closed) in the global map.
    let file = make_leaf(
        "repo",
        vec![Track {
            title: "P".to_string(),
            blocks: vec![block_with_dep(
                "B",
                "Block B",
                Some("open"),
                Some(1),
                "other",
                "X",
            )],
        }],
    );
    let graph = empty_graph();
    let mut global_status = std::collections::HashMap::new();
    global_status.insert("other:X".to_string(), Some("open".to_string()));

    let table = render_wave_table("repo", &file, &graph, &global_status);

    let b_row = table
        .lines()
        .find(|l| l.contains("| B |"))
        .unwrap_or_else(|| panic!("no row for B in:\n{table}"));
    assert!(
        b_row.contains("blocked"),
        "expected 'blocked' (cross-repo dep still open) in B's row; got: {b_row}"
    );
}

#[test]
fn render_wave_table_cross_repo_absent_dep_shows_blocked() {
    // Block B depends on "other:X", which is absent from the global map entirely.
    let file = make_leaf(
        "repo",
        vec![Track {
            title: "P".to_string(),
            blocks: vec![block_with_dep(
                "B",
                "Block B",
                Some("open"),
                Some(1),
                "other",
                "X",
            )],
        }],
    );
    let graph = empty_graph();
    let global_status = std::collections::HashMap::new();

    let table = render_wave_table("repo", &file, &graph, &global_status);

    let b_row = table
        .lines()
        .find(|l| l.contains("| B |"))
        .unwrap_or_else(|| panic!("no row for B in:\n{table}"));
    assert!(
        b_row.contains("blocked"),
        "expected 'blocked' (cross-repo dep absent from global map) in B's row; got: {b_row}"
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
    use mev::brain::config::{BrainConfig, RepoEntry};
    use mev::brain::emit::{apply_plan, plan_master_plan_tables, plan_state_json};
    use mev::brain::state::{
        Block, BlockedBy, Focus, RepoRollup, StateFile, StateSource, Track, TrackBlock,
        build_state_graph,
    };
    use std::path::PathBuf;

    /// Build a minimal [`BrainConfig`] with one `[[repos]]` entry per given
    /// `(slug, tier)` pair — enough to drive [`tier_scope_for`]/[`derive_rollup`]/
    /// [`derive_brain_focus`] in these tests.
    fn config_with_repos(entries: &[(&str, &str)]) -> BrainConfig {
        BrainConfig {
            repos: entries
                .iter()
                .map(|(slug, tier)| RepoEntry {
                    slug: slug.to_string(),
                    tier: tier.to_string(),
                    repo_path: String::new(),
                    status_file: String::new(),
                    cache_doc: String::new(),
                    heading: String::new(),
                })
                .collect(),
            ..BrainConfig::default()
        }
    }

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
            due: None,
            priority: None,
            sdlc_workflow: None,
            model: None,
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
            carryover: vec![],
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
            carryover: vec![],
        }
    }

    fn focus_with_now(block_id: &str, title: &str) -> Focus {
        Focus {
            now: vec![Block {
                due: None,
                priority: None,
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
        let config = config_with_repos(&[("myrepo", "core")]);

        let plan = plan_state_json(&files, &graph, &config);

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
    fn leaf_focus_carries_priority_and_due_from_tracks() {
        // "A" is in_progress with priority/due set; "B" is next with only priority set.
        let tracks = vec![Track {
            title: "Phase 1".to_string(),
            blocks: vec![
                TrackBlock {
                    due: Some("2026-07-10".to_string()),
                    priority: Some(1),
                    sdlc_workflow: None,
                    model: None,
                    id: "A".to_string(),
                    title: "Block A".to_string(),
                    status: Some("in_progress".to_string()),
                    depends_on: vec![],
                    wave: Some(1),
                    origin: None,
                },
                TrackBlock {
                    due: None,
                    priority: Some(2),
                    sdlc_workflow: None,
                    model: None,
                    id: "B".to_string(),
                    title: "Block B".to_string(),
                    status: None,
                    depends_on: vec![],
                    wave: Some(2),
                    origin: None,
                },
            ],
        }];
        let file = make_leaf_file("myrepo", tracks, Focus::default());
        let src = StateSource {
            repo_slug: "myrepo".to_string(),
            abs_path: PathBuf::from("/fake/myrepo/planning/state.json"),
            expected_kind: "project",
        };
        let files = vec![(src.clone(), file)];
        let graph = build_state_graph(&files);
        let config = config_with_repos(&[("myrepo", "core")]);

        let plan = plan_state_json(&files, &graph, &config);

        let action = plan
            .actions
            .iter()
            .find(|a| a.path == src.abs_path)
            .expect("expected an action regenerating focus from tracks");
        let derived: StateFile =
            serde_json::from_str(&action.new_content).expect("new_content must be valid JSON");

        let now_a = derived
            .focus
            .now
            .iter()
            .find(|b| b.id == "A")
            .expect("A must be in focus.now");
        assert_eq!(now_a.priority, Some(1));
        assert_eq!(now_a.due.as_deref(), Some("2026-07-10"));

        let next_b = derived
            .focus
            .next
            .iter()
            .find(|b| b.id == "B")
            .expect("B must be in focus.next");
        assert_eq!(next_b.priority, Some(2));
        assert_eq!(
            next_b.due, None,
            "block with no source due date must carry None, not a fabricated value"
        );
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
                due: None,
                priority: None,
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
        let config = config_with_repos(&[("myrepo", "core")]);

        let plan = plan_state_json(&files, &graph, &config);

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
        let config = config_with_repos(&[("myrepo", "core")]);

        let plan = plan_state_json(&files, &graph, &config);

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
    fn brain_focus_regenerated_as_repo_tagged_union() {
        // Brain file has a stale authored focus that must be replaced by the
        // repo-tagged union of in-scope children's derived focus (MV.3B.U task 2/3).
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

        let stale_focus = focus_with_now("special-block", "Special");
        let brain_file = make_brain_file(vec![], vec![], stale_focus);
        let brain_src = StateSource {
            repo_slug: "brain".to_string(),
            abs_path: PathBuf::from("/fake/brain/planning/state.json"),
            expected_kind: "brain",
        };

        let files = vec![(leaf_src, leaf_file), (brain_src.clone(), brain_file)];
        let graph = build_state_graph(&files);
        let config = config_with_repos(&[("myrepo", "core")]);

        let plan = plan_state_json(&files, &graph, &config);

        let brain_action = plan
            .actions
            .iter()
            .find(|a| a.path == brain_src.abs_path)
            .expect("expected an action for the brain state.json");

        let derived: StateFile =
            serde_json::from_str(&brain_action.new_content).expect("valid JSON");

        // focus.now must be the repo-tagged union of myrepo's derived focus, not
        // the stale authored "special-block".
        assert_eq!(derived.focus.now.len(), 1);
        assert_eq!(derived.focus.now[0].id, "BA.1");
        assert_eq!(derived.focus.now[0].repo.as_deref(), Some("myrepo"));
        assert!(
            derived.focus.now.iter().all(|b| b.id != "special-block"),
            "stale authored focus block must not survive"
        );
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
    fn portfolio_kind_with_no_master_plan_is_skipped_without_warning() {
        let tmp = tempfile::tempdir().unwrap();
        let state_path = tmp.path().join("state.json");
        // No master-plan.md created — portfolio repos never have one.

        let mut file = make_leaf_file("re-rs", vec![], Focus::default());
        file.kind = "portfolio".to_string();
        file.note = Some("Completed — live on GitHub".to_string());
        let src = StateSource {
            repo_slug: "re-rs".to_string(),
            abs_path: state_path,
            expected_kind: "portfolio",
        };

        let files = vec![(src, file)];
        let graph = build_state_graph(&files);
        let plan = plan_master_plan_tables(&files, &graph);

        assert_eq!(
            plan.actions.len(),
            0,
            "no action expected for a portfolio-kind file; got {}",
            plan.actions.len()
        );
        assert!(
            plan.diagnostics.is_empty(),
            "portfolio-kind file should never produce W_EMIT_NO_SENTINEL; got: {:?}",
            plan.diagnostics
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
    // plan_project_caches
    // -----------------------------------------------------------------------

    /// Build a [`BrainConfig`] with one `[[repos]]` entry naming `cache_doc`.
    fn config_with_cache_doc(slug: &str, tier: &str, cache_doc: &str) -> BrainConfig {
        config_with_cache_and_status(slug, tier, cache_doc, "")
    }

    fn config_with_cache_and_status(
        slug: &str,
        tier: &str,
        cache_doc: &str,
        status_file: &str,
    ) -> BrainConfig {
        BrainConfig {
            repos: vec![RepoEntry {
                slug: slug.to_string(),
                tier: tier.to_string(),
                repo_path: String::new(),
                status_file: status_file.to_string(),
                cache_doc: cache_doc.to_string(),
                heading: String::new(),
            }],
            ..BrainConfig::default()
        }
    }

    /// A minimal `planning/status.md`-shaped source file carrying only the
    /// OKF `timestamp` watermark [`plan_project_caches`] reads.
    fn status_file_with_timestamp(timestamp: &str) -> String {
        format!(
            "---\n\
             type: ProjectStatus\n\
             title: myrepo Status\n\
             description: Test status file.\n\
             timestamp: \"{timestamp}\"\n\
             ---\n\n\
             # Status\n"
        )
    }

    fn cache_doc_with_sentinel(synced_from: &str) -> String {
        format!(
            "---\n\
             type: ProjectContext\n\
             title: myrepo Project Context\n\
             description: Test cache.\n\
             synced_from: \"{synced_from}\"\n\
             ---\n\n\
             # myrepo\n\n\
             Narrative before.\n\n\
             <!-- BEGIN generated:project-cache -->\n\
             <!-- END generated:project-cache -->\n\n\
             Narrative after.\n"
        )
    }

    #[test]
    fn project_cache_splice_produces_expected_content() {
        use mev::brain::emit::plan_project_caches;

        let tmp = tempfile::tempdir().unwrap();
        let cache_rel = "docs/projects/myrepo.md";
        let cache_path = tmp.path().join(cache_rel);
        std::fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        std::fs::write(&cache_path, cache_doc_with_sentinel("2026-01-01T00:00:00Z")).unwrap();

        let status_rel = "myrepo/planning/status.md";
        let status_path = tmp.path().join(status_rel);
        std::fs::create_dir_all(status_path.parent().unwrap()).unwrap();
        std::fs::write(
            &status_path,
            status_file_with_timestamp("2026-07-04T02:21:44Z"),
        )
        .unwrap();

        let tracks = vec![Track {
            title: "P".to_string(),
            blocks: vec![track_block(
                "B1",
                "Block One",
                Some("in_progress"),
                Some(1),
                vec![],
            )],
        }];
        let mut file = make_leaf_file("myrepo", tracks, Focus::default());
        // The brain's own coarse freshness scalar — deliberately different from the
        // status_file's `timestamp` to prove the emitter no longer reads this field.
        file.updated = "2026-06-01".to_string();
        let src = StateSource {
            repo_slug: "myrepo".to_string(),
            abs_path: PathBuf::from("/fake/myrepo/planning/state.json"),
            expected_kind: "project",
        };

        let files = vec![(src, file)];
        let graph = build_state_graph(&files);
        let config = config_with_cache_and_status("myrepo", "core", cache_rel, status_rel);

        let plan = plan_project_caches(tmp.path(), &files, &graph, &config);

        assert_eq!(
            plan.actions.len(),
            1,
            "expected one action; got {}",
            plan.actions.len()
        );
        let action = &plan.actions[0];
        assert_eq!(action.path, cache_path);

        assert!(
            action.new_content.contains("Narrative before."),
            "narrative before sentinel was lost"
        );
        assert!(
            action.new_content.contains("Narrative after."),
            "narrative after sentinel was lost"
        );
        assert!(
            action.new_content.contains("`B1` — Block One"),
            "focus-line missing derived block: {}",
            action.new_content
        );
        assert!(
            action
                .new_content
                .contains("synced_from: \"2026-07-04T02:21:44Z\""),
            "synced_from watermark not reconciled to the status_file's timestamp: {}",
            action.new_content
        );
        assert!(
            !action.new_content.contains("2026-01-01T00:00:00Z"),
            "stale synced_from watermark should not survive"
        );
        assert!(
            !action.new_content.contains("2026-06-01"),
            "synced_from must not be sourced from state.json's own coarse 'updated' scalar"
        );
    }

    #[test]
    fn project_cache_missing_sentinel_warns_no_action() {
        use mev::brain::emit::plan_project_caches;

        let tmp = tempfile::tempdir().unwrap();
        let cache_rel = "docs/projects/myrepo.md";
        let cache_path = tmp.path().join(cache_rel);
        std::fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        std::fs::write(
            &cache_path,
            "---\ntype: ProjectContext\n---\n\n# myrepo\n\nNo sentinels.\n",
        )
        .unwrap();

        let file = make_leaf_file("myrepo", vec![], Focus::default());
        let src = StateSource {
            repo_slug: "myrepo".to_string(),
            abs_path: PathBuf::from("/fake/myrepo/planning/state.json"),
            expected_kind: "project",
        };

        let files = vec![(src, file)];
        let graph = build_state_graph(&files);
        let config = config_with_cache_doc("myrepo", "core", cache_rel);

        let plan = plan_project_caches(tmp.path(), &files, &graph, &config);

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
    fn project_cache_missing_file_warns_no_action() {
        use mev::brain::emit::plan_project_caches;

        let tmp = tempfile::tempdir().unwrap();
        // No cache doc written at all.
        let cache_rel = "docs/projects/myrepo.md";

        let file = make_leaf_file("myrepo", vec![], Focus::default());
        let src = StateSource {
            repo_slug: "myrepo".to_string(),
            abs_path: PathBuf::from("/fake/myrepo/planning/state.json"),
            expected_kind: "project",
        };

        let files = vec![(src, file)];
        let graph = build_state_graph(&files);
        let config = config_with_cache_doc("myrepo", "core", cache_rel);

        let plan = plan_project_caches(tmp.path(), &files, &graph, &config);

        assert_eq!(
            plan.actions.len(),
            0,
            "no action expected when the cache doc is missing; got {}",
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
    fn project_cache_missing_status_timestamp_warns_no_action() {
        use mev::brain::emit::plan_project_caches;

        let tmp = tempfile::tempdir().unwrap();
        let cache_rel = "docs/projects/myrepo.md";
        let cache_path = tmp.path().join(cache_rel);
        std::fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        std::fs::write(&cache_path, cache_doc_with_sentinel("2026-01-01T00:00:00Z")).unwrap();

        // status_file exists but carries no `timestamp` field.
        let status_rel = "myrepo/planning/status.md";
        let status_path = tmp.path().join(status_rel);
        std::fs::create_dir_all(status_path.parent().unwrap()).unwrap();
        std::fs::write(&status_path, "---\ntype: ProjectStatus\n---\n\n# Status\n").unwrap();

        let file = make_leaf_file("myrepo", vec![], Focus::default());
        let src = StateSource {
            repo_slug: "myrepo".to_string(),
            abs_path: PathBuf::from("/fake/myrepo/planning/state.json"),
            expected_kind: "project",
        };

        let files = vec![(src, file)];
        let graph = build_state_graph(&files);
        let config = config_with_cache_and_status("myrepo", "core", cache_rel, status_rel);

        let plan = plan_project_caches(tmp.path(), &files, &graph, &config);

        assert_eq!(
            plan.actions.len(),
            0,
            "no action expected when status_file has no timestamp; got {}",
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
    fn project_cache_fixed_point_no_action_on_second_pass() {
        use mev::brain::emit::plan_project_caches;

        let tmp = tempfile::tempdir().unwrap();
        let cache_rel = "docs/projects/myrepo.md";
        let cache_path = tmp.path().join(cache_rel);
        std::fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        std::fs::write(&cache_path, cache_doc_with_sentinel("2026-01-01T00:00:00Z")).unwrap();

        let status_rel = "myrepo/planning/status.md";
        let status_path = tmp.path().join(status_rel);
        std::fs::create_dir_all(status_path.parent().unwrap()).unwrap();
        std::fs::write(
            &status_path,
            status_file_with_timestamp("2026-07-04T02:21:44Z"),
        )
        .unwrap();

        let tracks = vec![Track {
            title: "P".to_string(),
            blocks: vec![track_block(
                "B1",
                "Block One",
                Some("in_progress"),
                Some(1),
                vec![],
            )],
        }];
        let mut file = make_leaf_file("myrepo", tracks, Focus::default());
        file.updated = "2026-06-01".to_string();
        let src = StateSource {
            repo_slug: "myrepo".to_string(),
            abs_path: PathBuf::from("/fake/myrepo/planning/state.json"),
            expected_kind: "project",
        };

        let files = vec![(src, file)];
        let graph = build_state_graph(&files);
        let config = config_with_cache_and_status("myrepo", "core", cache_rel, status_rel);

        // First pass produces an action; write it to disk.
        let plan1 = plan_project_caches(tmp.path(), &files, &graph, &config);
        assert_eq!(
            plan1.actions.len(),
            1,
            "first pass should produce an action"
        );
        std::fs::write(&cache_path, &plan1.actions[0].new_content).unwrap();

        // Second pass over the already-correct content: no action.
        let plan2 = plan_project_caches(tmp.path(), &files, &graph, &config);
        assert_eq!(
            plan2.actions.len(),
            0,
            "second pass (fixed point) should produce no action; got {}",
            plan2.actions.len()
        );
    }

    #[test]
    fn project_cache_non_project_kind_is_skipped() {
        use mev::brain::emit::plan_project_caches;

        let tmp = tempfile::tempdir().unwrap();
        let cache_rel = "docs/projects/myrepo.md";
        let cache_path = tmp.path().join(cache_rel);
        std::fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        std::fs::write(&cache_path, cache_doc_with_sentinel("2026-01-01T00:00:00Z")).unwrap();

        let mut file = make_leaf_file("myrepo", vec![], Focus::default());
        file.kind = "portfolio".to_string();
        let src = StateSource {
            repo_slug: "myrepo".to_string(),
            abs_path: PathBuf::from("/fake/myrepo/planning/state.json"),
            expected_kind: "portfolio",
        };

        let files = vec![(src, file)];
        let graph = build_state_graph(&files);
        let config = config_with_cache_doc("myrepo", "portfolio", cache_rel);

        let plan = plan_project_caches(tmp.path(), &files, &graph, &config);

        assert_eq!(
            plan.actions.len(),
            0,
            "portfolio-kind repos should never be targeted; got {}",
            plan.actions.len()
        );
        assert!(
            plan.diagnostics.is_empty(),
            "no diagnostics expected for a skipped non-project-kind repo; got: {:?}",
            plan.diagnostics
        );
    }

    // -----------------------------------------------------------------------
    // plan_tier_rollups
    // -----------------------------------------------------------------------

    fn status_doc_with_sentinel() -> String {
        "---\n\
         type: ProjectStatus\n\
         title: core tier status\n\
         description: Test tier status doc.\n\
         ---\n\n\
         # core\n\n\
         Narrative before.\n\n\
         <!-- BEGIN generated:tier-rollup -->\n\
         <!-- END generated:tier-rollup -->\n\n\
         Narrative after.\n"
            .to_string()
    }

    #[test]
    fn tier_rollup_splice_produces_expected_content() {
        use mev::brain::emit::plan_tier_rollups;

        let tmp = tempfile::tempdir().unwrap();
        let status_path = tmp.path().join("core/planning/status.md");
        std::fs::create_dir_all(status_path.parent().unwrap()).unwrap();
        std::fs::write(&status_path, status_doc_with_sentinel()).unwrap();

        // Leaf repo "repo-a" (core tier) with one in_progress block.
        let leaf_tracks = vec![Track {
            title: "P".to_string(),
            blocks: vec![track_block(
                "RA.1.A",
                "Repo A block",
                Some("in_progress"),
                Some(1),
                vec![],
            )],
        }];
        let leaf_file = make_leaf_file("repo-a", leaf_tracks, Focus::default());
        let leaf_src = StateSource {
            repo_slug: "repo-a".to_string(),
            abs_path: PathBuf::from("/fake/repo-a/planning/state.json"),
            expected_kind: "project",
        };

        // Tier brain: repo == "core", matches brain.toml's tier = "core", so
        // tier_scope_for resolves TierScope::Tier("core").
        let mut tier_file = make_brain_file(vec![], vec![], Focus::default());
        tier_file.repo = "core".to_string();
        let tier_src = StateSource {
            repo_slug: "core".to_string(),
            abs_path: tmp.path().join("core/planning/state.json"),
            expected_kind: "brain",
        };

        let files = vec![(leaf_src, leaf_file), (tier_src, tier_file)];
        let graph = build_state_graph(&files);
        let config = config_with_repos(&[("repo-a", "core")]);

        let plan = plan_tier_rollups(&files, &graph, &config);

        assert_eq!(
            plan.actions.len(),
            1,
            "expected one action; got {}",
            plan.actions.len()
        );
        let action = &plan.actions[0];
        assert_eq!(action.path, status_path);
        assert!(
            action.new_content.contains("Narrative before."),
            "narrative before sentinel was lost"
        );
        assert!(
            action.new_content.contains("Narrative after."),
            "narrative after sentinel was lost"
        );
        assert!(
            action.new_content.contains("`RA.1.A` — Repo A block"),
            "tier rollup table missing derived block: {}",
            action.new_content
        );
        assert!(
            action.new_content.contains("repo-a"),
            "tier rollup table missing repo row: {}",
            action.new_content
        );
    }

    #[test]
    fn tier_rollup_missing_sentinel_warns_no_action() {
        use mev::brain::emit::plan_tier_rollups;

        let tmp = tempfile::tempdir().unwrap();
        let status_path = tmp.path().join("core/planning/status.md");
        std::fs::create_dir_all(status_path.parent().unwrap()).unwrap();
        std::fs::write(
            &status_path,
            "---\ntype: ProjectStatus\n---\n\n# core\n\nNo sentinels.\n",
        )
        .unwrap();

        let mut tier_file = make_brain_file(vec![], vec![], Focus::default());
        tier_file.repo = "core".to_string();
        let tier_src = StateSource {
            repo_slug: "core".to_string(),
            abs_path: tmp.path().join("core/planning/state.json"),
            expected_kind: "brain",
        };

        let files = vec![(tier_src, tier_file)];
        let graph = build_state_graph(&files);
        let config = config_with_repos(&[("repo-a", "core")]);

        let plan = plan_tier_rollups(&files, &graph, &config);

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
    fn tier_rollup_missing_file_warns_no_action() {
        use mev::brain::emit::plan_tier_rollups;

        let tmp = tempfile::tempdir().unwrap();
        // No status.md written at all.

        let mut tier_file = make_brain_file(vec![], vec![], Focus::default());
        tier_file.repo = "core".to_string();
        let tier_src = StateSource {
            repo_slug: "core".to_string(),
            abs_path: tmp.path().join("core/planning/state.json"),
            expected_kind: "brain",
        };

        let files = vec![(tier_src, tier_file)];
        let graph = build_state_graph(&files);
        let config = config_with_repos(&[("repo-a", "core")]);

        let plan = plan_tier_rollups(&files, &graph, &config);

        assert_eq!(
            plan.actions.len(),
            0,
            "no action expected when status.md is missing; got {}",
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
    fn tier_rollup_hq_root_is_skipped() {
        use mev::brain::emit::plan_tier_rollups;

        let tmp = tempfile::tempdir().unwrap();
        // HQ root's own state.json -- repo "hq" matches no declared tier, so
        // tier_scope_for resolves TierScope::All and this planner must skip it
        // entirely (no action, no diagnostic) -- MV.4.C's plan_hq_board owns it.
        let hq_status_path = tmp.path().join("planning/status.md");
        std::fs::create_dir_all(hq_status_path.parent().unwrap()).unwrap();
        std::fs::write(&hq_status_path, status_doc_with_sentinel()).unwrap();

        let hq_file = make_brain_file(vec![], vec![], Focus::default());
        let hq_src = StateSource {
            repo_slug: "hq".to_string(),
            abs_path: tmp.path().join("planning/state.json"),
            expected_kind: "brain",
        };

        let files = vec![(hq_src, hq_file)];
        let graph = build_state_graph(&files);
        let config = config_with_repos(&[("repo-a", "core")]);

        let plan = plan_tier_rollups(&files, &graph, &config);

        assert_eq!(
            plan.actions.len(),
            0,
            "HQ root (TierScope::All) must never be targeted; got {}",
            plan.actions.len()
        );
        assert!(
            plan.diagnostics.is_empty(),
            "no diagnostics expected for the skipped HQ root; got: {:?}",
            plan.diagnostics
        );
    }

    #[test]
    fn tier_rollup_fixed_point_no_action_on_second_pass() {
        use mev::brain::emit::plan_tier_rollups;

        let tmp = tempfile::tempdir().unwrap();
        let status_path = tmp.path().join("core/planning/status.md");
        std::fs::create_dir_all(status_path.parent().unwrap()).unwrap();
        std::fs::write(&status_path, status_doc_with_sentinel()).unwrap();

        let leaf_tracks = vec![Track {
            title: "P".to_string(),
            blocks: vec![track_block(
                "RA.1.A",
                "Repo A block",
                Some("in_progress"),
                Some(1),
                vec![],
            )],
        }];
        let leaf_file = make_leaf_file("repo-a", leaf_tracks, Focus::default());
        let leaf_src = StateSource {
            repo_slug: "repo-a".to_string(),
            abs_path: PathBuf::from("/fake/repo-a/planning/state.json"),
            expected_kind: "project",
        };

        let mut tier_file = make_brain_file(vec![], vec![], Focus::default());
        tier_file.repo = "core".to_string();
        let tier_src = StateSource {
            repo_slug: "core".to_string(),
            abs_path: tmp.path().join("core/planning/state.json"),
            expected_kind: "brain",
        };

        let files = vec![(leaf_src, leaf_file), (tier_src, tier_file)];
        let graph = build_state_graph(&files);
        let config = config_with_repos(&[("repo-a", "core")]);

        // First pass produces an action; write it to disk.
        let plan1 = plan_tier_rollups(&files, &graph, &config);
        assert_eq!(
            plan1.actions.len(),
            1,
            "first pass should produce an action"
        );
        std::fs::write(&status_path, &plan1.actions[0].new_content).unwrap();

        // Second pass over the already-correct content: no action.
        let plan2 = plan_tier_rollups(&files, &graph, &config);
        assert_eq!(
            plan2.actions.len(),
            0,
            "second pass (fixed point) should produce no action; got {}",
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

// ---------------------------------------------------------------------------
// MV.3B.U Task 4 — end-to-end `emit_state` tier-scoping + brain-focus
// integration tests.
//
// Covers: tier-scoped `repos[]` rollup (derived + preserved + stub branches,
// none dropped), the malformed-child preserve regression (reproduces the live
// bastion-drop incident), repo-tagged brain-focus union, an HQ-shaped fixture
// aggregating across all tiers, and the write/write fixed-point property.
// ---------------------------------------------------------------------------

mod task4_tier_scoping_integration {
    use std::fs;
    use std::path::Path;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("mev-tier-scoping-{tag}"));
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

    /// `brain.toml` declaring 5 core-tier repos (repo-a..repo-e) and 1
    /// portfolio-tier repo (repo-p) — the "≥5 core-tier repos where only 2 have
    /// a state.json" fixture shape required by the spec.
    fn write_brain_toml(root: &Path) {
        let toml = r#"[vocab]
layer = ["brain", "engine", "factory", "console", "surface", "infra", "business", "content", "meta"]
status = ["active", "draft", "deprecated", "superseded", "archived"]

[crawl]
skip_dirs = ["target", "node_modules", ".git"]

[[repos]]
slug = "repo-a"
tier = "core"
repo_path = "repos/repo-a"
status_file = "repos/repo-a/planning/status.md"
cache_doc = "docs/projects/repo-a.md"
heading = "Repo A"

[[repos]]
slug = "repo-b"
tier = "core"
repo_path = "repos/repo-b"
status_file = "repos/repo-b/planning/status.md"
cache_doc = "docs/projects/repo-b.md"
heading = "Repo B"

[[repos]]
slug = "repo-c"
tier = "core"
repo_path = "repos/repo-c"
status_file = "repos/repo-c/planning/status.md"
cache_doc = "docs/projects/repo-c.md"
heading = "Repo C"

[[repos]]
slug = "repo-d"
tier = "core"
repo_path = "repos/repo-d"
status_file = "repos/repo-d/planning/status.md"
cache_doc = "docs/projects/repo-d.md"
heading = "Repo D"

[[repos]]
slug = "repo-e"
tier = "core"
repo_path = "repos/repo-e"
status_file = "repos/repo-e/planning/status.md"
cache_doc = "docs/projects/repo-e.md"
heading = "Repo E"

[[repos]]
slug = "repo-p"
tier = "portfolio"
repo_path = "repos/repo-p"
status_file = "repos/repo-p/planning/status.md"
cache_doc = "docs/projects/repo-p.md"
heading = "Repo P"
"#;
        fs::write(root.join("brain.toml"), toml.as_bytes()).unwrap();
    }

    /// The core-tier brain's own `planning/state.json`. `repo: "core"` matches
    /// the `tier = "core"` value in `brain.toml`, so `tier_scope_for` scopes it
    /// to just the core-tier repos. Carries pre-existing `repos[]` entries for
    /// repo-c (no state.json at all) and repo-d (state.json present but
    /// malformed) so the preserve branch has something to preserve.
    fn write_core_brain_state(root: &Path) {
        let state = serde_json::json!({
            "repo": "core",
            "kind": "brain",
            "updated": "2026-06-29",
            "focus": { "now": [], "next": [], "blocked": [] },
            "repos": [
                {
                    "repo": "repo-c",
                    "now": [{ "id": "RC.1.A", "title": "Repo C cached now", "status": "in_progress" }],
                    "next": [],
                    "blocked": []
                },
                {
                    "repo": "repo-d",
                    "now": [],
                    "next": [{ "id": "RD.1.A", "title": "Repo D cached next", "status": null }],
                    "blocked": []
                }
            ],
            "cross_repo": []
        });
        write_json(root, "planning/state.json", &state);
    }

    fn write_leaf_state(root: &Path, rel: &str, repo: &str, block_id: &str, title: &str) {
        let state = serde_json::json!({
            "repo": repo,
            "kind": "project",
            "updated": "2026-06-29",
            "focus": { "now": [], "next": [], "blocked": [] },
            "tracks": [
                {
                    "title": "Phase 1",
                    "blocks": [
                        { "id": block_id, "title": title, "status": "in_progress" }
                    ]
                }
            ]
        });
        write_json(root, rel, &state);
    }

    /// Full core-tier fixture: brain.toml + core brain state + repo-a/repo-b
    /// (loadable) + repo-d (malformed) + repo-p (portfolio-tier control,
    /// loadable but must be excluded from the core rollup). repo-c and repo-e
    /// intentionally have no `planning/state.json` on disk at all.
    fn write_core_fixture(root: &Path) {
        write_brain_toml(root);
        write_core_brain_state(root);
        write_leaf_state(
            root,
            "repos/repo-a/planning/state.json",
            "repo-a",
            "RA.1.A",
            "Repo A block A",
        );
        write_leaf_state(
            root,
            "repos/repo-b/planning/state.json",
            "repo-b",
            "RB.1.A",
            "Repo B block A",
        );
        // repo-d: state.json exists but is not valid JSON (E_STATE_MALFORMED_JSON).
        write_file(
            root,
            "repos/repo-d/planning/state.json",
            "{ this is not valid json ",
        );
        write_leaf_state(
            root,
            "repos/repo-p/planning/state.json",
            "repo-p",
            "RP.1.A",
            "Repo P block A",
        );
    }

    fn repos_by_slug(
        state: &serde_json::Value,
    ) -> std::collections::HashMap<String, serde_json::Value> {
        state["repos"]
            .as_array()
            .expect("repos[] must be an array")
            .iter()
            .map(|r| (r["repo"].as_str().unwrap().to_string(), r.clone()))
            .collect()
    }

    // -----------------------------------------------------------------------
    // Test 1 — tier-scoped rollup: all 5 core-tier repos present, none dropped,
    // portfolio-tier repo excluded, tier populated on every entry.
    // -----------------------------------------------------------------------

    #[test]
    fn core_tier_rollup_contains_all_in_scope_repos_and_excludes_other_tiers() {
        let dir = temp_dir("core-rollup");
        write_core_fixture(&dir);

        let report = mev::emit_state(&dir, true).expect("emit_state should not error (panic)");
        // The only expected error is E_STATE_MALFORMED_JSON for repo-d (see the
        // dedicated regression test below) — it must not abort the rollup.
        let non_malformed_errors: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.severity == mev::Severity::Error && d.locator != "E_STATE_MALFORMED_JSON")
            .collect();
        assert!(
            non_malformed_errors.is_empty(),
            "core fixture emit should have no unexpected errors; got: {non_malformed_errors:#?}"
        );

        let brain_after: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(dir.join("planning/state.json")).unwrap())
                .unwrap();
        let repos = repos_by_slug(&brain_after);

        // All 5 in-scope core-tier repos present, none dropped.
        for slug in ["repo-a", "repo-b", "repo-c", "repo-d", "repo-e"] {
            assert!(
                repos.contains_key(slug),
                "repos[] must contain '{slug}'; got slugs: {:?}",
                repos.keys().collect::<Vec<_>>()
            );
            assert_eq!(
                repos[slug]["tier"].as_str(),
                Some("core"),
                "'{slug}' must have tier populated as 'core'"
            );
        }

        // Portfolio-tier repo must NOT appear in the core brain's rollup.
        assert!(
            !repos.contains_key("repo-p"),
            "portfolio-tier repo-p must be excluded from the core-tier rollup"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------------
    // Test 2 — derived branch: repo-a/repo-b get freshly derived headlines.
    // -----------------------------------------------------------------------

    #[test]
    fn derived_repos_get_fresh_headline_from_child_tracks() {
        let dir = temp_dir("derived-branch");
        write_core_fixture(&dir);

        mev::emit_state(&dir, true).expect("emit_state should not error");

        let brain_after: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(dir.join("planning/state.json")).unwrap())
                .unwrap();
        let repos = repos_by_slug(&brain_after);

        let repo_a_now = repos["repo-a"]["now"].as_array().unwrap();
        assert!(
            repo_a_now
                .iter()
                .any(|b| b["id"].as_str() == Some("RA.1.A")),
            "repo-a's derived now[] must include its in_progress block RA.1.A; got: {repo_a_now:?}"
        );

        let repo_b_now = repos["repo-b"]["now"].as_array().unwrap();
        assert!(
            repo_b_now
                .iter()
                .any(|b| b["id"].as_str() == Some("RB.1.A")),
            "repo-b's derived now[] must include its in_progress block RB.1.A; got: {repo_b_now:?}"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------------
    // Test 3 — preserve branch: repo-c (no state.json at all) keeps its
    // existing cached entry verbatim, just with tier backfilled.
    // -----------------------------------------------------------------------

    #[test]
    fn sourceless_repo_preserves_existing_cached_entry() {
        let dir = temp_dir("preserve-branch");
        write_core_fixture(&dir);

        mev::emit_state(&dir, true).expect("emit_state should not error");

        let brain_after: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(dir.join("planning/state.json")).unwrap())
                .unwrap();
        let repos = repos_by_slug(&brain_after);

        let repo_c = &repos["repo-c"];
        assert_eq!(repo_c["tier"].as_str(), Some("core"));
        let now = repo_c["now"].as_array().unwrap();
        assert!(
            now.iter().any(|b| b["id"].as_str() == Some("RC.1.A")),
            "repo-c's cached now[] entry (RC.1.A) must be preserved verbatim; got: {now:?}"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------------
    // Test 4 — REGRESSION: a malformed child state.json must NOT truncate the
    // brain repos[] — its existing entry is preserved (reproduces the live
    // bastion-drop incident this block fixes).
    // -----------------------------------------------------------------------

    #[test]
    fn malformed_child_state_json_does_not_truncate_rollup() {
        let dir = temp_dir("malformed-regression");
        write_core_fixture(&dir);

        let report = mev::emit_state(&dir, true).expect("emit_state should not error");

        // The malformed repo-d state.json should surface as a load error, but
        // must not be a hard failure that aborts the whole rollup.
        let malformed_diag = report
            .diagnostics
            .iter()
            .find(|d| d.locator == "E_STATE_MALFORMED_JSON");
        assert!(
            malformed_diag.is_some(),
            "expected E_STATE_MALFORMED_JSON for repo-d; got: {:#?}",
            report.diagnostics
        );

        let brain_after: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(dir.join("planning/state.json")).unwrap())
                .unwrap();
        let repos = repos_by_slug(&brain_after);

        // repo-d must still be present (preserved), not dropped.
        assert!(
            repos.contains_key("repo-d"),
            "repo-d must be preserved despite malformed state.json; got slugs: {:?}",
            repos.keys().collect::<Vec<_>>()
        );
        let repo_d = &repos["repo-d"];
        assert_eq!(repo_d["tier"].as_str(), Some("core"));
        let next = repo_d["next"].as_array().unwrap();
        assert!(
            next.iter().any(|b| b["id"].as_str() == Some("RD.1.A")),
            "repo-d's cached next[] entry (RD.1.A) must be preserved verbatim; got: {next:?}"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------------
    // Test 5 — stub branch: repo-e (no state.json, no existing entry) gets a
    // tier-tagged empty stub.
    // -----------------------------------------------------------------------

    #[test]
    fn repo_with_no_source_and_no_existing_entry_gets_empty_stub() {
        let dir = temp_dir("stub-branch");
        write_core_fixture(&dir);

        mev::emit_state(&dir, true).expect("emit_state should not error");

        let brain_after: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(dir.join("planning/state.json")).unwrap())
                .unwrap();
        let repos = repos_by_slug(&brain_after);

        let repo_e = &repos["repo-e"];
        assert_eq!(repo_e["tier"].as_str(), Some("core"));
        assert!(repo_e["now"].as_array().unwrap().is_empty());
        assert!(repo_e["next"].as_array().unwrap().is_empty());
        assert!(repo_e["blocked"].as_array().unwrap().is_empty());

        let _ = fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------------
    // Test 6 — brain focus is the repo-tagged union of loadable children's
    // derived focus (repo-c/d/e contribute nothing — no live tracks).
    // -----------------------------------------------------------------------

    #[test]
    fn brain_focus_is_repo_tagged_union_of_loadable_children() {
        let dir = temp_dir("focus-union");
        write_core_fixture(&dir);

        mev::emit_state(&dir, true).expect("emit_state should not error");

        let brain_after: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(dir.join("planning/state.json")).unwrap())
                .unwrap();
        let now = brain_after["focus"]["now"].as_array().unwrap();

        let has = |repo: &str, id: &str| {
            now.iter()
                .any(|b| b["repo"].as_str() == Some(repo) && b["id"].as_str() == Some(id))
        };
        assert!(
            has("repo-a", "RA.1.A"),
            "brain focus.now must include repo-a's RA.1.A tagged with repo: repo-a; got: {now:?}"
        );
        assert!(
            has("repo-b", "RB.1.A"),
            "brain focus.now must include repo-b's RB.1.A tagged with repo: repo-b; got: {now:?}"
        );

        // Portfolio-tier repo-p must not leak into the core brain's focus union.
        assert!(
            !now.iter().any(|b| b["repo"].as_str() == Some("repo-p")),
            "portfolio-tier repo-p must not appear in the core brain's focus union; got: {now:?}"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------------
    // Test 7 — HQ-shaped fixture (repo not a tier name) aggregates across ALL
    // tiers (core + portfolio).
    // -----------------------------------------------------------------------

    #[test]
    fn hq_shaped_brain_aggregates_across_all_tiers() {
        let dir = temp_dir("hq-all-tiers");
        write_brain_toml(&dir);

        // HQ brain: repo "hq" matches no tier value in brain.toml -> TierScope::All.
        let state = serde_json::json!({
            "repo": "hq",
            "kind": "brain",
            "updated": "2026-06-29",
            "focus": { "now": [], "next": [], "blocked": [] },
            "repos": [],
            "cross_repo": []
        });
        write_json(&dir, "planning/state.json", &state);

        write_leaf_state(
            &dir,
            "repos/repo-a/planning/state.json",
            "repo-a",
            "RA.1.A",
            "Repo A block A",
        );
        write_leaf_state(
            &dir,
            "repos/repo-p/planning/state.json",
            "repo-p",
            "RP.1.A",
            "Repo P block A",
        );

        mev::emit_state(&dir, true).expect("emit_state should not error");

        let brain_after: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(dir.join("planning/state.json")).unwrap())
                .unwrap();
        let repos = repos_by_slug(&brain_after);

        // HQ scope is "All" -> every configured repo appears, core and portfolio alike.
        for slug in ["repo-a", "repo-b", "repo-c", "repo-d", "repo-e", "repo-p"] {
            assert!(
                repos.contains_key(slug),
                "HQ rollup (TierScope::All) must include '{slug}'; got slugs: {:?}",
                repos.keys().collect::<Vec<_>>()
            );
        }
        assert_eq!(repos["repo-a"]["tier"].as_str(), Some("core"));
        assert_eq!(repos["repo-p"]["tier"].as_str(), Some("portfolio"));

        let now = brain_after["focus"]["now"].as_array().unwrap();
        assert!(
            now.iter()
                .any(|b| b["repo"].as_str() == Some("repo-a") && b["id"].as_str() == Some("RA.1.A")),
            "HQ focus.now must include repo-a's block; got: {now:?}"
        );
        assert!(
            now.iter()
                .any(|b| b["repo"].as_str() == Some("repo-p") && b["id"].as_str() == Some("RP.1.A")),
            "HQ focus.now must include repo-p's block (all-tiers union); got: {now:?}"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------------
    // Test 8 — fixed point: a second `emit_state --write` over the core
    // fixture is a no-op (no I_EMIT_WROTE diagnostics, files unchanged).
    // -----------------------------------------------------------------------

    #[test]
    fn second_write_over_core_fixture_is_a_no_op() {
        let dir = temp_dir("fixed-point");
        write_core_fixture(&dir);

        mev::emit_state(&dir, true).expect("first emit should not error");

        let brain_after1 = fs::read(dir.join("planning/state.json")).unwrap();
        let repo_a_after1 = fs::read(dir.join("repos/repo-a/planning/state.json")).unwrap();
        let repo_b_after1 = fs::read(dir.join("repos/repo-b/planning/state.json")).unwrap();

        let report2 = mev::emit_state(&dir, true).expect("second emit should not error");

        let wrote2: Vec<_> = report2
            .diagnostics
            .iter()
            .filter(|d| d.locator == "I_EMIT_WROTE")
            .collect();
        assert!(
            wrote2.is_empty(),
            "second write over already-derived core fixture must be a no-op; got: {wrote2:#?}"
        );

        assert_eq!(
            fs::read(dir.join("planning/state.json")).unwrap(),
            brain_after1,
            "brain state.json must be stable after second write"
        );
        assert_eq!(
            fs::read(dir.join("repos/repo-a/planning/state.json")).unwrap(),
            repo_a_after1,
            "repo-a state.json must be stable after second write"
        );
        assert_eq!(
            fs::read(dir.join("repos/repo-b/planning/state.json")).unwrap(),
            repo_b_after1,
            "repo-b state.json must be stable after second write"
        );

        let _ = fs::remove_dir_all(&dir);
    }
}

// ---------------------------------------------------------------------------
// MV.4.E — brain-wide close-A-unblocks-B ripple integration test.
//
// Builds a multi-repo fixture corpus (HQ brain + one tier sub-brain + two leaf
// project repos) where repo-b's block depends cross-repo on repo-a's block,
// and every generated surface (leaf state.json focus, leaf project-cache doc,
// tier rollup table, HQ operating board, master-plan wave table) is wired to
// a sentinel-bearing document. Flipping repo-a's block from `in_progress` to
// `closed` and running `emit_state` once must ripple the change through every
// one of those surfaces in a single pass, and a second pass must be a no-op
// (the fixed-point property MV.4.B/C/D already guarantee per-surface).
// ---------------------------------------------------------------------------

mod mv4e_ripple {
    use std::fs;
    use std::path::Path;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("mev-mv4e-ripple-{tag}"));
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

    /// `brain.toml` registering one tier ("core") with two leaf project repos
    /// (repo-a, repo-b), each with a project-cache doc.
    fn write_brain_toml(root: &Path) {
        let toml = r#"[vocab]
layer = ["brain", "engine", "factory", "console", "surface", "infra", "business", "content", "meta"]
status = ["active", "draft", "deprecated", "superseded", "archived"]

[crawl]
skip_dirs = ["target", "node_modules", ".git"]

[[repos]]
slug = "repo-a"
tier = "core"
repo_path = "repos/repo-a"
status_file = "repos/repo-a/planning/status.md"
cache_doc = "docs/projects/repo-a.md"
heading = "Repo A"

[[repos]]
slug = "repo-b"
tier = "core"
repo_path = "repos/repo-b"
status_file = "repos/repo-b/planning/status.md"
cache_doc = "docs/projects/repo-b.md"
heading = "Repo B"
"#;
        fs::write(root.join("brain.toml"), toml.as_bytes()).unwrap();
    }

    /// HQ brain `planning/state.json` (kind:"brain", repo "hq" matches no
    /// declared tier -> TierScope::All) pointing at the "core" tier sub-brain.
    fn write_hq_state(root: &Path) {
        let state = serde_json::json!({
            "repo": "hq",
            "kind": "brain",
            "updated": "2026-07-01",
            "focus": { "now": [], "next": [], "blocked": [] },
            "repos": [],
            "cross_repo": [],
            "tiers": [
                { "tier": "core", "rollup": "core/planning/state.json", "summary": null }
            ]
        });
        write_json(root, "planning/state.json", &state);
    }

    /// HQ `status.md` carrying the HQ_BOARD sentinel (OKF-fronted).
    fn write_hq_status_md(root: &Path) {
        let doc = "---\n\
                    type: ProjectStatus\n\
                    title: HQ status\n\
                    description: HQ operating board fixture.\n\
                    ---\n\n\
                    # HQ Status\n\n\
                    <!-- BEGIN generated:hq-board -->\n\
                    <!-- END generated:hq-board -->\n";
        write_file(root, "planning/status.md", doc);
    }

    /// The "core" tier sub-brain's own `planning/state.json`.
    fn write_core_tier_state(root: &Path) {
        let state = serde_json::json!({
            "repo": "core",
            "kind": "brain",
            "updated": "2026-07-01",
            "focus": { "now": [], "next": [], "blocked": [] },
            "repos": [],
            "cross_repo": []
        });
        write_json(root, "core/planning/state.json", &state);
    }

    /// The "core" tier sub-brain's `status.md` carrying the TIER_ROLLUP sentinel.
    fn write_core_status_md(root: &Path) {
        let doc = "---\n\
                    type: ProjectStatus\n\
                    title: Core tier status\n\
                    description: Core tier rollup fixture.\n\
                    ---\n\n\
                    # Core Tier Status\n\n\
                    <!-- BEGIN generated:tier-rollup -->\n\
                    <!-- END generated:tier-rollup -->\n";
        write_file(root, "core/planning/status.md", doc);
    }

    /// A leaf project repo's `planning/state.json`, with a single track/block.
    fn write_leaf_state(
        root: &Path,
        repo: &str,
        block_id: &str,
        title: &str,
        status: &str,
        depends_on: &serde_json::Value,
    ) {
        let state = serde_json::json!({
            "repo": repo,
            "kind": "project",
            "updated": "2026-07-01",
            "focus": { "now": [], "next": [], "blocked": [] },
            "tracks": [
                {
                    "title": "Phase 1",
                    "blocks": [
                        {
                            "id": block_id,
                            "title": title,
                            "status": status,
                            "depends_on": depends_on,
                            "wave": 1
                        }
                    ]
                }
            ]
        });
        write_json(root, &format!("repos/{repo}/planning/state.json"), &state);
    }

    /// A leaf project repo's `master-plan.md`, carrying the WAVE_TABLE sentinel.
    fn write_leaf_master_plan(root: &Path, repo: &str) {
        let doc = format!(
            "---\n\
             type: Plan\n\
             title: {repo} master plan\n\
             description: Wave table fixture for {repo}.\n\
             ---\n\n\
             # Master Plan\n\n\
             <!-- BEGIN generated:wave-table -->\n\
             <!-- END generated:wave-table -->\n"
        );
        write_file(root, &format!("repos/{repo}/planning/master-plan.md"), &doc);
    }

    /// A leaf project repo's own `planning/status.md`, carrying the OKF
    /// `timestamp` watermark [`plan_project_caches`] reconciles the brain
    /// cache doc's `synced_from` field against.
    fn write_leaf_status_md(root: &Path, repo: &str, timestamp: &str) {
        let doc = format!(
            "---\n\
             type: ProjectStatus\n\
             title: {repo} status\n\
             description: Status fixture for {repo}.\n\
             timestamp: \"{timestamp}\"\n\
             ---\n\n\
             # Status\n"
        );
        write_file(root, &format!("repos/{repo}/planning/status.md"), &doc);
    }

    /// A leaf project repo's brain cache doc, carrying the PROJECT_CACHE sentinel.
    fn write_project_cache_doc(root: &Path, repo: &str) {
        let doc = format!(
            "---\n\
             type: ProjectStatus\n\
             title: {repo} cache\n\
             description: Project cache fixture for {repo}.\n\
             ---\n\n\
             # {repo}\n\n\
             <!-- BEGIN generated:project-cache -->\n\
             <!-- END generated:project-cache -->\n"
        );
        write_file(root, &format!("docs/projects/{repo}.md"), &doc);
    }

    /// Full fixture corpus: brain.toml + HQ (state.json + status.md) + core
    /// tier (state.json + status.md) + repo-a/repo-b (state.json +
    /// master-plan.md + project-cache doc). repo-b's block `RB.1.A` depends
    /// cross-repo on repo-a's block `RA.1.A`.
    ///
    /// `a_status` controls repo-a's block's authored status, so the caller
    /// can build the "before" (`in_progress`) and "after" (`closed`) fixture
    /// states without duplicating the rest of the corpus.
    fn write_corpus(root: &Path, a_status: &str) {
        write_brain_toml(root);
        write_hq_state(root);
        write_hq_status_md(root);
        write_core_tier_state(root);
        write_core_status_md(root);

        write_leaf_state(
            root,
            "repo-a",
            "RA.1.A",
            "Repo A block",
            a_status,
            &serde_json::json!([]),
        );
        write_leaf_master_plan(root, "repo-a");
        write_leaf_status_md(root, "repo-a", "2026-07-01T12:00:00Z");
        write_project_cache_doc(root, "repo-a");

        write_leaf_state(
            root,
            "repo-b",
            "RB.1.A",
            "Repo B block",
            "open",
            &serde_json::json!([{ "type": "block", "repo": "repo-a", "id": "RA.1.A" }]),
        );
        write_leaf_master_plan(root, "repo-b");
        write_leaf_status_md(root, "repo-b", "2026-07-01T12:00:00Z");
        write_project_cache_doc(root, "repo-b");
    }

    fn read(root: &Path, rel: &str) -> String {
        fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("failed to read {rel}: {e}"))
    }

    fn read_json(root: &Path, rel: &str) -> serde_json::Value {
        serde_json::from_str(&read(root, rel)).unwrap_or_else(|e| panic!("bad json {rel}: {e}"))
    }

    #[test]
    fn close_a_unblocks_b_ripples_across_every_surface() {
        let dir = temp_dir("close-a-unblocks-b");

        // --- Before: repo-a's block is in_progress; repo-b's block is open
        // and depends on it, so it must render as blocked everywhere. ---
        write_corpus(&dir, "in_progress");

        let report_before = mev::emit_state(&dir, true).expect("first emit should not error");
        let unexpected_errors: Vec<_> = report_before
            .diagnostics
            .iter()
            .filter(|d| d.severity == mev::Severity::Error)
            .collect();
        assert!(
            unexpected_errors.is_empty(),
            "fixture emit should have no errors; got: {unexpected_errors:#?}"
        );

        let repo_b_master_plan_before = read(&dir, "repos/repo-b/planning/master-plan.md");
        assert!(
            repo_b_master_plan_before
                .contains("| RB.1.A | Repo B block | blocked | repo-a:RA.1.A |"),
            "repo-b's wave table must render RB.1.A as blocked before the flip; got:\n{repo_b_master_plan_before}"
        );

        let hq_board_before = read(&dir, "planning/status.md");
        assert!(
            hq_board_before.contains("- repo-a:RA.1.A — Repo A block"),
            "HQ board must list repo-a's in_progress block in NOW before the flip; got:\n{hq_board_before}"
        );
        assert!(
            hq_board_before.contains("- repo-b:RB.1.A — Repo B block (blocked by repo-a:RA.1.A)"),
            "HQ board must list repo-b's block as BLOCKED before the flip; got:\n{hq_board_before}"
        );
        let now_idx = hq_board_before.find("## NOW").unwrap();
        let next_idx = hq_board_before.find("## NEXT").unwrap();
        let now_section = &hq_board_before[now_idx..next_idx];
        assert!(
            !now_section.contains("repo-a:RA.1.A")
                || now_section.contains("- repo-a:RA.1.A — Repo A block"),
            "sanity: repo-a's block should be in NOW, not NEXT, before the flip"
        );
        assert!(
            !hq_board_before.contains("- repo-a:RA.1.A")
                || hq_board_before.find("- repo-a:RA.1.A").unwrap() < next_idx,
            "repo-a's block must not appear in NEXT before the flip"
        );

        // --- Flip: close repo-a's block, rewriting only its state.json. ---
        write_leaf_state(
            &dir,
            "repo-a",
            "RA.1.A",
            "Repo A block",
            "closed",
            &serde_json::json!([]),
        );

        let report_after = mev::emit_state(&dir, true).expect("second emit should not error");
        let unexpected_errors: Vec<_> = report_after
            .diagnostics
            .iter()
            .filter(|d| d.severity == mev::Severity::Error)
            .collect();
        assert!(
            unexpected_errors.is_empty(),
            "post-flip emit should have no errors; got: {unexpected_errors:#?}"
        );

        // 1. repo-a's leaf cache: focus line updated, synced_from reconciled.
        let repo_a_cache = read(&dir, "docs/projects/repo-a.md");
        assert!(
            repo_a_cache.contains("**Current focus:** none. Next: none. Blocked: none."),
            "repo-a's project cache must reflect the closed block (empty focus); got:\n{repo_a_cache}"
        );
        assert!(
            repo_a_cache.contains("synced_from:"),
            "repo-a's project cache must have a reconciled synced_from watermark; got:\n{repo_a_cache}"
        );

        // 2. repo-b's leaf cache: focus line now shows RB.1.A as ready (`next`).
        let repo_b_cache = read(&dir, "docs/projects/repo-b.md");
        assert!(
            repo_b_cache.contains("Next: `RB.1.A` — Repo B block."),
            "repo-b's project cache must show RB.1.A as next once unblocked; got:\n{repo_b_cache}"
        );

        // 3. Tier rollup row (core/planning/status.md) reflects both repos.
        let tier_rollup = read(&dir, "core/planning/status.md");
        assert!(
            tier_rollup.contains("| repo-a |"),
            "tier rollup must contain a repo-a row; got:\n{tier_rollup}"
        );
        assert!(
            tier_rollup.contains("`RB.1.A` — Repo B block"),
            "tier rollup's repo-b row must list RB.1.A once unblocked; got:\n{tier_rollup}"
        );

        // 4. HQ board: B moved from BLOCKED to NEXT, A dropped out entirely
        // (closed blocks appear in no section).
        let hq_board_after = read(&dir, "planning/status.md");
        assert!(
            !hq_board_after.contains("repo-a:RA.1.A"),
            "closed repo-a block must no longer appear anywhere on the HQ board; got:\n{hq_board_after}"
        );
        assert!(
            !hq_board_after.contains("- repo-b:RB.1.A — Repo B block (blocked"),
            "repo-b's block must no longer render as BLOCKED on the HQ board; got:\n{hq_board_after}"
        );
        assert!(
            hq_board_after.contains("- repo-b:RB.1.A — Repo B block"),
            "repo-b's block must appear (unblocked) on the HQ board; got:\n{hq_board_after}"
        );
        let next_idx_after = hq_board_after.find("## NEXT").unwrap();
        let blocked_idx_after = hq_board_after.find("## BLOCKED").unwrap();
        let next_section_after = &hq_board_after[next_idx_after..blocked_idx_after];
        assert!(
            next_section_after.contains("repo-b:RB.1.A"),
            "repo-b's block must be listed under NEXT on the HQ board; got:\n{hq_board_after}"
        );

        // 5. repo-b's own leaf state.json `focus` no longer shows RB.1.A blocked.
        let repo_b_state = read_json(&dir, "repos/repo-b/planning/state.json");
        let blocked = repo_b_state["focus"]["blocked"].as_array().unwrap();
        assert!(
            !blocked.iter().any(|b| b["id"].as_str() == Some("RB.1.A")),
            "repo-b's derived focus.blocked must no longer contain RB.1.A; got: {blocked:?}"
        );
        let next = repo_b_state["focus"]["next"].as_array().unwrap();
        assert!(
            next.iter().any(|b| b["id"].as_str() == Some("RB.1.A")),
            "repo-b's derived focus.next must contain RB.1.A once unblocked; got: {next:?}"
        );

        // 6. The master-plan wave/status cell for RB.1.A reflects the unblock.
        let repo_b_master_plan_after = read(&dir, "repos/repo-b/planning/master-plan.md");
        assert!(
            repo_b_master_plan_after.contains("| RB.1.A | Repo B block | open | repo-a:RA.1.A |"),
            "repo-b's wave table must render RB.1.A as open once repo-a is closed; got:\n{repo_b_master_plan_after}"
        );

        // --- Fixed point: a second write over the emitted corpus is a no-op. ---
        let snapshot: Vec<(std::path::PathBuf, Vec<u8>)> = [
            "planning/state.json",
            "planning/status.md",
            "core/planning/state.json",
            "core/planning/status.md",
            "repos/repo-a/planning/state.json",
            "repos/repo-a/planning/master-plan.md",
            "repos/repo-b/planning/state.json",
            "repos/repo-b/planning/master-plan.md",
            "docs/projects/repo-a.md",
            "docs/projects/repo-b.md",
        ]
        .iter()
        .map(|rel| (dir.join(rel), fs::read(dir.join(rel)).unwrap()))
        .collect();

        let report_fixed_point =
            mev::emit_state(&dir, true).expect("fixed-point emit should not error");
        let wrote: Vec<_> = report_fixed_point
            .diagnostics
            .iter()
            .filter(|d| d.locator == "I_EMIT_WROTE")
            .collect();
        assert!(
            wrote.is_empty(),
            "second write over the emitted corpus must be a no-op; got: {wrote:#?}"
        );

        for (path, before) in &snapshot {
            let after = fs::read(path).unwrap();
            assert_eq!(
                &after,
                before,
                "{} must be byte-identical after the fixed-point pass",
                path.display()
            );
        }

        let _ = fs::remove_dir_all(&dir);
    }
}

// ---------------------------------------------------------------------------
// Task 1 (MV.4.A) — generated-marker name constants
// ---------------------------------------------------------------------------

mod task1_markers {
    use super::*;

    #[test]
    fn marker_constants_have_expected_values() {
        assert_eq!(markers::WAVE_TABLE, "wave-table");
        assert_eq!(markers::PROJECT_CACHE, "project-cache");
        assert_eq!(markers::TIER_ROLLUP, "tier-rollup");
        assert_eq!(markers::HQ_BOARD, "hq-board");
    }

    #[test]
    fn splice_generated_works_with_wave_table_constant() {
        let original = format!(
            "before\n<!-- BEGIN generated:{m} -->\nold\n<!-- END generated:{m} -->\nafter\n",
            m = markers::WAVE_TABLE
        );
        let result = splice_generated(&original, markers::WAVE_TABLE, "new-table").unwrap();
        let expected = format!(
            "before\n<!-- BEGIN generated:{m} -->\nnew-table\n<!-- END generated:{m} -->\nafter\n",
            m = markers::WAVE_TABLE
        );
        assert_eq!(result, expected);
    }

    #[test]
    fn splice_generated_works_with_each_marker_constant() {
        for marker in [
            markers::WAVE_TABLE,
            markers::PROJECT_CACHE,
            markers::TIER_ROLLUP,
            markers::HQ_BOARD,
        ] {
            let original = format!(
                "<!-- BEGIN generated:{marker} -->\nstale\n<!-- END generated:{marker} -->\n"
            );
            let result = splice_generated(&original, marker, "fresh").unwrap();
            assert!(
                result.contains("fresh"),
                "marker '{marker}' should splice successfully"
            );
            // Idempotent: re-splicing with the same content yields the same result.
            let result2 = splice_generated(&result, marker, "fresh").unwrap();
            assert_eq!(
                result, result2,
                "splice for marker '{marker}' must be idempotent"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// global_status_map tests — Task 2
// ---------------------------------------------------------------------------

mod task2_global_status_map {
    use super::*;

    #[test]
    fn multi_repo_input_produces_correctly_namespaced_keys() {
        let files = vec![
            (
                make_src("core"),
                make_leaf(
                    "core",
                    vec![Track {
                        title: "Phase 1".to_string(),
                        blocks: vec![block("A", "Block A", Some("closed"), Some(1))],
                    }],
                ),
            ),
            (
                make_src("mev"),
                make_leaf(
                    "mev",
                    vec![Track {
                        title: "Phase 1".to_string(),
                        blocks: vec![block("A", "Block A (mev)", Some("open"), Some(1))],
                    }],
                ),
            ),
        ];

        let map = global_status_map(&files);

        assert_eq!(map.get("core:A"), Some(&Some("closed".to_string())));
        assert_eq!(map.get("mev:A"), Some(&Some("open".to_string())));
    }

    #[test]
    fn keys_from_different_repos_do_not_collide() {
        let files = vec![
            (
                make_src("core"),
                make_leaf(
                    "core",
                    vec![Track {
                        title: "Phase 1".to_string(),
                        blocks: vec![block("X", "Core X", Some("closed"), None)],
                    }],
                ),
            ),
            (
                make_src("bastion"),
                make_leaf(
                    "bastion",
                    vec![Track {
                        title: "Phase 1".to_string(),
                        blocks: vec![block("X", "Bastion X", Some("in_progress"), None)],
                    }],
                ),
            ),
        ];

        let map = global_status_map(&files);

        assert_eq!(map.len(), 2);
        assert_eq!(map.get("core:X"), Some(&Some("closed".to_string())));
        assert_eq!(map.get("bastion:X"), Some(&Some("in_progress".to_string())));
        assert_ne!(map.get("core:X"), map.get("bastion:X"));
    }

    #[test]
    fn block_with_absent_status_maps_to_none() {
        let files = vec![(
            make_src("mev"),
            make_leaf(
                "mev",
                vec![Track {
                    title: "Phase 1".to_string(),
                    blocks: vec![block("NOSTATUS", "No status block", None, Some(2))],
                }],
            ),
        )];

        let map = global_status_map(&files);

        assert_eq!(map.get("mev:NOSTATUS"), Some(&None));
    }

    #[test]
    fn empty_files_produce_empty_map() {
        let files: Vec<(mev::brain::state::StateSource, StateFile)> = vec![];
        let map = global_status_map(&files);
        assert!(map.is_empty());
    }
}

// ---------------------------------------------------------------------------
// render_hq_board tests (Task 1, MV.4.C)
// ---------------------------------------------------------------------------

mod task1_render_hq_board {
    use mev::brain::emit::render_hq_board;
    use mev::brain::state::{Block, BlockedBy, CrossRepoEdge, Endpoint, Focus};

    /// Build a repo-tagged `Block` with no `blocked_by` entries (NOW/NEXT shape).
    fn tagged_block(repo: &str, id: &str, title: &str) -> Block {
        Block {
            due: None,
            priority: None,
            id: id.to_string(),
            title: title.to_string(),
            status: None,
            note: None,
            repo: Some(repo.to_string()),
            blocked_by: Vec::new(),
        }
    }

    /// Build a repo-tagged `Block` with the given `blocked_by` entries (BLOCKED shape).
    fn blocked_block(repo: &str, id: &str, title: &str, blocked_by: Vec<BlockedBy>) -> Block {
        Block {
            due: None,
            priority: None,
            id: id.to_string(),
            title: title.to_string(),
            status: None,
            note: None,
            repo: Some(repo.to_string()),
            blocked_by,
        }
    }

    fn cross_repo_edge(
        from_repo: &str,
        from_id: &str,
        to_repo: &str,
        to_id: &str,
        note: Option<&str>,
    ) -> CrossRepoEdge {
        CrossRepoEdge {
            from: Endpoint {
                repo: from_repo.to_string(),
                id: from_id.to_string(),
            },
            to: Endpoint {
                repo: to_repo.to_string(),
                id: to_id.to_string(),
            },
            note: note.map(|s| s.to_string()),
        }
    }

    #[test]
    fn renders_now_next_blocked_sections_with_repo_tagged_lines() {
        let focus = Focus {
            now: vec![tagged_block("core", "A", "Block A")],
            next: vec![tagged_block("bastion", "B", "Block B")],
            blocked: vec![],
        };

        let rendered = render_hq_board(&focus, &[]);

        let expected = "## NOW\n\
- core:A — Block A\n\
\n\
## NEXT\n\
- bastion:B — Block B\n\
\n\
## BLOCKED\n\
_none_";

        assert_eq!(rendered, expected);
    }

    #[test]
    fn multi_repo_ordering_preserves_focus_order_across_sections() {
        let focus = Focus {
            now: vec![
                tagged_block("bastion", "X", "Bastion X"),
                tagged_block("core", "Y", "Core Y"),
            ],
            next: vec![],
            blocked: vec![],
        };

        let rendered = render_hq_board(&focus, &[]);

        // NOW must list bastion:X before core:Y — the renderer preserves the
        // caller-supplied (already-deterministic) order, it never re-sorts.
        let now_idx = rendered.find("- bastion:X").unwrap();
        let core_idx = rendered.find("- core:Y").unwrap();
        assert!(now_idx < core_idx);
    }

    #[test]
    fn blocked_entry_annotated_by_matching_cross_repo_edge_note() {
        let focus = Focus {
            now: vec![],
            next: vec![],
            blocked: vec![blocked_block(
                "mev",
                "C",
                "Block C",
                vec![BlockedBy::Block {
                    repo: "core".to_string(),
                    id: "D".to_string(),
                    what: None,
                }],
            )],
        };
        let edges = vec![cross_repo_edge(
            "mev",
            "C",
            "core",
            "D",
            Some("waiting on schema freeze"),
        )];

        let rendered = render_hq_board(&focus, &edges);

        assert!(rendered.contains(
            "## BLOCKED\n- mev:C — Block C (blocked by core:D (waiting on schema freeze))"
        ));
    }

    #[test]
    fn blocked_entry_falls_back_to_dep_what_when_no_edge_matches() {
        let focus = Focus {
            now: vec![],
            next: vec![],
            blocked: vec![blocked_block(
                "mev",
                "C",
                "Block C",
                vec![BlockedBy::Block {
                    repo: "core".to_string(),
                    id: "D".to_string(),
                    what: Some("needs the shared schema".to_string()),
                }],
            )],
        };

        // No cross_repo[] edges supplied — the dependency's own `what` is used.
        let rendered = render_hq_board(&focus, &[]);

        assert!(
            rendered.contains("- mev:C — Block C (blocked by core:D (needs the shared schema))")
        );
    }

    #[test]
    fn blocked_entry_external_dependency_renders_without_edge_lookup() {
        let focus = Focus {
            now: vec![],
            next: vec![],
            blocked: vec![blocked_block(
                "mev",
                "E",
                "Block E",
                vec![BlockedBy::External {
                    what: "waiting on hardware".to_string(),
                }],
            )],
        };

        let rendered = render_hq_board(&focus, &[]);

        assert!(rendered.contains("- mev:E — Block E (blocked by external:waiting on hardware)"));
    }

    #[test]
    fn blocked_entry_with_no_matching_note_renders_bare_target() {
        let focus = Focus {
            now: vec![],
            next: vec![],
            blocked: vec![blocked_block(
                "mev",
                "F",
                "Block F",
                vec![BlockedBy::Block {
                    repo: "core".to_string(),
                    id: "G".to_string(),
                    what: None,
                }],
            )],
        };

        let rendered = render_hq_board(&focus, &[]);

        assert!(rendered.contains("- mev:F — Block F (blocked by core:G)"));
    }

    #[test]
    fn multiple_blockers_on_one_block_are_comma_joined() {
        let focus = Focus {
            now: vec![],
            next: vec![],
            blocked: vec![blocked_block(
                "mev",
                "H",
                "Block H",
                vec![
                    BlockedBy::Block {
                        repo: "core".to_string(),
                        id: "I".to_string(),
                        what: None,
                    },
                    BlockedBy::External {
                        what: "budget approval".to_string(),
                    },
                ],
            )],
        };

        let rendered = render_hq_board(&focus, &[]);

        assert!(
            rendered.contains("- mev:H — Block H (blocked by core:I, external:budget approval)")
        );
    }

    #[test]
    fn rendered_board_has_no_trailing_newline() {
        let focus = Focus::default();
        let rendered = render_hq_board(&focus, &[]);
        assert!(!rendered.ends_with('\n'));
    }

    #[test]
    fn empty_focus_renders_none_in_all_three_sections() {
        let focus = Focus::default();
        let rendered = render_hq_board(&focus, &[]);

        let expected = "## NOW\n_none_\n\n## NEXT\n_none_\n\n## BLOCKED\n_none_";
        assert_eq!(rendered, expected);
    }
}

// ---------------------------------------------------------------------------
// render_unified_board tests (Task 2, MV.6.B)
// ---------------------------------------------------------------------------

mod task2_render_unified_board {
    use mev::brain::config::{BrainConfig, RepoEntry};
    use mev::brain::emit::render_unified_board;
    use mev::brain::state::{Block, BlockedBy, CrossRepoEdge, Endpoint, Focus};

    fn config_with_repos(entries: &[(&str, &str)]) -> BrainConfig {
        BrainConfig {
            repos: entries
                .iter()
                .map(|(slug, tier)| RepoEntry {
                    slug: slug.to_string(),
                    tier: tier.to_string(),
                    repo_path: String::new(),
                    status_file: String::new(),
                    cache_doc: String::new(),
                    heading: String::new(),
                })
                .collect(),
            ..BrainConfig::default()
        }
    }

    /// Build a repo-tagged `Block` with an optional priority/due (NOW/NEXT shape).
    fn tagged_block(
        repo: &str,
        id: &str,
        title: &str,
        priority: Option<u8>,
        due: Option<&str>,
    ) -> Block {
        Block {
            due: due.map(|s| s.to_string()),
            priority,
            id: id.to_string(),
            title: title.to_string(),
            status: None,
            note: None,
            repo: Some(repo.to_string()),
            blocked_by: Vec::new(),
        }
    }

    fn blocked_block(
        repo: &str,
        id: &str,
        title: &str,
        priority: Option<u8>,
        due: Option<&str>,
        blocked_by: Vec<BlockedBy>,
    ) -> Block {
        Block {
            due: due.map(|s| s.to_string()),
            priority,
            id: id.to_string(),
            title: title.to_string(),
            status: None,
            note: None,
            repo: Some(repo.to_string()),
            blocked_by,
        }
    }

    fn today() -> chrono::NaiveDate {
        chrono::NaiveDate::from_ymd_opt(2026, 7, 5).expect("valid date")
    }

    #[test]
    fn tags_business_tier_biz_and_other_tiers_eng() {
        let config = config_with_repos(&[("business", "business"), ("mev", "engine")]);
        let focus = Focus {
            now: vec![
                tagged_block("business", "BR.1", "Biz block", None, None),
                tagged_block("mev", "MV.1", "Eng block", None, None),
            ],
            next: vec![],
            blocked: vec![],
        };

        let rendered = render_unified_board(&focus, &[], &config, today());

        assert!(rendered.contains("- [BIZ] business:BR.1 — Biz block"));
        assert!(rendered.contains("- [ENG] mev:MV.1 — Eng block"));
    }

    #[test]
    fn unrecognised_repo_slug_defaults_to_eng() {
        let config = config_with_repos(&[]);
        let focus = Focus {
            now: vec![tagged_block("mystery", "X.1", "Unknown", None, None)],
            next: vec![],
            blocked: vec![],
        };

        let rendered = render_unified_board(&focus, &[], &config, today());

        assert!(rendered.contains("- [ENG] mystery:X.1 — Unknown"));
    }

    #[test]
    fn next_orders_p1_business_block_above_p2_engineering_block() {
        let config = config_with_repos(&[("business", "business"), ("mev", "engine")]);
        let focus = Focus {
            now: vec![],
            next: vec![
                tagged_block("mev", "MV.2", "Eng P2", Some(2), None),
                tagged_block("business", "BR.2", "Biz P1", Some(1), None),
            ],
            blocked: vec![],
        };

        let rendered = render_unified_board(&focus, &[], &config, today());

        let biz_idx = rendered.find("BR.2").unwrap();
        let eng_idx = rendered.find("MV.2").unwrap();
        assert!(
            biz_idx < eng_idx,
            "expected P1 business block before P2 engineering block:\n{rendered}"
        );
    }

    #[test]
    fn next_orders_by_priority_then_due_then_preserves_wave_order_as_tiebreak() {
        let config = config_with_repos(&[("mev", "engine")]);
        let focus = Focus {
            now: vec![],
            next: vec![
                // Same priority; B has an earlier due date so it should sort first.
                tagged_block("mev", "A", "Later due", Some(1), Some("2026-08-01")),
                tagged_block("mev", "B", "Earlier due", Some(1), Some("2026-07-10")),
                // No priority/due at all — sorts last, but preserves relative
                // (wave) order against other None-priority entries.
                tagged_block("mev", "C", "No priority first", None, None),
                tagged_block("mev", "D", "No priority second", None, None),
            ],
            blocked: vec![],
        };

        let rendered = render_unified_board(&focus, &[], &config, today());

        let idx_b = rendered.find("mev:B").unwrap();
        let idx_a = rendered.find("mev:A").unwrap();
        let idx_c = rendered.find("mev:C").unwrap();
        let idx_d = rendered.find("mev:D").unwrap();

        assert!(idx_b < idx_a, "earlier due should sort first:\n{rendered}");
        assert!(
            idx_a < idx_c,
            "prioritised blocks sort before unprioritised:\n{rendered}"
        );
        assert!(
            idx_c < idx_d,
            "wave (input) order preserved as tiebreak among unprioritised blocks:\n{rendered}"
        );
    }

    #[test]
    fn due_soon_includes_in_window_and_overdue_excludes_far_future_and_dateless() {
        let config = config_with_repos(&[("mev", "engine")]);
        let focus = Focus {
            now: vec![tagged_block(
                "mev",
                "SOON",
                "Due soon",
                None,
                Some("2026-07-12"), // 7 days out — within the 14-day window
            )],
            next: vec![
                tagged_block("mev", "OVERDUE", "Overdue", None, Some("2026-06-20")),
                tagged_block("mev", "FAR", "Far future", None, Some("2027-01-01")),
                tagged_block("mev", "NODATE", "No date", None, None),
            ],
            blocked: vec![],
        };

        let rendered = render_unified_board(&focus, &[], &config, today());
        let due_soon_section = rendered
            .split("## DUE-SOON")
            .nth(1)
            .expect("DUE-SOON section");

        assert!(due_soon_section.contains("mev:SOON"));
        assert!(due_soon_section.contains("mev:OVERDUE"));
        assert!(due_soon_section.contains("(overdue)"));
        assert!(!due_soon_section.contains("mev:FAR"));
        assert!(!due_soon_section.contains("mev:NODATE"));
    }

    #[test]
    fn due_soon_sorted_by_date_ascending_overdue_first() {
        let config = config_with_repos(&[("mev", "engine")]);
        let focus = Focus {
            now: vec![tagged_block(
                "mev",
                "LATER",
                "Later",
                None,
                Some("2026-07-15"),
            )],
            next: vec![tagged_block(
                "mev",
                "EARLIER",
                "Earlier",
                None,
                Some("2026-06-01"),
            )],
            blocked: vec![],
        };

        let rendered = render_unified_board(&focus, &[], &config, today());
        let due_soon_section = rendered
            .split("## DUE-SOON")
            .nth(1)
            .expect("DUE-SOON section");

        let earlier_idx = due_soon_section.find("mev:EARLIER").unwrap();
        let later_idx = due_soon_section.find("mev:LATER").unwrap();
        assert!(earlier_idx < later_idx);
    }

    #[test]
    fn blocked_by_annotation_reused_from_hq_board_helper() {
        let config = config_with_repos(&[("mev", "engine")]);
        let focus = Focus {
            now: vec![],
            next: vec![],
            blocked: vec![blocked_block(
                "mev",
                "C",
                "Block C",
                None,
                None,
                vec![BlockedBy::Block {
                    repo: "core".to_string(),
                    id: "D".to_string(),
                    what: None,
                }],
            )],
        };
        let edges = vec![CrossRepoEdge {
            from: Endpoint {
                repo: "mev".to_string(),
                id: "C".to_string(),
            },
            to: Endpoint {
                repo: "core".to_string(),
                id: "D".to_string(),
            },
            note: Some("waiting on schema freeze".to_string()),
        }];

        let rendered = render_unified_board(&focus, &edges, &config, today());

        assert!(
            rendered
                .contains("- [ENG] mev:C — Block C (blocked by core:D (waiting on schema freeze))")
        );
    }

    #[test]
    fn empty_focus_renders_none_in_all_four_sections() {
        let config = config_with_repos(&[]);
        let rendered = render_unified_board(&Focus::default(), &[], &config, today());

        let expected =
            "## NOW\n_none_\n\n## NEXT\n_none_\n\n## BLOCKED\n_none_\n\n## DUE-SOON\n_none_";
        assert_eq!(rendered, expected);
    }

    #[test]
    fn rendered_board_has_no_trailing_newline() {
        let config = config_with_repos(&[]);
        let rendered = render_unified_board(&Focus::default(), &[], &config, today());
        assert!(!rendered.ends_with('\n'));
    }
}

// ---------------------------------------------------------------------------
// plan_hq_board tests (Task 2, MV.4.C)
// ---------------------------------------------------------------------------

mod task2_plan_hq_board {
    use mev::brain::config::{BrainConfig, RepoEntry};
    use mev::brain::emit::{markers, plan_hq_board};
    use mev::brain::state::{
        BlockedBy, Focus, RepoRollup, StateFile, StateSource, Track, TrackBlock, build_state_graph,
    };
    use std::path::PathBuf;

    fn config_with_repos(entries: &[(&str, &str)]) -> BrainConfig {
        BrainConfig {
            repos: entries
                .iter()
                .map(|(slug, tier)| RepoEntry {
                    slug: slug.to_string(),
                    tier: tier.to_string(),
                    repo_path: String::new(),
                    status_file: String::new(),
                    cache_doc: String::new(),
                    heading: String::new(),
                })
                .collect(),
            ..BrainConfig::default()
        }
    }

    fn track_block(
        id: &str,
        title: &str,
        status: Option<&str>,
        wave: Option<i64>,
        deps: Vec<BlockedBy>,
    ) -> TrackBlock {
        TrackBlock {
            due: None,
            priority: None,
            sdlc_workflow: None,
            model: None,
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
            carryover: vec![],
        }
    }

    fn make_brain_file(
        repos: Vec<RepoRollup>,
        cross_repo: Vec<mev::brain::state::CrossRepoEdge>,
        focus: Focus,
    ) -> StateFile {
        StateFile {
            repo: "hq".to_string(),
            kind: "brain".to_string(),
            updated: "2026-06-30".to_string(),
            focus,
            tracks: vec![],
            repos,
            cross_repo,
            tiers: vec![],
            note: None,
            backlog: vec![],
            carryover: vec![],
        }
    }

    fn board_doc_with_sentinel() -> String {
        "---\n\
         type: ProjectStatus\n\
         title: HQ status\n\
         description: Test HQ status doc.\n\
         ---\n\n\
         # Status\n\n\
         Narrative before.\n\n\
         <!-- BEGIN generated:hq-board -->\n\
         <!-- END generated:hq-board -->\n\n\
         Narrative after.\n"
            .to_string()
    }

    #[test]
    fn hq_board_splice_produces_expected_content() {
        let tmp = tempfile::tempdir().unwrap();
        let status_path = tmp.path().join("planning/status.md");
        std::fs::create_dir_all(status_path.parent().unwrap()).unwrap();
        std::fs::write(&status_path, board_doc_with_sentinel()).unwrap();

        // Leaf repo "repo-a" (core tier) with one in_progress block.
        let leaf_tracks = vec![Track {
            title: "P".to_string(),
            blocks: vec![track_block(
                "RA.1.A",
                "Repo A block",
                Some("in_progress"),
                Some(1),
                vec![],
            )],
        }];
        let leaf_file = make_leaf_file("repo-a", leaf_tracks, Focus::default());
        let leaf_src = StateSource {
            repo_slug: "repo-a".to_string(),
            abs_path: PathBuf::from("/fake/repo-a/planning/state.json"),
            expected_kind: "project",
        };

        // HQ brain: repo_slug "hq" matches no declared tier, so tier_scope_for
        // resolves TierScope::All.
        let hq_file = make_brain_file(vec![], vec![], Focus::default());
        let hq_src = StateSource {
            repo_slug: "hq".to_string(),
            abs_path: status_path.parent().unwrap().join("state.json"),
            expected_kind: "brain",
        };

        let files = vec![(leaf_src, leaf_file), (hq_src, hq_file)];
        let graph = build_state_graph(&files);
        let config = config_with_repos(&[("repo-a", "core")]);

        let plan = plan_hq_board(&files, &graph, &config);

        assert_eq!(
            plan.actions.len(),
            1,
            "expected one action; got {}",
            plan.actions.len()
        );
        let action = &plan.actions[0];
        assert_eq!(action.path, status_path);
        assert!(
            action.new_content.contains("Narrative before."),
            "narrative before sentinel was lost"
        );
        assert!(
            action.new_content.contains("Narrative after."),
            "narrative after sentinel was lost"
        );
        assert!(
            action
                .new_content
                .contains("- repo-a:RA.1.A — Repo A block"),
            "HQ board missing derived NOW entry: {}",
            action.new_content
        );
        assert!(
            action.new_content.contains("## NOW"),
            "HQ board missing NOW heading: {}",
            action.new_content
        );
    }

    #[test]
    fn hq_board_missing_sentinel_warns_no_action() {
        let tmp = tempfile::tempdir().unwrap();
        let status_path = tmp.path().join("planning/status.md");
        std::fs::create_dir_all(status_path.parent().unwrap()).unwrap();
        std::fs::write(
            &status_path,
            "---\ntype: ProjectStatus\n---\n\n# Status\n\nNo sentinels.\n",
        )
        .unwrap();

        let hq_file = make_brain_file(vec![], vec![], Focus::default());
        let hq_src = StateSource {
            repo_slug: "hq".to_string(),
            abs_path: status_path.parent().unwrap().join("state.json"),
            expected_kind: "brain",
        };

        let files = vec![(hq_src, hq_file)];
        let graph = build_state_graph(&files);
        let config = config_with_repos(&[("repo-a", "core")]);

        let plan = plan_hq_board(&files, &graph, &config);

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
    fn hq_board_missing_file_warns_no_action() {
        let tmp = tempfile::tempdir().unwrap();
        // No status.md written at all.

        let hq_file = make_brain_file(vec![], vec![], Focus::default());
        let hq_src = StateSource {
            repo_slug: "hq".to_string(),
            abs_path: tmp.path().join("planning/state.json"),
            expected_kind: "brain",
        };

        let files = vec![(hq_src, hq_file)];
        let graph = build_state_graph(&files);
        let config = config_with_repos(&[("repo-a", "core")]);

        let plan = plan_hq_board(&files, &graph, &config);

        assert_eq!(
            plan.actions.len(),
            0,
            "no action expected when status.md is missing; got {}",
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
    fn hq_board_tier_sub_brain_is_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        // Tier "core" status.md carries the hq-board sentinel too (in principle
        // it shouldn't, but the point of this test is that plan_hq_board must
        // never target a tier sub-brain regardless of the doc's content) --
        // that's plan_tier_rollups's responsibility, keyed off tier-rollup.
        let status_path = tmp.path().join("core/planning/status.md");
        std::fs::create_dir_all(status_path.parent().unwrap()).unwrap();
        std::fs::write(&status_path, board_doc_with_sentinel()).unwrap();

        let mut tier_file = make_brain_file(vec![], vec![], Focus::default());
        tier_file.repo = "core".to_string();
        let tier_src = StateSource {
            repo_slug: "core".to_string(),
            abs_path: tmp.path().join("core/planning/state.json"),
            expected_kind: "brain",
        };

        let files = vec![(tier_src, tier_file)];
        let graph = build_state_graph(&files);
        let config = config_with_repos(&[("repo-a", "core")]);

        let plan = plan_hq_board(&files, &graph, &config);

        assert_eq!(
            plan.actions.len(),
            0,
            "tier sub-brain must never be targeted by plan_hq_board; got {}",
            plan.actions.len()
        );
        assert!(
            plan.diagnostics.is_empty(),
            "no diagnostics expected for the skipped tier sub-brain; got: {:?}",
            plan.diagnostics
        );
    }

    #[test]
    fn hq_board_fixed_point_no_action_on_second_pass() {
        let tmp = tempfile::tempdir().unwrap();
        let status_path = tmp.path().join("planning/status.md");
        std::fs::create_dir_all(status_path.parent().unwrap()).unwrap();
        std::fs::write(&status_path, board_doc_with_sentinel()).unwrap();

        let leaf_tracks = vec![Track {
            title: "P".to_string(),
            blocks: vec![track_block(
                "RA.1.A",
                "Repo A block",
                Some("in_progress"),
                Some(1),
                vec![],
            )],
        }];
        let leaf_file = make_leaf_file("repo-a", leaf_tracks, Focus::default());
        let leaf_src = StateSource {
            repo_slug: "repo-a".to_string(),
            abs_path: PathBuf::from("/fake/repo-a/planning/state.json"),
            expected_kind: "project",
        };

        let hq_file = make_brain_file(vec![], vec![], Focus::default());
        let hq_src = StateSource {
            repo_slug: "hq".to_string(),
            abs_path: status_path.parent().unwrap().join("state.json"),
            expected_kind: "brain",
        };

        let files = vec![(leaf_src, leaf_file), (hq_src, hq_file)];
        let graph = build_state_graph(&files);
        let config = config_with_repos(&[("repo-a", "core")]);

        // First pass: apply the spliced content back to disk.
        let plan = plan_hq_board(&files, &graph, &config);
        assert_eq!(plan.actions.len(), 1);
        std::fs::write(&status_path, &plan.actions[0].new_content).unwrap();

        // Second pass: already-correct content produces no action.
        let plan2 = plan_hq_board(&files, &graph, &config);
        assert_eq!(
            plan2.actions.len(),
            0,
            "expected fixed-point no-action on second pass; got {}",
            plan2.actions.len()
        );
        assert!(plan2.diagnostics.is_empty());
    }

    #[test]
    fn hq_board_marker_constant_matches_sentinel() {
        assert_eq!(markers::HQ_BOARD, "hq-board");
    }
}

// ---------------------------------------------------------------------------
// update-write-state-in-trees Task 3 — CLI-level coverage for the
// `emit-state --write` linked-worktree refusal.
//
// Exercises the real `mev` binary against a real git repo with a linked
// worktree (`git worktree add`):
//   (a) `emit-state --write` from inside the worktree -> non-zero exit, no
//       files written, stderr names the worktree path.
//   (b) `emit-state` (no --write, dry-run) from the same worktree -> exit 0,
//       unaffected by the guard.
//   (c) `emit-state --write` from the main tree of that same repo -> exit 0,
//       files written (regression guard: unchanged behavior).
// ---------------------------------------------------------------------------

mod task3_cli_worktree_guard {
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("mev-emit-state-cli-guard-{tag}"));
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

    /// Write a minimal `brain.toml` that registers a single leaf repo (alpha).
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
"#;
        fs::write(root.join("brain.toml"), toml.as_bytes()).unwrap();
    }

    /// Write an already-consistent alpha leaf state.json (no drift) so a
    /// clean `emit_state` run produces zero errors and is well-formed for a
    /// regression comparison.
    fn write_alpha_state(root: &Path) {
        let state = serde_json::json!({
            "repo": "alpha",
            "kind": "project",
            "updated": "2026-07-04",
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
        write_file(
            root,
            "repos/alpha/planning/state.json",
            &serde_json::to_string_pretty(&state).unwrap(),
        );
    }

    /// Write a stale brain-level state.json (HQ) so `emit_state` has real
    /// work to do (W_EMIT_DRY_RUN / I_EMIT_WROTE diagnostics get produced).
    fn write_brain_state(root: &Path) {
        let state = serde_json::json!({
            "repo": "hq",
            "kind": "brain",
            "updated": "2026-06-29",
            "focus": { "now": [], "next": [], "blocked": [] },
            "repos": [
                { "repo": "alpha", "now": [], "next": [], "blocked": [] }
            ],
            "cross_repo": []
        });
        write_file(
            root,
            "planning/state.json",
            &serde_json::to_string_pretty(&state).unwrap(),
        );
    }

    fn run_git(dir: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("failed to spawn git");
        assert!(
            output.status.success(),
            "git {:?} failed in {}: {}",
            args,
            dir.display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// Build a real git repo (with an initial commit) plus a linked worktree,
    /// and a `brain.toml` fixture with real derived-state files at the main
    /// repo root. Returns (main_repo_path, worktree_path).
    fn build_repo_with_worktree(tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let parent = temp_dir(tag);
        let main_repo = parent.join("main");
        fs::create_dir_all(&main_repo).unwrap();

        write_brain_toml(&main_repo);
        write_brain_state(&main_repo);
        write_alpha_state(&main_repo);

        run_git(&main_repo, &["init", "-q"]);
        run_git(&main_repo, &["config", "user.email", "test@example.com"]);
        run_git(&main_repo, &["config", "user.name", "Test"]);
        run_git(&main_repo, &["add", "."]);
        run_git(&main_repo, &["commit", "-q", "-m", "initial commit"]);

        let worktree_path = parent.join("wt");
        run_git(
            &main_repo,
            &[
                "worktree",
                "add",
                worktree_path.to_str().expect("utf8 path"),
            ],
        );

        (main_repo, worktree_path)
    }

    #[test]
    fn write_from_linked_worktree_is_refused() {
        let (_main_repo, worktree_path) = build_repo_with_worktree("refused");

        let alpha_state_path = worktree_path.join("repos/alpha/planning/state.json");
        let brain_state_path = worktree_path.join("planning/state.json");
        let alpha_before = fs::read(&alpha_state_path).unwrap();
        let brain_before = fs::read(&brain_state_path).unwrap();

        let output = Command::new(env!("CARGO_BIN_EXE_mev"))
            .args(["emit-state", "--write"])
            .arg(&worktree_path)
            .current_dir(&worktree_path)
            .output()
            .expect("failed to spawn mev binary");

        assert!(
            !output.status.success(),
            "emit-state --write from a linked worktree must exit non-zero; status: {:?}",
            output.status
        );

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains(worktree_path.file_name().unwrap().to_str().unwrap())
                || stderr.contains(worktree_path.to_str().unwrap()),
            "stderr must name the worktree path; stderr: {stderr}"
        );

        // No files written.
        assert_eq!(
            fs::read(&alpha_state_path).unwrap(),
            alpha_before,
            "alpha state.json must be unchanged when --write is refused"
        );
        assert_eq!(
            fs::read(&brain_state_path).unwrap(),
            brain_before,
            "brain state.json must be unchanged when --write is refused"
        );

        let _ = fs::remove_dir_all(worktree_path.parent().unwrap());
    }

    #[test]
    fn dry_run_from_linked_worktree_still_succeeds() {
        let (_main_repo, worktree_path) = build_repo_with_worktree("dry-run-ok");

        let output = Command::new(env!("CARGO_BIN_EXE_mev"))
            .args(["emit-state"])
            .arg(&worktree_path)
            .current_dir(&worktree_path)
            .output()
            .expect("failed to spawn mev binary");

        assert!(
            output.status.success(),
            "dry-run emit-state from a linked worktree must still succeed; status: {:?}, stderr: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );

        let _ = fs::remove_dir_all(worktree_path.parent().unwrap());
    }

    #[test]
    fn write_from_main_tree_is_unchanged_regression() {
        let (main_repo, worktree_path) = build_repo_with_worktree("main-tree-ok");

        let brain_state_path = main_repo.join("planning/state.json");
        let brain_before: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&brain_state_path).unwrap()).unwrap();

        let output = Command::new(env!("CARGO_BIN_EXE_mev"))
            .args(["emit-state", "--write"])
            .arg(&main_repo)
            .current_dir(&main_repo)
            .output()
            .expect("failed to spawn mev binary");

        assert!(
            output.status.success(),
            "emit-state --write from the main tree must still succeed; status: {:?}, stderr: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );

        let brain_after: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&brain_state_path).unwrap()).unwrap();
        assert_ne!(
            brain_before, brain_after,
            "main-tree --write must have applied the derived-view update (regression: no-op means the guard broke the normal path)"
        );

        let _ = fs::remove_dir_all(worktree_path.parent().unwrap());
    }
}

mod task_yaml_frontmatter_drift_tests {
    use mev::brain::config::{BrainConfig, RepoEntry};
    use mev::brain::emit::{plan_status_frontmatter, reconcile_status_scalars};
    use mev::brain::state::{
        Block, Focus, StateFile, StateSource, Track, TrackBlock, build_state_graph,
    };
    use std::path::PathBuf;

    #[test]
    fn reconcile_status_scalars_replaces_and_preserves() {
        let original = "---\n\
                        title: Foo\n\
                        now: \"stale\"\n\
                        next: \"stale\"\n\
                        blocked: \"stale\"\n\
                        ---\n\n\
                        # Body\n";
        let focus = Focus {
            now: vec![Block {
                due: None,
                priority: None,
                id: "1".into(),
                title: "One".into(),
                status: None,
                note: None,
                repo: Some("core".into()),
                blocked_by: vec![],
            }],
            next: vec![],
            blocked: vec![],
        };
        let new_content = reconcile_status_scalars(original, &focus);
        assert!(new_content.contains("now: \"core:1 — One\""));
        assert!(new_content.contains("next: []"));
        assert!(new_content.contains("blocked: []"));
        assert!(!new_content.contains("stale"));
        assert!(new_content.contains("# Body\n"));
    }

    #[test]
    fn reconcile_status_scalars_appends_if_missing() {
        let original = "---\n\
                        title: Foo\n\
                        ---\n\n\
                        # Body\n";
        let focus = Focus {
            now: vec![],
            next: vec![],
            blocked: vec![],
        };
        let new_content = reconcile_status_scalars(original, &focus);
        assert!(new_content.contains("now: []"));
        assert!(new_content.contains("next: []"));
        assert!(new_content.contains("blocked: []"));
    }

    #[test]
    fn plan_status_frontmatter_emits_action() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("planning")).unwrap();
        let status_path = tmp.path().join("planning/status.md");
        std::fs::write(&status_path, "---\ntitle: test\n---\n").unwrap();

        let src = StateSource {
            repo_slug: "myrepo".to_string(),
            abs_path: tmp.path().join("planning/state.json"),
            expected_kind: "project",
        };
        let file = StateFile {
            repo: "myrepo".to_string(),
            kind: "project".to_string(),
            updated: "2026-06-30".to_string(),
            focus: Focus::default(),
            tracks: vec![Track {
                title: "P".to_string(),
                blocks: vec![TrackBlock {
                    due: None,
                    priority: None,
                    sdlc_workflow: None,
                    model: None,
                    id: "B1".into(),
                    title: "Block One".into(),
                    status: Some("in_progress".into()),
                    depends_on: vec![],
                    wave: Some(1),
                    origin: None,
                }],
            }],
            repos: vec![],
            cross_repo: vec![],
            tiers: vec![],
            note: None,
            backlog: vec![],
            carryover: vec![],
        };

        let files = vec![(src, file)];
        let graph = build_state_graph(&files);
        let config = BrainConfig {
            repos: vec![RepoEntry {
                slug: "myrepo".to_string(),
                tier: "core".to_string(),
                repo_path: "".to_string(),
                status_file: "".to_string(), // fallback to sibling
                cache_doc: "".to_string(),
                heading: "".to_string(),
            }],
            ..BrainConfig::default()
        };

        let plan = plan_status_frontmatter(tmp.path(), &files, &graph, &config);
        assert_eq!(plan.actions.len(), 1);
        let action = &plan.actions[0];
        assert_eq!(action.path, status_path);
        assert!(action.new_content.contains("now: \"B1 — Block One\""));
    }
}
