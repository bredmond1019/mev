//! Integration tests for epic parking — `mev defer-epic` / `resume-epic` / `sync-epics`.
//!
//! These drive the real public entry point (`mev::epic_status`) against a real
//! on-disk brain, because the planner compares its re-serialized output against
//! the bytes actually on disk to decide whether a write is needed. A pure
//! in-memory test would not exercise that no-churn path.

use std::path::Path;

use mev::brain::epics::EpicAction;

const BRAIN_TOML: &str = r#"
[vocab]
layer = ["brain", "engine", "meta"]
status = ["active", "draft", "deprecated", "superseded", "archived"]

[crawl]
skip_dirs = ["target", "node_modules", ".git"]

[[repos]]
slug = "brain"
prefix = "BR"
tier = "_root"
repo_path = "."
status_file = "planning/status.md"
cache_doc = "README.md"
heading = "Company Brain"

[[repos]]
slug = "alpha"
prefix = "AL"
tier = "core"
repo_path = "alpha"
status_file = "alpha/planning/status.md"
cache_doc = "docs/projects/alpha.md"
heading = "Alpha"
"#;

/// Build a two-repo brain whose `alpha` blocks carry the given
/// `(id, status, epic)` triples, and whose HQ registry carries `(slug, status)`.
fn brain(blocks: &[(&str, &str, &str)], epics: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    std::fs::create_dir_all(root.join("planning")).unwrap();
    std::fs::create_dir_all(root.join("alpha/planning")).unwrap();
    std::fs::write(root.join("brain.toml"), BRAIN_TOML).unwrap();

    let block_json: Vec<String> = blocks
        .iter()
        .enumerate()
        .map(|(i, (id, status, epic))| {
            format!(
                r#"{{ "id": "{id}", "title": "{id}", "status": "{status}", "wave": {}, "epics": ["{epic}"] }}"#,
                i + 1
            )
        })
        .collect();
    std::fs::write(
        root.join("alpha/planning/state.json"),
        format!(
            r#"{{ "repo": "alpha", "kind": "project", "updated": "2026-07-26",
  "focus": {{ "now": [], "next": [], "blocked": [] }},
  "tracks": [{{ "title": "P1", "blocks": [{}] }}] }}"#,
            block_json.join(",\n")
        ),
    )
    .unwrap();

    let epic_json: Vec<String> = epics
        .iter()
        .map(|(slug, status)| {
            format!(r#"{{ "slug": "{slug}", "title": "{slug}", "status": "{status}" }}"#)
        })
        .collect();
    std::fs::write(
        root.join("planning/state.json"),
        format!(
            r#"{{ "repo": "brain", "kind": "brain", "updated": "2026-07-26",
  "focus": {{ "now": [], "next": [], "blocked": [] }},
  "tracks": [], "repos": [], "cross_repo": [],
  "epics": [{}] }}"#,
            epic_json.join(",\n")
        ),
    )
    .unwrap();

    let fm = "---\ntype: ProjectStatus\ntitle: t\ndescription: d\n---\n\n# S\n";
    std::fs::write(root.join("alpha/planning/status.md"), fm).unwrap();
    std::fs::write(root.join("planning/status.md"), fm).unwrap();
    dir
}

/// `(block_id, status)` pairs as currently written in alpha's state.json.
fn block_statuses(root: &Path) -> Vec<(String, String)> {
    let v: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("alpha/planning/state.json")).unwrap(),
    )
    .unwrap();
    v["tracks"][0]["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| {
            (
                b["id"].as_str().unwrap().to_string(),
                b["status"].as_str().unwrap_or("open").to_string(),
            )
        })
        .collect()
}

/// `(epic_slug, status)` pairs as currently written in the HQ registry.
fn epic_statuses(root: &Path) -> Vec<(String, String)> {
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("planning/state.json")).unwrap())
            .unwrap();
    v["epics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["slug"].as_str().unwrap().to_string(),
                e["status"].as_str().unwrap_or("active").to_string(),
            )
        })
        .collect()
}

fn status_of(pairs: &[(String, String)], key: &str) -> String {
    pairs
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.clone())
        .unwrap_or_else(|| panic!("no entry for {key}"))
}

// ── defer ────────────────────────────────────────────────────────────────────

#[test]
fn defer_epic_pauses_registry_and_defers_open_blocks() {
    let dir = brain(
        &[
            ("AL.1.A", "open", "tui"),
            ("AL.1.B", "open", "tui"),
            ("AL.1.C", "open", "other"),
        ],
        &[("tui", "active"), ("other", "active")],
    );
    let root = dir.path();

    let report = mev::epic_status(root, Some("tui"), EpicAction::Defer, true).unwrap();
    assert!(!report.is_failure(), "defer must succeed: {report:?}");

    let blocks = block_statuses(root);
    assert_eq!(status_of(&blocks, "AL.1.A"), "deferred");
    assert_eq!(status_of(&blocks, "AL.1.B"), "deferred");
    assert_eq!(
        status_of(&blocks, "AL.1.C"),
        "open",
        "a block in a DIFFERENT epic must not be touched"
    );

    let epics = epic_statuses(root);
    assert_eq!(status_of(&epics, "tui"), "paused");
    assert_eq!(status_of(&epics, "other"), "active");
}

