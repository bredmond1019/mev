//! Fixture suite pinning the three carryover-lane soundness fixes from
//! `MV.ticket.carryover-sweep-must-not-clear-what-it-never-evaluated`:
//!
//! 1. A prose `clears_when` must never land `Cleared`, even when every ref
//!    mined from it is satisfied (Task 1).
//! 2. A typed `command_exits_zero` predicate whose command could not run at
//!    all (exit 127/126) must land `NotEvaluable` with a reason distinct
//!    from both `CommandSpawnFailed` and `CommandTimedOut` — never
//!    `Actionable` (Task 2).
//! 3. A missing `cwd` must already route through `CommandSpawnFailed` (this
//!    is verified, not assumed).
//!
//! Every fixture drives `evaluate_carryover` directly (the same pipeline
//! piece `tests/it/brain_carryover_already_satisfied.rs` uses) against a
//! `tempfile`-backed corpus — never the real repo's own `state.json`.

use std::collections::HashMap;
use std::fs;

use mev::brain::state::{StateFile, StateSource};
use mev::{CarryoverLane, NotEvaluableReason, evaluate_carryover};

fn temp_dir(suffix: &str) -> std::path::PathBuf {
    let dir =
        mev::testsupport::unique_temp_dir(&format!("mev-carryover-lane-soundness-it-{suffix}"));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn make_source(path: &std::path::Path, repo_slug: &str) -> StateSource {
    StateSource {
        repo_slug: repo_slug.to_string(),
        abs_path: path.to_path_buf(),
        expected_kind: "project",
    }
}

fn parse_file(json: &str) -> StateFile {
    serde_json::from_str(json).expect("fixture state.json must parse")
}

/// Same shape as `brain_carryover_already_satisfied.rs`'s `evaluate_one`
/// helper: a single-file corpus, `own_repo`'s path mapped to `brain_root` so
/// path assertions and `command_exits_zero`'s `cwd` both resolve there.
fn evaluate_one(
    src: &StateSource,
    file: &StateFile,
    brain_root: &std::path::Path,
    status_map: &HashMap<String, Option<String>>,
    allow_exec: bool,
) -> mev::CarryoverReport {
    let files = vec![(src.clone(), file.clone())];
    let repo_paths: HashMap<String, std::path::PathBuf> =
        HashMap::from([(src.repo_slug.clone(), brain_root.to_path_buf())]);
    let cfg = mev::brain::config::AttentionThresholds::default();
    evaluate_carryover(
        &files,
        status_map,
        brain_root,
        &repo_paths,
        "2026-09-06",
        &cfg,
        None,
        allow_exec,
        mev::COMMAND_EXEC_TIMEOUT,
    )
}

fn lane_for<'a>(report: &'a mev::CarryoverReport, slug: &str) -> &'a mev::CarryoverVerdict {
    report
        .entries
        .iter()
        .find(|v| v.slug == slug)
        .unwrap_or_else(|| panic!("verdict for {slug} must exist"))
}

// ---------------------------------------------------------------------------
// 1. Prose `clears_when` whose mined refs are ALL satisfied — must be
//    NotEvaluable. Without the "all satisfied" part this fixture would pass
//    against the pre-fix evaluator too and prove nothing: a mined block ref
//    that closes AND a mined path ref that exists, both true at once.
// ---------------------------------------------------------------------------

