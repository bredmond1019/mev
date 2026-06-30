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
