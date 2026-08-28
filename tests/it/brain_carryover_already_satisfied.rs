//! Integration fixture tests for `W_STATE_CARRYOVER_ALREADY_SATISFIED`
//! (`MV.ticket.carryover-already-satisfied-gate`, Task 4).
//!
//! Reproduces the two real 2026-08-19 false-clear incidents as fixtures built
//! under `tempfile` — never against the live corpus, which changes shape as
//! later blocks land. Drives the same pipeline pieces
//! `validate_brain_state` composes from: `evaluate_carryover` (`src/brain/carryover.rs`)
//! feeding `check_carryover_already_satisfied` (`src/brain/state.rs`).

use std::collections::HashMap;
use std::fs;

use mev::brain::state::{StateFile, StateSource, check_carryover_already_satisfied};
use mev::{CarryoverLane, Report, Severity, evaluate_carryover};

fn temp_dir(suffix: &str) -> std::path::PathBuf {
    let dir =
        mev::testsupport::unique_temp_dir(&format!("mev-carryover-already-satisfied-it-{suffix}"));
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

/// Build a one-file `CarryoverReport` the way `validate_brain_state` does:
/// `allow_exec: false`, no `repo_filter`. Mirrors `src/brain/state.rs`'s own
/// `evaluate_one` unit-test helper, but from the integration side.
fn evaluate_one(
    src: &StateSource,
    file: &StateFile,
    brain_root: &std::path::Path,
) -> mev::CarryoverReport {
    let files = vec![(src.clone(), file.clone())];
    let status_map: HashMap<String, Option<String>> = HashMap::new();
    let repo_paths: HashMap<String, std::path::PathBuf> =
        HashMap::from([(src.repo_slug.clone(), brain_root.to_path_buf())]);
    let cfg = mev::brain::config::AttentionThresholds::default();
    evaluate_carryover(
        &files,
        &status_map,
        brain_root,
        &repo_paths,
        "2026-08-19",
        &cfg,
        None,
        false,
        mev::COMMAND_EXEC_TIMEOUT,
    )
}

// ---------------------------------------------------------------------------
// 1. SUB-CLASS A — the real `postgres-14-17-cleanup-pending` incident.
// ---------------------------------------------------------------------------

#[test]
fn subclass_a_unanchored_file_contains_matches_prose_and_fires() {
    let dir = temp_dir("subclass-a-fires");
    // Frontmatter still reads `status: draft`, but the runbook's own Phase 7
    // prose contains the literal string `status: archived` — the exact shape
    // that cleared this entry on 2026-08-19.
    fs::write(
        dir.join("runbook.md"),
        "---\nstatus: draft\n---\n\nPhase 7 flips the field to status: archived once done.\n",
    )
    .unwrap();
    let state_path = dir.join("state.json");
    let src = make_source(&state_path, "test");

    let file = parse_file(
        r#"{"repo":"mev","kind":"project","updated":"2026-08-19",
            "carryover":[{"slug":"postgres-14-17-cleanup-pending","scope":{"repo":"test"},
                          "kind":"deferred","text":"Postgres 14->17 cleanup pending.",
                          "created":"2026-08-19",
                          "clears_when":{"type":"file_contains","path":"runbook.md",
                                          "pattern":"status: archived"}}]}"#,
    );

    let report = evaluate_one(&src, &file, &dir);
    // Sanity: the predicate really did clear (that's the whole bug).
    let verdict = report
        .entries
        .iter()
        .find(|v| v.slug == "postgres-14-17-cleanup-pending")
        .expect("verdict must exist");
    assert_eq!(verdict.lane, CarryoverLane::Cleared);

    let d = check_carryover_already_satisfied(&src, &file, &report);
    assert_eq!(d.len(), 1, "unanchored prose match must warn: {d:?}");
    assert_eq!(d[0].locator, "W_STATE_CARRYOVER_ALREADY_SATISFIED");
    assert!(
        d[0].message.contains("SUB-CLASS A"),
        "message must name sub-class A: {}",
        d[0].message
    );
}

// ---------------------------------------------------------------------------
// 2. Positive control — the SAME fixture, anchored (as authored today), is
//    silent. Proves the check discriminates rather than always firing.
// ---------------------------------------------------------------------------