#[test]
fn defer_epic_never_parks_in_progress_work_and_says_so() {
    let dir = brain(
        &[("AL.1.A", "in_progress", "tui"), ("AL.1.B", "open", "tui")],
        &[("tui", "active")],
    );
    let root = dir.path();

    let report = mev::epic_status(root, Some("tui"), EpicAction::Defer, true).unwrap();

    let blocks = block_statuses(root);
    assert_eq!(
        status_of(&blocks, "AL.1.A"),
        "in_progress",
        "work in flight must be left alone, not silently parked"
    );
    assert_eq!(status_of(&blocks, "AL.1.B"), "deferred");
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.locator == "W_EPIC_SKIPPED_IN_PROGRESS"),
        "skipping in-progress work must be reported, not silent: {report:?}"
    );
}

#[test]
fn defer_epic_leaves_closed_blocks_closed() {
    let dir = brain(
        &[("AL.1.A", "closed", "tui"), ("AL.1.B", "open", "tui")],
        &[("tui", "active")],
    );
    let root = dir.path();

    mev::epic_status(root, Some("tui"), EpicAction::Defer, true).unwrap();

    let blocks = block_statuses(root);
    assert_eq!(
        status_of(&blocks, "AL.1.A"),
        "closed",
        "done is done — deferring an epic must never reopen finished work"
    );
    assert_eq!(status_of(&blocks, "AL.1.B"), "deferred");
}

#[test]
fn defer_epic_dry_run_changes_nothing_on_disk() {
    let dir = brain(&[("AL.1.A", "open", "tui")], &[("tui", "active")]);
    let root = dir.path();
    let before = std::fs::read_to_string(root.join("alpha/planning/state.json")).unwrap();

    let report = mev::epic_status(root, Some("tui"), EpicAction::Defer, false).unwrap();

    assert_eq!(
        std::fs::read_to_string(root.join("alpha/planning/state.json")).unwrap(),
        before,
        "a dry run must not touch the corpus"
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.locator == "W_EMIT_DRY_RUN"),
        "a dry run must still report what it would do"
    );
}

#[test]
fn defer_epic_unknown_slug_is_an_error() {
    let dir = brain(&[("AL.1.A", "open", "tui")], &[("tui", "active")]);

    let report = mev::epic_status(dir.path(), Some("nope"), EpicAction::Defer, true).unwrap();

    assert!(report.is_failure());
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.locator == "E_EPIC_UNKNOWN")
    );
}

#[test]
fn defer_epic_regenerates_derived_focus_in_the_same_run() {
    // The authored edit and the derived views must not be left disagreeing —
    // otherwise the very next `validate-brain` reports focus drift.
    let dir = brain(
        &[("AL.1.A", "open", "tui"), ("AL.1.B", "open", "other")],
        &[("tui", "active"), ("other", "active")],
    );
    let root = dir.path();

    mev::epic_status(root, Some("tui"), EpicAction::Defer, true).unwrap();

    let v: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("alpha/planning/state.json")).unwrap(),
    )
    .unwrap();
    let deferred: Vec<&str> = v["focus"]["deferred"]
        .as_array()
        .map(|a| a.iter().map(|b| b["id"].as_str().unwrap()).collect())
        .unwrap_or_default();
    let next: Vec<&str> = v["focus"]["next"]
        .as_array()
        .map(|a| a.iter().map(|b| b["id"].as_str().unwrap()).collect())
        .unwrap_or_default();

    assert_eq!(
        deferred,
        vec!["AL.1.A"],
        "focus.deferred must be regenerated"
    );
    assert_eq!(
        next,
        vec!["AL.1.B"],
        "the parked block must leave focus.next"
    );
}

// ── resume ───────────────────────────────────────────────────────────────────

#[test]
fn resume_epic_reactivates_and_reopens_deferred_blocks() {
    let dir = brain(
        &[
            ("AL.1.A", "deferred", "tui"),
            ("AL.1.B", "closed", "tui"),
            ("AL.1.C", "in_progress", "tui"),
        ],
        &[("tui", "paused")],
    );
    let root = dir.path();

    mev::epic_status(root, Some("tui"), EpicAction::Resume, true).unwrap();

    let blocks = block_statuses(root);
    assert_eq!(status_of(&blocks, "AL.1.A"), "open");
    assert_eq!(status_of(&blocks, "AL.1.B"), "closed");
    assert_eq!(status_of(&blocks, "AL.1.C"), "in_progress");
    assert_eq!(status_of(&epic_statuses(root), "tui"), "active");
}