#[test]
fn prose_clears_when_with_all_mined_refs_satisfied_is_not_evaluable() {
    let dir = temp_dir("prose-all-refs-satisfied");
    // The path half of the mined refs: a file that really exists.
    fs::create_dir_all(dir.join("notes")).unwrap();
    fs::write(dir.join("notes/marker.md"), b"marker").unwrap();

    let state_path = dir.join("state.json");
    let src = make_source(&state_path, "test");

    // The block half: MV.9.Z is present in this same file's tracks, with
    // status "closed" — so the mined block ref is satisfied too.
    let file = parse_file(
        r#"{"repo":"test","kind":"project","updated":"2026-09-06",
            "tracks":[{"title":"t","blocks":[{"id":"MV.9.Z","title":"x","status":"closed"}]}],
            "carryover":[{"slug":"prose-all-refs-satisfied","scope":{"repo":"test"},
                          "kind":"deferred",
                          "text":"Some free-form finding.",
                          "created":"2026-09-06",
                          "clears_when":"MV.9.Z ships and notes/marker.md exists."}]}"#,
    );

    let mut status_map: HashMap<String, Option<String>> = HashMap::new();
    status_map.insert("test:MV.9.Z".to_string(), Some("closed".to_string()));

    let report = evaluate_one(&src, &file, &dir, &status_map, false);
    let verdict = lane_for(&report, "prose-all-refs-satisfied");

    assert_eq!(
        verdict.lane,
        CarryoverLane::NotEvaluable,
        "a prose predicate must never land Cleared, even when every mined ref is satisfied: {verdict:?}"
    );
}

// ---------------------------------------------------------------------------
// 2. Prose `clears_when` of the recognisable "<path> exists." shape, where
//    the path DOES exist — must be NotEvaluable.
// ---------------------------------------------------------------------------

#[test]
fn prose_path_exists_shape_with_real_path_is_not_evaluable() {
    let dir = temp_dir("prose-path-exists-shape");
    fs::create_dir_all(dir.join("docs")).unwrap();
    fs::write(dir.join("docs/report.md"), b"content").unwrap();

    let state_path = dir.join("state.json");
    let src = make_source(&state_path, "test");

    let file = parse_file(
        r#"{"repo":"test","kind":"project","updated":"2026-09-06",
            "carryover":[{"slug":"path-exists-shape","scope":{"repo":"test"},
                          "kind":"deferred",
                          "text":"Some free-form finding.",
                          "created":"2026-09-06",
                          "clears_when":"docs/report.md exists."}]}"#,
    );

    let status_map: HashMap<String, Option<String>> = HashMap::new();
    let report = evaluate_one(&src, &file, &dir, &status_map, false);
    let verdict = lane_for(&report, "path-exists-shape");

    assert_eq!(
        verdict.lane,
        CarryoverLane::NotEvaluable,
        "a prose '<path> exists.' predicate must never land Cleared: {verdict:?}"
    );
}

// ---------------------------------------------------------------------------
// 3. Typed `command_exits_zero` whose command exits 1 — Actionable.
// ---------------------------------------------------------------------------

#[test]
fn typed_command_exits_one_is_actionable() {
    let dir = temp_dir("typed-exit-1");
    let state_path = dir.join("state.json");
    let src = make_source(&state_path, "test");

    let file = parse_file(
        r#"{"repo":"test","kind":"project","updated":"2026-09-06",
            "carryover":[{"slug":"typed-exit-1","scope":{"repo":"test"},
                          "kind":"defect",
                          "text":"A real, still-live finding.",
                          "created":"2026-09-06",
                          "clears_when":{"type":"command_exits_zero","command":"exit 1"}}]}"#,
    );

    let status_map: HashMap<String, Option<String>> = HashMap::new();
    let report = evaluate_one(&src, &file, &dir, &status_map, true);
    let verdict = lane_for(&report, "typed-exit-1");

    assert_eq!(
        verdict.lane,
        CarryoverLane::Actionable,
        "a command that genuinely exits 1 must land Actionable: {verdict:?}"
    );
}

// ---------------------------------------------------------------------------
// 4. Typed `command_exits_zero` whose command names a program that does not
//    exist (exit 127) — NotEvaluable with the Indeterminate reason, NOT the
//    spawn-failed one.
// ---------------------------------------------------------------------------