#[test]
fn subclass_a_anchored_pattern_does_not_fire() {
    let dir = temp_dir("subclass-a-anchored");
    // Same file as test 1 — frontmatter still `status: draft`, the string
    // `status: archived` appears only mid-sentence in prose, with no leading
    // newline directly before it.
    fs::write(
        dir.join("runbook.md"),
        "---\nstatus: draft\n---\n\nPhase 7 flips the field to status: archived once done.\n",
    )
    .unwrap();
    let state_path = dir.join("state.json");
    let src = make_source(&state_path, "test");

    // Leading '\n' anchors the pattern to a frontmatter field — it does NOT
    // match the mid-sentence prose occurrence, so the predicate stays
    // unsatisfied (unlike test 1's unanchored bare substring).
    let file = parse_file(
        r#"{"repo":"mev","kind":"project","updated":"2026-08-19",
            "carryover":[{"slug":"postgres-14-17-cleanup-pending","scope":{"repo":"test"},
                          "kind":"deferred","text":"Postgres 14->17 cleanup pending.",
                          "created":"2026-08-19",
                          "clears_when":{"type":"file_contains","path":"runbook.md",
                                          "pattern":"\nstatus: archived"}}]}"#,
    );

    let report = evaluate_one(&src, &file, &dir);
    let verdict = report
        .entries
        .iter()
        .find(|v| v.slug == "postgres-14-17-cleanup-pending")
        .expect("verdict must exist");
    assert_ne!(
        verdict.lane,
        CarryoverLane::Cleared,
        "anchored pattern must not match the mid-sentence prose occurrence"
    );

    let d = check_carryover_already_satisfied(&src, &file, &report);
    assert!(
        d.is_empty(),
        "anchoring the pattern fixes the false match — no warning should fire: {d:?}"
    );
}

// ---------------------------------------------------------------------------
// 3. SUB-CLASS B — the real `client-wild-trail-photo-missing-on-mini`
//    incident. Tier-level scope (`repo: null, tier: "client"`).
// ---------------------------------------------------------------------------

#[test]
fn subclass_b_path_resolves_locally_but_text_scopes_to_mini_fires() {
    let dir = temp_dir("subclass-b-fires");
    fs::write(dir.join("photo.jpg"), b"fake-bytes").unwrap();
    let state_path = dir.join("state.json");
    let src = make_source(&state_path, "test");

    let file = parse_file(
        r#"{"repo":"mev","kind":"project","updated":"2026-08-19",
            "carryover":[{"slug":"client-wild-trail-photo-missing-on-mini",
                          "scope":{"tier":"client"},
                          "kind":"defect",
                          "text":"Client Wild Trail photo missing on the Mac Mini.",
                          "created":"2026-08-19",
                          "clears_when":{"type":"file_exists","path":"photo.jpg"}}]}"#,
    );

    let report = evaluate_one(&src, &file, &dir);
    let verdict = report
        .entries
        .iter()
        .find(|v| v.slug == "client-wild-trail-photo-missing-on-mini")
        .expect("tier-scoped entry must still be evaluated");
    assert_eq!(verdict.lane, CarryoverLane::Cleared);

    let d = check_carryover_already_satisfied(&src, &file, &report);
    assert_eq!(
        d.len(),
        1,
        "path resolving locally on a finding scoped to another machine must warn: {d:?}"
    );
    assert!(
        d[0].message.contains("SUB-CLASS B"),
        "message must name sub-class B: {}",
        d[0].message
    );
}

// ---------------------------------------------------------------------------
// 4. Negative control — the same entry with today's deliberately-prose
//    `clears_when` (free-form string, not a typed predicate) is silent: it
//    lands not-evaluable, which is the outcome its author intended.
// ---------------------------------------------------------------------------

#[test]
fn subclass_b_entry_with_prose_clears_when_is_not_evaluable_and_silent() {
    let dir = temp_dir("subclass-b-prose");
    fs::write(dir.join("photo.jpg"), b"fake-bytes").unwrap();
    let state_path = dir.join("state.json");
    let src = make_source(&state_path, "test");

    let file = parse_file(
        r#"{"repo":"mev","kind":"project","updated":"2026-08-19",
            "carryover":[{"slug":"client-wild-trail-photo-missing-on-mini",
                          "scope":{"tier":"client"},
                          "kind":"defect",
                          "text":"Client Wild Trail photo missing on the Mac Mini.",
                          "created":"2026-08-19",
                          "clears_when":"confirm on the Mini that the photo now renders"}]}"#,
    );

    let report = evaluate_one(&src, &file, &dir);
    let verdict = report
        .entries
        .iter()
        .find(|v| v.slug == "client-wild-trail-photo-missing-on-mini")
        .expect("entry must still be evaluated (as not-evaluable)");
    assert_ne!(
        verdict.lane,
        CarryoverLane::Cleared,
        "free-form prose clears_when must never land Cleared"
    );

    let d = check_carryover_already_satisfied(&src, &file, &report);
    assert!(
        d.is_empty(),
        "not-evaluable prose predicate must never warn: {d:?}"
    );
}