#[test]
fn defer_then_resume_round_trips_to_the_original_statuses() {
    let dir = brain(
        &[("AL.1.A", "open", "tui"), ("AL.1.B", "open", "tui")],
        &[("tui", "active")],
    );
    let root = dir.path();
    let before = block_statuses(root);

    mev::epic_status(root, Some("tui"), EpicAction::Defer, true).unwrap();
    mev::epic_status(root, Some("tui"), EpicAction::Resume, true).unwrap();

    assert_eq!(block_statuses(root), before);
    assert_eq!(status_of(&epic_statuses(root), "tui"), "active");
}

// ── sync ─────────────────────────────────────────────────────────────────────

#[test]
fn sync_epics_pauses_an_epic_whose_work_is_all_deferred() {
    // The "vice versa" direction: you deferred every block by hand, so the epic
    // itself should follow.
    let dir = brain(
        &[("AL.1.A", "deferred", "tui"), ("AL.1.B", "closed", "tui")],
        &[("tui", "active")],
    );
    let root = dir.path();

    mev::epic_status(root, None, EpicAction::Defer, true).unwrap();

    assert_eq!(status_of(&epic_statuses(root), "tui"), "paused");
}

#[test]
fn sync_epics_defers_stragglers_in_an_already_paused_epic() {
    let dir = brain(
        &[("AL.1.A", "deferred", "tui"), ("AL.1.B", "open", "tui")],
        &[("tui", "paused")],
    );
    let root = dir.path();

    mev::epic_status(root, None, EpicAction::Defer, true).unwrap();

    let blocks = block_statuses(root);
    assert_eq!(
        status_of(&blocks, "AL.1.B"),
        "deferred",
        "a paused epic must not keep leaking open work into focus.next"
    );
}

#[test]
fn sync_epics_never_pauses_an_epic_that_still_has_live_work() {
    let dir = brain(
        &[("AL.1.A", "deferred", "tui"), ("AL.1.B", "open", "tui")],
        &[("tui", "active")],
    );
    let root = dir.path();

    mev::epic_status(root, None, EpicAction::Defer, true).unwrap();

    assert_eq!(
        status_of(&epic_statuses(root), "tui"),
        "active",
        "partially-deferred is a normal state and must not auto-pause"
    );
    assert_eq!(status_of(&block_statuses(root), "AL.1.B"), "open");
}

#[test]
fn sync_epics_never_pauses_a_fully_closed_epic() {
    // All members closed = complete, not deferred. `is_fully_deferred` requires
    // at least one deferred member precisely so this case is not mislabelled.
    let dir = brain(
        &[("AL.1.A", "closed", "tui"), ("AL.1.B", "closed", "tui")],
        &[("tui", "active")],
    );
    let root = dir.path();

    mev::epic_status(root, None, EpicAction::Defer, true).unwrap();

    assert_eq!(status_of(&epic_statuses(root), "tui"), "active");
}

#[test]
fn sync_epics_is_a_no_op_when_everything_already_agrees() {
    let dir = brain(
        &[("AL.1.A", "deferred", "tui"), ("AL.1.B", "open", "other")],
        &[("tui", "paused"), ("other", "active")],
    );
    let root = dir.path();
    let before = std::fs::read_to_string(root.join("planning/state.json")).unwrap();

    let report = mev::epic_status(root, None, EpicAction::Defer, false).unwrap();

    assert_eq!(
        std::fs::read_to_string(root.join("planning/state.json")).unwrap(),
        before
    );
    assert!(
        !report
            .diagnostics
            .iter()
            .any(|d| d.locator == "W_EMIT_DRY_RUN"),
        "a consistent corpus must plan zero actions: {report:?}"
    );
}

// ── focused (active-equivalent) ──────────────────────────────────────────────

#[test]
fn sync_epics_pauses_a_focused_epic_whose_work_is_all_deferred() {
    // `focused` is a refinement of `active`, so the pause rule must fire on it
    // identically — otherwise `focused` is a hole in the reconciler.
    let dir = brain(
        &[("AL.1.A", "deferred", "tui"), ("AL.1.B", "closed", "tui")],
        &[("tui", "focused")],
    );
    let root = dir.path();

    mev::epic_status(root, None, EpicAction::Defer, true).unwrap();

    assert_eq!(status_of(&epic_statuses(root), "tui"), "paused");
}

#[test]
fn sync_epics_never_pauses_a_focused_epic_that_still_has_live_work() {
    let dir = brain(
        &[("AL.1.A", "deferred", "tui"), ("AL.1.B", "open", "tui")],
        &[("tui", "focused")],
    );
    let root = dir.path();

    mev::epic_status(root, None, EpicAction::Defer, true).unwrap();

    assert_eq!(
        status_of(&epic_statuses(root), "tui"),
        "focused",
        "partially-deferred is normal; a focused epic must be no more eager to pause than an active one"
    );
}

#[test]
fn resume_epic_sets_active_never_focused() {
    // Un-parking restores the epic to live; it does not promote it to priority.
    let dir = brain(&[("AL.1.A", "deferred", "tui")], &[("tui", "paused")]);
    let root = dir.path();

    mev::epic_status(root, Some("tui"), EpicAction::Resume, true).unwrap();

    assert_eq!(status_of(&epic_statuses(root), "tui"), "active");
}
