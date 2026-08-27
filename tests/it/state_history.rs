//! Integration tests for the append-only revision history end-to-end
//! (`ticket-append-only-emit-state-writer`, Task 5).
//!
//! `src/brain/history.rs` and `src/brain/emit.rs::apply_plan_history_tests`
//! already carry unit-level coverage of the primitives in isolation. This
//! file is the headline proof the ticket exists for: that the incident class
//! it closes — a derivation silently dropping authored content on a real
//! `emit_state --write` run — is now recoverable via `mev state-history`,
//! exercised through the real library entry point (`mev::emit_state`)
//! against a fixture brain, following the conventions of
//! `tests/brain_emit.rs::incomplete_corpus_guard` and `tests/emit_state_lock.rs`.

use std::fs;
use std::path::Path;

use mev::brain::history;

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let d = mev::testsupport::unique_temp_dir(&format!("mev-state-history-{tag}"));
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

/// Minimal `brain.toml` registering a single leaf repo ("alpha"), mirroring
/// the fixture shape used across `tests/brain_emit.rs`. `keep_override`, when
/// `Some`, adds a `[history]` table with that `keep` value; `None` leaves the
/// section absent so the default (`keep = 10`) applies.
fn write_brain_toml(root: &Path, keep_override: Option<usize>) {
    let mut toml = String::from(
        r#"[vocab]
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
"#,
    );
    if let Some(keep) = keep_override {
        toml.push_str(&format!("\n[history]\nkeep = {keep}\n"));
    }
    fs::write(root.join("brain.toml"), toml.as_bytes()).unwrap();
}

/// Brain-root `planning/state.json` with an empty `repos[]` rollup so the
/// first write populates it from alpha's leaf.
fn write_brain_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "hq",
        "kind": "brain",
        "updated": "2026-08-01",
        "focus": { "now": [], "next": [], "blocked": [] },
        "repos": [],
        "cross_repo": []
    });
    write_json(root, "planning/state.json", &state);
}

/// Alpha leaf `planning/state.json` carrying one authored `in_progress`
/// block, "Alpha block A" — the content whose survival across a bad
/// derivation and a restore this test proves.
fn write_alpha_state_with_block(root: &Path) {
    let state = serde_json::json!({
        "repo": "alpha",
        "kind": "project",
        "updated": "2026-08-01",
        "focus": {
            "now": [{ "id": "AL.1.A", "title": "Alpha block A", "status": "in_progress" }],
            "next": [],
            "blocked": []
        },
        "tracks": [
            {
                "title": "Phase 1",
                "blocks": [
                    { "id": "AL.1.A", "title": "Alpha block A", "status": "in_progress" }
                ]
            }
        ]
    });
    write_json(root, "repos/alpha/planning/state.json", &state);
}

/// Alpha leaf `planning/state.json` with "Alpha block A" removed — simulates
/// the `derive_rollup`-drops-a-repo defect class: the next `emit_state`
/// write regenerates the brain-root rollup from this input and the authored
/// block silently disappears from the live derived file.
fn write_alpha_state_block_dropped(root: &Path) {
    let state = serde_json::json!({
        "repo": "alpha",
        "kind": "project",
        "updated": "2026-08-02",
        "focus": { "now": [], "next": [], "blocked": [] },
        "tracks": [
            {
                "title": "Phase 1",
                "blocks": []
            }
        ]
    });
    write_json(root, "repos/alpha/planning/state.json", &state);
}

fn brain_state_path(root: &Path) -> std::path::PathBuf {
    root.join("planning/state.json")
}

// ---------------------------------------------------------------------------
// Headline test — the incident class is recoverable end to end.
// ---------------------------------------------------------------------------