#[test]
fn typed_command_exit_127_is_not_evaluable_with_indeterminate_reason() {
    let dir = temp_dir("typed-exit-127");
    let state_path = dir.join("state.json");
    let src = make_source(&state_path, "test");

    let file = parse_file(
        r#"{"repo":"test","kind":"project","updated":"2026-09-06",
            "carryover":[{"slug":"typed-exit-127","scope":{"repo":"test"},
                          "kind":"defect",
                          "text":"A predicate naming a program that does not exist.",
                          "created":"2026-09-06",
                          "clears_when":{"type":"command_exits_zero",
                                          "command":"this-program-does-not-exist-anywhere-xyz"}}]}"#,
    );

    let status_map: HashMap<String, Option<String>> = HashMap::new();
    let report = evaluate_one(&src, &file, &dir, &status_map, true);
    let verdict = lane_for(&report, "typed-exit-127");

    assert_eq!(
        verdict.lane,
        CarryoverLane::NotEvaluable,
        "a command that cannot run at all must never land Actionable: {verdict:?}"
    );
    assert_eq!(
        verdict.reason,
        Some(NotEvaluableReason::CommandIndeterminate),
        "an exit-127 command must be reported Indeterminate, distinct from a spawn failure: {verdict:?}"
    );
}

// ---------------------------------------------------------------------------
// 5. Typed `command_exits_zero` whose cwd does not exist — NotEvaluable.
//    Asserts the CONCRETE reason (CommandSpawnFailed), so the task-2 claim
//    that this already routes through SpawnFailed is checked, not assumed.
// ---------------------------------------------------------------------------

#[test]
fn typed_command_with_missing_cwd_is_not_evaluable_via_spawn_failed() {
    // Do NOT create this directory — the whole point of the fixture is that
    // the owning repo's path (used as the command's cwd) does not exist.
    let dir = mev::testsupport::unique_temp_dir("mev-carryover-lane-soundness-it-missing-cwd");
    let state_path = dir.join("state.json");
    let src = make_source(&state_path, "test");

    let file = parse_file(
        r#"{"repo":"test","kind":"project","updated":"2026-09-06",
            "carryover":[{"slug":"missing-cwd","scope":{"repo":"test"},
                          "kind":"defect",
                          "text":"A predicate whose owning repo path is gone.",
                          "created":"2026-09-06",
                          "clears_when":{"type":"command_exits_zero","command":"exit 0"}}]}"#,
    );

    let status_map: HashMap<String, Option<String>> = HashMap::new();
    // Note: `dir` itself is used as `brain_root` below, but it is never
    // created, so both the brain root and the mapped repo path used as
    // `cwd` are missing.
    let report = evaluate_one(&src, &file, &dir, &status_map, true);
    let verdict = lane_for(&report, "missing-cwd");

    assert_eq!(
        verdict.lane,
        CarryoverLane::NotEvaluable,
        "a predicate whose cwd does not exist must never land Actionable or Cleared: {verdict:?}"
    );
    assert_eq!(
        verdict.reason,
        Some(NotEvaluableReason::CommandSpawnFailed),
        "a missing cwd must fail at spawn, distinct from Indeterminate: {verdict:?}"
    );
}

// ---------------------------------------------------------------------------
// 6. POSITIVE CONTROL: a typed predicate that is genuinely satisfied — must
//    be Cleared. Without this, a change that reported everything
//    NotEvaluable would pass every assertion above and the suite would be
//    worthless.
// ---------------------------------------------------------------------------

#[test]
fn typed_command_exits_zero_is_cleared() {
    let dir = temp_dir("typed-exit-0");
    let state_path = dir.join("state.json");
    let src = make_source(&state_path, "test");

    let file = parse_file(
        r#"{"repo":"test","kind":"project","updated":"2026-09-06",
            "carryover":[{"slug":"typed-exit-0","scope":{"repo":"test"},
                          "kind":"defect",
                          "text":"A finding that has genuinely been resolved.",
                          "created":"2026-09-06",
                          "clears_when":{"type":"command_exits_zero","command":"exit 0"}}]}"#,
    );

    let status_map: HashMap<String, Option<String>> = HashMap::new();
    let report = evaluate_one(&src, &file, &dir, &status_map, true);
    let verdict = lane_for(&report, "typed-exit-0");

    assert_eq!(
        verdict.lane,
        CarryoverLane::Cleared,
        "a genuinely satisfied typed predicate must still land Cleared: {verdict:?}"
    );
}
