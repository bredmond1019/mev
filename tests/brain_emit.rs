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
use mev::brain::state::{
    BlockDep, BlockedBy, ExternalDep, StateFile, StateGraph, StateSource, Track, TrackBlock,
};
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
        epics: Vec::new(),
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
        ..Default::default()
    }
}

/// Build a `TrackBlock` with a given wave and no deps.
fn block(id: &str, title: &str, status: Option<&str>, wave: Option<i64>) -> TrackBlock {
    TrackBlock {
        epics: Vec::new(),
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
        note: None,
        description: None,
        ..Default::default()
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
        epics: Vec::new(),
        due: None,
        priority: None,
        sdlc_workflow: None,
        model: None,
        id: id.to_string(),
        title: title.to_string(),
        status: status.map(|s| s.to_string()),
        depends_on: vec![BlockedBy::Block(BlockDep {
            repo: dep_repo.to_string(),
            id: dep_id.to_string(),
            what: None,
        })],
        wave,
        origin: None,
        note: None,
        description: None,
        ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
        }],
    );
    let src_b = make_src("beta");
    let file_b = make_leaf(
        "beta",
        vec![Track {
            title: "P".to_string(),
            blocks: vec![block("B1", "Beta 1", None, Some(1))],
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
    b.depends_on.push(BlockedBy::External(ExternalDep {
        what: "deploy-gate".to_string(),
    }));
    let file = make_leaf(
        "repo",
        vec![Track {
            title: "P".to_string(),
            blocks: vec![block("A", "A", Some("closed"), Some(1)), b],
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
            attention: Default::default(),
            history: Default::default(),
            repos: entries
                .iter()
                .map(|(slug, tier)| RepoEntry {
                    slug: slug.to_string(),
                    tier: tier.to_string(),
                    repo_path: String::new(),
                    status_file: String::new(),
                    cache_doc: String::new(),
                    heading: String::new(),
                    prefix: None,
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
            epics: Vec::new(),
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
            note: None,
            description: None,
            ..Default::default()
        }
    }

    fn make_leaf_file(repo: &str, tracks: Vec<Track>, focus: Focus) -> StateFile {
        StateFile {
            epics: Vec::new(),
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
            ..Default::default()
        }
    }

    fn make_brain_file(
        repos: Vec<RepoRollup>,
        cross_repo: Vec<mev::brain::state::CrossRepoEdge>,
        focus: Focus,
    ) -> StateFile {
        StateFile {
            epics: Vec::new(),
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
            ..Default::default()
        }
    }

    fn focus_with_now(block_id: &str, title: &str) -> Focus {
        Focus {
            now: vec![Block {
                epics: Vec::new(),
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
            deferred: Vec::new(),
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
            ..Default::default()
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
                    epics: Vec::new(),
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
                    note: None,
                    description: None,
                    ..Default::default()
                },
                TrackBlock {
                    epics: Vec::new(),
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
                    note: None,
                    description: None,
                    ..Default::default()
                },
            ],
            ..Default::default()
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
            ..Default::default()
        }];
        // Pre-derive the correct focus.
        let correct_focus = Focus {
            now: vec![Block {
                epics: Vec::new(),
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
            deferred: Vec::new(),
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
            attention: Default::default(),
            history: Default::default(),
            repos: vec![RepoEntry {
                slug: slug.to_string(),
                tier: tier.to_string(),
                repo_path: String::new(),
                status_file: status_file.to_string(),
                cache_doc: cache_doc.to_string(),
                heading: String::new(),
                prefix: None,
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
            ..Default::default()
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
            ..Default::default()
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
    // plan_brain_cache_watermarks
    // -----------------------------------------------------------------------

    #[test]
    fn brain_cache_watermark_reconciles_synced_from() {
        use mev::brain::emit::plan_brain_cache_watermarks;

        let tmp = tempfile::tempdir().unwrap();
        let cache_rel = "docs/projects/mytier.md";
        let cache_path = tmp.path().join(cache_rel);
        std::fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        std::fs::write(&cache_path, cache_doc_with_sentinel("2026-01-01T00:00:00Z")).unwrap();

        let status_rel = "mytier/planning/status.md";
        let status_path = tmp.path().join(status_rel);
        std::fs::create_dir_all(status_path.parent().unwrap()).unwrap();
        std::fs::write(
            &status_path,
            status_file_with_timestamp("2026-07-04T02:21:44Z"),
        )
        .unwrap();

        let file = make_brain_file(vec![], vec![], Focus::default());
        let src = StateSource {
            repo_slug: "mytier".to_string(),
            abs_path: PathBuf::from("/fake/mytier/planning/state.json"),
            expected_kind: "brain",
        };

        let files = vec![(src, file)];
        let config = config_with_cache_and_status("mytier", "core", cache_rel, status_rel);

        let plan = plan_brain_cache_watermarks(tmp.path(), &files, &config);

        assert_eq!(
            plan.actions.len(),
            1,
            "expected one action; got {}",
            plan.actions.len()
        );
        let action = &plan.actions[0];
        assert_eq!(action.path, cache_path);
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
    }

    #[test]
    fn brain_cache_missing_file_warns_no_action() {
        use mev::brain::emit::plan_brain_cache_watermarks;

        let tmp = tempfile::tempdir().unwrap();
        // No cache doc written at all.
        let cache_rel = "docs/projects/mytier.md";

        let file = make_brain_file(vec![], vec![], Focus::default());
        let src = StateSource {
            repo_slug: "mytier".to_string(),
            abs_path: PathBuf::from("/fake/mytier/planning/state.json"),
            expected_kind: "brain",
        };

        let files = vec![(src, file)];
        let config = config_with_cache_doc("mytier", "core", cache_rel);

        let plan = plan_brain_cache_watermarks(tmp.path(), &files, &config);

        assert_eq!(
            plan.actions.len(),
            0,
            "no action expected when the cache doc is missing; got {}",
            plan.actions.len()
        );
        let warn = plan
            .diagnostics
            .iter()
            .find(|d| d.locator == "W_EMIT_IO_ERROR");
        assert!(
            warn.is_some(),
            "expected W_EMIT_IO_ERROR diagnostic; got: {:?}",
            plan.diagnostics
        );
    }

    #[test]
    fn brain_cache_missing_timestamp_warns_no_action() {
        use mev::brain::emit::plan_brain_cache_watermarks;

        let tmp = tempfile::tempdir().unwrap();
        let cache_rel = "docs/projects/mytier.md";
        let cache_path = tmp.path().join(cache_rel);
        std::fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        std::fs::write(&cache_path, cache_doc_with_sentinel("2026-01-01T00:00:00Z")).unwrap();

        // status_file left blank in the config below, so read_watermark finds nothing.
        let file = make_brain_file(vec![], vec![], Focus::default());
        let src = StateSource {
            repo_slug: "mytier".to_string(),
            abs_path: PathBuf::from("/fake/mytier/planning/state.json"),
            expected_kind: "brain",
        };

        let files = vec![(src, file)];
        let config = config_with_cache_doc("mytier", "core", cache_rel);

        let plan = plan_brain_cache_watermarks(tmp.path(), &files, &config);

        assert_eq!(
            plan.actions.len(),
            0,
            "no action expected when the status_file has no timestamp; got {}",
            plan.actions.len()
        );
        let warn = plan
            .diagnostics
            .iter()
            .find(|d| d.locator == "W_EMIT_NO_SENTINEL");
        assert!(
            warn.is_some(),
            "expected W_EMIT_NO_SENTINEL diagnostic; got: {:?}",
            plan.diagnostics
        );
    }

    #[test]
    fn brain_cache_non_brain_kind_is_skipped() {
        use mev::brain::emit::plan_brain_cache_watermarks;

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

        let file = make_leaf_file("myrepo", vec![], Focus::default());
        let src = StateSource {
            repo_slug: "myrepo".to_string(),
            abs_path: PathBuf::from("/fake/myrepo/planning/state.json"),
            expected_kind: "project",
        };

        let files = vec![(src, file)];
        let config = config_with_cache_and_status("myrepo", "core", cache_rel, status_rel);

        let plan = plan_brain_cache_watermarks(tmp.path(), &files, &config);

        assert_eq!(
            plan.actions.len(),
            0,
            "project-kind repos should never be targeted; got {}",
            plan.actions.len()
        );
        assert!(
            plan.diagnostics.is_empty(),
            "no diagnostics expected for a skipped non-brain-kind repo; got: {:?}",
            plan.diagnostics
        );
    }

    #[test]
    fn brain_cache_no_config_entry_is_skipped() {
        use mev::brain::emit::plan_brain_cache_watermarks;

        let file = make_brain_file(vec![], vec![], Focus::default());
        let src = StateSource {
            repo_slug: "mytier".to_string(),
            abs_path: PathBuf::from("/fake/mytier/planning/state.json"),
            expected_kind: "brain",
        };

        let files = vec![(src, file)];
        let config = BrainConfig::default();

        let plan = plan_brain_cache_watermarks(std::path::Path::new("/fake"), &files, &config);

        assert_eq!(
            plan.actions.len(),
            0,
            "no config entry for the repo should skip it silently; got {}",
            plan.actions.len()
        );
        assert!(
            plan.diagnostics.is_empty(),
            "no diagnostics expected when there's no matching [[repos]] entry; got: {:?}",
            plan.diagnostics
        );
    }

    #[test]
    fn brain_cache_watermark_second_pass_is_fixed_point() {
        use mev::brain::emit::plan_brain_cache_watermarks;

        let tmp = tempfile::tempdir().unwrap();
        let cache_rel = "docs/projects/mytier.md";
        let cache_path = tmp.path().join(cache_rel);
        std::fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        std::fs::write(&cache_path, cache_doc_with_sentinel("2026-01-01T00:00:00Z")).unwrap();

        let status_rel = "mytier/planning/status.md";
        let status_path = tmp.path().join(status_rel);
        std::fs::create_dir_all(status_path.parent().unwrap()).unwrap();
        std::fs::write(
            &status_path,
            status_file_with_timestamp("2026-07-04T02:21:44Z"),
        )
        .unwrap();

        let file = make_brain_file(vec![], vec![], Focus::default());
        let src = StateSource {
            repo_slug: "mytier".to_string(),
            abs_path: PathBuf::from("/fake/mytier/planning/state.json"),
            expected_kind: "brain",
        };

        let files = vec![(src, file)];
        let config = config_with_cache_and_status("mytier", "core", cache_rel, status_rel);

        let plan1 = plan_brain_cache_watermarks(tmp.path(), &files, &config);
        assert_eq!(plan1.actions.len(), 1);
        std::fs::write(&cache_path, &plan1.actions[0].new_content).unwrap();

        let plan2 = plan_brain_cache_watermarks(tmp.path(), &files, &config);
        assert_eq!(
            plan2.actions.len(),
            0,
            "second pass (idempotent) should produce no action; got {}",
            plan2.actions.len()
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
            ..Default::default()
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
            ..Default::default()
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
        let d = mev::testsupport::unique_temp_dir(&format!("mev-emit-state-{tag}"));
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

        let report = mev::emit_state(&dir, false, None).expect("emit_state should not error");

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

        let report = mev::emit_state(&dir, true, None).expect("emit_state should not error");

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
        mev::emit_state(&dir, true, None).expect("first emit should not error");

        // Snapshot file contents after first write.
        let alpha_after1 = fs::read(dir.join("repos/alpha/planning/state.json")).unwrap();
        let beta_after1 = fs::read(dir.join("repos/beta/planning/state.json")).unwrap();
        let brain_after1 = fs::read(dir.join("planning/state.json")).unwrap();

        // Second write.
        let report2 = mev::emit_state(&dir, true, None).expect("second emit should not error");

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

        let report = mev::emit_state(&dir, false, None).expect("emit_state should not panic");
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
// emit-state-incomplete-corpus-guard — Task 2
//
// Covers: `emit_state(&dir, true)` refuses to write derived views when any
// discovered `state.json` fails to load (`E_EMIT_INCOMPLETE_CORPUS`, cause
// `E_STATE_MALFORMED_JSON` preserved, no file touched), `emit_state(&dir,
// false)` (dry-run) remains fully exempt from the guard, and a healthy
// corpus is unaffected (regression + non-vacuousness for the byte-identity
// assertion above).
// ---------------------------------------------------------------------------

mod incomplete_corpus_guard {
    use std::fs;
    use std::path::Path;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let d = mev::testsupport::unique_temp_dir(&format!("mev-incomplete-corpus-guard-{tag}"));
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

    /// Minimal `brain.toml` registering two leaf repos (alpha, beta) — the
    /// same corpus-builder shape used by `task4_emit_state`.
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

    /// Stale HQ brain `planning/state.json` — `repos[]` caches alpha with an
    /// empty `now` even though alpha's leaf has an `in_progress` block, so a
    /// healthy write regenerates it.
    fn write_stale_brain_state(root: &Path) {
        let state = serde_json::json!({
            "repo": "hq",
            "kind": "brain",
            "updated": "2026-06-29",
            "focus": { "now": [], "next": [], "blocked": [] },
            "repos": [
                {
                    "repo": "alpha",
                    "now": [],
                    "next": [],
                    "blocked": []
                }
            ],
            "cross_repo": []
        });
        write_json(root, "planning/state.json", &state);
    }

    /// Alpha leaf `planning/state.json` with one `in_progress` block.
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

    /// Healthy beta leaf `planning/state.json` — a stale `focus.now` entry
    /// (the block is `open`, not `in_progress`), so a healthy write clears it.
    /// This is the file whose byte-identity (refused) vs. byte-change
    /// (healthy) makes test 1's assertion non-vacuous.
    fn write_beta_state(root: &Path) {
        let state = serde_json::json!({
            "repo": "beta",
            "kind": "project",
            "updated": "2026-06-29",
            "focus": {
                "now": [{ "id": "BE.1.A", "title": "Beta block A", "status": "in_progress" }],
                "next": [],
                "blocked": []
            },
            "tracks": [
                {
                    "title": "Phase 1",
                    "blocks": [
                        { "id": "BE.1.A", "title": "Beta block A", "status": "open" }
                    ]
                }
            ]
        });
        write_json(root, "repos/beta/planning/state.json", &state);
    }

    /// Beta leaf `planning/state.json` carrying a schema-invalid `depends_on`
    /// entry — `{"type":"block","repo":"x","id":null}` — mirroring the real
    /// 2026-07-24 defect (`okf_core::state::BlockedBy::Block.id` is a
    /// non-optional `String`; a JSON `null` fails deserialization, so the
    /// whole file is rejected as `StateLoadError::Parse`, not merely one
    /// field). Valid JSON, invalid schema.
    fn write_corrupt_beta_state(root: &Path) {
        let state = serde_json::json!({
            "repo": "beta",
            "kind": "project",
            "updated": "2026-06-29",
            "focus": {
                "now": [{ "id": "BE.1.A", "title": "Beta block A", "status": "in_progress" }],
                "next": [],
                "blocked": []
            },
            "tracks": [
                {
                    "title": "Phase 1",
                    "blocks": [
                        {
                            "id": "BE.1.A",
                            "title": "Beta block A",
                            "status": "open",
                            "depends_on": [
                                { "type": "block", "repo": "x", "id": null }
                            ]
                        }
                    ]
                }
            ]
        });
        write_json(root, "repos/beta/planning/state.json", &state);
    }

    /// The healthy fixture: brain.toml + stale brain state + alpha + healthy
    /// (but stale-focus) beta. A `--write` run rewrites both the HQ brain
    /// state and beta's leaf state.
    fn write_healthy_fixture(root: &Path) {
        write_brain_toml(root);
        write_stale_brain_state(root);
        write_alpha_state(root);
        write_beta_state(root);
    }

    /// The broken fixture: identical to the healthy one except beta's leaf
    /// `state.json` fails to load (schema-invalid `depends_on`).
    fn write_broken_fixture(root: &Path) {
        write_brain_toml(root);
        write_stale_brain_state(root);
        write_alpha_state(root);
        write_corrupt_beta_state(root);
    }

    /// Snapshot the byte content of every file a healthy write would rewrite
    /// — alpha, beta, and the HQ brain `state.json`.
    struct Snapshot {
        alpha: Vec<u8>,
        beta: Vec<u8>,
        brain: Vec<u8>,
    }

    fn snapshot(root: &Path) -> Snapshot {
        Snapshot {
            alpha: fs::read(root.join("repos/alpha/planning/state.json")).unwrap(),
            beta: fs::read(root.join("repos/beta/planning/state.json")).unwrap(),
            brain: fs::read(root.join("planning/state.json")).unwrap(),
        }
    }

    // -----------------------------------------------------------------------
    // Test 1 — write=true refuses on a corpus with a failed-to-load file.
    // -----------------------------------------------------------------------

    #[test]
    fn emit_state_write_refuses_when_a_state_file_fails_to_load() {
        let dir = temp_dir("write-refuses");
        write_broken_fixture(&dir);

        let before = snapshot(&dir);

        let report = mev::emit_state(&dir, true, None).expect("emit_state should not error");

        let has_incomplete_corpus = report
            .diagnostics
            .iter()
            .any(|d| d.locator == "E_EMIT_INCOMPLETE_CORPUS");
        assert!(
            has_incomplete_corpus,
            "expected E_EMIT_INCOMPLETE_CORPUS; got: {:#?}",
            report.diagnostics
        );

        let has_malformed_cause = report
            .diagnostics
            .iter()
            .any(|d| d.locator == "E_STATE_MALFORMED_JSON");
        assert!(
            has_malformed_cause,
            "the underlying E_STATE_MALFORMED_JSON cause must not be swallowed; got: {:#?}",
            report.diagnostics
        );

        let wrote_diags: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.locator == "I_EMIT_WROTE")
            .collect();
        assert!(
            wrote_diags.is_empty(),
            "a refused write must not emit I_EMIT_WROTE; got: {wrote_diags:#?}"
        );

        let after = snapshot(&dir);
        assert_eq!(
            after.alpha, before.alpha,
            "alpha state.json must be byte-identical after a refused write"
        );
        assert_eq!(
            after.beta, before.beta,
            "beta state.json must be byte-identical after a refused write"
        );
        assert_eq!(
            after.brain, before.brain,
            "brain state.json must be byte-identical after a refused write"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------------
    // Test 2 — dry-run stays fully exempt from the guard.
    // -----------------------------------------------------------------------

    #[test]
    fn emit_state_dry_run_still_reports_on_an_incomplete_corpus() {
        let dir = temp_dir("dry-run-exempt");
        write_broken_fixture(&dir);

        let report = mev::emit_state(&dir, false, None).expect("emit_state should not error");

        let dry_run_diags: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.locator == "W_EMIT_DRY_RUN")
            .collect();
        assert!(
            !dry_run_diags.is_empty(),
            "dry-run must still run every planner and report W_EMIT_DRY_RUN; got: {:#?}",
            report.diagnostics
        );

        let has_malformed_cause = report
            .diagnostics
            .iter()
            .any(|d| d.locator == "E_STATE_MALFORMED_JSON");
        assert!(
            has_malformed_cause,
            "E_STATE_MALFORMED_JSON must be reported in dry-run too; got: {:#?}",
            report.diagnostics
        );

        let has_incomplete_corpus = report
            .diagnostics
            .iter()
            .any(|d| d.locator == "E_EMIT_INCOMPLETE_CORPUS");
        assert!(
            !has_incomplete_corpus,
            "dry-run must never be refused by E_EMIT_INCOMPLETE_CORPUS; got: {:#?}",
            report.diagnostics
        );

        let _ = fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------------
    // Test 3 — regression: a complete corpus (loaded.len() == sources.len())
    // is unaffected, including the degenerate all-loaded case, and its
    // rewritten files are what make test 1's byte-identity assertion
    // non-vacuous.
    // -----------------------------------------------------------------------

    #[test]
    fn emit_state_write_still_proceeds_on_a_complete_corpus() {
        let dir = temp_dir("healthy-write-proceeds");
        write_healthy_fixture(&dir);

        let before = snapshot(&dir);

        let report = mev::emit_state(&dir, true, None).expect("emit_state should not error");

        let has_incomplete_corpus = report
            .diagnostics
            .iter()
            .any(|d| d.locator == "E_EMIT_INCOMPLETE_CORPUS");
        assert!(
            !has_incomplete_corpus,
            "a complete corpus must never trip the guard; got: {:#?}",
            report.diagnostics
        );

        let wrote_diags: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.locator == "I_EMIT_WROTE")
            .collect();
        assert!(
            !wrote_diags.is_empty(),
            "a healthy corpus must still write derived views; got: {:#?}",
            report.diagnostics
        );

        let after = snapshot(&dir);
        assert_ne!(
            after.beta, before.beta,
            "beta state.json must change on a healthy write — this is what makes \
             test 1's byte-identity assertion on the same file non-vacuous"
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
        let d = mev::testsupport::unique_temp_dir(&format!("mev-tier-scoping-{tag}"));
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

    /// Full core-tier fixture: brain.toml + core brain state + repo-a/repo-b/
    /// repo-d (all loadable) + repo-p (portfolio-tier control, loadable but
    /// must be excluded from the core rollup). repo-c and repo-e intentionally
    /// have no `planning/state.json` on disk at all.
    ///
    /// repo-d is loadable here (not malformed) so the tier-scoping assertions
    /// below — which are not themselves about malformed-corpus handling — are
    /// exercised against a *complete* corpus and aren't tripped by the
    /// `E_EMIT_INCOMPLETE_CORPUS` guard (`emit-state-incomplete-corpus-guard`).
    /// The malformed-repo-d scenario has its own dedicated fixture:
    /// [`corrupt_repo_d_state_json`], used only by the guard regression test.
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
        write_leaf_state(
            root,
            "repos/repo-d/planning/state.json",
            "repo-d",
            "RD.1.A",
            "Repo D block A",
        );
        write_leaf_state(
            root,
            "repos/repo-p/planning/state.json",
            "repo-p",
            "RP.1.A",
            "Repo P block A",
        );
    }

    /// Corrupts repo-d's `state.json` in an already-written core fixture with
    /// content that is not valid JSON, mirroring the real 2026-07-24 defect.
    /// Used only by the `E_EMIT_INCOMPLETE_CORPUS` guard regression test —
    /// every other test in this module runs against the fully-loadable
    /// [`write_core_fixture`] corpus.
    fn corrupt_repo_d_state_json(root: &Path) {
        write_file(
            root,
            "repos/repo-d/planning/state.json",
            "{ this is not valid json ",
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

        let report =
            mev::emit_state(&dir, true, None).expect("emit_state should not error (panic)");
        // This fixture's corpus is fully loadable (see write_core_fixture's
        // doc comment) — no errors of any kind are expected here. The
        // malformed-repo-d / E_EMIT_INCOMPLETE_CORPUS scenario has its own
        // dedicated regression test below.
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

        mev::emit_state(&dir, true, None).expect("emit_state should not error");

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

        mev::emit_state(&dir, true, None).expect("emit_state should not error");

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
    // brain repos[] (reproduces the live bastion-drop incident this block
    // fixes). Superseded by `emit-state-incomplete-corpus-guard`: rather than
    // regenerating the rollup while preserving repo-d's cached entry,
    // `emit_state` now refuses the whole write (E_EMIT_INCOMPLETE_CORPUS) —
    // a stronger guarantee that repo-d's entry (and everything else) is left
    // byte-identical rather than regenerated at all.
    // -----------------------------------------------------------------------

    #[test]
    fn malformed_child_state_json_does_not_truncate_rollup() {
        let dir = temp_dir("malformed-regression");
        write_core_fixture(&dir);
        corrupt_repo_d_state_json(&dir);

        let before = fs::read(dir.join("planning/state.json")).unwrap();

        let report = mev::emit_state(&dir, true, None).expect("emit_state should not error");

        // The malformed repo-d state.json should still surface as a load
        // error — the guard adds a refusal, it does not replace the cause.
        let malformed_diag = report
            .diagnostics
            .iter()
            .find(|d| d.locator == "E_STATE_MALFORMED_JSON");
        assert!(
            malformed_diag.is_some(),
            "expected E_STATE_MALFORMED_JSON for repo-d; got: {:#?}",
            report.diagnostics
        );

        // The incomplete-corpus guard must refuse the write outright.
        let guard_diag = report
            .diagnostics
            .iter()
            .find(|d| d.locator == "E_EMIT_INCOMPLETE_CORPUS");
        assert!(
            guard_diag.is_some(),
            "expected E_EMIT_INCOMPLETE_CORPUS guard; got: {:#?}",
            report.diagnostics
        );

        let wrote: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.locator == "I_EMIT_WROTE")
            .collect();
        assert!(
            wrote.is_empty(),
            "a refused write must not produce any I_EMIT_WROTE diagnostic; got: {wrote:#?}"
        );

        let after = fs::read(dir.join("planning/state.json")).unwrap();
        assert_eq!(
            before, after,
            "brain state.json must be byte-identical when the corpus is incomplete"
        );

        // repo-d's pre-existing cached entry is untouched (nothing was
        // regenerated), not dropped.
        let brain_after: serde_json::Value = serde_json::from_slice(&after).unwrap();
        let repos = repos_by_slug(&brain_after);
        assert!(
            repos.contains_key("repo-d"),
            "repo-d must still be present (refused write leaves it untouched); got slugs: {:?}",
            repos.keys().collect::<Vec<_>>()
        );
        let repo_d = &repos["repo-d"];
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

        mev::emit_state(&dir, true, None).expect("emit_state should not error");

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
    // derived focus (repo-c/e contribute nothing — no source and no live
    // tracks; repo-p is excluded as out-of-tier).
    // -----------------------------------------------------------------------

    #[test]
    fn brain_focus_is_repo_tagged_union_of_loadable_children() {
        let dir = temp_dir("focus-union");
        write_core_fixture(&dir);

        mev::emit_state(&dir, true, None).expect("emit_state should not error");

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

        mev::emit_state(&dir, true, None).expect("emit_state should not error");

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

        mev::emit_state(&dir, true, None).expect("first emit should not error");

        let brain_after1 = fs::read(dir.join("planning/state.json")).unwrap();
        let repo_a_after1 = fs::read(dir.join("repos/repo-a/planning/state.json")).unwrap();
        let repo_b_after1 = fs::read(dir.join("repos/repo-b/planning/state.json")).unwrap();

        let report2 = mev::emit_state(&dir, true, None).expect("second emit should not error");

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
        let d = mev::testsupport::unique_temp_dir(&format!("mev-mv4e-ripple-{tag}"));
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

        let report_before = mev::emit_state(&dir, true, None).expect("first emit should not error");
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

        let report_after = mev::emit_state(&dir, true, None).expect("second emit should not error");
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
            mev::emit_state(&dir, true, None).expect("fixed-point emit should not error");
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
                        ..Default::default()
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
                        ..Default::default()
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
                        ..Default::default()
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
                        ..Default::default()
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
                    ..Default::default()
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
    use mev::brain::state::{
        Block, BlockDep, BlockedBy, CrossRepoEdge, Endpoint, ExternalDep, Focus,
    };

    /// Build a repo-tagged `Block` with no `blocked_by` entries (NOW/NEXT shape).
    fn tagged_block(repo: &str, id: &str, title: &str) -> Block {
        Block {
            epics: Vec::new(),
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

    #[test]
    fn hq_board_deliberately_omits_the_deferred_lane() {
        // Pins an intentional divergence, not an oversight. The Operating Board
        // is terse three-lane triage — "what is live right now". Surfacing
        // back-burner work there would defeat the point of deferring it. The
        // unified board is the superset that DOES render a DEFERRED section.
        let focus = Focus {
            now: vec![],
            next: vec![],
            blocked: vec![],
            deferred: vec![tagged_block("mev", "MV.9", "Back burner")],
        };

        let rendered = render_hq_board(&focus, &[]);

        assert_eq!(
            rendered, "## NOW\n_none_\n\n## NEXT\n_none_\n\n## BLOCKED\n_none_",
            "HQ board must stay three lanes and ignore focus.deferred"
        );
        assert!(!rendered.contains("MV.9"));
    }

    /// Build a repo-tagged `Block` with the given `blocked_by` entries (BLOCKED shape).
    fn blocked_block(repo: &str, id: &str, title: &str, blocked_by: Vec<BlockedBy>) -> Block {
        Block {
            epics: Vec::new(),
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
            deferred: Vec::new(),
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
            deferred: Vec::new(),
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
                vec![BlockedBy::Block(BlockDep {
                    repo: "core".to_string(),
                    id: "D".to_string(),
                    what: None,
                })],
            )],
            deferred: Vec::new(),
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
                vec![BlockedBy::Block(BlockDep {
                    repo: "core".to_string(),
                    id: "D".to_string(),
                    what: Some("needs the shared schema".to_string()),
                })],
            )],
            deferred: Vec::new(),
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
                vec![BlockedBy::External(ExternalDep {
                    what: "waiting on hardware".to_string(),
                })],
            )],
            deferred: Vec::new(),
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
                vec![BlockedBy::Block(BlockDep {
                    repo: "core".to_string(),
                    id: "G".to_string(),
                    what: None,
                })],
            )],
            deferred: Vec::new(),
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
                    BlockedBy::Block(BlockDep {
                        repo: "core".to_string(),
                        id: "I".to_string(),
                        what: None,
                    }),
                    BlockedBy::External(ExternalDep {
                        what: "budget approval".to_string(),
                    }),
                ],
            )],
            deferred: Vec::new(),
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
    use std::collections::HashMap;

    use mev::brain::config::{BrainConfig, RepoEntry};
    use mev::brain::emit::render_unified_board;
    use mev::brain::state::{Block, BlockDep, BlockedBy, CrossRepoEdge, Endpoint, Focus};

    fn config_with_repos(entries: &[(&str, &str)]) -> BrainConfig {
        BrainConfig {
            attention: Default::default(),
            history: Default::default(),
            repos: entries
                .iter()
                .map(|(slug, tier)| RepoEntry {
                    slug: slug.to_string(),
                    tier: tier.to_string(),
                    repo_path: String::new(),
                    status_file: String::new(),
                    cache_doc: String::new(),
                    heading: String::new(),
                    prefix: None,
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
            epics: Vec::new(),
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
            epics: Vec::new(),
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
            deferred: Vec::new(),
        };

        let rendered = render_unified_board(&focus, &[], &HashMap::new(), &config, today());

        assert!(rendered.contains("- [BIZ] business:BR.1 — Biz block"));
        assert!(rendered.contains("- [ENG] mev:MV.1 — Eng block"));
    }

    #[test]
    fn tags_business_tier_root_itself_biz_not_just_its_children() {
        // Regression for the backlog defect (2026-07-17): the real
        // `brain.toml` registers the `business` tier ROOT with
        // `tier = "_root"` (like every other tier root — `core`, `side`,
        // `client`) — only its CHILDREN (e.g. `bastiel`) carry
        // `tier = "business"`. A block tagged with `repo: "business"` (the
        // tier root's own authored `tracks[]`, e.g. `BZ.*`) must still render
        // `[BIZ]`, not fall through to the `[ENG]` default because its own
        // `tier` field doesn't literally equal `"business"`.
        let config = config_with_repos(&[("business", "_root"), ("bastiel", "business")]);
        let focus = Focus {
            now: vec![
                tagged_block("business", "BZ.1", "Revenue block", None, None),
                tagged_block("bastiel", "BL.1", "Demo block", None, None),
            ],
            next: vec![],
            blocked: vec![],
            deferred: Vec::new(),
        };

        let rendered = render_unified_board(&focus, &[], &HashMap::new(), &config, today());

        assert!(
            rendered.contains("- [BIZ] business:BZ.1 — Revenue block"),
            "the business tier root's own tracks[] must render [BIZ], got:\n{rendered}"
        );
        assert!(rendered.contains("- [BIZ] bastiel:BL.1 — Demo block"));
    }

    #[test]
    fn unrecognised_repo_slug_defaults_to_eng() {
        let config = config_with_repos(&[]);
        let focus = Focus {
            now: vec![tagged_block("mystery", "X.1", "Unknown", None, None)],
            next: vec![],
            blocked: vec![],
            deferred: Vec::new(),
        };

        let rendered = render_unified_board(&focus, &[], &HashMap::new(), &config, today());

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
            deferred: Vec::new(),
        };

        let rendered = render_unified_board(&focus, &[], &HashMap::new(), &config, today());

        let biz_idx = rendered.find("BR.2").unwrap();
        let eng_idx = rendered.find("MV.2").unwrap();
        assert!(
            biz_idx < eng_idx,
            "expected P1 business block before P2 engineering block:\n{rendered}"
        );
    }

    #[test]
    fn next_orders_by_effective_priority_when_provided_overriding_raw_priority() {
        let config = config_with_repos(&[("mev", "engine")]);
        let focus = Focus {
            now: vec![],
            next: vec![
                // Own priority 2 (cold), but effective-priority map says P0
                // (inherited from a hotter dependent it gates).
                tagged_block("mev", "GATE", "Gates a P0 block", Some(2), None),
                // Own priority 1, no entry in the effective map — falls back
                // to its own raw priority.
                tagged_block("mev", "SOLO", "No hot dependents", Some(1), None),
            ],
            blocked: vec![],
            deferred: Vec::new(),
        };
        let mut effective = HashMap::new();
        effective.insert("mev:GATE".to_string(), 0u8);

        let rendered = render_unified_board(&focus, &[], &effective, &config, today());

        let gate_idx = rendered.find("mev:GATE").unwrap();
        let solo_idx = rendered.find("mev:SOLO").unwrap();
        assert!(
            gate_idx < solo_idx,
            "effective priority (P0) must win over raw priority (P2) for sorting:\n{rendered}"
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
            deferred: Vec::new(),
        };

        let rendered = render_unified_board(&focus, &[], &HashMap::new(), &config, today());

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
            deferred: Vec::new(),
        };

        let rendered = render_unified_board(&focus, &[], &HashMap::new(), &config, today());
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
            deferred: Vec::new(),
        };

        let rendered = render_unified_board(&focus, &[], &HashMap::new(), &config, today());
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
                vec![BlockedBy::Block(BlockDep {
                    repo: "core".to_string(),
                    id: "D".to_string(),
                    what: None,
                })],
            )],
            deferred: Vec::new(),
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

        let rendered = render_unified_board(&focus, &edges, &HashMap::new(), &config, today());

        assert!(
            rendered
                .contains("- [ENG] mev:C — Block C (blocked by core:D (waiting on schema freeze))")
        );
    }

    #[test]
    fn deferred_lane_renders_its_own_section() {
        let config = config_with_repos(&[("mev", "engine")]);
        let focus = Focus {
            now: vec![],
            next: vec![],
            blocked: vec![],
            deferred: vec![tagged_block("mev", "MV.9", "Back burner", None, None)],
        };

        let rendered = render_unified_board(&focus, &[], &HashMap::new(), &config, today());

        assert!(rendered.contains("## DEFERRED"), "got: {rendered}");
        assert!(
            rendered.contains("MV.9 — Back burner"),
            "deferred block must be listed, got: {rendered}"
        );
        // And it must NOT have leaked into the lane it was deferred out of.
        let next_section = rendered
            .split("## NEXT")
            .nth(1)
            .and_then(|s| s.split("##").next())
            .unwrap_or_default();
        assert!(
            next_section.contains("_none_"),
            "deferred work must not appear in NEXT, got: {next_section}"
        );
    }

    #[test]
    fn deferred_block_is_excluded_from_due_soon_even_when_overdue() {
        // Deferring a block is the decision to let its date pass. An overdue
        // deferred block must stay silent rather than nagging from DUE-SOON.
        let config = config_with_repos(&[("mev", "engine")]);
        let focus = Focus {
            now: vec![],
            next: vec![],
            blocked: vec![],
            deferred: vec![tagged_block(
                "mev",
                "MV.9",
                "Overdue but parked",
                None,
                Some("2026-01-01"),
            )],
        };

        let rendered = render_unified_board(&focus, &[], &HashMap::new(), &config, today());

        let due_soon = rendered.split("## DUE-SOON").nth(1).unwrap_or_default();
        assert!(
            due_soon.contains("_none_"),
            "deferred block must not appear in DUE-SOON, got: {due_soon}"
        );
        assert!(!due_soon.contains("overdue"), "got: {due_soon}");
    }

    #[test]
    fn empty_focus_renders_none_in_all_five_sections() {
        let config = config_with_repos(&[]);
        let rendered =
            render_unified_board(&Focus::default(), &[], &HashMap::new(), &config, today());

        let expected = "## NOW\n_none_\n\n## NEXT\n_none_\n\n## BLOCKED\n_none_\n\n\
             ## DEFERRED\n_none_\n\n## DUE-SOON\n_none_";
        assert_eq!(rendered, expected);
    }

    #[test]
    fn rendered_board_has_no_trailing_newline() {
        let config = config_with_repos(&[]);
        let rendered =
            render_unified_board(&Focus::default(), &[], &HashMap::new(), &config, today());
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
            attention: Default::default(),
            history: Default::default(),
            repos: entries
                .iter()
                .map(|(slug, tier)| RepoEntry {
                    slug: slug.to_string(),
                    tier: tier.to_string(),
                    repo_path: String::new(),
                    status_file: String::new(),
                    cache_doc: String::new(),
                    heading: String::new(),
                    prefix: None,
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
            epics: Vec::new(),
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
            note: None,
            description: None,
            ..Default::default()
        }
    }

    fn make_leaf_file(repo: &str, tracks: Vec<Track>, focus: Focus) -> StateFile {
        StateFile {
            epics: Vec::new(),
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
            ..Default::default()
        }
    }

    fn make_brain_file(
        repos: Vec<RepoRollup>,
        cross_repo: Vec<mev::brain::state::CrossRepoEdge>,
        focus: Focus,
    ) -> StateFile {
        StateFile {
            epics: Vec::new(),
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
// plan_unified_board — I/O-level coverage for the unified priority board
// splice (MV.6.B). Mirrors task2_plan_hq_board's shape, targeting the
// separate `unified-board` sentinel and `today`-parameterized DUE-SOON.
// ---------------------------------------------------------------------------

mod task2_plan_unified_board {
    use mev::brain::config::{BrainConfig, RepoEntry};
    use mev::brain::emit::{markers, plan_unified_board};
    use mev::brain::state::{
        BlockedBy, Focus, RepoRollup, StateFile, StateSource, Track, TrackBlock, build_state_graph,
    };
    use std::path::PathBuf;

    fn config_with_repos(entries: &[(&str, &str)]) -> BrainConfig {
        BrainConfig {
            attention: Default::default(),
            history: Default::default(),
            repos: entries
                .iter()
                .map(|(slug, tier)| RepoEntry {
                    slug: slug.to_string(),
                    tier: tier.to_string(),
                    repo_path: String::new(),
                    status_file: String::new(),
                    cache_doc: String::new(),
                    heading: String::new(),
                    prefix: None,
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
            epics: Vec::new(),
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
            note: None,
            description: None,
            ..Default::default()
        }
    }

    fn make_leaf_file(repo: &str, tracks: Vec<Track>, focus: Focus) -> StateFile {
        StateFile {
            epics: Vec::new(),
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
            ..Default::default()
        }
    }

    fn make_brain_file(
        repos: Vec<RepoRollup>,
        cross_repo: Vec<mev::brain::state::CrossRepoEdge>,
        focus: Focus,
    ) -> StateFile {
        StateFile {
            epics: Vec::new(),
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
            ..Default::default()
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
         <!-- BEGIN generated:unified-board -->\n\
         <!-- END generated:unified-board -->\n\n\
         Narrative after.\n"
            .to_string()
    }

    fn today() -> chrono::NaiveDate {
        chrono::NaiveDate::from_ymd_opt(2026, 7, 5).unwrap()
    }

    #[test]
    fn unified_board_splice_produces_expected_content() {
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
            ..Default::default()
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

        let plan = plan_unified_board(&files, &graph, &config, today());

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
                .contains("[ENG] repo-a:RA.1.A — Repo A block"),
            "unified board missing derived, tagged NOW entry: {}",
            action.new_content
        );
        assert!(
            action.new_content.contains("## DUE-SOON"),
            "unified board missing DUE-SOON heading: {}",
            action.new_content
        );
    }

    #[test]
    fn unified_board_missing_sentinel_warns_no_action() {
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

        let plan = plan_unified_board(&files, &graph, &config, today());

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
    fn unified_board_missing_file_warns_no_action() {
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

        let plan = plan_unified_board(&files, &graph, &config, today());

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
    fn unified_board_tier_sub_brain_is_skipped() {
        let tmp = tempfile::tempdir().unwrap();
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

        let plan = plan_unified_board(&files, &graph, &config, today());

        assert_eq!(
            plan.actions.len(),
            0,
            "tier sub-brain must never be targeted by plan_unified_board; got {}",
            plan.actions.len()
        );
        assert!(
            plan.diagnostics.is_empty(),
            "no diagnostics expected for the skipped tier sub-brain; got: {:?}",
            plan.diagnostics
        );
    }

    #[test]
    fn unified_board_fixed_point_no_action_on_second_pass() {
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
            ..Default::default()
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

        let plan = plan_unified_board(&files, &graph, &config, today());
        assert_eq!(plan.actions.len(), 1);
        std::fs::write(&status_path, &plan.actions[0].new_content).unwrap();

        let plan2 = plan_unified_board(&files, &graph, &config, today());
        assert_eq!(
            plan2.actions.len(),
            0,
            "expected fixed-point no-action on second pass; got {}",
            plan2.actions.len()
        );
        assert!(plan2.diagnostics.is_empty());
    }

    #[test]
    fn unified_board_marker_constant_matches_sentinel() {
        assert_eq!(markers::UNIFIED_BOARD, "unified-board");
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
        let d = mev::testsupport::unique_temp_dir(&format!("mev-emit-state-cli-guard-{tag}"));
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
                epics: Vec::new(),
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
            deferred: Vec::new(),
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
            deferred: Vec::new(),
        };
        let new_content = reconcile_status_scalars(original, &focus);
        assert!(new_content.contains("now: []"));
        assert!(new_content.contains("next: []"));
        assert!(new_content.contains("blocked: []"));
    }

    #[test]
    fn reconcile_status_scalars_does_not_append_empty_deferred() {
        // Churn guard. ~23 `status.md` files have no `deferred:` key and nothing
        // deferred. Appending `deferred: []` to each would rewrite every one of
        // them on the first `emit-state --write` after this change.
        let original = "---\n\
                        title: Foo\n\
                        now: []\n\
                        next: []\n\
                        blocked: []\n\
                        ---\n\n\
                        # Body\n";
        let focus = Focus::default();
        let new_content = reconcile_status_scalars(original, &focus);

        assert_eq!(
            new_content, original,
            "an empty deferred lane must leave the frontmatter byte-identical"
        );
    }

    #[test]
    fn reconcile_status_scalars_appends_deferred_when_non_empty() {
        let original = "---\n\
                        title: Foo\n\
                        ---\n\n\
                        # Body\n";
        let focus = Focus {
            now: vec![],
            next: vec![],
            blocked: vec![],
            deferred: vec![Block {
                epics: Vec::new(),
                due: None,
                priority: None,
                id: "9".into(),
                title: "Nine".into(),
                status: Some("deferred".into()),
                note: None,
                repo: Some("core".into()),
                blocked_by: vec![],
            }],
        };
        let new_content = reconcile_status_scalars(original, &focus);
        assert!(
            new_content.contains("deferred: \"core:9 — Nine\""),
            "got: {new_content}"
        );
    }

    #[test]
    fn reconcile_status_scalars_empties_an_existing_deferred_key() {
        // The other half of the conditional-append rule: once a repo HAS a
        // `deferred:` key, un-deferring everything must set it to `[]` rather
        // than leaving a stale value behind.
        let original = "---\n\
                        title: Foo\n\
                        deferred: \"core:9 — Nine\"\n\
                        ---\n\n\
                        # Body\n";
        let new_content = reconcile_status_scalars(original, &Focus::default());
        assert!(new_content.contains("deferred: []"), "got: {new_content}");
        assert!(!new_content.contains("Nine"));
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
            epics: Vec::new(),
            repo: "myrepo".to_string(),
            kind: "project".to_string(),
            updated: "2026-06-30".to_string(),
            focus: Focus::default(),
            tracks: vec![Track {
                title: "P".to_string(),
                blocks: vec![TrackBlock {
                    epics: Vec::new(),
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
                    note: None,
                    description: None,
                    ..Default::default()
                }],
                ..Default::default()
            }],
            repos: vec![],
            cross_repo: vec![],
            tiers: vec![],
            note: None,
            backlog: vec![],
            carryover: vec![],
            ..Default::default()
        };

        let files = vec![(src, file)];
        let graph = build_state_graph(&files);
        let config = BrainConfig {
            attention: Default::default(),
            history: Default::default(),
            repos: vec![RepoEntry {
                slug: "myrepo".to_string(),
                tier: "core".to_string(),
                repo_path: "".to_string(),
                status_file: "".to_string(), // fallback to sibling
                cache_doc: "".to_string(),
                heading: "".to_string(),
                prefix: None,
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

// ---------------------------------------------------------------------------
// Task 4 (MV.6.B) — unified board integration test: HQ brain fixture with a
// business-tier repo and an engineering-tier repo, exercised end to end
// through `mev::emit_state`. Asserts [BIZ]/[ENG] tagging, NEXT priority/due
// ordering, DUE-SOON windowing, and fixed-point idempotence on a second pass.
// ---------------------------------------------------------------------------

mod task4_unified_board_integration {
    use std::fs;
    use std::path::Path;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let d = mev::testsupport::unique_temp_dir(&format!("mev-unified-board-{tag}"));
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

    /// `brain.toml` registering one business-tier repo and one engineering-tier
    /// repo (no tier sub-brains — `derive_brain_focus` at `TierScope::All`
    /// unions every `[[repos]]` entry directly regardless of declared tiers).
    fn write_brain_toml(root: &Path) {
        let toml = r#"[vocab]
layer = ["brain", "engine", "factory", "console", "surface", "infra", "business", "content", "meta"]
status = ["active", "draft", "deprecated", "superseded", "archived"]

[crawl]
skip_dirs = ["target", "node_modules", ".git"]

[[repos]]
slug = "biz-repo"
tier = "business"
repo_path = "repos/biz-repo"
status_file = "repos/biz-repo/planning/status.md"
cache_doc = "docs/projects/biz-repo.md"
heading = "Biz Repo"

[[repos]]
slug = "eng-repo"
tier = "engine"
repo_path = "repos/eng-repo"
status_file = "repos/eng-repo/planning/status.md"
cache_doc = "docs/projects/eng-repo.md"
heading = "Eng Repo"
"#;
        fs::write(root.join("brain.toml"), toml.as_bytes()).unwrap();
    }

    /// HQ brain `planning/state.json` (`repo: "hq"` matches no declared tier,
    /// so `tier_scope_for` resolves it to `TierScope::All`).
    fn write_hq_state(root: &Path) {
        let state = serde_json::json!({
            "repo": "hq",
            "kind": "brain",
            "updated": "2026-07-05",
            "focus": { "now": [], "next": [], "blocked": [] },
            "repos": [],
            "cross_repo": []
        });
        write_json(root, "planning/state.json", &state);
    }

    /// HQ `status.md` carrying the `unified-board` sentinel (OKF-fronted).
    fn write_hq_status_md(root: &Path) {
        let doc = "---\n\
                    type: ProjectStatus\n\
                    title: HQ status\n\
                    description: HQ unified board fixture.\n\
                    ---\n\n\
                    # HQ Status\n\n\
                    <!-- BEGIN generated:unified-board -->\n\
                    <!-- END generated:unified-board -->\n";
        write_file(root, "planning/status.md", doc);
    }

    /// The business-tier repo's `planning/state.json`: a single ready (`open`,
    /// no unmet deps) `priority: 1` block with no `due`, so it lands in NEXT.
    fn write_biz_repo_state(root: &Path) {
        let state = serde_json::json!({
            "repo": "biz-repo",
            "kind": "project",
            "updated": "2026-07-05",
            "focus": { "now": [], "next": [], "blocked": [] },
            "tracks": [
                {
                    "title": "Phase 1",
                    "blocks": [
                        {
                            "id": "BR.1",
                            "title": "Biz P1 block",
                            "status": "open",
                            "depends_on": [],
                            "wave": 1,
                            "priority": 1,
                            "due": null
                        }
                    ]
                }
            ]
        });
        write_json(root, "repos/biz-repo/planning/state.json", &state);
    }

    /// The engineering-tier repo's `planning/state.json`: a `priority: 2`
    /// ready block (NEXT — must sort below the business `priority: 1` block),
    /// plus three `in_progress` (NOW) blocks exercising DUE-SOON: one due
    /// within the 14-day window, one overdue, and one far in the future
    /// (excluded from DUE-SOON).
    fn write_eng_repo_state(root: &Path, due_soon: &str, overdue: &str, far_future: &str) {
        let state = serde_json::json!({
            "repo": "eng-repo",
            "kind": "project",
            "updated": "2026-07-05",
            "focus": { "now": [], "next": [], "blocked": [] },
            "tracks": [
                {
                    "title": "Phase 1",
                    "blocks": [
                        {
                            "id": "MV.1",
                            "title": "Eng P2 block",
                            "status": "open",
                            "depends_on": [],
                            "wave": 1,
                            "priority": 2,
                            "due": null
                        },
                        {
                            "id": "MV.2",
                            "title": "Eng due-soon block",
                            "status": "in_progress",
                            "depends_on": [],
                            "wave": 1,
                            "due": due_soon
                        },
                        {
                            "id": "MV.3",
                            "title": "Eng overdue block",
                            "status": "in_progress",
                            "depends_on": [],
                            "wave": 2,
                            "due": overdue
                        },
                        {
                            "id": "MV.4",
                            "title": "Eng far-future block",
                            "status": "in_progress",
                            "depends_on": [],
                            "wave": 3,
                            "due": far_future
                        }
                    ]
                }
            ]
        });
        write_json(root, "repos/eng-repo/planning/state.json", &state);
    }

    fn read(root: &Path, rel: &str) -> String {
        fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("failed to read {rel}: {e}"))
    }

    #[test]
    fn unified_board_emit_tags_orders_and_is_a_fixed_point() {
        let dir = temp_dir("emit");

        // Anchor due dates to "today" so the DUE-SOON window assertions are
        // stable regardless of the real current date.
        let today = chrono::Local::now().date_naive();
        let due_soon = (today + chrono::Duration::days(5))
            .format("%Y-%m-%d")
            .to_string();
        let overdue = (today - chrono::Duration::days(10))
            .format("%Y-%m-%d")
            .to_string();
        let far_future = (today + chrono::Duration::days(400))
            .format("%Y-%m-%d")
            .to_string();

        write_brain_toml(&dir);
        write_hq_state(&dir);
        write_hq_status_md(&dir);
        write_biz_repo_state(&dir);
        write_eng_repo_state(&dir, &due_soon, &overdue, &far_future);

        let report = mev::emit_state(&dir, true, None).expect("emit should not error");
        let unexpected_errors: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.severity == mev::Severity::Error)
            .collect();
        assert!(
            unexpected_errors.is_empty(),
            "fixture emit should have no errors; got: {unexpected_errors:#?}"
        );

        let status_after = read(&dir, "planning/status.md");
        let board = status_after
            .split("<!-- BEGIN generated:unified-board -->")
            .nth(1)
            .and_then(|s| s.split("<!-- END generated:unified-board -->").next())
            .expect("unified-board region must be present");

        // --- [BIZ]/[ENG] tagging. ---
        assert!(
            board.contains("[BIZ] biz-repo:BR.1"),
            "business-tier repo's block must be tagged [BIZ]; got:\n{board}"
        );
        assert!(
            board.contains("[ENG] eng-repo:MV.1"),
            "engineering-tier repo's block must be tagged [ENG]; got:\n{board}"
        );

        // --- NEXT ordering: P1 business block sorts above P2 eng block. ---
        let next_idx = board.find("## NEXT").unwrap();
        let blocked_idx = board.find("## BLOCKED").unwrap();
        let next_section = &board[next_idx..blocked_idx];
        let biz_pos = next_section
            .find("biz-repo:BR.1")
            .expect("BR.1 must appear in NEXT");
        let eng_pos = next_section
            .find("eng-repo:MV.1")
            .expect("MV.1 must appear in NEXT");
        assert!(
            biz_pos < eng_pos,
            "P1 business block must sort above P2 engineering block in NEXT; got:\n{next_section}"
        );

        // --- DUE-SOON: in-window + overdue included, far-future excluded. ---
        let due_soon_section = board
            .split("## DUE-SOON")
            .nth(1)
            .expect("DUE-SOON section must be present");
        assert!(
            due_soon_section.contains("eng-repo:MV.2"),
            "in-window due block must be listed in DUE-SOON; got:\n{due_soon_section}"
        );
        assert!(
            due_soon_section.contains("eng-repo:MV.3"),
            "overdue block must be listed in DUE-SOON; got:\n{due_soon_section}"
        );
        assert!(
            due_soon_section.contains("(overdue)"),
            "overdue block must be surfaced louder with an (overdue) marker; got:\n{due_soon_section}"
        );
        assert!(
            !due_soon_section.contains("eng-repo:MV.4"),
            "far-future due block must be excluded from DUE-SOON; got:\n{due_soon_section}"
        );

        // --- Fixed point: a second emit over the already-derived corpus must
        // be a no-op, and the unified-board region byte-identical. ---
        let snapshot = fs::read(dir.join("planning/status.md")).unwrap();

        let report_second =
            mev::emit_state(&dir, true, None).expect("second emit should not error");
        let wrote: Vec<_> = report_second
            .diagnostics
            .iter()
            .filter(|d| d.locator == "I_EMIT_WROTE")
            .collect();
        assert!(
            wrote.is_empty(),
            "second emit over the already-derived corpus must be a no-op; got: {wrote:#?}"
        );

        let after_second = fs::read(dir.join("planning/status.md")).unwrap();
        assert_eq!(
            after_second, snapshot,
            "planning/status.md must be byte-identical after the fixed-point pass"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------------
    // MV.7.A — effective-priority inheritance: an engineering block that
    // `depends_on`-gates a P0 business block must inherit that P0 hotness
    // and float above an ungated P2 engineering block in NEXT, even though
    // it carries no own priority.
    // -----------------------------------------------------------------------

    /// The business-tier repo's `planning/state.json` for the gating
    /// fixture: a `priority: 0` (P0) block that `depends_on` the
    /// engineering repo's gate block, so it lands in BLOCKED (its
    /// dependency is still `open`, not `closed`) — but its P0 hotness must
    /// still propagate backward onto the eng block that gates it.
    fn write_biz_repo_state_gating(root: &Path) {
        let state = serde_json::json!({
            "repo": "biz-repo",
            "kind": "project",
            "updated": "2026-07-05",
            "focus": { "now": [], "next": [], "blocked": [] },
            "tracks": [
                {
                    "title": "Phase 1",
                    "blocks": [
                        {
                            "id": "BR.0",
                            "title": "Biz P0 block gated by eng",
                            "status": "open",
                            "depends_on": [
                                { "type": "block", "repo": "eng-repo", "id": "MV.0" }
                            ],
                            "wave": 1,
                            "priority": 0,
                            "due": null
                        }
                    ]
                }
            ]
        });
        write_json(root, "repos/biz-repo/planning/state.json", &state);
    }

    /// The engineering-tier repo's `planning/state.json` for the gating
    /// fixture: `MV.0` is ready with no own priority and gates the P0
    /// business block; `MV.1` is a ready `priority: 2` block with no hot
    /// dependents. `MV.0` must sort above `MV.1` in NEXT once effective
    /// (not raw) priority is the sort key.
    fn write_eng_repo_state_gating(root: &Path) {
        let state = serde_json::json!({
            "repo": "eng-repo",
            "kind": "project",
            "updated": "2026-07-05",
            "focus": { "now": [], "next": [], "blocked": [] },
            "tracks": [
                {
                    "title": "Phase 1",
                    "blocks": [
                        {
                            "id": "MV.0",
                            "title": "Eng gate block (no own priority)",
                            "status": "open",
                            "depends_on": [],
                            "wave": 1,
                            "due": null
                        },
                        {
                            "id": "MV.1",
                            "title": "Eng P2 block, no hot dependents",
                            "status": "open",
                            "depends_on": [],
                            "wave": 2,
                            "priority": 2,
                            "due": null
                        }
                    ]
                }
            ]
        });
        write_json(root, "repos/eng-repo/planning/state.json", &state);
    }

    #[test]
    fn unified_board_effective_priority_gating_floats_eng_block_and_is_idempotent() {
        let dir = temp_dir("effective-priority");

        write_brain_toml(&dir);
        write_hq_state(&dir);
        write_hq_status_md(&dir);
        write_biz_repo_state_gating(&dir);
        write_eng_repo_state_gating(&dir);

        let report = mev::emit_state(&dir, true, None).expect("emit should not error");
        let unexpected_errors: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.severity == mev::Severity::Error)
            .collect();
        assert!(
            unexpected_errors.is_empty(),
            "fixture emit should have no errors; got: {unexpected_errors:#?}"
        );

        let status_after = read(&dir, "planning/status.md");
        let board = status_after
            .split("<!-- BEGIN generated:unified-board -->")
            .nth(1)
            .and_then(|s| s.split("<!-- END generated:unified-board -->").next())
            .expect("unified-board region must be present");

        let next_idx = board.find("## NEXT").unwrap();
        let blocked_idx = board.find("## BLOCKED").unwrap();
        let next_section = &board[next_idx..blocked_idx];

        let gate_pos = next_section
            .find("eng-repo:MV.0")
            .expect("MV.0 (the P0 gate) must appear in NEXT");
        let cold_pos = next_section
            .find("eng-repo:MV.1")
            .expect("MV.1 (cold, no hot dependents) must appear in NEXT");
        assert!(
            gate_pos < cold_pos,
            "eng block gating a P0 business block must sort above a P2 eng \
             block with no hot dependents; got:\n{next_section}"
        );

        // The gated business block itself is BLOCKED (its dependency is open,
        // not closed) — confirms this is genuinely inherited hotness, not the
        // business block's own priority leaking into NEXT.
        let blocked_section = &board[blocked_idx..];
        assert!(
            blocked_section.contains("biz-repo:BR.0"),
            "the P0 business block must be BLOCKED (its eng dependency is \
             still open), not NEXT; got:\n{blocked_section}"
        );

        // --- Fixed point: a second emit over the already-derived corpus must
        // be a no-op, and the unified-board region byte-identical. ---
        let snapshot = fs::read(dir.join("planning/status.md")).unwrap();

        let report_second =
            mev::emit_state(&dir, true, None).expect("second emit should not error");
        let wrote: Vec<_> = report_second
            .diagnostics
            .iter()
            .filter(|d| d.locator == "I_EMIT_WROTE")
            .collect();
        assert!(
            wrote.is_empty(),
            "second emit over the already-derived corpus must be a no-op; got: {wrote:#?}"
        );

        let after_second = fs::read(dir.join("planning/status.md")).unwrap();
        assert_eq!(
            after_second, snapshot,
            "planning/status.md must be byte-identical after the fixed-point pass"
        );

        let _ = fs::remove_dir_all(&dir);
    }
}

// ---------------------------------------------------------------------------
// emit-state-same-file-batching regression — hq-board, unified-board, and
// attention all splice into the SAME planning/status.md via disjoint
// sentinels. Before the fix, emit_state planned all eight batch-1 planners
// from the same pre-batch original before applying any of them, so writing
// a later planner's action (based on that stale original) would silently
// drop an earlier planner's just-applied sentinel edit for the same file.
// This proves all three regions survive a single emit_state(write=true) run.
// ---------------------------------------------------------------------------

mod same_file_batching_regression {
    use std::fs;
    use std::path::Path;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let d = mev::testsupport::unique_temp_dir(&format!("mev-same-file-batching-{tag}"));
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

    /// `brain.toml` registering one leaf repo under a tier that doesn't match
    /// the HQ root's own `repo` value, so the root resolves to `TierScope::All`.
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

    fn write_hq_state(root: &Path) {
        let state = serde_json::json!({
            "repo": "hq",
            "kind": "brain",
            "updated": "2026-07-24",
            "focus": { "now": [], "next": [], "blocked": [] },
            "repos": [],
            "cross_repo": []
        });
        write_json(root, "planning/state.json", &state);
    }

    /// HQ `status.md` carrying all three batch-1 sentinels that target this
    /// one file: `hq-board`, `unified-board`, and `attention`.
    fn write_hq_status_md(root: &Path) {
        let doc = "---\n\
                    type: ProjectStatus\n\
                    title: HQ status\n\
                    description: Same-file batching regression fixture.\n\
                    ---\n\n\
                    # HQ Status\n\n\
                    <!-- BEGIN generated:hq-board -->\n\
                    <!-- END generated:hq-board -->\n\n\
                    <!-- BEGIN generated:unified-board -->\n\
                    <!-- END generated:unified-board -->\n\n\
                    <!-- BEGIN generated:attention -->\n\
                    <!-- END generated:attention -->\n";
        write_file(root, "planning/status.md", doc);
    }

    /// One leaf repo with an open, ready, `priority: 1` block (renders in both
    /// hq-board's NEXT and unified-board's NEXT) and a stale carryover past
    /// its default staleness threshold (renders in attention).
    fn write_alpha_state(root: &Path) {
        let state = serde_json::json!({
            "repo": "alpha",
            "kind": "project",
            "updated": "2026-07-24",
            "focus": { "now": [], "next": [], "blocked": [] },
            "tracks": [
                {
                    "title": "Phase 1",
                    "blocks": [
                        {
                            "id": "AL.1",
                            "title": "Alpha ready block",
                            "status": "open",
                            "depends_on": [],
                            "wave": 1,
                            "priority": 1
                        }
                    ]
                }
            ],
            "carryover": [
                {
                    "slug": "alpha-stale-thing",
                    "scope": { "repo": "alpha" },
                    "kind": "known_issue",
                    "text": "A carryover old enough to clear the default staleness threshold.",
                    "clears_when": "Never, for this fixture.",
                    "created": "2020-01-01"
                }
            ]
        });
        write_json(root, "repos/alpha/planning/state.json", &state);
    }

    fn read(root: &Path, rel: &str) -> String {
        fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("failed to read {rel}: {e}"))
    }

    fn region<'a>(doc: &'a str, marker: &str) -> &'a str {
        doc.split(&format!("<!-- BEGIN generated:{marker} -->"))
            .nth(1)
            .and_then(|s| s.split(&format!("<!-- END generated:{marker} -->")).next())
            .unwrap_or_else(|| panic!("{marker} region must be present; got:\n{doc}"))
    }

    #[test]
    fn hq_board_unified_board_and_attention_all_survive_one_write() {
        let dir = temp_dir("survive");
        write_brain_toml(&dir);
        write_hq_state(&dir);
        write_hq_status_md(&dir);
        write_alpha_state(&dir);

        let report = mev::emit_state(&dir, true, None).expect("emit_state should not error");
        let errors: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.severity == mev::Severity::Error)
            .collect();
        assert!(
            errors.is_empty(),
            "fixture emit should have no errors; got: {errors:#?}"
        );

        let status_after = read(&dir, "planning/status.md");

        // Before the fix, only the LAST batch-1 planner to touch this file
        // (attention) would have real content; hq-board and unified-board
        // would each have been silently reverted to empty by a later
        // planner's stale-original write.
        assert!(
            region(&status_after, "hq-board").contains("alpha:AL.1"),
            "hq-board region must survive the later unified-board/attention writes to the \
             same file; got:\n{status_after}"
        );
        assert!(
            region(&status_after, "unified-board").contains("alpha:AL.1"),
            "unified-board region must survive the later attention write to the same file; \
             got:\n{status_after}"
        );
        assert!(
            region(&status_after, "attention").contains("alpha-stale-thing"),
            "attention region must be populated; got:\n{status_after}"
        );

        let _ = fs::remove_dir_all(&dir);
    }
}

// ---------------------------------------------------------------------------
// brain-focus-dual-role-drift task 4 — end-to-end proof that the writer
// (`emit_state`) and the validator (`validate_brain_state`'s
// `check_focus_drift`) now agree for a dual-role brain (a `kind: "brain"`
// file that also carries its own `tracks[]`).
// ---------------------------------------------------------------------------

mod task_dual_role_focus_drift_integration {
    use std::fs;
    use std::path::Path;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let d = mev::testsupport::unique_temp_dir(&format!("mev-dual-role-drift-{tag}"));
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

    /// `brain.toml` with one `core`-tier child ("alpha"), so the root brain's
    /// own `repo: "core"` slug scopes (via `tier_scope_for`) to just that tier
    /// — the same dual-role shape as the live `business` brain.
    fn write_brain_toml(root: &Path) {
        let toml = r#"[vocab]
layer = ["brain", "engine", "factory", "console", "surface", "infra", "business", "content", "meta"]
status = ["active", "draft", "deprecated", "superseded", "archived"]

[crawl]
skip_dirs = ["target", "node_modules", ".git"]

[[repos]]
slug = "alpha"
tier = "core"
repo_path = "repos/alpha"
status_file = "repos/alpha/planning/status.md"
cache_doc = "docs/projects/alpha.md"
heading = "Alpha"
"#;
        fs::write(root.join("brain.toml"), toml.as_bytes()).unwrap();
    }

    /// The dual-role brain's own `planning/state.json`: `kind: "brain"`, own
    /// `tracks[]` (one ready block `CO.1.A`), and a deliberately stale stored
    /// `focus` (empty) — proving `emit_state --write` derives it correctly and
    /// the validator then agrees.
    fn write_core_brain_state(root: &Path) {
        let state = serde_json::json!({
            "repo": "core",
            "kind": "brain",
            "updated": "2026-06-29",
            "focus": { "now": [], "next": [], "blocked": [] },
            "tracks": [
                {
                    "title": "Own Track",
                    "blocks": [
                        { "id": "CO.1.A", "title": "Core's own ready block", "status": "open" }
                    ]
                }
            ],
            "repos": [],
            "cross_repo": []
        });
        write_json(root, "planning/state.json", &state);
    }

    /// One in-scope `project`-kind child with its own ready block.
    fn write_alpha_state(root: &Path) {
        let state = serde_json::json!({
            "repo": "alpha",
            "kind": "project",
            "updated": "2026-06-29",
            "focus": { "now": [], "next": [], "blocked": [] },
            "tracks": [
                {
                    "title": "Phase 1",
                    "blocks": [
                        { "id": "AL.1.A", "title": "Alpha's ready block", "status": "open" }
                    ]
                }
            ]
        });
        write_json(root, "repos/alpha/planning/state.json", &state);
    }

    fn write_fixture(root: &Path) {
        write_brain_toml(root);
        write_core_brain_state(root);
        write_alpha_state(root);
    }

    #[test]
    fn emit_then_validate_dual_role_brain_produces_zero_focus_drift() {
        let dir = temp_dir("zero-drift");
        write_fixture(&dir);

        // Write pass — this is what regenerates the brain's stored `focus`
        // via `derive_brain_focus` (children ∪ own tracks[], Facet A).
        let emit_report = mev::emit_state(&dir, true, None).expect("emit_state should not error");
        let emit_errors: Vec<_> = emit_report
            .diagnostics
            .iter()
            .filter(|d| d.severity == mev::Severity::Error)
            .collect();
        assert!(
            emit_errors.is_empty(),
            "emit_state over the dual-role fixture should have no errors; got: {emit_errors:#?}"
        );

        // The written focus must include BOTH the self ready block and the
        // child's ready block — the whole point of Facet A.
        let brain_after: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(dir.join("planning/state.json")).unwrap())
                .unwrap();
        let next_ids: Vec<&str> = brain_after["focus"]["next"]
            .as_array()
            .expect("focus.next must be an array")
            .iter()
            .map(|b| b["id"].as_str().unwrap())
            .collect();
        assert!(
            next_ids.contains(&"CO.1.A"),
            "core's own ready block must surface in the written focus.next; got: {next_ids:?}"
        );
        assert!(
            next_ids.contains(&"AL.1.A"),
            "alpha's ready block must surface in the written focus.next; got: {next_ids:?}"
        );

        // Validate pass — the validator (Facet B) must agree with what the
        // writer just emitted: zero W_STATE_FOCUS_DRIFT.
        let validate_report =
            mev::validate_brain_state(&dir).expect("validate_brain_state should not error");
        let drift: Vec<_> = validate_report
            .diagnostics
            .iter()
            .filter(|d| d.locator == "W_STATE_FOCUS_DRIFT")
            .collect();
        assert!(
            drift.is_empty(),
            "writer and validator must agree after emit_state --write; got: {drift:#?}"
        );

        // Fixed point — a second emit over the already-derived corpus must be
        // a no-op (byte-identical planning/state.json).
        let snapshot = fs::read(dir.join("planning/state.json")).unwrap();
        let emit_report_2 =
            mev::emit_state(&dir, true, None).expect("second emit_state should not error");
        let wrote: Vec<_> = emit_report_2
            .diagnostics
            .iter()
            .filter(|d| d.locator == "I_EMIT_WROTE")
            .collect();
        assert!(
            wrote.is_empty(),
            "second emit over the already-derived dual-role corpus must be a no-op; got: {wrote:#?}"
        );
        let after_second = fs::read(dir.join("planning/state.json")).unwrap();
        assert_eq!(
            after_second, snapshot,
            "planning/state.json must be byte-identical after the fixed-point pass"
        );

        let _ = fs::remove_dir_all(&dir);
    }
}

// ---------------------------------------------------------------------------
// state-load-error-surfacing task 3 — end-to-end proof (via the public
// `validate_brain_state` entry point) that:
// (a) a serde-schema violation surfaces the underlying serde detail
//     (offending field/type + line:column) inside `E_STATE_MALFORMED_JSON`,
//     rather than the old opaque fixed string (Facet 1); and
// (b) a malformed HQ root no longer cascades into spurious
//     `E_STATE_SCHEMA_BAD_KIND` on correctly-`kind:"brain"` tier sub-brains —
//     exactly one detailed root `E_STATE_MALFORMED_JSON` plus the new
//     `E_STATE_ROOT_LOAD_FAILED` degraded-classification diagnostic, and zero
//     `E_STATE_SCHEMA_BAD_KIND` (Facet 2); with a regression check that a
//     well-formed brain validates with no unexpected diagnostics.
// ---------------------------------------------------------------------------

mod task_state_load_error_surfacing {
    use std::fs;
    use std::path::Path;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let d = mev::testsupport::unique_temp_dir(&format!("mev-state-load-error-{tag}"));
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

    /// `brain.toml` with two tier-container self-entries (`slug == repo_path`,
    /// mirroring the real HQ shape: `core`, `side`) plus one ordinary leaf repo
    /// (`alpha`, nested under `core`) — enough to exercise both the tier
    /// rollup path and the leaf `[[repos]]` path.
    fn write_brain_toml(root: &Path) {
        let toml = r#"[vocab]
layer = ["brain", "engine", "factory", "console", "surface", "infra", "business", "content", "meta"]
status = ["active", "draft", "deprecated", "superseded", "archived"]

[crawl]
skip_dirs = ["target", "node_modules", ".git"]

[[repos]]
slug = "core"
tier = "_root"
repo_path = "core"
status_file = "core/planning/status.md"
cache_doc = "docs/projects/core.md"
heading = "Core"

[[repos]]
slug = "side"
tier = "_root"
repo_path = "side"
status_file = "side/planning/status.md"
cache_doc = "docs/projects/side.md"
heading = "Side"

[[repos]]
slug = "alpha"
tier = "core"
repo_path = "core/alpha"
status_file = "core/alpha/planning/status.md"
cache_doc = "docs/projects/alpha.md"
heading = "Alpha"
"#;
        fs::write(root.join("brain.toml"), toml.as_bytes()).unwrap();
    }

    /// The HQ root `planning/state.json`. When `malformed` is `true`, the
    /// `carryover[].related` field is authored as bare slug strings instead of
    /// the required `Vec<BlockedBy>` edge objects — the exact live data defect
    /// this ticket was written against — which fails to deserialize with a
    /// `serde_json::Error` (not just a JSON-syntax error).
    fn write_root_state(root: &Path, malformed: bool) {
        let related = if malformed {
            serde_json::json!(["some-other-slug"])
        } else {
            serde_json::json!([])
        };
        let state = serde_json::json!({
            "repo": "hq",
            "kind": "brain",
            "updated": "2026-06-29",
            "focus": { "now": [], "next": [], "blocked": [] },
            "repos": [],
            "tiers": [
                { "tier": "core", "rollup": "core/planning/state.json" },
                { "tier": "side", "rollup": "side/planning/state.json" }
            ],
            "carryover": [
                {
                    "slug": "some-carryover",
                    "scope": { "cross_repo": true },
                    "kind": "deferred",
                    "text": "a durable note",
                    "related": related,
                    "created": "2026-06-29"
                }
            ]
        });
        write_json(root, "planning/state.json", &state);
    }

    /// A tier sub-brain's own `planning/state.json` — correctly `kind:"brain"`.
    fn write_tier_brain_state(root: &Path, rel: &str, repo: &str) {
        let state = serde_json::json!({
            "repo": repo,
            "kind": "brain",
            "updated": "2026-06-29",
            "focus": { "now": [], "next": [], "blocked": [] },
            "repos": []
        });
        write_json(root, rel, &state);
    }

    fn write_alpha_state(root: &Path) {
        let state = serde_json::json!({
            "repo": "alpha",
            "kind": "project",
            "updated": "2026-06-29",
            "focus": { "now": [], "next": [], "blocked": [] },
            "tracks": [
                {
                    "title": "Phase 1",
                    "blocks": [{ "id": "AL.1.A", "title": "Start", "status": "open" }]
                }
            ]
        });
        write_json(root, "core/alpha/planning/state.json", &state);
    }

    fn write_fixture(root: &Path, malformed_root: bool) {
        write_brain_toml(root);
        write_root_state(root, malformed_root);
        write_tier_brain_state(root, "core/planning/state.json", "core");
        write_tier_brain_state(root, "side/planning/state.json", "side");
        write_alpha_state(root);
    }

    // -----------------------------------------------------------------------
    // Facet 1 — the serde detail (offending field/type + line:column) must be
    // present in the E_STATE_MALFORMED_JSON message, not just the old generic
    // fixed string.
    // -----------------------------------------------------------------------

    #[test]
    fn malformed_root_error_includes_serde_detail() {
        let dir = temp_dir("serde-detail");
        write_fixture(&dir, true);

        let report =
            mev::validate_brain_state(&dir).expect("validate_brain_state should not error");

        let malformed: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.locator == "E_STATE_MALFORMED_JSON")
            .collect();
        assert_eq!(
            malformed.len(),
            1,
            "expected exactly one E_STATE_MALFORMED_JSON (the root); got: {:#?}",
            report.diagnostics
        );

        let msg = &malformed[0].message;
        assert!(
            msg.contains("line") && msg.contains("column"),
            "E_STATE_MALFORMED_JSON message must carry the serde error's line:column detail, \
             not just the generic string; got: {msg:?}"
        );
        assert!(
            msg.len() > "state.json is not valid JSON or does not match the expected schema".len(),
            "message must be more than the old generic fixed string; got: {msg:?}"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------------
    // Facet 2 — a malformed root must not cascade into spurious
    // E_STATE_SCHEMA_BAD_KIND on correctly kind:"brain" tier sub-brains;
    // exactly one root E_STATE_MALFORMED_JSON + one E_STATE_ROOT_LOAD_FAILED
    // is the whole story. Also proves the well-formed regression case: no
    // unexpected diagnostics at all.
    // -----------------------------------------------------------------------

    #[test]
    fn well_formed_root_validates_clean_malformed_root_does_not_cascade() {
        // Regression: a well-formed brain validates with no unexpected errors.
        let clean_dir = temp_dir("cascade-clean");
        write_fixture(&clean_dir, false);

        let clean_report =
            mev::validate_brain_state(&clean_dir).expect("validate_brain_state should not error");
        let clean_errors: Vec<_> = clean_report
            .diagnostics
            .iter()
            .filter(|d| d.severity == mev::Severity::Error)
            .collect();
        assert!(
            clean_errors.is_empty(),
            "well-formed brain must validate with zero errors; got: {clean_errors:#?}"
        );
        let clean_bad_kind: Vec<_> = clean_report
            .diagnostics
            .iter()
            .filter(|d| d.locator == "E_STATE_SCHEMA_BAD_KIND")
            .collect();
        assert!(
            clean_bad_kind.is_empty(),
            "well-formed brain must have zero E_STATE_SCHEMA_BAD_KIND; got: {clean_bad_kind:#?}"
        );
        let _ = fs::remove_dir_all(&clean_dir);

        // Facet 2: corrupt ONLY the root so it fails to parse.
        let dir = temp_dir("cascade-malformed");
        write_fixture(&dir, true);

        let report =
            mev::validate_brain_state(&dir).expect("validate_brain_state should not error");

        let malformed: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.locator == "E_STATE_MALFORMED_JSON")
            .collect();
        assert_eq!(
            malformed.len(),
            1,
            "exactly one detailed root E_STATE_MALFORMED_JSON expected; got: {:#?}",
            report.diagnostics
        );

        let root_load_failed: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.locator == "E_STATE_ROOT_LOAD_FAILED")
            .collect();
        assert_eq!(
            root_load_failed.len(),
            1,
            "expected exactly one degraded-classification diagnostic; got: {:#?}",
            report.diagnostics
        );

        let bad_kind: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.locator == "E_STATE_SCHEMA_BAD_KIND")
            .collect();
        assert!(
            bad_kind.is_empty(),
            "tier sub-brains must NOT cascade into E_STATE_SCHEMA_BAD_KIND when the root fails \
             to load; got: {bad_kind:#?}"
        );

        let _ = fs::remove_dir_all(&dir);
    }
}

// ===========================================================================
// Attention board (render_attention_section + plan_attention_board)
// ===========================================================================
mod attention_board {
    use mev::brain::config::{AttentionThresholds, BrainConfig, RepoEntry};
    use mev::brain::distill::DistilledEntry;
    use mev::brain::emit::{
        plan_attention_board, render_attention_section, render_attention_section_with_distilled,
    };
    use mev::brain::state::{
        Backlog, BacklogOrigin, Carryover, CarryoverScope, StateFile, StateSource,
        build_state_graph, carryover_kind_from_str,
    };
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn day(s: &str) -> chrono::NaiveDate {
        chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    /// Empty block-side maps for tests that don't exercise `blocks[]`
    /// propagation or `BLOCKING` membership — `rank_carryover` treats an
    /// absent block target as unresolvable/no-priority, matching the
    /// production caller's `effective_priorities`/`global_status_map` when
    /// there are simply no `tracks[]` blocks in the fixture.
    fn no_blocks() -> (HashMap<String, u8>, HashMap<String, Option<String>>) {
        (HashMap::new(), HashMap::new())
    }

    fn carry(slug: &str, kind: &str, created: &str, repo: &str) -> Carryover {
        Carryover {
            slug: slug.to_string(),
            scope: CarryoverScope {
                repo: Some(repo.to_string()),
                tier: None,
                cross_repo: None,
            },
            kind: carryover_kind_from_str(kind),
            text: format!("text for {slug}"),
            related: vec![],
            clears_when: None,
            created: created.to_string(),
            reviewed: None,
            snoozed_until: None,
            ..Default::default()
        }
    }

    fn leaf(repo: &str, carryover: Vec<Carryover>) -> StateFile {
        StateFile {
            epics: Vec::new(),
            repo: repo.to_string(),
            kind: "project".to_string(),
            updated: "2026-07-15".to_string(),
            focus: Default::default(),
            tracks: vec![],
            repos: vec![],
            cross_repo: vec![],
            tiers: vec![],
            note: None,
            backlog: vec![],
            carryover,
            ..Default::default()
        }
    }

    fn brain(repo: &str, backlog: Vec<Backlog>, carryover: Vec<Carryover>) -> StateFile {
        StateFile {
            epics: Vec::new(),
            repo: repo.to_string(),
            kind: "brain".to_string(),
            updated: "2026-07-15".to_string(),
            focus: Default::default(),
            tracks: vec![],
            repos: vec![],
            cross_repo: vec![],
            tiers: vec![],
            note: None,
            backlog,
            carryover,
            ..Default::default()
        }
    }

    fn repo_entry(slug: &str, tier: &str) -> RepoEntry {
        RepoEntry {
            slug: slug.to_string(),
            tier: tier.to_string(),
            repo_path: String::new(),
            status_file: String::new(),
            cache_doc: String::new(),
            heading: String::new(),
            prefix: None,
        }
    }

    fn sentinel_doc() -> String {
        "# status\n\nbefore\n\n<!-- BEGIN generated:attention -->\n<!-- END generated:attention -->\n\nafter\n".to_string()
    }

    #[test]
    fn render_section_lanes_and_snooze_and_capture() {
        let today = day("2026-07-15");
        let cfg = AttentionThresholds::default(); // deferred 5, backlog 7

        // stale deferred (14d), fresh env (1d), snoozed deferred.
        let stale = carry("old", "deferred", "2026-07-01", "mev");
        let fresh = carry("new", "env", "2026-07-14", "mev");
        let mut snoozed = carry("zzz", "deferred", "2026-07-01", "mev");
        snoozed.snoozed_until = Some("2026-07-20".to_string());
        let carryover = vec![
            ("mev".to_string(), &stale),
            ("mev".to_string(), &fresh),
            ("mev".to_string(), &snoozed),
        ];

        // one plain aging backlog + one orphaned capture.
        let mut idea = Backlog {
            slug: "aged-idea".to_string(),
            title: "Aged idea".to_string(),
            repo: "mev".to_string(),
            kind: "research".to_string(),
            status: "idea".to_string(),
            created: Some("2026-07-01".to_string()),
            ..Default::default()
        };
        let mut cap = idea.clone();
        cap.slug = "captured".to_string();
        cap.origin = Some(BacklogOrigin {
            kind: "capture".to_string(),
            notes: Some("core/planning/captured/notes.md".to_string()),
        });
        idea.origin = Some(BacklogOrigin {
            kind: "backlog".to_string(),
            notes: None,
        });
        let backlog = vec![("mev".to_string(), &idea), ("mev".to_string(), &cap)];

        let (block_priorities, block_status) = no_blocks();
        let out = render_attention_section(
            &carryover,
            &backlog,
            today,
            &cfg,
            &block_priorities,
            &block_status,
        );

        // Board membership no longer gates on staleness alone
        // (`MV.ticket.carryover-triage-ranking`): the stale deferred entry
        // (no priority, no blocks -> stale) lands in AGING, and the fresh
        // env entry (no priority, no blocks, not stale) now ALSO appears —
        // in STANDING — where the old staleness-only gate would have hidden
        // it.
        assert!(out.contains("## AGING"));
        assert!(out.contains("## STANDING"));
        assert!(
            out.contains("deferred old"),
            "stale deferred should show in AGING: {out}"
        );
        assert!(
            out.contains("env new"),
            "fresh env must now appear (in STANDING) — membership is not staleness-only: {out}"
        );
        assert!(!out.contains("zzz"), "snoozed must be excluded: {out}");
        assert!(out.contains("## Aging backlog"));
        assert!(out.contains("aged-idea"));
        assert!(out.contains("## Orphaned captures"));
        assert!(out.contains("captured"));
        assert!(out.contains("core/planning/captured/notes.md"));
        // capture must not be double-listed in the plain backlog lane
        let aging_lane = out.split("## Orphaned captures").next().unwrap();
        assert!(
            !aging_lane.contains("captured"),
            "capture leaked into backlog lane"
        );
    }

    #[test]
    fn render_section_empty_lanes_are_none() {
        let (block_priorities, block_status) = no_blocks();
        let out = render_attention_section(
            &[],
            &[],
            day("2026-07-15"),
            &AttentionThresholds::default(),
            &block_priorities,
            &block_status,
        );
        assert!(out.contains("## BLOCKING"));
        assert!(out.contains("## HOT"));
        assert!(out.contains("## AGING"));
        assert!(out.contains("## STANDING"));
        assert!(
            out.contains("## Stale distilled knowledge"),
            "distilled lane heading present even with no distilled entries: {out}"
        );
        assert_eq!(
            out.matches("_none_").count(),
            7,
            "all seven lanes (4 triage + backlog + captures + distilled) empty: {out}"
        );
    }

    #[test]
    fn fresh_p0_appears_where_the_old_staleness_gate_hid_it() {
        // Measured before this block: only 6 of 142 entries were stale, so
        // the old `if let Some(age) = carryover_stale_age(...)` membership
        // gate hid 136 entries — including every P0 filed the same day,
        // which is by construction not yet stale. This is the litmus test
        // for the fix: a fresh (1-day-old), non-stale P0 must now surface,
        // in HOT.
        let today = day("2026-07-15");
        let mut fresh_p0 = carry("urgent", "deferred", "2026-07-14", "mev");
        fresh_p0.priority = Some(0);
        let carryover = vec![("mev".to_string(), &fresh_p0)];

        let (block_priorities, block_status) = no_blocks();
        let out = render_attention_section(
            &carryover,
            &[],
            today,
            &AttentionThresholds::default(),
            &block_priorities,
            &block_status,
        );

        let hot_lane = out
            .split("## HOT")
            .nth(1)
            .expect("HOT lane present")
            .split("## AGING")
            .next()
            .unwrap();
        assert!(
            hot_lane.contains("urgent"),
            "fresh P0 must appear in HOT, not be hidden by a staleness gate: {out}"
        );
        assert!(
            !hot_lane.contains("_none_"),
            "HOT lane must not read empty once a fresh P0 exists: {hot_lane}"
        );
    }

    #[test]
    fn carryover_lane_caps_with_accurate_hidden_count() {
        let today = day("2026-07-15");
        // 25 non-stale, no-priority, no-blocks entries -> all land in
        // STANDING, which is capped at `CARRYOVER_LANE_CAP` (20).
        let entries: Vec<Carryover> = (0..25)
            .map(|i| carry(&format!("standing-{i}"), "deferred", "2026-07-14", "mev"))
            .collect();
        let carryover: Vec<(String, &Carryover)> =
            entries.iter().map(|c| ("mev".to_string(), c)).collect();

        let (block_priorities, block_status) = no_blocks();
        let out = render_attention_section(
            &carryover,
            &[],
            today,
            &AttentionThresholds::default(),
            &block_priorities,
            &block_status,
        );

        let standing_lane = out
            .split("## STANDING")
            .nth(1)
            .expect("STANDING lane present");
        let shown = standing_lane.matches("- [mev] deferred standing-").count();
        assert_eq!(
            shown, 20,
            "must cap at CARRYOVER_LANE_CAP rows: {standing_lane}"
        );
        assert!(
            standing_lane.contains("…and 5 more"),
            "must print the true hidden count: {standing_lane}"
        );
    }

    #[test]
    fn backlog_capture_and_distilled_lanes_stay_byte_identical() {
        // This block only touches carryover triage rendering — the
        // backlog / capture / distilled lanes must render exactly as they
        // did before `AttentionRow` grew structured fields.
        let today = day("2026-07-15");
        let cfg = AttentionThresholds::default();

        let mut idea = Backlog {
            slug: "aged-idea".to_string(),
            title: "Aged idea".to_string(),
            repo: "mev".to_string(),
            kind: "research".to_string(),
            status: "idea".to_string(),
            created: Some("2026-07-01".to_string()),
            ..Default::default()
        };
        let mut cap = idea.clone();
        cap.slug = "captured".to_string();
        cap.origin = Some(BacklogOrigin {
            kind: "capture".to_string(),
            notes: Some("core/planning/captured/notes.md".to_string()),
        });
        idea.origin = Some(BacklogOrigin {
            kind: "backlog".to_string(),
            notes: None,
        });
        let backlog = vec![("mev".to_string(), &idea), ("mev".to_string(), &cap)];

        let (block_priorities, block_status) = no_blocks();
        let out =
            render_attention_section(&[], &backlog, today, &cfg, &block_priorities, &block_status);

        let aging_backlog = out
            .split("## Aging backlog")
            .nth(1)
            .unwrap()
            .split("## Orphaned captures")
            .next()
            .unwrap();
        assert!(
            aging_backlog.contains("- [mev] aged-idea (idea) — Aged idea — 14d"),
            "backlog row format must be unchanged: {aging_backlog}"
        );

        let captures = out.split("## Orphaned captures").nth(1).unwrap();
        assert!(
            captures.contains(
                "- [mev] captured — Aged idea — notes: core/planning/captured/notes.md — 14d"
            ),
            "capture row format must be unchanged: {captures}"
        );
    }

    #[test]
    fn plan_is_tier_scoped_across_hq_and_tiers() {
        let tmp = tempfile::tempdir().unwrap();
        // status.md for hq, core, side.
        for rel in ["planning", "core/planning", "side/planning"] {
            let dir = tmp.path().join(rel);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("status.md"), sentinel_doc()).unwrap();
        }

        let bastion = leaf(
            "bastion",
            vec![carry("cortex-rename", "deferred", "2026-07-01", "bastion")],
        );
        let amistad = leaf(
            "amistad",
            vec![carry("amistad-thing", "deferred", "2026-07-01", "amistad")],
        );

        // HQ backlog: a capture whose repo (bastion) is in the core tier.
        let cap = Backlog {
            slug: "tailscale-db".to_string(),
            title: "Tailscale DB".to_string(),
            repo: "bastion".to_string(),
            kind: "research".to_string(),
            status: "idea".to_string(),
            created: Some("2026-07-01".to_string()),
            origin: Some(BacklogOrigin {
                kind: "capture".to_string(),
                notes: Some("p/notes.md".to_string()),
            }),
            ..Default::default()
        };
        let hq = brain("hq", vec![cap], vec![]);
        let core = brain("core", vec![], vec![]);
        let side = brain("side", vec![], vec![]);

        let src = |slug: &str, abs: PathBuf, kind: &'static str| StateSource {
            repo_slug: slug.to_string(),
            abs_path: abs,
            expected_kind: kind,
        };
        let files = vec![
            (
                src(
                    "bastion",
                    PathBuf::from("/fake/bastion/planning/state.json"),
                    "project",
                ),
                bastion,
            ),
            (
                src(
                    "amistad",
                    PathBuf::from("/fake/amistad/planning/state.json"),
                    "project",
                ),
                amistad,
            ),
            (
                src("hq", tmp.path().join("planning/state.json"), "brain"),
                hq,
            ),
            (
                src("core", tmp.path().join("core/planning/state.json"), "brain"),
                core,
            ),
            (
                src("side", tmp.path().join("side/planning/state.json"), "brain"),
                side,
            ),
        ];

        let config = BrainConfig {
            repos: vec![repo_entry("bastion", "core"), repo_entry("amistad", "side")],
            ..Default::default()
        };

        let graph = build_state_graph(&files);
        let plan = plan_attention_board(&files, &graph, &config, day("2026-07-15"));

        let by_path = |needle: &str| -> String {
            plan.actions
                .iter()
                .find(|a| a.path.to_string_lossy().contains(needle))
                .unwrap_or_else(|| {
                    panic!(
                        "no action for {needle}; actions: {:?}",
                        plan.actions
                            .iter()
                            .map(|a| a.path.clone())
                            .collect::<Vec<_>>()
                    )
                })
                .new_content
                .clone()
        };

        let hq_doc = by_path("/planning/status.md");
        let core_doc = by_path("/core/planning/status.md");
        let side_doc = by_path("/side/planning/status.md");

        // HQ sees everything.
        assert!(hq_doc.contains("cortex-rename"));
        assert!(hq_doc.contains("amistad-thing"));
        assert!(hq_doc.contains("tailscale-db"));

        // Core sees only core-tier items.
        assert!(
            core_doc.contains("cortex-rename"),
            "core board missing its carryover"
        );
        assert!(
            core_doc.contains("tailscale-db"),
            "core board missing its capture"
        );
        assert!(
            !core_doc.contains("amistad-thing"),
            "side item leaked into core board"
        );

        // Side sees only side-tier items.
        assert!(side_doc.contains("amistad-thing"));
        assert!(!side_doc.contains("cortex-rename"));
        assert!(!side_doc.contains("tailscale-db"));

        // Idempotency: apply core content back, re-plan, expect no core action.
        std::fs::write(tmp.path().join("core/planning/status.md"), &core_doc).unwrap();
        let graph = build_state_graph(&files);
        let plan2 = plan_attention_board(&files, &graph, &config, day("2026-07-15"));
        assert!(
            !plan2.actions.iter().any(|a| a
                .path
                .to_string_lossy()
                .contains("/core/planning/status.md")),
            "second pass should be idempotent for core"
        );
    }

    #[test]
    fn plan_missing_sentinel_warns_and_skips() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("planning");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("status.md"), "# status\n\nno sentinels here\n").unwrap();

        let hq = brain(
            "hq",
            vec![],
            vec![carry("x", "deferred", "2026-07-01", "hq")],
        );
        let files = vec![(
            StateSource {
                repo_slug: "hq".to_string(),
                abs_path: tmp.path().join("planning/state.json"),
                expected_kind: "brain",
            },
            hq,
        )];
        let config = BrainConfig {
            repos: vec![repo_entry("bastion", "core")],
            ..Default::default()
        };

        let graph = build_state_graph(&files);
        let plan = plan_attention_board(&files, &graph, &config, day("2026-07-15"));
        assert!(plan.actions.is_empty(), "no write without sentinels");
        assert!(
            plan.diagnostics
                .iter()
                .any(|d| d.locator == "W_EMIT_NO_SENTINEL"),
            "missing sentinel must warn"
        );
    }

    fn distilled_entry(claim: &str, freshness: &str) -> DistilledEntry {
        DistilledEntry {
            claim: claim.to_string(),
            date: Some(day(freshness)),
            freshness: Some(day(freshness)),
            line: 1,
        }
    }

    #[test]
    fn render_section_with_distilled_shows_stale_and_hides_fresh() {
        let today = day("2026-08-01");
        let cfg = AttentionThresholds::default(); // knowledge_days = 45

        let stale = distilled_entry("Stale claim.", "2026-06-01"); // 61d old
        let fresh = distilled_entry("Fresh claim.", "2026-07-30"); // 2d old
        let distilled = vec![
            ("mev".to_string(), "knowledge", &stale),
            ("mev".to_string(), "memory", &fresh),
        ];

        let (block_priorities, block_status) = no_blocks();
        let out = render_attention_section_with_distilled(
            &[],
            &[],
            &distilled,
            today,
            &cfg,
            &block_priorities,
            &block_status,
        );

        assert!(out.contains("## Stale distilled knowledge"));
        assert!(
            out.contains("Stale claim."),
            "stale entry should show: {out}"
        );
        assert!(
            !out.contains("Fresh claim."),
            "fresh entry must be excluded: {out}"
        );
    }

    #[test]
    fn render_section_distilled_caps_at_ten_with_hidden_count() {
        let today = day("2026-08-01");
        let cfg = AttentionThresholds::default();

        let entries: Vec<DistilledEntry> = (0..13)
            .map(|i| distilled_entry(&format!("Claim {i}."), "2026-06-01"))
            .collect();
        let distilled: Vec<(String, &str, &DistilledEntry)> = entries
            .iter()
            .map(|e| ("mev".to_string(), "knowledge", e))
            .collect();

        let (block_priorities, block_status) = no_blocks();
        let out = render_attention_section_with_distilled(
            &[],
            &[],
            &distilled,
            today,
            &cfg,
            &block_priorities,
            &block_status,
        );

        let lane = out
            .split("## Stale distilled knowledge")
            .nth(1)
            .expect("lane present");
        let row_count = lane.matches("Claim ").count();
        assert_eq!(row_count, 10, "must cap at 10 rows: {lane}");
        assert!(
            lane.contains("…and 3 more"),
            "must print the true hidden count: {lane}"
        );
    }

    #[test]
    fn plan_distilled_lane_is_tier_scoped_and_agrees_with_single_predicate() {
        let tmp = tempfile::tempdir().unwrap();
        for rel in ["planning", "core/planning", "side/planning"] {
            let dir = tmp.path().join(rel);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("status.md"), sentinel_doc()).unwrap();
        }

        // "bastion" (core tier) and "amistad" (side tier) each carry a knowledge.md
        // beside their state.json, as brain-scoped tier files would in the real corpus.
        let bastion_dir = tmp.path().join("core/planning");
        std::fs::write(
            bastion_dir.join("knowledge.md"),
            "- **Bastion stale claim.** Body.\n  \
             source: log.md · date: 2026-06-01 · supersedes: — · freshness: 2026-06-01\n",
        )
        .unwrap();

        let amistad_dir = tmp.path().join("side/planning");
        std::fs::write(
            amistad_dir.join("knowledge.md"),
            "- **Amistad stale claim.** Body.\n  \
             source: log.md · date: 2026-06-01 · supersedes: — · freshness: 2026-06-01\n",
        )
        .unwrap();

        let hq = brain("hq", vec![], vec![]);
        let core = brain("core", vec![], vec![]);
        let side = brain("side", vec![], vec![]);

        let src = |slug: &str, abs: PathBuf, kind: &'static str| StateSource {
            repo_slug: slug.to_string(),
            abs_path: abs,
            expected_kind: kind,
        };
        let files = vec![
            (
                src("hq", tmp.path().join("planning/state.json"), "brain"),
                hq,
            ),
            (
                src("core", tmp.path().join("core/planning/state.json"), "brain"),
                core,
            ),
            (
                src("side", tmp.path().join("side/planning/state.json"), "brain"),
                side,
            ),
        ];

        let config = BrainConfig {
            repos: vec![repo_entry("core", "core"), repo_entry("side", "side")],
            ..Default::default()
        };

        let today = day("2026-08-01"); // 61d past the 2026-06-01 stamp, > 45d threshold

        let graph = build_state_graph(&files);
        let plan = plan_attention_board(&files, &graph, &config, today);

        let by_path = |needle: &str| -> String {
            plan.actions
                .iter()
                .find(|a| a.path.to_string_lossy().contains(needle))
                .unwrap_or_else(|| panic!("no action for {needle}"))
                .new_content
                .clone()
        };

        let hq_doc = by_path("/planning/status.md");
        let core_doc = by_path("/core/planning/status.md");
        let side_doc = by_path("/side/planning/status.md");

        // HQ unions both.
        assert!(hq_doc.contains("Bastion stale claim."));
        assert!(hq_doc.contains("Amistad stale claim."));

        // Core sees only its own.
        assert!(core_doc.contains("Bastion stale claim."));
        assert!(
            !core_doc.contains("Amistad stale claim."),
            "side entry leaked into core board: {core_doc}"
        );

        // Side sees only its own.
        assert!(side_doc.contains("Amistad stale claim."));
        assert!(
            !side_doc.contains("Bastion stale claim."),
            "core entry leaked into side board: {side_doc}"
        );

        // Single-predicate proof: the same entry the board renders also fires
        // W_DISTILL_STALE via the shared distill_stale_age predicate.
        use mev::brain::distill::{check_distill_staleness, parse_distilled};
        let contents = std::fs::read_to_string(bastion_dir.join("knowledge.md")).unwrap();
        let parsed = parse_distilled(&contents);
        assert_eq!(parsed.len(), 1);
        let diags = check_distill_staleness(&bastion_dir, today, &config.attention);
        assert_eq!(diags.len(), 1, "warning must fire for the same entry");
        assert_eq!(diags[0].locator, "W_DISTILL_STALE");
    }

    #[test]
    fn plan_distilled_missing_file_is_silent_skip() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("planning");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("status.md"), sentinel_doc()).unwrap();
        // No knowledge.md / memory.md written at all.

        let hq = brain("hq", vec![], vec![]);
        let files = vec![(
            StateSource {
                repo_slug: "hq".to_string(),
                abs_path: tmp.path().join("planning/state.json"),
                expected_kind: "brain",
            },
            hq,
        )];
        let config = BrainConfig::default();

        let graph = build_state_graph(&files);
        let plan = plan_attention_board(&files, &graph, &config, day("2026-08-01"));
        assert!(
            !plan
                .diagnostics
                .iter()
                .any(|d| d.locator == "W_EMIT_NO_SENTINEL" && d.message.contains("knowledge")),
            "missing knowledge/memory must not warn: {:?}",
            plan.diagnostics
        );
        let hq_doc = &plan
            .actions
            .iter()
            .find(|a| a.path.to_string_lossy().contains("/planning/status.md"))
            .unwrap()
            .new_content;
        assert!(hq_doc.contains("## Stale distilled knowledge"));
        assert!(
            hq_doc
                .split("## Stale distilled knowledge")
                .nth(1)
                .unwrap()
                .trim_start()
                .starts_with("_none_"),
            "no distilled files -> empty lane: {hq_doc}"
        );
    }

    /// `MV.ticket.attention-queue-delivery` task 6 — the gate on task 1's
    /// row/render split: a fixture corpus covering all four carryover triage
    /// lanes plus backlog / captures / distilled, including an empty
    /// (`_none_`) lane and both cap-with-hidden-count lines (`CARRYOVER_LANE_CAP`
    /// for STANDING, `DISTILL_LANE_CAP` for the distilled lane), must render
    /// BYTE-IDENTICAL markdown to what this repo produced before
    /// `collect_attention_rows` was split out of
    /// `render_attention_section_with_distilled`. The row-collection refactor
    /// must not move a single character of what the fleet's `status.md` files
    /// carry.
    #[test]
    fn render_attention_section_pinned_byte_identical_all_lanes() {
        let today = day("2026-07-15");
        let cfg = AttentionThresholds::default();

        // HOT: one fresh P0 (not yet stale, but priority routes it to HOT).
        let mut hot = carry("urgent", "deferred", "2026-07-14", "mev");
        hot.priority = Some(0);
        // AGING: one stale, no-priority entry.
        let aging = carry("old", "deferred", "2026-07-01", "mev");
        // STANDING: 25 non-stale, no-priority entries -> caps at
        // CARRYOVER_LANE_CAP (20), "…and 5 more". BLOCKING is left empty
        // (_none_) — nothing here sets `blocks[]`.
        let standing_entries: Vec<Carryover> = (0..25)
            .map(|i| carry(&format!("standing-{i}"), "deferred", "2026-07-14", "mev"))
            .collect();
        let mut carryover: Vec<(String, &Carryover)> =
            vec![("mev".to_string(), &hot), ("mev".to_string(), &aging)];
        carryover.extend(standing_entries.iter().map(|c| ("mev".to_string(), c)));

        // Aging backlog: one plain row. Orphaned captures: left empty (_none_).
        let mut idea = Backlog {
            slug: "aged-idea".to_string(),
            title: "Aged idea".to_string(),
            repo: "mev".to_string(),
            kind: "research".to_string(),
            status: "idea".to_string(),
            created: Some("2026-07-01".to_string()),
            ..Default::default()
        };
        idea.origin = Some(BacklogOrigin {
            kind: "backlog".to_string(),
            notes: None,
        });
        let backlog = vec![("mev".to_string(), &idea)];

        // Stale distilled knowledge: 13 entries -> caps at DISTILL_LANE_CAP
        // (10), "…and 3 more".
        let distilled_entries: Vec<DistilledEntry> = (0..13)
            .map(|i| distilled_entry(&format!("Claim {i}."), "2026-05-01"))
            .collect();
        let distilled: Vec<(String, &str, &DistilledEntry)> = distilled_entries
            .iter()
            .map(|e| ("mev".to_string(), "knowledge", e))
            .collect();

        let (block_priorities, block_status) = no_blocks();
        let out = render_attention_section_with_distilled(
            &carryover,
            &backlog,
            &distilled,
            today,
            &cfg,
            &block_priorities,
            &block_status,
        );

        let expected = "## BLOCKING\n\
_none_\n\n\
## HOT\n\
- [mev] deferred urgent — text for urgent [P0] (Hot) — 1d\n\n\
## AGING\n\
- [mev] deferred old — text for old (Aging) — 14d\n\n\
## STANDING\n\
- [mev] deferred standing-0 — text for standing-0 (Standing) — 1d\n\
- [mev] deferred standing-1 — text for standing-1 (Standing) — 1d\n\
- [mev] deferred standing-10 — text for standing-10 (Standing) — 1d\n\
- [mev] deferred standing-11 — text for standing-11 (Standing) — 1d\n\
- [mev] deferred standing-12 — text for standing-12 (Standing) — 1d\n\
- [mev] deferred standing-13 — text for standing-13 (Standing) — 1d\n\
- [mev] deferred standing-14 — text for standing-14 (Standing) — 1d\n\
- [mev] deferred standing-15 — text for standing-15 (Standing) — 1d\n\
- [mev] deferred standing-16 — text for standing-16 (Standing) — 1d\n\
- [mev] deferred standing-17 — text for standing-17 (Standing) — 1d\n\
- [mev] deferred standing-18 — text for standing-18 (Standing) — 1d\n\
- [mev] deferred standing-19 — text for standing-19 (Standing) — 1d\n\
- [mev] deferred standing-2 — text for standing-2 (Standing) — 1d\n\
- [mev] deferred standing-20 — text for standing-20 (Standing) — 1d\n\
- [mev] deferred standing-21 — text for standing-21 (Standing) — 1d\n\
- [mev] deferred standing-22 — text for standing-22 (Standing) — 1d\n\
- [mev] deferred standing-23 — text for standing-23 (Standing) — 1d\n\
- [mev] deferred standing-24 — text for standing-24 (Standing) — 1d\n\
- [mev] deferred standing-3 — text for standing-3 (Standing) — 1d\n\
- [mev] deferred standing-4 — text for standing-4 (Standing) — 1d\n\
- …and 5 more\n\n\
## Aging backlog\n\
- [mev] aged-idea (idea) — Aged idea — 14d\n\n\
## Orphaned captures\n\
_none_\n\n\
## Stale distilled knowledge\n\
- [mev] knowledge — Claim 0. — 75d\n\
- [mev] knowledge — Claim 1. — 75d\n\
- [mev] knowledge — Claim 2. — 75d\n\
- [mev] knowledge — Claim 3. — 75d\n\
- [mev] knowledge — Claim 4. — 75d\n\
- [mev] knowledge — Claim 5. — 75d\n\
- [mev] knowledge — Claim 6. — 75d\n\
- [mev] knowledge — Claim 7. — 75d\n\
- [mev] knowledge — Claim 8. — 75d\n\
- [mev] knowledge — Claim 9. — 75d\n\
- …and 3 more";

        assert_eq!(
            out, expected,
            "Attention board markdown must be byte-identical to the pre-refactor \
             (task 1) rendering — the row/render split must not move a single \
             character of rendered output:\n--- actual ---\n{out}\n--- expected ---\n{expected}"
        );
    }
}

// ---------------------------------------------------------------------------
// epic_members tests
// ---------------------------------------------------------------------------

/// Build a `TrackBlock` that claims one or more epics.
fn block_in_epics(id: &str, wave: Option<i64>, epics: &[&str]) -> TrackBlock {
    let mut b = block(id, &format!("{id} title"), Some("open"), wave);
    b.epics = epics.iter().map(|s| s.to_string()).collect();
    b
}

#[test]
fn epic_members_returns_only_members_in_cross_repo_wave_order() {
    // Waves interleave across repos: the epic's sequence must follow wave order
    // globally, not repo-by-repo.
    let bastion = (
        make_src("bastion"),
        make_leaf(
            "bastion",
            vec![Track {
                title: "P".to_string(),
                blocks: vec![
                    block_in_epics("BA.1", Some(1), &["bastion-os"]),
                    block_in_epics("BA.2", Some(3), &["bastion-os", "bastion-web"]),
                    block("BA.3", "untagged", Some("open"), Some(2)),
                ],
                ..Default::default()
            }],
        ),
    );
    let web = (
        make_src("bastion-web"),
        make_leaf(
            "bastion-web",
            vec![Track {
                title: "P".to_string(),
                blocks: vec![block_in_epics("BW.1", Some(2), &["bastion-web"])],
                ..Default::default()
            }],
        ),
    );
    let files = vec![bastion, web];

    let os: Vec<String> = mev::brain::emit::epic_members(&empty_graph(), &files, "bastion-os")
        .into_iter()
        .map(|(repo, b)| format!("{repo}:{}", b.id))
        .collect();
    assert_eq!(
        os,
        vec!["bastion:BA.1", "bastion:BA.2"],
        "untagged BA.3 must not appear even though its wave sits between them"
    );

    let web_members: Vec<String> =
        mev::brain::emit::epic_members(&empty_graph(), &files, "bastion-web")
            .into_iter()
            .map(|(repo, b)| format!("{repo}:{}", b.id))
            .collect();
    assert_eq!(
        web_members,
        vec!["bastion-web:BW.1", "bastion:BA.2"],
        "wave 2 (other repo) must sort before wave 3 (same repo); BA.2 is a \
         member of both epics"
    );
}

#[test]
fn epic_members_follows_depends_on_over_mismatched_cross_repo_wave_scales() {
    // Per-repo wave scales are not comparable (bastion uses small numbers,
    // this corpus stands in for a repo whose scale runs much higher, e.g.
    // the real bastion-web 10-60 range). BW.1 (wave 1) depends on BA.1
    // (wave 250) — raw wave order would wrongly sort the dependent BW.1
    // first; the topological sort must still put BA.1 first.
    let bastion = (
        make_src("bastion"),
        make_leaf(
            "bastion",
            vec![Track {
                title: "P".to_string(),
                blocks: vec![block_in_epics("BA.1", Some(250), &["ep"])],
                ..Default::default()
            }],
        ),
    );
    let web = (
        make_src("bastion-web"),
        make_leaf(
            "bastion-web",
            vec![Track {
                title: "P".to_string(),
                blocks: vec![{
                    let mut b = block_with_dep(
                        "BW.1",
                        "BW.1 title",
                        Some("open"),
                        Some(1),
                        "bastion",
                        "BA.1",
                    );
                    b.epics = vec!["ep".to_string()];
                    b
                }],
                ..Default::default()
            }],
        ),
    );
    let files = vec![bastion, web];
    let graph = mev::brain::state::build_state_graph(&files);

    let members: Vec<String> = mev::brain::emit::epic_members(&graph, &files, "ep")
        .into_iter()
        .map(|(repo, b)| format!("{repo}:{}", b.id))
        .collect();

    assert_eq!(
        members,
        vec!["bastion:BA.1", "bastion-web:BW.1"],
        "BA.1 must precede BW.1 despite its higher raw wave number, because BW.1 depends on it"
    );
}

#[test]
fn epic_members_is_empty_for_an_unclaimed_slug() {
    let files = vec![(
        make_src("mev"),
        make_leaf(
            "mev",
            vec![Track {
                title: "P".to_string(),
                blocks: vec![block("MV.1", "untagged", Some("open"), Some(1))],
                ..Default::default()
            }],
        ),
    )];
    assert!(mev::brain::emit::epic_members(&empty_graph(), &files, "ghost").is_empty());
}

#[test]
fn epic_members_cycle_terminates_without_hang_or_panic() {
    // A depends_on cycle (A -> B -> A) must not be assumed away here — the
    // corpus-level cycle check (MV.3.P2) may not have run yet. The DFS guard
    // should just terminate deterministically instead of hanging or panicking.
    let file = make_leaf(
        "repo",
        vec![Track {
            title: "P".to_string(),
            blocks: vec![
                {
                    let mut b = block_with_dep("A", "A", Some("open"), Some(1), "repo", "B");
                    b.epics = vec!["ep".to_string()];
                    b
                },
                {
                    let mut b = block_with_dep("B", "B", Some("open"), Some(2), "repo", "A");
                    b.epics = vec!["ep".to_string()];
                    b
                },
            ],
            ..Default::default()
        }],
    );
    let files = vec![(make_src("repo"), file)];
    let graph = mev::brain::state::build_state_graph(&files);

    let members: Vec<String> = mev::brain::emit::epic_members(&graph, &files, "ep")
        .into_iter()
        .map(|(repo, b)| format!("{repo}:{}", b.id))
        .collect();

    assert_eq!(
        members.len(),
        2,
        "both cyclic members must still appear exactly once"
    );
}

// ---------------------------------------------------------------------------
// topo_order tests
// ---------------------------------------------------------------------------

#[test]
fn topo_order_dependency_edge_forces_order() {
    // B depends on A; even though A has a later wave than B, A must precede
    // B in the topological order.
    let file = make_leaf(
        "repo",
        vec![Track {
            title: "P".to_string(),
            blocks: vec![
                block_with_dep("B", "B", Some("open"), Some(1), "repo", "A"),
                block("A", "A", Some("open"), Some(5)),
            ],
            ..Default::default()
        }],
    );
    let files = vec![(make_src("repo"), file)];
    let graph = mev::brain::state::build_state_graph(&files);

    let order = mev::brain::emit::topo_order(&graph, &files);
    let a_pos = order.iter().position(|k| k == "repo:A").unwrap();
    let b_pos = order.iter().position(|k| k == "repo:B").unwrap();
    assert!(
        a_pos < b_pos,
        "A must precede B despite its later wave, because B depends on it: {order:?}"
    );
}

#[test]
fn topo_order_unconstrained_pairs_keep_wave_order() {
    // No dependency edge between X and Y: topo_order must fall back to
    // wave_order's stable wave-then-iteration order.
    let file = make_leaf(
        "repo",
        vec![Track {
            title: "P".to_string(),
            blocks: vec![
                block("Y", "Y", Some("open"), Some(2)),
                block("X", "X", Some("open"), Some(1)),
            ],
            ..Default::default()
        }],
    );
    let files = vec![(make_src("repo"), file)];
    let graph = empty_graph();

    let order = mev::brain::emit::topo_order(&graph, &files);
    assert_eq!(
        order,
        wave_order(&graph, &files),
        "unconstrained pairs must match wave_order exactly"
    );
}

#[test]
fn topo_order_cycle_terminates_without_hang_or_panic() {
    // A depends_on cycle (A -> B -> A) must not hang or panic; the on_stack
    // short-circuit must terminate the DFS deterministically.
    let file = make_leaf(
        "repo",
        vec![Track {
            title: "P".to_string(),
            blocks: vec![
                block_with_dep("A", "A", Some("open"), Some(1), "repo", "B"),
                block_with_dep("B", "B", Some("open"), Some(2), "repo", "A"),
            ],
            ..Default::default()
        }],
    );
    let files = vec![(make_src("repo"), file)];
    let graph = mev::brain::state::build_state_graph(&files);

    let order = mev::brain::emit::topo_order(&graph, &files);
    assert_eq!(
        order.len(),
        2,
        "both cyclic nodes must still appear exactly once: {order:?}"
    );
}

#[test]
fn topo_order_external_deps_do_not_constrain_order() {
    // An External dep has no target node in this corpus and must not
    // participate in ordering — B must fall back to wave order relative to
    // any other block, not be forced after some phantom target.
    let mut b = block("B", "B", Some("open"), Some(1));
    b.depends_on.push(BlockedBy::External(ExternalDep {
        what: "deploy-gate".to_string(),
    }));
    let file = make_leaf(
        "repo",
        vec![Track {
            title: "P".to_string(),
            blocks: vec![block("A", "A", Some("open"), Some(2)), b],
            ..Default::default()
        }],
    );
    let files = vec![(make_src("repo"), file)];
    let graph = empty_graph();

    let order = mev::brain::emit::topo_order(&graph, &files);
    assert_eq!(
        order,
        wave_order(&graph, &files),
        "External dep must not constrain order; must match plain wave_order"
    );
}

// ---------------------------------------------------------------------------
// Epic board + sequence table tests
// ---------------------------------------------------------------------------

mod epic_emit {
    use mev::brain::config::{BrainConfig, RepoEntry};
    use mev::brain::emit::{
        epic_members, markers, plan_epic_boards, plan_epic_sequences, render_epic_sequence_table,
    };
    use mev::brain::state::{
        BlockDep, BlockedBy, Epic, Focus, StateFile, StateSource, Track, TrackBlock,
        build_state_graph,
    };

    fn config() -> BrainConfig {
        BrainConfig {
            attention: Default::default(),
            history: Default::default(),
            repos: [
                ("hq", "_root"),
                ("core", "_root"),
                ("bastion", "core"),
                ("bastion-web", "core"),
                ("amistad", "side"),
            ]
            .iter()
            .map(|(slug, tier)| RepoEntry {
                slug: slug.to_string(),
                tier: tier.to_string(),
                repo_path: String::new(),
                status_file: String::new(),
                cache_doc: String::new(),
                heading: String::new(),
                prefix: None,
            })
            .collect(),
            ..BrainConfig::default()
        }
    }

    fn tb(id: &str, status: &str, wave: i64, epics: &[&str], deps: Vec<BlockedBy>) -> TrackBlock {
        TrackBlock {
            epics: epics.iter().map(|s| s.to_string()).collect(),
            due: None,
            priority: None,
            sdlc_workflow: None,
            model: None,
            id: id.to_string(),
            title: format!("{id} title"),
            status: Some(status.to_string()),
            depends_on: deps,
            wave: Some(wave),
            origin: None,
            note: None,
            description: None,
            ..Default::default()
        }
    }

    fn leaf(
        dir: &std::path::Path,
        repo: &str,
        blocks: Vec<TrackBlock>,
    ) -> (StateSource, StateFile) {
        (
            StateSource {
                repo_slug: repo.to_string(),
                abs_path: dir.join(repo).join("planning/state.json"),
                expected_kind: "project",
            },
            StateFile {
                epics: Vec::new(),
                repo: repo.to_string(),
                kind: "project".to_string(),
                updated: "2026-07-24".to_string(),
                focus: Focus::default(),
                tracks: vec![Track {
                    title: "P".to_string(),
                    blocks,
                    ..Default::default()
                }],
                repos: vec![],
                cross_repo: vec![],
                tiers: vec![],
                note: None,
                backlog: vec![],
                carryover: vec![],
                ..Default::default()
            },
        )
    }

    fn epic(slug: &str, title: &str, status: &str, plan: Option<&str>) -> Epic {
        Epic {
            slug: slug.to_string(),
            title: title.to_string(),
            description: None,
            status: Some(status.to_string()),
            weight: None,
            plan: plan.map(|p| p.to_string()),
            repos: vec![],
            ..Default::default()
        }
    }

    fn hq(dir: &std::path::Path, repo: &str, epics: Vec<Epic>) -> (StateSource, StateFile) {
        (
            StateSource {
                repo_slug: repo.to_string(),
                abs_path: dir.join(repo).join("planning/state.json"),
                expected_kind: "brain",
            },
            StateFile {
                epics,
                repo: repo.to_string(),
                kind: "brain".to_string(),
                updated: "2026-07-24".to_string(),
                focus: Focus::default(),
                tracks: vec![],
                repos: vec![],
                cross_repo: vec![],
                tiers: vec![],
                note: None,
                backlog: vec![],
                carryover: vec![],
                ..Default::default()
            },
        )
    }

    fn doc_with(marker: &str) -> String {
        format!(
            "---\ntype: ProjectStatus\ntitle: T\ndescription: D\n---\n\n\
             # Status\n\nBefore.\n\n\
             <!-- BEGIN generated:{marker} -->\n<!-- END generated:{marker} -->\n\n\
             After.\n"
        )
    }

    /// Corpus: bastion-web's BW.1 (bastion-web epic) waits on bastion's BA.7
    /// (bastion-os epic); amistad's AM.1 is unrelated side-tier work.
    fn corpus(dir: &std::path::Path) -> Vec<(StateSource, StateFile)> {
        vec![
            leaf(
                dir,
                "bastion",
                vec![
                    tb("BA.6", "closed", 1, &["bastion-os"], vec![]),
                    tb("BA.7", "in_progress", 2, &["bastion-os"], vec![]),
                ],
            ),
            leaf(
                dir,
                "bastion-web",
                vec![tb(
                    "BW.1",
                    "open",
                    3,
                    &["bastion-web"],
                    vec![BlockedBy::Block(BlockDep {
                        repo: "bastion".to_string(),
                        id: "BA.7".to_string(),
                        what: None,
                    })],
                )],
            ),
            leaf(dir, "amistad", vec![tb("AM.1", "open", 1, &[], vec![])]),
        ]
    }

    // -- render_epic_sequence_table ------------------------------------------

    #[test]
    fn sequence_table_orders_across_repos_and_derives_blocked_status() {
        let tmp = tempfile::tempdir().unwrap();
        let files = corpus(tmp.path());
        let graph = build_state_graph(&files);
        let status = mev::brain::emit::global_status_map(&files);

        let table =
            render_epic_sequence_table(&epic_members(&graph, &files, "bastion-os"), &status);
        assert!(table.contains("| Wave | Repo | Block | Title | Status | Depends on |"));
        assert!(table.contains("| 1 | bastion | BA.6 |"), "got:\n{table}");
        assert!(table.contains("| 2 | bastion | BA.7 |"), "got:\n{table}");
        assert!(
            !table.contains("AM.1") && !table.contains("BW.1"),
            "only bastion-os members belong in this table:\n{table}"
        );

        // BW.1 is open and depends on the still-open BA.7 → derived `blocked`.
        let web = render_epic_sequence_table(&epic_members(&graph, &files, "bastion-web"), &status);
        assert!(
            web.contains("| 3 | bastion-web | BW.1 | BW.1 title | blocked | bastion:BA.7 |"),
            "got:\n{web}"
        );
    }

    #[test]
    fn sequence_table_renders_a_placeholder_for_an_empty_epic() {
        let table = render_epic_sequence_table(&[], &Default::default());
        assert!(table.contains("_no member blocks_"), "got:\n{table}");
    }

    // -- plan_epic_boards ----------------------------------------------------

    #[test]
    fn epic_board_splices_progress_lanes_and_relationships_and_is_a_fixed_point() {
        let tmp = tempfile::tempdir().unwrap();
        let status_path = tmp.path().join("hq/planning/status.md");
        std::fs::create_dir_all(status_path.parent().unwrap()).unwrap();
        std::fs::write(&status_path, doc_with(markers::EPIC_BOARD)).unwrap();

        let mut files = corpus(tmp.path());
        files.push(hq(
            tmp.path(),
            "hq",
            vec![
                epic("bastion-os", "Bastion OS", "active", None),
                epic("bastion-web", "Bastion Web + UI", "active", None),
            ],
        ));
        let graph = build_state_graph(&files);

        let plan = plan_epic_boards(tmp.path(), &files, &graph, &config());
        assert_eq!(plan.actions.len(), 1, "one board write expected");
        let out = &plan.actions[0].new_content;

        assert!(out.contains("### Bastion OS"), "got:\n{out}");
        assert!(out.contains("### Bastion Web + UI"), "got:\n{out}");
        // 1 of 2 bastion-os blocks closed, 1 in progress, 0 open.
        assert!(
            out.contains("**1/2 closed** · 1 in progress · 0 open"),
            "got:\n{out}"
        );
        // BA.7 is in_progress → NOW lane of the Bastion OS board.
        assert!(out.contains("[ENG] bastion:BA.7"), "got:\n{out}");
        // The cross-epic gate is named, attributed to the epic that owns it.
        assert!(
            out.contains("**Waiting on**") && out.contains("- bastion:BA.7 (bastion-os)"),
            "got:\n{out}"
        );
        assert!(
            out.contains("**Holding up**") && out.contains("- bastion-web:BW.1 (bastion-web)"),
            "got:\n{out}"
        );
        // Lanes must nest UNDER their epic heading, not outrank it.
        assert!(
            out.contains("#### NOW") && out.contains("#### NEXT") && out.contains("#### BLOCKED"),
            "epic lanes must be h4 beneath the h3 epic heading, got:\n{out}"
        );
        assert!(
            !out.contains("\n## NOW"),
            "an h2 lane would outrank the h3 epic heading and break the outline, got:\n{out}"
        );
        // Unrelated side-tier work never appears.
        assert!(!out.contains("AM.1"), "got:\n{out}");
        // Narrative outside the sentinels survives.
        assert!(
            out.contains("Before.") && out.contains("After."),
            "got:\n{out}"
        );

        // Fixed point: re-planning against the emitted output yields no action.
        std::fs::write(&status_path, out).unwrap();
        let again = plan_epic_boards(tmp.path(), &files, &graph, &config());
        assert!(
            again.actions.is_empty(),
            "re-running against its own output must be a no-op, got: {:?}",
            again.actions.iter().map(|a| &a.note).collect::<Vec<_>>()
        );
    }

    #[test]
    fn epic_board_omits_non_active_epics() {
        let tmp = tempfile::tempdir().unwrap();
        let status_path = tmp.path().join("hq/planning/status.md");
        std::fs::create_dir_all(status_path.parent().unwrap()).unwrap();
        std::fs::write(&status_path, doc_with(markers::EPIC_BOARD)).unwrap();

        let mut files = corpus(tmp.path());
        files.push(hq(
            tmp.path(),
            "hq",
            vec![
                epic("bastion-os", "Bastion OS", "complete", None),
                epic("bastion-web", "Bastion Web + UI", "active", None),
            ],
        ));
        let graph = build_state_graph(&files);

        let plan = plan_epic_boards(tmp.path(), &files, &graph, &config());
        let out = &plan.actions[0].new_content;
        assert!(!out.contains("### Bastion OS"), "got:\n{out}");
        assert!(out.contains("### Bastion Web + UI"), "got:\n{out}");
    }

    #[test]
    fn epic_board_renders_a_focused_epic_in_full() {
        // `focused` is active-equivalent, so it must render as a full board — not
        // collapsed like `paused`, and certainly not omitted like `complete`.
        // The current-priority epic vanishing from the board is the worst case.
        let tmp = tempfile::tempdir().unwrap();
        let status_path = tmp.path().join("hq/planning/status.md");
        std::fs::create_dir_all(status_path.parent().unwrap()).unwrap();
        std::fs::write(&status_path, doc_with(markers::EPIC_BOARD)).unwrap();

        let mut files = corpus(tmp.path());
        files.push(hq(
            tmp.path(),
            "hq",
            vec![
                epic("bastion-os", "Bastion OS", "focused", None),
                epic("bastion-web", "Bastion Web + UI", "active", None),
            ],
        ));
        let graph = build_state_graph(&files);

        let plan = plan_epic_boards(tmp.path(), &files, &graph, &config());
        let out = &plan.actions[0].new_content;
        assert!(out.contains("### Bastion OS"), "got:\n{out}");
        assert!(
            out.contains("#### NOW"),
            "a focused epic must get full lanes, not a collapsed one-liner, got:\n{out}"
        );
    }

    #[test]
    fn epic_board_warns_and_skips_when_the_sentinel_is_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let status_path = tmp.path().join("hq/planning/status.md");
        std::fs::create_dir_all(status_path.parent().unwrap()).unwrap();
        // A status.md with only some *other* board's sentinels.
        std::fs::write(&status_path, doc_with(markers::HQ_BOARD)).unwrap();

        let mut files = corpus(tmp.path());
        files.push(hq(
            tmp.path(),
            "hq",
            vec![epic("bastion-os", "Bastion OS", "active", None)],
        ));
        let graph = build_state_graph(&files);

        let plan = plan_epic_boards(tmp.path(), &files, &graph, &config());
        assert!(plan.actions.is_empty(), "must never invent sentinels");
        assert_eq!(
            plan.diagnostics
                .iter()
                .map(|d| d.locator.as_str())
                .collect::<Vec<_>>(),
            vec!["W_EMIT_NO_SENTINEL"]
        );
    }

    #[test]
    fn epic_board_is_a_no_op_without_a_registry() {
        // Today's corpus: no epics[] authored anywhere. The planner must not
        // touch a single file — not even one carrying the sentinel.
        let tmp = tempfile::tempdir().unwrap();
        let status_path = tmp.path().join("hq/planning/status.md");
        std::fs::create_dir_all(status_path.parent().unwrap()).unwrap();
        std::fs::write(&status_path, doc_with(markers::EPIC_BOARD)).unwrap();

        let mut files = corpus(tmp.path());
        files.push(hq(tmp.path(), "hq", vec![]));
        let graph = build_state_graph(&files);

        let plan = plan_epic_boards(tmp.path(), &files, &graph, &config());
        assert!(plan.actions.is_empty());
        assert!(plan.diagnostics.is_empty());
    }

    // -- plan_epic_sequences -------------------------------------------------

    #[test]
    fn epic_sequences_splice_into_the_registry_plan_doc() {
        let tmp = tempfile::tempdir().unwrap();
        let plan_rel = "core/planning/bastion-os.md";
        let plan_path = tmp.path().join(plan_rel);
        std::fs::create_dir_all(plan_path.parent().unwrap()).unwrap();
        std::fs::write(&plan_path, doc_with(markers::EPIC_SEQUENCE)).unwrap();

        let mut files = corpus(tmp.path());
        files.push(hq(
            tmp.path(),
            "hq",
            vec![
                epic("bastion-os", "Bastion OS", "active", Some(plan_rel)),
                // No `plan` path — skipped silently, not a warning.
                epic("bastion-web", "Bastion Web", "active", None),
            ],
        ));
        let graph = build_state_graph(&files);

        let plan = plan_epic_sequences(tmp.path(), &files, &graph, &config());
        assert_eq!(plan.actions.len(), 1);
        assert!(plan.diagnostics.is_empty(), "{:?}", plan.diagnostics);
        let out = &plan.actions[0].new_content;
        assert!(out.contains("| 1 | bastion | BA.6 |"), "got:\n{out}");
        assert!(out.contains("Before.") && out.contains("After."));

        // Fixed point.
        std::fs::write(&plan_path, out).unwrap();
        assert!(
            plan_epic_sequences(tmp.path(), &files, &graph, &config())
                .actions
                .is_empty()
        );
    }

    #[test]
    fn epic_sequences_warn_when_the_plan_doc_is_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let mut files = corpus(tmp.path());
        files.push(hq(
            tmp.path(),
            "hq",
            vec![epic("bastion-os", "Bastion OS", "active", Some("nope.md"))],
        ));
        let graph = build_state_graph(&files);

        let plan = plan_epic_sequences(tmp.path(), &files, &graph, &config());
        assert!(plan.actions.is_empty());
        assert_eq!(
            plan.diagnostics
                .iter()
                .map(|d| d.locator.as_str())
                .collect::<Vec<_>>(),
            vec!["W_EMIT_NO_SENTINEL"]
        );
    }

    #[test]
    fn two_epics_sharing_a_plan_doc_warn_instead_of_clobbering() {
        let tmp = tempfile::tempdir().unwrap();
        let rel = "core/planning/shared.md";
        let path = tmp.path().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, doc_with(markers::EPIC_SEQUENCE)).unwrap();

        let mut files = corpus(tmp.path());
        files.push(hq(
            tmp.path(),
            "hq",
            vec![
                epic("bastion-os", "Bastion OS", "active", Some(rel)),
                epic("bastion-web", "Bastion Web", "active", Some(rel)),
            ],
        ));
        let graph = build_state_graph(&files);

        let plan = plan_epic_sequences(tmp.path(), &files, &graph, &config());
        assert_eq!(
            plan.actions.len(),
            1,
            "only the first claimant may write; the second must not queue a \
             competing full-document write"
        );
        assert_eq!(
            plan.diagnostics
                .iter()
                .map(|d| d.locator.as_str())
                .collect::<Vec<_>>(),
            vec!["W_EMIT_EPIC_PLAN_CONFLICT"]
        );
        // The surviving table is the first epic's.
        assert!(plan.actions[0].new_content.contains("BA.6"));
    }

    // -- epic_members_resolved (`MV.13.D` Task 3 — precedence rule) ----------

    fn program_epic(slug: &str, title: &str, plan: &str) -> Epic {
        let mut e = epic(slug, title, "active", Some(plan));
        e.extra.insert(
            "kind".to_string(),
            serde_json::Value::String("program".to_string()),
        );
        e
    }

    fn write_lane_file(root: &std::path::Path, roadmap: &str, lane: &str, content: &str) {
        let dir = root.join("planning/roadmaps").join(roadmap);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("lane-{lane}.txt")), content).unwrap();
    }

    #[test]
    fn epic_members_resolved_area_kind_falls_back_to_authored_membership() {
        let tmp = tempfile::tempdir().unwrap();
        let files = corpus(tmp.path());
        let graph = build_state_graph(&files);

        // No `kind` authored at all — must behave exactly like `epic_members` did
        // before this block, not silently start deriving.
        let area = epic("bastion-os", "Bastion OS", "active", None);
        let resolved = mev::brain::emit::epic_members_resolved(tmp.path(), &graph, &files, &area);
        let authored = epic_members(&graph, &files, "bastion-os");
        let resolved_ids: Vec<&str> = resolved.iter().map(|(_, b)| b.id.as_str()).collect();
        let authored_ids: Vec<&str> = authored.iter().map(|(_, b)| b.id.as_str()).collect();
        assert_eq!(resolved_ids, authored_ids, "got:\n{resolved_ids:?}");
    }

    #[test]
    fn epic_members_resolved_program_kind_prefers_derived_lane_membership_over_authored_tags() {
        // The live conflict shape: BA.6 is authored to the epic's slug via
        // `block.epics`, but the epic's lane file claims a disjoint set of
        // blocks (BW.1, AM.1) instead. A `kind: program` epic must reflect the
        // lane file exactly — never the authored tag, never a union of both.
        let tmp = tempfile::tempdir().unwrap();
        let mut files = vec![
            leaf(
                tmp.path(),
                "bastion",
                vec![tb("BA.6", "closed", 1, &["conflict-prog"], vec![])],
            ),
            leaf(
                tmp.path(),
                "bastion-web",
                vec![tb("BW.1", "open", 2, &[], vec![])],
            ),
            leaf(
                tmp.path(),
                "amistad",
                vec![tb("AM.1", "open", 3, &[], vec![])],
            ),
        ];
        files.push(hq(
            tmp.path(),
            "hq",
            vec![program_epic(
                "conflict-prog",
                "Conflict Prog",
                "planning/roadmaps/conflict-prog/roadmap.md",
            )],
        ));
        let graph = build_state_graph(&files);

        write_lane_file(tmp.path(), "conflict-prog", "main", "BW.1\nAM.1\n");

        let epic = files
            .iter()
            .find_map(|(_, f)| f.epics.iter().find(|e| e.slug == "conflict-prog"))
            .unwrap();
        let members = mev::brain::emit::epic_members_resolved(tmp.path(), &graph, &files, epic);
        let ids: Vec<&str> = members.iter().map(|(_, b)| b.id.as_str()).collect();

        assert_eq!(
            ids,
            vec!["BW.1", "AM.1"],
            "derived lane membership must win, in lane-file order; got {ids:?}"
        );
        assert!(
            !ids.contains(&"BA.6"),
            "authored-only tag must not add a member to a program epic; got {ids:?}"
        );
    }

    #[test]
    fn epic_members_resolved_program_kind_with_non_roadmap_plan_is_empty() {
        // An area's plan doc (`.../epics/<slug>.md`) never names a roadmap slug —
        // a mis-tagged `kind: program` epic must not silently derive from
        // whatever the last path segment happens to be.
        let tmp = tempfile::tempdir().unwrap();
        let files = corpus(tmp.path());
        let graph = build_state_graph(&files);

        let bad = program_epic(
            "bastion-os",
            "Bastion OS",
            "core/planning/epics/bastion-os.md",
        );
        let resolved = mev::brain::emit::epic_members_resolved(tmp.path(), &graph, &files, &bad);
        assert!(resolved.is_empty(), "got: {resolved:?}");
    }

    #[test]
    fn epic_members_resolved_program_kind_with_no_lane_files_falls_back_to_authored() {
        // BA.6/BA.7 are authored to "bastion-os", and no lane file exists for it —
        // a program that finished before lane tooling existed. Amended MV.13.D
        // rule: derived wins *where derivable*; with nothing derivable, the
        // program falls back to its authored `block.epics` instead of
        // rendering an empty table (was: `..._is_empty_not_authored`, pinning
        // the pre-amendment behaviour this test now inverts).
        let tmp = tempfile::tempdir().unwrap();
        let files = corpus(tmp.path());
        let graph = build_state_graph(&files);

        let program = program_epic(
            "bastion-os",
            "Bastion OS",
            "planning/roadmaps/bastion-os/roadmap.md",
        );
        let resolved =
            mev::brain::emit::epic_members_resolved(tmp.path(), &graph, &files, &program);
        let ids: Vec<&str> = resolved.iter().map(|(_, b)| b.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["BA.6", "BA.7"],
            "no derivable lane membership must fall back to authored block.epics; got {ids:?}"
        );
    }

    #[test]
    fn epic_members_resolved_origin_roadmap_adoption_renders_once_under_executing_roadmap() {
        // BA.9 is claimed by both prog-a's and prog-b's lane files. Only prog-b's
        // claim carries `# ORIGIN:`, so per D57's two-axis rule it is the
        // executing roadmap: BA.9 must render under prog-b only, never prog-a,
        // and never both.
        let tmp = tempfile::tempdir().unwrap();
        let mut files = vec![leaf(
            tmp.path(),
            "bastion",
            vec![tb("BA.9", "open", 1, &[], vec![])],
        )];
        files.push(hq(
            tmp.path(),
            "hq",
            vec![
                program_epic("prog-a", "Prog A", "planning/roadmaps/prog-a/roadmap.md"),
                program_epic("prog-b", "Prog B", "planning/roadmaps/prog-b/roadmap.md"),
            ],
        ));
        let graph = build_state_graph(&files);

        write_lane_file(tmp.path(), "prog-a", "main", "BA.9\n");
        let origin_path = tmp.path().join("planning/roadmaps/prog-a/roadmap.md");
        write_lane_file(
            tmp.path(),
            "prog-b",
            "main",
            &format!("# ORIGIN: {}\nBA.9\n", origin_path.display()),
        );

        let hq_epics = &files
            .iter()
            .find(|(_, f)| f.kind == "brain")
            .unwrap()
            .1
            .epics;
        let a = hq_epics.iter().find(|e| e.slug == "prog-a").unwrap();
        let b = hq_epics.iter().find(|e| e.slug == "prog-b").unwrap();

        let a_members = mev::brain::emit::epic_members_resolved(tmp.path(), &graph, &files, a);
        let b_members = mev::brain::emit::epic_members_resolved(tmp.path(), &graph, &files, b);
        let a_ids: Vec<&str> = a_members.iter().map(|(_, blk)| blk.id.as_str()).collect();
        let b_ids: Vec<&str> = b_members.iter().map(|(_, blk)| blk.id.as_str()).collect();

        assert!(
            a_ids.is_empty(),
            "unannotated claim loses to the annotated one; got {a_ids:?}"
        );
        assert_eq!(
            b_ids,
            vec!["BA.9"],
            "adopted block renders under its executing roadmap only; got {b_ids:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Task 2 (ticket-emit-state-scope-and-lock) — filter_plan_by_scope
// ---------------------------------------------------------------------------

mod task2_scope_filter {
    use mev::brain::config::{BrainConfig, RepoEntry};
    use mev::brain::emit::{EmitAction, EmitPlan, filter_plan_by_scope};
    use std::path::PathBuf;

    fn repo_entry(slug: &str, tier: &str, repo_path: &str) -> RepoEntry {
        RepoEntry {
            slug: slug.to_string(),
            tier: tier.to_string(),
            repo_path: repo_path.to_string(),
            status_file: format!("{repo_path}/planning/status.md"),
            cache_doc: format!("docs/projects/{slug}.md"),
            heading: slug.to_string(),
            prefix: None,
        }
    }

    /// Mirrors the real HQ `brain.toml` shape used by `config.rs`'s own
    /// `scope_dependencies` tests: an HQ root, a `core` tier-container
    /// self-entry, and one leaf under it plus one unrelated leaf under a
    /// second tier.
    fn scoped_fixture_config() -> BrainConfig {
        BrainConfig {
            repos: vec![
                RepoEntry {
                    slug: "brain".to_string(),
                    tier: "_root".to_string(),
                    repo_path: ".".to_string(),
                    status_file: "planning/status.md".to_string(),
                    cache_doc: "README.md".to_string(),
                    heading: "Company Brain".to_string(),
                    prefix: None,
                },
                repo_entry("core", "_root", "core"),
                repo_entry("mev", "core", "core/mev"),
                repo_entry("business", "_root", "business"),
                repo_entry("bastiel", "business", "business/bastiel"),
            ],
            ..BrainConfig::default()
        }
    }

    fn action(path: PathBuf) -> EmitAction {
        EmitAction {
            path,
            new_content: "irrelevant".to_string(),
            note: "test action".to_string(),
        }
    }

    #[test]
    fn none_scope_passes_every_action_through_unfiltered() {
        let root = PathBuf::from("/hq");
        let plan = EmitPlan {
            actions: vec![
                action(root.join("core/mev/planning/state.json")),
                action(root.join("business/bastiel/planning/state.json")),
                action(root.join("docs/projects/anything.md")),
            ],
            diagnostics: vec![],
        };
        let action_count = plan.actions.len();

        let filtered = filter_plan_by_scope(plan, &root, None);

        assert_eq!(
            filtered.actions.len(),
            action_count,
            "unscoped filtering must be a byte-for-byte no-op"
        );
    }

    #[test]
    fn scoped_filter_keeps_only_in_scope_targets_and_preserves_diagnostics() {
        let cfg = scoped_fixture_config();
        let deps = cfg.scope_dependencies("mev").expect("mev is registered");
        let root = PathBuf::from("/hq");

        let mev_state = root.join("core/mev/planning/state.json");
        let mev_cache = root.join("docs/projects/mev.md");
        let core_tier_rollup = root.join("core/planning/status.md");
        let hq_board = root.join("planning/status.md");
        let bastiel_state = root.join("business/bastiel/planning/state.json");
        let unrelated_doc = root.join("docs/projects/unrelated.md");

        let diag = mev::Diagnostic::warning(root.join("wherever"), "W_EMIT_NO_SENTINEL", "note");
        let plan = EmitPlan {
            actions: vec![
                action(mev_state.clone()),
                action(mev_cache.clone()),
                action(core_tier_rollup.clone()),
                action(hq_board.clone()),
                action(bastiel_state),
                action(unrelated_doc),
            ],
            diagnostics: vec![diag],
        };

        let filtered = filter_plan_by_scope(plan, &root, Some(&deps));

        let mut kept_paths: Vec<_> = filtered.actions.iter().map(|a| a.path.clone()).collect();
        kept_paths.sort();
        let mut expected = vec![mev_state, mev_cache, core_tier_rollup, hq_board];
        expected.sort();
        assert_eq!(
            kept_paths, expected,
            "exactly the four scope surfaces survive filtering: own state.json, \
             cache_doc, tier rollup status.md, and the HQ board status.md"
        );
        assert_eq!(
            filtered.diagnostics.len(),
            1,
            "diagnostics always pass through regardless of scope"
        );
    }

    #[test]
    fn scoped_filter_drops_everything_when_scope_matches_nothing() {
        let cfg = scoped_fixture_config();
        let deps = cfg
            .scope_dependencies("bastiel")
            .expect("bastiel is registered");
        let root = PathBuf::from("/hq");

        let plan = EmitPlan {
            actions: vec![action(root.join("core/mev/planning/state.json"))],
            diagnostics: vec![],
        };

        let filtered = filter_plan_by_scope(plan, &root, Some(&deps));
        assert!(
            filtered.actions.is_empty(),
            "an out-of-scope action must be dropped, not defaulted through"
        );
    }
}

// ---------------------------------------------------------------------------
// task3_wontfix_progress — `wontfix` is tallied separately from `closed`
// ---------------------------------------------------------------------------

mod task3_wontfix_progress {
    use super::block;
    use mev::brain::emit::epic_progress;

    #[test]
    fn wontfix_members_are_tallied_separately_from_closed() {
        let a = block("A.1", "A.1", Some("closed"), Some(1));
        let b = block("A.2", "A.2", Some("wontfix"), Some(2));
        let c = block("A.3", "A.3", Some("open"), Some(3));
        let members: Vec<(String, &_)> = vec![
            ("alpha".to_string(), &a),
            ("alpha".to_string(), &b),
            ("alpha".to_string(), &c),
        ];

        let p = epic_progress(&members);

        assert_eq!(p.closed, 1, "wontfix must not inflate the closed tally");
        assert_eq!(p.wontfix, 1);
        assert_eq!(p.open, 1);
        assert_eq!(p.in_progress, 0);
        assert_eq!(p.deferred, 0);
        assert_eq!(p.total(), 3, "total() must still count every member");
    }

    #[test]
    fn wontfix_with_no_members_leaves_progress_at_zero() {
        let members: Vec<(String, &mev::brain::state::TrackBlock)> = vec![];
        let p = epic_progress(&members);
        assert_eq!(p.wontfix, 0);
        assert_eq!(p.total(), 0);
    }
}

// ---------------------------------------------------------------------------
// task5_shared_identity_dedup — one operator/approval slug gating N blocks
// renders as exactly ONE item (ticket-operator-edge-graph, Task 5)
// ---------------------------------------------------------------------------

mod task5_shared_identity_dedup {
    use std::collections::HashMap;

    use mev::brain::emit::{group_blocked_by_gate, render_hq_board, render_unified_board};
    use mev::brain::state::{ApprovalDep, Block, BlockedBy, ExternalDep, Focus, OperatorDep};

    /// Build a repo-tagged `Block` with the given `blocked_by` entries.
    fn blocked_block(repo: &str, id: &str, title: &str, blocked_by: Vec<BlockedBy>) -> Block {
        Block {
            epics: Vec::new(),
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

    fn operator(slug: &str) -> BlockedBy {
        BlockedBy::Operator(OperatorDep {
            slug: slug.to_string(),
            exit: "artifact exists".to_string(),
            start: "mev do-thing".to_string(),
            what: None,
        })
    }

    fn approval(slug: &str) -> BlockedBy {
        BlockedBy::Approval(ApprovalDep {
            slug: slug.to_string(),
            what: "ship it?".to_string(),
            digest: "deadbeef".to_string(),
        })
    }

    /// Extract the bullet lines (`"- ..."`) of the `## {heading}` (or
    /// `## {heading}` for the HQ board) section of a rendered board, up to the
    /// next `##` heading or end of string.
    fn section_bullets<'a>(rendered: &'a str, heading: &str) -> Vec<&'a str> {
        let marker = format!("## {heading}\n");
        let start = rendered.find(&marker).expect("heading present") + marker.len();
        let rest = &rendered[start..];
        let end = rest.find("\n\n##").unwrap_or(rest.len());
        rest[..end]
            .lines()
            .filter(|l| l.starts_with("- "))
            .collect()
    }

    #[test]
    fn hq_board_one_operator_slug_on_three_blocks_across_two_repos_renders_one_item() {
        let focus = Focus {
            now: vec![],
            next: vec![],
            blocked: vec![
                blocked_block("core", "A.1", "Block A1", vec![operator("gate-x")]),
                blocked_block("core", "A.2", "Block A2", vec![operator("gate-x")]),
                blocked_block("bastion", "B.1", "Block B1", vec![operator("gate-x")]),
            ],
            deferred: Vec::new(),
        };

        let rendered = render_hq_board(&focus, &[]);
        let bullets = section_bullets(&rendered, "BLOCKED");

        assert_eq!(
            bullets.len(),
            1,
            "one shared slug across 3 blocks must render as exactly one item, got: {bullets:?}"
        );
        let item = bullets[0];
        assert!(item.contains("core:A.1"), "{item}");
        assert!(item.contains("core:A.2"), "{item}");
        assert!(item.contains("bastion:B.1"), "{item}");
        assert!(item.contains("operator:gate-x"), "{item}");
    }

    #[test]
    fn unified_board_one_operator_slug_on_three_blocks_across_two_repos_renders_one_item() {
        let focus = Focus {
            now: vec![],
            next: vec![],
            blocked: vec![
                blocked_block("core", "A.1", "Block A1", vec![operator("gate-x")]),
                blocked_block("core", "A.2", "Block A2", vec![operator("gate-x")]),
                blocked_block("bastion", "B.1", "Block B1", vec![operator("gate-x")]),
            ],
            deferred: Vec::new(),
        };
        let config = mev::brain::config::BrainConfig::default();

        let rendered = render_unified_board(
            &focus,
            &[],
            &HashMap::new(),
            &config,
            chrono::NaiveDate::from_ymd_opt(2026, 7, 5).expect("valid date"),
        );
        let bullets = section_bullets(&rendered, "BLOCKED");

        assert_eq!(
            bullets.len(),
            1,
            "unified board must dedup the shared slug too, got: {bullets:?}"
        );
        let item = bullets[0];
        assert!(item.contains("core:A.1"), "{item}");
        assert!(item.contains("core:A.2"), "{item}");
        assert!(item.contains("bastion:B.1"), "{item}");
        assert!(item.contains("operator:gate-x"), "{item}");
    }

    #[test]
    fn two_distinct_operator_slugs_render_as_two_items() {
        let focus = Focus {
            now: vec![],
            next: vec![],
            blocked: vec![
                blocked_block("core", "A.1", "Block A1", vec![operator("gate-x")]),
                blocked_block("core", "A.2", "Block A2", vec![operator("gate-y")]),
            ],
            deferred: Vec::new(),
        };

        let rendered = render_hq_board(&focus, &[]);
        let bullets = section_bullets(&rendered, "BLOCKED");

        assert_eq!(bullets.len(), 2, "distinct slugs must not be merged");
    }

    #[test]
    fn approval_edges_dedup_by_slug_same_as_operator() {
        let focus = Focus {
            now: vec![],
            next: vec![],
            blocked: vec![
                blocked_block("core", "A.1", "Block A1", vec![approval("ship-v2")]),
                blocked_block("bastion", "B.1", "Block B1", vec![approval("ship-v2")]),
            ],
            deferred: Vec::new(),
        };

        let rendered = render_hq_board(&focus, &[]);
        let bullets = section_bullets(&rendered, "BLOCKED");

        assert_eq!(bullets.len(), 1, "shared approval slug must dedup too");
        assert!(bullets[0].contains("approval:ship-v2"));
        assert!(bullets[0].contains("core:A.1"));
        assert!(bullets[0].contains("bastion:B.1"));
    }

    #[test]
    fn blocks_with_no_shared_slug_each_render_as_their_own_item() {
        let focus = Focus {
            now: vec![],
            next: vec![],
            blocked: vec![
                blocked_block(
                    "core",
                    "A.1",
                    "Block A1",
                    vec![BlockedBy::External(ExternalDep {
                        what: "waiting on vendor".to_string(),
                    })],
                ),
                blocked_block("core", "A.2", "Block A2", vec![operator("gate-solo")]),
            ],
            deferred: Vec::new(),
        };

        let rendered = render_hq_board(&focus, &[]);
        let bullets = section_bullets(&rendered, "BLOCKED");

        assert_eq!(
            bullets.len(),
            2,
            "an external dep and a solo operator gate must not be merged with each other"
        );
    }

    #[test]
    fn group_blocked_by_gate_deduped_group_carries_minimum_effective_priority() {
        let blocks = vec![
            blocked_block("core", "A.1", "Block A1", vec![operator("gate-x")]),
            blocked_block("core", "A.2", "Block A2", vec![operator("gate-x")]),
            blocked_block("bastion", "B.1", "Block B1", vec![operator("gate-x")]),
        ];

        let mut effective = HashMap::new();
        effective.insert("core:A.1".to_string(), 2u8);
        effective.insert("core:A.2".to_string(), 0u8);
        effective.insert("bastion:B.1".to_string(), 1u8);

        let groups = group_blocked_by_gate(&blocks);
        assert_eq!(groups.len(), 1, "all three share one slug");
        assert_eq!(groups[0].blocks.len(), 3);
        assert_eq!(
            groups[0].effective_priority(&effective),
            0,
            "the deduped item must carry the MINIMUM effective priority of the blocks it gates"
        );
    }

    #[test]
    fn group_blocked_by_gate_single_block_with_no_gate_is_a_singleton_with_no_gate() {
        let blocks = vec![blocked_block("core", "A.1", "Block A1", vec![])];

        let groups = group_blocked_by_gate(&blocks);

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].blocks.len(), 1);
        assert!(groups[0].gate.is_none());
    }
}