#[test]
fn dropped_authored_content_is_recoverable_via_restore() {
    let root = temp_dir("headline");
    write_brain_toml(&root, None);
    write_brain_state(&root);
    write_alpha_state_with_block(&root);

    // First emit: brain-root state.json is regenerated to include "Alpha
    // block A" via alpha's rollup.
    let report1 = mev::emit_state(&root, true, None).expect("first emit_state should not error");
    assert!(
        !report1.is_failure(),
        "first emit must succeed; diagnostics: {:#?}",
        report1.diagnostics
    );
    let after_first = fs::read_to_string(brain_state_path(&root)).unwrap();
    assert!(
        after_first.contains("Alpha block A"),
        "first write must populate the rollup with the authored block; got: {after_first}"
    );

    // Mutate the fixture so the second emit's derivation drops the content
    // (the derive_rollup-drops-a-repo defect class).
    write_alpha_state_block_dropped(&root);

    let report2 = mev::emit_state(&root, true, None).expect("second emit_state should not error");
    assert!(
        !report2.is_failure(),
        "second emit must succeed (it is a bad derivation, not a load failure); diagnostics: {:#?}",
        report2.diagnostics
    );
    let after_second = fs::read_to_string(brain_state_path(&root)).unwrap();
    assert!(
        !after_second.contains("Alpha block A"),
        "second write must have dropped the authored content from the live file; got: {after_second}"
    );

    // `apply_plan` is the single write point behind every planner
    // `emit_state` runs, so a single `emit_state(write=true)` call can touch
    // the same target path more than once (e.g. `plan_state_json` then
    // `plan_status_frontmatter`), recording more than one revision per call.
    // Find the revision whose content matches exactly what the first emit
    // left on disk — that is the snapshot the second emit's opening write
    // recorded, regardless of its exact `seq`.
    let revisions = history::list_revisions(&brain_state_path(&root)).unwrap();
    assert!(
        !revisions.is_empty(),
        "the second call's writes must have recorded at least one revision of prior content"
    );
    let matching = revisions
        .iter()
        .find(|r| {
            history::read_revision(&brain_state_path(&root), r.seq).unwrap()
                == after_first.as_bytes()
        })
        .unwrap_or_else(|| {
            panic!(
                "no recorded revision holds the exact first-emit content; revisions: {revisions:#?}"
            )
        });

    let restored_bytes = history::read_revision(&brain_state_path(&root), matching.seq).unwrap();
    assert_eq!(
        restored_bytes,
        after_first.as_bytes(),
        "the matched revision must hold exactly the content written by the first emit"
    );

    fs::write(brain_state_path(&root), &restored_bytes).unwrap();
    let after_restore = fs::read_to_string(brain_state_path(&root)).unwrap();
    assert!(
        after_restore.contains("Alpha block A"),
        "restoring revision 1 must bring the dropped content back"
    );
    assert_eq!(
        after_restore, after_first,
        "restored content must match the first write byte-for-byte"
    );
}

// ---------------------------------------------------------------------------
// Revision ordering across successive emits.
// ---------------------------------------------------------------------------

#[test]
fn successive_emits_record_revisions_in_order() {
    let root = temp_dir("ordering");
    write_brain_toml(&root, None);
    write_brain_state(&root);
    write_alpha_state_with_block(&root);

    mev::emit_state(&root, true, None).unwrap();
    // Change alpha's tracked block status so the second write produces
    // genuinely different content (not a no-op). `focus` is derived by
    // `emit_state` itself from `tracks`, so it is left untouched here —
    // reading it back after the first call and indexing into it would be
    // fragile (it may legitimately become empty once status leaves
    // `in_progress`).
    let mut alpha_state: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(root.join("repos/alpha/planning/state.json")).unwrap(),
    )
    .unwrap();
    alpha_state["tracks"][0]["blocks"][0]["status"] = serde_json::json!("blocked");
    write_json(&root, "repos/alpha/planning/state.json", &alpha_state);
    mev::emit_state(&root, true, None).unwrap();

    let revisions = history::list_revisions(&brain_state_path(&root)).unwrap();
    let seqs: Vec<u32> = revisions.iter().map(|r| r.seq).collect();
    assert_eq!(
        seqs,
        vec![1, 2],
        "two successive overwrites must yield revisions 1 then 2, ascending"
    );
}

// ---------------------------------------------------------------------------
// A no-op emit (identical planned content) must not churn a snapshot.
// ---------------------------------------------------------------------------

#[test]
fn noop_emit_records_no_new_revision() {
    let root = temp_dir("noop");
    write_brain_toml(&root, None);
    write_brain_state(&root);
    write_alpha_state_with_block(&root);

    mev::emit_state(&root, true, None).unwrap();
    let count_after_first = history::list_revisions(&brain_state_path(&root))
        .unwrap()
        .len();

    // Re-running emit_state against an unchanged fixture must be a fixed
    // point: apply_plan only pushes an action when content actually differs,
    // so a routine repeat run records no revision.
    mev::emit_state(&root, true, None).unwrap();
    let count_after_second = history::list_revisions(&brain_state_path(&root))
        .unwrap()
        .len();

    assert_eq!(
        count_after_first, count_after_second,
        "a no-op re-emit (planned content identical to on-disk) must not add a revision"
    );
}

// ---------------------------------------------------------------------------
// [history] keep caps the retained revisions to the newest N.
// ---------------------------------------------------------------------------