// ---------------------------------------------------------------------------
// 5. A healthy live entry with an unsatisfied typed predicate is silent.
// ---------------------------------------------------------------------------

#[test]
fn healthy_unsatisfied_predicate_is_silent() {
    let dir = temp_dir("healthy-unsatisfied");
    // marker.txt deliberately absent.
    let state_path = dir.join("state.json");
    let src = make_source(&state_path, "test");

    let file = parse_file(
        r#"{"repo":"mev","kind":"project","updated":"2026-08-19",
            "carryover":[{"slug":"still-pending","scope":{"repo":"test"},"kind":"deferred",
                          "text":"Not resolved yet.","created":"2026-08-19",
                          "clears_when":{"type":"file_exists","path":"marker.txt"}}]}"#,
    );

    let report = evaluate_one(&src, &file, &dir);
    let d = check_carryover_already_satisfied(&src, &file, &report);
    assert!(
        d.is_empty(),
        "a live entry with an unsatisfied predicate must not warn: {d:?}"
    );
}

// ---------------------------------------------------------------------------
// 6. Severity: Warning, and a Report containing it is not a failure.
// ---------------------------------------------------------------------------

#[test]
fn diagnostic_severity_is_warning_and_never_fails_the_state_pass() {
    let dir = temp_dir("severity");
    fs::write(dir.join("marker.txt"), "hello").unwrap();
    let state_path = dir.join("state.json");
    let src = make_source(&state_path, "test");

    let file = parse_file(
        r#"{"repo":"mev","kind":"project","updated":"2026-08-19",
            "carryover":[{"slug":"already-done","scope":{"repo":"test"},"kind":"deferred",
                          "text":"x","created":"2026-08-19",
                          "clears_when":{"type":"file_exists","path":"marker.txt"}}]}"#,
    );

    let report = evaluate_one(&src, &file, &dir);
    let d = check_carryover_already_satisfied(&src, &file, &report);
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].severity, Severity::Warning);

    let mut rep = Report::default();
    rep.diagnostics.extend(d);
    assert!(
        !rep.is_failure(),
        "W_STATE_CARRYOVER_ALREADY_SATISFIED must never fail validate-brain --state"
    );
}

// ---------------------------------------------------------------------------
// 7. `command_exits_zero` is never evaluated by the validator (allow_exec is
//    false), so it must never produce this warning — pinned explicitly so a
//    later change to `allow_exec` in the validator breaks a test rather than
//    silently starting to execute corpus-authored shell.
// ---------------------------------------------------------------------------

#[test]
fn command_exits_zero_predicate_never_fires_because_exec_is_disabled() {
    let dir = temp_dir("no-exec");
    let state_path = dir.join("state.json");
    let src = make_source(&state_path, "test");

    // `true` always exits 0 — if exec were enabled this would clear.
    let file = parse_file(
        r#"{"repo":"mev","kind":"project","updated":"2026-08-19",
            "carryover":[{"slug":"exec-guarded","scope":{"repo":"test"},"kind":"deferred",
                          "text":"x","created":"2026-08-19",
                          "clears_when":{"type":"command_exits_zero","command":"true"}}]}"#,
    );

    let report = evaluate_one(&src, &file, &dir);
    let verdict = report
        .entries
        .iter()
        .find(|v| v.slug == "exec-guarded")
        .expect("entry must still be evaluated (as not-evaluable, exec disabled)");
    assert_ne!(
        verdict.lane,
        CarryoverLane::Cleared,
        "command_exits_zero must never clear while allow_exec is false"
    );

    let d = check_carryover_already_satisfied(&src, &file, &report);
    assert!(
        d.is_empty(),
        "command_exits_zero must never warn while the validator runs with exec disabled: {d:?}"
    );
}