// ---------------------------------------------------------------------------
// task6_rendering — focus.blocked[] and the board show exit, start, and
// decisions in full (ticket-operator-edge-graph, Task 6)
// ---------------------------------------------------------------------------

mod task6_rendering {
    use mev::brain::emit::{render_epic_sequence_table, render_hq_board, render_unified_board};
    use mev::brain::state::{
        ApprovalDep, Block, BlockDep, BlockedBy, Focus, OperatorDep, TrackBlock,
    };
    use std::collections::HashMap;

    fn blocked_block(repo: &str, id: &str, title: &str, blocked_by: Vec<BlockedBy>) -> Block {
        Block {
            epics: Vec::new(),
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

    fn operator_gate(slug: &str, exit: &str, start: &str) -> BlockedBy {
        BlockedBy::Operator(OperatorDep {
            slug: slug.to_string(),
            exit: exit.to_string(),
            start: start.to_string(),
            what: None,
        })
    }

    fn approval_gate(slug: &str, what: &str, digest: &str) -> BlockedBy {
        BlockedBy::Approval(ApprovalDep {
            slug: slug.to_string(),
            what: what.to_string(),
            digest: digest.to_string(),
        })
    }

    #[test]
    fn hq_board_operator_gate_renders_exit_and_start() {
        let focus = Focus {
            now: vec![],
            next: vec![],
            blocked: vec![blocked_block(
                "core",
                "A.1",
                "Block A1",
                vec![operator_gate(
                    "gate-x",
                    "log.md entry exists",
                    "mev close-operator-gate gate-x --exit-verified",
                )],
            )],
            deferred: Vec::new(),
        };

        let rendered = render_hq_board(&focus, &[]);

        assert!(rendered.contains("exit: log.md entry exists"), "{rendered}");
        assert!(
            rendered.contains("start: `mev close-operator-gate gate-x --exit-verified`"),
            "{rendered}"
        );
    }

    #[test]
    fn unified_board_operator_gate_renders_exit_and_start() {
        let focus = Focus {
            now: vec![],
            next: vec![],
            blocked: vec![blocked_block(
                "core",
                "A.1",
                "Block A1",
                vec![operator_gate(
                    "gate-y",
                    "artifact published",
                    "mev close-operator-gate gate-y --exit-verified",
                )],
            )],
            deferred: Vec::new(),
        };
        let config = mev::brain::config::BrainConfig::default();

        let rendered = render_unified_board(
            &focus,
            &[],
            &HashMap::new(),
            &config,
            chrono::NaiveDate::from_ymd_opt(2026, 7, 5).expect("valid date"),
        );

        assert!(rendered.contains("exit: artifact published"), "{rendered}");
        assert!(
            rendered.contains("start: `mev close-operator-gate gate-y --exit-verified`"),
            "{rendered}"
        );
    }

    #[test]
    fn hq_board_approval_renders_what_as_a_decision() {
        let focus = Focus {
            now: vec![],
            next: vec![],
            blocked: vec![blocked_block(
                "core",
                "A.1",
                "Block A1",
                vec![approval_gate("ship-v2", "ship it?", "deadbeef")],
            )],
            deferred: Vec::new(),
        };

        let rendered = render_hq_board(&focus, &[]);

        assert!(
            rendered.contains("decision: ship it?"),
            "approval must render `what` labeled as a decision, not a description: {rendered}"
        );
    }

    #[test]
    fn epic_sequence_table_renders_operator_exit_start_and_approval_decision() {
        let operator_block = TrackBlock {
            id: "A.1".to_string(),
            title: "Block A1".to_string(),
            depends_on: vec![operator_gate(
                "gate-z",
                "PR merged",
                "mev close-operator-gate gate-z --exit-verified",
            )],
            ..Default::default()
        };
        let approval_block = TrackBlock {
            id: "A.2".to_string(),
            title: "Block A2".to_string(),
            depends_on: vec![approval_gate("ship-v3", "ship it?", "cafebabe")],
            ..Default::default()
        };
        let members: Vec<(String, &TrackBlock)> = vec![
            ("core".to_string(), &operator_block),
            ("core".to_string(), &approval_block),
        ];

        let table = render_epic_sequence_table(&members, &HashMap::new());

        assert!(table.contains("exit: PR merged"), "{table}");
        assert!(
            table.contains("start: `mev close-operator-gate gate-z --exit-verified`"),
            "{table}"
        );
        assert!(table.contains("decision: ship it?"), "{table}");
    }

    /// Blocks with no operator/approval edge must render byte-identically to
    /// before this task — a plain `Block` dependency's annotation is untouched.
    #[test]
    fn plain_block_dependency_rendering_is_unchanged() {
        let focus = Focus {
            now: vec![],
            next: vec![],
            blocked: vec![blocked_block(
                "core",
                "A.1",
                "Block A1",
                vec![BlockedBy::Block(BlockDep {
                    repo: "core".to_string(),
                    id: "A.0".to_string(),
                    what: Some("needs the shared schema".to_string()),
                })],
            )],
            deferred: Vec::new(),
        };

        let rendered = render_hq_board(&focus, &[]);

        assert!(
            rendered
                .contains("core:A.1 — Block A1 (blocked by core:A.0 (needs the shared schema))"),
            "{rendered}"
        );
    }
}

// ---------------------------------------------------------------------------
// render_block_graph_reconcile_failed — task 4 (ticket-reconcile-failed-consumer)
// ---------------------------------------------------------------------------

mod reconcile_failed_rendering {
    use mev::brain::block_graph::{
        BlockGraphEdge, BlockGraphExport, BlockGraphNode, BlockGraphScopeEcho, BlockLane,
    };
    use mev::brain::emit::render_block_graph_reconcile_failed;

    /// Build a minimal `BlockGraphNode` with every non-essential field defaulted, varying
    /// only `key`/`repo`/`id`/`title`/`reconcile_failed` — the fields this render function
    /// reads.
    fn make_node(
        repo: &str,
        id: &str,
        title: &str,
        reconcile_failed: Option<bool>,
    ) -> BlockGraphNode {
        BlockGraphNode {
            key: format!("{repo}:{id}"),
            repo: repo.to_string(),
            id: id.to_string(),
            title: title.to_string(),
            status: None,
            lane: BlockLane::Next,
            track: None,
            wave: None,
            priority: None,
            effective_priority: None,
            due: None,
            epics: Vec::new(),
            layer: 0,
            topo_index: 0,
            ready: false,
            in_cycle: false,
            in_scope: true,
            external_deps: Vec::new(),
            unmet_count: 0,
            dependent_count: 0,
            last_touched: None,
            reconcile_failed,
        }
    }

    fn make_export(nodes: Vec<BlockGraphNode>) -> BlockGraphExport {
        let total_nodes = nodes.len() as u32;
        BlockGraphExport {
            version: "1".to_string(),
            root: "/fake/root".to_string(),
            scope: BlockGraphScopeEcho {
                tier: None,
                epic: None,
                repo: None,
                include_closed: true,
                include_boundary: false,
            },
            nodes,
            edges: Vec::<BlockGraphEdge>::new(),
            cycles: Vec::new(),
            total_nodes,
            truncated: false,
        }
    }

    #[test]
    fn reconcile_failed_block_renders_the_annotation() {
        let export = make_export(vec![make_node("mev", "MV.10.C", "Block C", Some(true))]);
        let rendered = render_block_graph_reconcile_failed(&export);
        assert_eq!(rendered, "mev:MV.10.C — Block C (reconcile_failed)");
    }

    #[test]
    fn done_block_renders_with_no_annotation() {
        let export = make_export(vec![make_node("mev", "MV.10.C", "Block C", Some(false))]);
        let rendered = render_block_graph_reconcile_failed(&export);
        assert_eq!(rendered, "mev:MV.10.C — Block C");
    }

    #[test]
    fn never_run_block_renders_with_no_annotation() {
        let export = make_export(vec![make_node("mev", "MV.10.C", "Block C", None)]);
        let rendered = render_block_graph_reconcile_failed(&export);
        assert_eq!(rendered, "mev:MV.10.C — Block C");
    }

    /// A corpus with no `reconcile_failed` blocks at all must render byte-identical
    /// output to what this function would have produced before the annotation existed —
    /// i.e. the plain `"{repo}:{id} — {title}"` lines, joined by `\n`, with no stray
    /// annotation text anywhere.
    #[test]
    fn corpus_with_no_reconcile_failed_blocks_renders_byte_identical_plain_lines() {
        let export = make_export(vec![
            make_node("mev", "A.1", "Block A1", Some(false)),
            make_node("mev", "A.2", "Block A2", None),
        ]);
        let rendered = render_block_graph_reconcile_failed(&export);
        assert_eq!(rendered, "mev:A.1 — Block A1\nmev:A.2 — Block A2");
    }

    #[test]
    fn only_the_flagged_block_carries_the_annotation_in_a_mixed_corpus() {
        let export = make_export(vec![
            make_node("mev", "A.1", "Block A1", Some(false)),
            make_node("mev", "A.2", "Block A2", Some(true)),
            make_node("mev", "A.3", "Block A3", None),
        ]);
        let rendered = render_block_graph_reconcile_failed(&export);
        assert_eq!(
            rendered,
            "mev:A.1 — Block A1\nmev:A.2 — Block A2 (reconcile_failed)\nmev:A.3 — Block A3"
        );
    }

    #[test]
    fn empty_export_renders_empty_string() {
        let export = make_export(Vec::new());
        let rendered = render_block_graph_reconcile_failed(&export);
        assert_eq!(rendered, "");
    }
}