#[test]
fn keep_caps_retained_revisions_to_newest() {
    let root = temp_dir("keep-cap");
    write_brain_toml(&root, Some(2));
    write_brain_state(&root);
    write_alpha_state_with_block(&root);

    // Four emits, each mutating alpha's status so every write actually
    // differs from the last (avoids the no-op short-circuit).
    let statuses = ["in_progress", "blocked", "open", "in_progress"];
    for (i, status) in statuses.iter().enumerate() {
        let raw = fs::read_to_string(root.join("repos/alpha/planning/state.json")).unwrap();
        let mut alpha_state: serde_json::Value = serde_json::from_str(&raw).unwrap();
        // `focus` is derived by `emit_state` from `tracks` and is left
        // untouched here for the same reason as in
        // `successive_emits_record_revisions_in_order` — indexing into it
        // after a prior emit could hit an array that legitimately shrank.
        alpha_state["tracks"][0]["blocks"][0]["status"] = serde_json::json!(status);
        alpha_state["updated"] = serde_json::json!(format!("2026-08-{:02}", 3 + i));
        write_json(&root, "repos/alpha/planning/state.json", &alpha_state);
        mev::emit_state(&root, true, None).unwrap();
    }

    let revisions = history::list_revisions(&brain_state_path(&root)).unwrap();
    assert_eq!(
        revisions.len(),
        2,
        "keep = 2 must cap the retained revisions at 2 after four emits; got {revisions:#?}"
    );
    let seqs: Vec<u32> = revisions.iter().map(|r| r.seq).collect();
    assert_eq!(
        seqs,
        vec![3, 4],
        "pruning must keep the newest two revisions, not the oldest"
    );
}

// ---------------------------------------------------------------------------
// A dry-run emit against a fixture with existing history leaves it untouched.
// ---------------------------------------------------------------------------

#[test]
fn dry_run_leaves_existing_history_untouched() {
    let root = temp_dir("dry-run-untouched");
    write_brain_toml(&root, None);
    write_brain_state(&root);
    write_alpha_state_with_block(&root);

    mev::emit_state(&root, true, None).unwrap();
    write_alpha_state_block_dropped(&root);
    mev::emit_state(&root, true, None).unwrap();

    let before = history::list_revisions(&brain_state_path(&root)).unwrap();
    assert!(!before.is_empty());
    let content_before = fs::read_to_string(brain_state_path(&root)).unwrap();

    // A dry-run must not add, remove, or otherwise touch history, nor the
    // live file itself.
    mev::emit_state(&root, false, None).unwrap();

    let after = history::list_revisions(&brain_state_path(&root)).unwrap();
    assert_eq!(
        before, after,
        "a dry-run emit must leave existing history exactly as it found it"
    );
    let content_after = fs::read_to_string(brain_state_path(&root)).unwrap();
    assert_eq!(
        content_before, content_after,
        "a dry-run emit must not modify the live file"
    );
}

// ---------------------------------------------------------------------------
// Restoring a revision itself records the pre-restore content as a new one.
// ---------------------------------------------------------------------------

#[test]
fn restore_records_pre_restore_content_as_new_revision() {
    let root = temp_dir("restore-records");
    write_brain_toml(&root, None);
    write_brain_state(&root);
    write_alpha_state_with_block(&root);

    mev::emit_state(&root, true, None).unwrap();
    let after_first = fs::read(brain_state_path(&root)).unwrap();

    write_alpha_state_block_dropped(&root);
    mev::emit_state(&root, true, None).unwrap();

    let path = brain_state_path(&root);
    let current = fs::read(&path).unwrap();
    assert_ne!(
        current, after_first,
        "sanity: the second emit must have changed the live content"
    );

    // A single `emit_state(write=true)` call can write the same target path
    // more than once across its planners, so find the revision that holds
    // the exact first-emit content by matching bytes (mirrors
    // `dropped_authored_content_is_recoverable_via_restore`) rather than
    // assuming a fixed seq.
    let revisions_before = history::list_revisions(&path).unwrap();
    let restore_seq = revisions_before
        .iter()
        .find(|r| history::read_revision(&path, r.seq).unwrap() == after_first)
        .expect("a recorded revision must hold the exact first-emit content")
        .seq;

    // Perform a restore the way `mev state-history --restore` does: record
    // the current (pre-restore) content as a new revision, then write the
    // target revision's bytes back atomically.
    history::record_revision(&path, &current).unwrap();
    let restore_target = history::read_revision(&path, restore_seq).unwrap();
    assert_eq!(
        restore_target, after_first,
        "sanity: the chosen restore target must be exactly the first-emit content"
    );
    mev::brain::emit::write_atomic(&path, &restore_target).unwrap();

    let revisions_after = history::list_revisions(&path).unwrap();
    assert_eq!(
        revisions_after.len(),
        revisions_before.len() + 1,
        "the restore itself must have recorded exactly one new revision (the pre-restore content)"
    );
    let newest = revisions_after.last().unwrap();
    let recorded_content = history::read_revision(&path, newest.seq).unwrap();
    assert_eq!(
        recorded_content, current,
        "the newly recorded revision must hold exactly the pre-restore (dropped-content) bytes"
    );

    let restored_content = fs::read(&path).unwrap();
    assert_eq!(
        restored_content, restore_target,
        "the live file must now hold the restored revision's content"
    );

    // The pre-restore content (the dropped-content version) must itself be
    // recoverable — a wrong restore is undoable.
    let pre_restore_snapshot = history::read_revision(&path, newest.seq).unwrap();
    assert_eq!(pre_restore_snapshot, current);
}
