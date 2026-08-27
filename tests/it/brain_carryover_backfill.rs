//! Integration tests for `mev carryover --backfill` (and `--backfill --dry-run`) over
//! real git-history fixtures — `MV.16.B`, Task 5.
//!
//! Drives the public building blocks task 1-3 add (`enumerate_historical_removals`,
//! `run_backfill`, `build_historical_archive_row`, `derive_disposal_reason`), since the
//! CLI driver itself (`run_carryover_backfill`) lives in the `mev` binary and is not
//! reachable from an integration test crate — this is the same shape
//! `tests/brain_carryover_dispose.rs` already establishes for `--dispose`.
//!
//! The live HQ history is NOT the assertion surface here — it grows every day, so any
//! assertion against a live removal count would be red tomorrow. Every case below builds
//! its own real git repository in a tempdir.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use mev::brain::carryover::{
    BackfillCollision, HistoryWalkPlan, archive_path_for, build_historical_archive_row,
    derive_disposal_reason, enumerate_historical_removals, run_backfill,
};
use mev::testsupport::{git_command, unique_temp_dir};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn temp_dir(suffix: &str) -> PathBuf {
    let dir = unique_temp_dir(&format!("mev-carryover-backfill-it-{suffix}"));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_json(root: &Path, rel: &str, value: &serde_json::Value) {
    let full = root.join(rel);
    fs::create_dir_all(full.parent().unwrap()).unwrap();
    fs::write(
        &full,
        serde_json::to_string_pretty(value).unwrap().as_bytes(),
    )
    .unwrap();
}

fn run_git(dir: &Path, args: &[&str]) {
    let output = git_command()
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

fn git_init(dir: &Path) {
    run_git(dir, &["init", "-q"]);
    run_git(dir, &["config", "user.email", "test@example.com"]);
    run_git(dir, &["config", "user.name", "Test"]);
}

fn git_commit_all(dir: &Path, message: &str) {
    run_git(dir, &["add", "-A"]);
    run_git(dir, &["commit", "-q", "-m", message]);
}

/// Write a minimal `brain.toml` registering every slug in `repos` as a leaf `[[repos]]`
/// entry, following the shape `tests/brain_carryover_dispose.rs` already establishes.
fn write_brain_toml(root: &Path, repos: &[&str]) {
    let mut toml = String::from(
        r#"[vocab]
layer = ["brain", "engine", "factory", "console", "surface", "infra", "business", "content", "meta"]
status = ["active", "draft", "deprecated", "superseded", "archived"]

[crawl]
skip_dirs = ["target", "node_modules", ".git"]

"#,
    );
    for slug in repos {
        toml.push_str(&format!(
            r#"[[repos]]
slug = "{slug}"
tier = "primary"
repo_path = "repos/{slug}"
status_file = "repos/{slug}/planning/status.md"
cache_doc = "docs/projects/{slug}.md"
heading = "{slug}"

"#
        ));
    }
    fs::write(root.join("brain.toml"), toml.as_bytes()).unwrap();
}

/// Build a minimal, otherwise-valid `state.json` value for `repo` carrying exactly
/// `carryover` as its `carryover[]` array.
fn state_value(repo: &str, carryover: Vec<serde_json::Value>) -> serde_json::Value {
    serde_json::json!({
        "repo": repo,
        "kind": "project",
        "updated": "2026-08-01",
        "focus": { "now": [], "next": [], "blocked": [] },
        "tracks": [],
        "carryover": carryover
    })
}

/// One carryover entry, as a `serde_json::Value`, with `slug` and `text` filled in and
/// every other required field defaulted. `extra` (optional) is merged in on top, so a
/// caller can attach a key `Carryover` does not model.
fn entry_value(
    slug: &str,
    text: &str,
    extra: Option<(&str, serde_json::Value)>,
) -> serde_json::Value {
    let mut v = serde_json::json!({
        "slug": slug,
        "scope": { "repo": "alpha" },
        "kind": "deferred",
        "text": text,
        "created": "2026-06-01"
    });
    if let Some((key, val)) = extra {
        v.as_object_mut().unwrap().insert(key.to_string(), val);
    }
    v
}

fn write_alpha_state(root: &Path, carryover: Vec<serde_json::Value>) {
    write_json(
        root,
        "repos/alpha/planning/state.json",
        &state_value("alpha", carryover),
    );
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// A commit removing one entry yields exactly one row, and the row's embedded entry is
/// byte-equal (as parsed structure) to the parent blob's version — including a key
/// `Carryover` does not model, put in the fixture on purpose.
#[test]
fn single_removal_row_is_byte_equal_including_unmodeled_key() {
    let dir = temp_dir("single-removal");
    write_brain_toml(&dir, &["alpha"]);

    let target = entry_value(
        "alpha-target",
        "Alpha is waiting on something.",
        Some(("legacy_field", serde_json::json!("unmodeled-value"))),
    );
    let kept = entry_value("alpha-kept", "Alpha keeps this one.", None);

    git_init(&dir);
    write_alpha_state(&dir, vec![target.clone(), kept.clone()]);
    git_commit_all(&dir, "chore: seed alpha carryover");

    write_alpha_state(&dir, vec![kept.clone()]);
    git_commit_all(&dir, "chore: drop alpha-target");

    let plan = enumerate_historical_removals(&dir, None).expect("walk should succeed");
    assert_eq!(
        plan.removals.len(),
        1,
        "expected exactly one removal, got: {:#?}",
        plan.removals
    );
    let removal = &plan.removals[0];
    assert_eq!(removal.repo, "alpha");

    let row = build_historical_archive_row(removal);
    // Compare against the entry as `Carryover` itself parses it (not the raw authored
    // JSON) — parsing fills in `#[serde(default)]` fields (`related: []`, `scope.tier:
    // null`, ...) that are absent from the hand-authored fixture value but present once
    // round-tripped through the real type. What must hold is that the ARCHIVED entry is
    // exactly what `Carryover` would parse the parent blob's version as — i.e. nothing
    // was re-synthesized from a subset of fields.
    let expected_entry: okf_core::Carryover = serde_json::from_value(target.clone()).unwrap();
    let expected_as_value = serde_json::to_value(&expected_entry).unwrap();
    let entry_as_value = serde_json::to_value(&row.entry).unwrap();
    assert_eq!(
        entry_as_value, expected_as_value,
        "the archived entry must be byte-equal to the parent blob's version, including \
         the unmodeled `legacy_field` key"
    );
    assert!(
        row.entry.extra.contains_key("legacy_field"),
        "the unmodeled key must survive into Carryover::extra, got: {:#?}",
        row.entry.extra
    );
    assert!(
        row.reconstructed,
        "every backfilled row must be reconstructed=true"
    );

    let _ = fs::remove_dir_all(&dir);
}

/// A commit removing three entries yields three rows, all from the same commit.
#[test]
fn multi_removal_commit_yields_three_rows() {
    let dir = temp_dir("multi-removal");
    write_brain_toml(&dir, &["alpha"]);

    let x = entry_value("alpha-x", "X", None);
    let y = entry_value("alpha-y", "Y", None);
    let z = entry_value("alpha-z", "Z", None);
    let kept = entry_value("alpha-kept", "Kept", None);

    git_init(&dir);
    write_alpha_state(&dir, vec![x, y, z, kept.clone()]);
    git_commit_all(&dir, "chore: seed alpha carryover");

    write_alpha_state(&dir, vec![kept]);
    git_commit_all(&dir, "chore: bulk drop x, y, z");

    let plan = enumerate_historical_removals(&dir, None).expect("walk should succeed");
    assert_eq!(
        plan.removals.len(),
        3,
        "expected exactly three removals from one commit, got: {:#?}",
        plan.removals
    );
    let mut slugs: Vec<&str> = plan
        .removals
        .iter()
        .map(|r| r.entry.slug.as_str())
        .collect();
    slugs.sort_unstable();
    assert_eq!(slugs, vec!["alpha-x", "alpha-y", "alpha-z"]);

    let _ = fs::remove_dir_all(&dir);
}

/// A commit that only ADDS an entry yields no row, and a commit that only EDITS an
/// existing entry (same slug survives) yields no row either.
#[test]
fn add_only_and_edit_only_commits_yield_no_rows() {
    let dir = temp_dir("add-edit-only");
    write_brain_toml(&dir, &["alpha"]);

    let a = entry_value("alpha-a", "Original text.", None);

    git_init(&dir);
    write_alpha_state(&dir, vec![a.clone()]);
    git_commit_all(&dir, "chore: seed alpha-a");

    // Add-only: alpha-b appears, alpha-a untouched.
    let b = entry_value("alpha-b", "New entry.", None);
    write_alpha_state(&dir, vec![a.clone(), b.clone()]);
    git_commit_all(&dir, "chore: add alpha-b");

    // Edit-only: alpha-a's text changes but the slug survives.
    let a_edited = entry_value("alpha-a", "Edited text.", None);
    write_alpha_state(&dir, vec![a_edited, b]);
    git_commit_all(&dir, "chore: edit alpha-a text");

    let plan = enumerate_historical_removals(&dir, None).expect("walk should succeed");
    assert!(
        plan.removals.is_empty(),
        "add-only and edit-only commits must yield no removals, got: {:#?}",
        plan.removals
    );

    let _ = fs::remove_dir_all(&dir);
}

/// A removing commit whose subject names a disposal reason maps to that
/// `DisposalReason`; changing the fixture's subject changes the mapped reason
/// (the shown-failing case). A subject naming none maps to `Withdrawn`, with `evidence`
/// recording that the reason was not attributable.
#[test]
fn reason_is_derived_from_commit_subject_and_defaults_to_withdrawn() {
    // Direct unit-level check of the matcher itself, independent of git.
    let (reason, attributable) = derive_disposal_reason("fix: resolve alpha-target issue");
    assert_eq!(reason, okf_core::DisposalReason::Cleared);
    assert!(attributable);

    let (reason, attributable) = derive_disposal_reason("chore: drop alpha-target");
    assert_eq!(reason, okf_core::DisposalReason::Withdrawn);
    assert!(!attributable);

    // Now the shown-failing case over a real fixture: same removal, different subject.
    let dir = temp_dir("reason-cleared");
    write_brain_toml(&dir, &["alpha"]);
    let target = entry_value("alpha-target", "Target entry.", None);
    let kept = entry_value("alpha-kept", "Kept.", None);

    git_init(&dir);
    write_alpha_state(&dir, vec![target.clone(), kept.clone()]);
    git_commit_all(&dir, "chore: seed alpha carryover");
    write_alpha_state(&dir, vec![kept.clone()]);
    git_commit_all(&dir, "fix: resolve alpha-target issue");

    let plan = enumerate_historical_removals(&dir, None).expect("walk should succeed");
    assert_eq!(plan.removals.len(), 1);
    let row = build_historical_archive_row(&plan.removals[0]);
    assert_eq!(row.reason, okf_core::DisposalReason::Cleared);
    assert!(
        !row.evidence
            .as_deref()
            .unwrap_or_default()
            .contains("not attributable"),
        "an attributable reason must not carry the not-attributable note, got: {:?}",
        row.evidence
    );
    let _ = fs::remove_dir_all(&dir);

    // Same shape, but a subject naming nothing: mapped reason changes to Withdrawn.
    let dir2 = temp_dir("reason-withdrawn");
    write_brain_toml(&dir2, &["alpha"]);
    git_init(&dir2);
    write_alpha_state(&dir2, vec![target, kept.clone()]);
    git_commit_all(&dir2, "chore: seed alpha carryover");
    write_alpha_state(&dir2, vec![kept]);
    git_commit_all(&dir2, "chore: drop alpha-target");

    let plan2 = enumerate_historical_removals(&dir2, None).expect("walk should succeed");
    assert_eq!(plan2.removals.len(), 1);
    let row2 = build_historical_archive_row(&plan2.removals[0]);
    assert_eq!(
        row2.reason,
        okf_core::DisposalReason::Withdrawn,
        "changing the commit subject to name no reason must change the mapped reason"
    );
    assert!(
        row2.evidence
            .as_deref()
            .unwrap_or_default()
            .contains("not attributable"),
        "evidence must say the reason was not attributable, got: {:?}",
        row2.evidence
    );
    assert!(row2.reconstructed);

    let _ = fs::remove_dir_all(&dir2);
}

/// A second `--backfill` run over a populated archive exits with a collision naming the
/// `(slug, disposed_at)` pair, and leaves the archive file byte-identical. Deleting the
/// archive makes the same invocation succeed.
#[test]
fn second_run_over_populated_archive_refuses_and_deleting_archive_lets_it_succeed() {
    let dir = temp_dir("rerun-refusal");
    write_brain_toml(&dir, &["alpha"]);
    let target = entry_value("alpha-target", "Target.", None);
    let kept = entry_value("alpha-kept", "Kept.", None);

    git_init(&dir);
    write_alpha_state(&dir, vec![target, kept.clone()]);
    git_commit_all(&dir, "chore: seed alpha carryover");
    write_alpha_state(&dir, vec![kept]);
    git_commit_all(&dir, "chore: drop alpha-target");

    let plan = enumerate_historical_removals(&dir, None).expect("walk should succeed");
    assert_eq!(plan.removals.len(), 1);

    let report = run_backfill(&plan, false).expect("first run should succeed");
    assert!(report.succeeded());
    let archive_path = archive_path_for(&dir.join("repos/alpha/planning/state.json"));
    assert!(archive_path.exists());
    let after_first_run = fs::read_to_string(&archive_path).unwrap();

    // Re-run the identical plan (git history is unchanged) — must refuse.
    let plan2 = enumerate_historical_removals(&dir, None).expect("walk should succeed");
    let err =
        run_backfill(&plan2, false).expect_err("second run over a populated archive must refuse");
    let BackfillCollision {
        repo,
        slug,
        disposed_at,
        ..
    } = &err;
    assert_eq!(repo, "alpha");
    assert_eq!(slug, "alpha-target");
    assert!(!disposed_at.is_empty());

    // Archive file must be byte-identical to what the first run wrote.
    assert_eq!(
        fs::read_to_string(&archive_path).unwrap(),
        after_first_run,
        "a refused second run must leave the archive byte-identical"
    );

    // Delete the archive and the same invocation now succeeds.
    fs::remove_file(&archive_path).unwrap();
    let plan3 = enumerate_historical_removals(&dir, None).expect("walk should succeed");
    let report3 = run_backfill(&plan3, false).expect("run over a missing archive must succeed");
    assert!(report3.succeeded());
    assert!(archive_path.exists());

    let _ = fs::remove_dir_all(&dir);
}

/// `--backfill --dry-run` computes the identical plan and writes nothing — the fixture
/// tree is byte-identical after the run, and a subsequent real run still succeeds.
#[test]
fn dry_run_writes_nothing_and_real_run_still_succeeds_after() {
    let dir = temp_dir("dry-run");
    write_brain_toml(&dir, &["alpha"]);
    let target = entry_value("alpha-target", "Target.", None);
    let kept = entry_value("alpha-kept", "Kept.", None);

    git_init(&dir);
    write_alpha_state(&dir, vec![target, kept.clone()]);
    git_commit_all(&dir, "chore: seed alpha carryover");
    write_alpha_state(&dir, vec![kept]);
    git_commit_all(&dir, "chore: drop alpha-target");

    let archive_path = archive_path_for(&dir.join("repos/alpha/planning/state.json"));

    let plan = enumerate_historical_removals(&dir, None).expect("walk should succeed");
    let dry_report = run_backfill(&plan, true).expect("dry-run should succeed");
    assert!(dry_report.succeeded());
    assert!(
        !archive_path.exists(),
        "dry-run must never create the archive file"
    );
    assert_eq!(dry_report.writes.len(), 1);
    assert!(
        !dry_report.writes[0].written,
        "dry-run must never mark a repo as written"
    );
    assert_eq!(dry_report.writes[0].rows.len(), 1);

    // A real run afterward still succeeds and actually writes.
    let plan2 = enumerate_historical_removals(&dir, None).expect("walk should succeed");
    let real_report = run_backfill(&plan2, false).expect("real run after a dry-run should succeed");
    assert!(real_report.succeeded());
    assert!(archive_path.exists());

    let _ = fs::remove_dir_all(&dir);
}

/// A repo whose archive file cannot be written (its `planning/` dir is read-only) aborts
/// that repo with the file reverted to its original (nonexistent) state, and the run
/// reports a failure rather than exiting cleanly.
#[test]
fn unwritable_archive_aborts_and_reverts() {
    let dir = temp_dir("unwritable");
    write_brain_toml(&dir, &["alpha"]);
    let target = entry_value("alpha-target", "Target.", None);
    let kept = entry_value("alpha-kept", "Kept.", None);

    git_init(&dir);
    write_alpha_state(&dir, vec![target, kept.clone()]);
    git_commit_all(&dir, "chore: seed alpha carryover");
    write_alpha_state(&dir, vec![kept]);
    git_commit_all(&dir, "chore: drop alpha-target");

    let plan = enumerate_historical_removals(&dir, None).expect("walk should succeed");
    assert_eq!(plan.removals.len(), 1);

    let alpha_planning_dir = dir.join("repos/alpha/planning");
    let archive_path = archive_path_for(&dir.join("repos/alpha/planning/state.json"));
    assert!(!archive_path.exists(), "archive must not exist yet");

    let mut perms = fs::metadata(&alpha_planning_dir).unwrap().permissions();
    let original_mode = perms.mode();
    perms.set_mode(0o555);
    fs::set_permissions(&alpha_planning_dir, perms).unwrap();

    let report = run_backfill(&plan, false).expect("guard should not fire — no prior archive");

    // Restore permissions immediately so cleanup can proceed regardless of outcome below.
    let mut restore = fs::metadata(&alpha_planning_dir).unwrap().permissions();
    restore.set_mode(original_mode);
    fs::set_permissions(&alpha_planning_dir, restore).unwrap();

    assert!(
        !report.succeeded(),
        "a repo whose write fails must be reported as a failure"
    );
    assert_eq!(report.failures.len(), 1);
    assert_eq!(report.failures[0].repo, "alpha");
    assert!(
        !archive_path.exists(),
        "the archive write must be reverted to its prior (nonexistent) state, found: {}",
        archive_path.display()
    );

    let _ = fs::remove_dir_all(&dir);
}

/// `--repo <slug>` restricts both the walk and the writes to that repo's state file —
/// a sibling repo's removals are neither enumerated nor archived.
#[test]
fn repo_filter_restricts_walk_and_writes() {
    let dir = temp_dir("repo-filter");
    write_brain_toml(&dir, &["alpha", "beta"]);

    let alpha_target = entry_value("alpha-target", "Alpha target.", None);
    let alpha_kept = entry_value("alpha-kept", "Alpha kept.", None);
    let beta_target = serde_json::json!({
        "slug": "beta-target",
        "scope": { "repo": "beta" },
        "kind": "deferred",
        "text": "Beta target.",
        "created": "2026-06-01"
    });
    let beta_kept = serde_json::json!({
        "slug": "beta-kept",
        "scope": { "repo": "beta" },
        "kind": "deferred",
        "text": "Beta kept.",
        "created": "2026-06-01"
    });

    git_init(&dir);
    write_alpha_state(&dir, vec![alpha_target, alpha_kept.clone()]);
    write_json(
        &dir,
        "repos/beta/planning/state.json",
        &state_value("beta", vec![beta_target, beta_kept.clone()]),
    );
    git_commit_all(&dir, "chore: seed alpha and beta carryover");

    write_alpha_state(&dir, vec![alpha_kept]);
    write_json(
        &dir,
        "repos/beta/planning/state.json",
        &state_value("beta", vec![beta_kept]),
    );
    git_commit_all(&dir, "chore: drop alpha-target and beta-target");

    let plan = enumerate_historical_removals(&dir, Some("alpha")).expect("walk should succeed");
    assert_eq!(plan.removals.len(), 1, "got: {:#?}", plan.removals);
    assert_eq!(plan.removals[0].repo, "alpha");
    assert_eq!(plan.removals[0].entry.slug, "alpha-target");

    let report = run_backfill(&plan, false).expect("run should succeed");
    assert!(report.succeeded());
    assert_eq!(report.writes.len(), 1);
    assert_eq!(report.writes[0].repo, "alpha");

    let alpha_archive = archive_path_for(&dir.join("repos/alpha/planning/state.json"));
    let beta_archive = archive_path_for(&dir.join("repos/beta/planning/state.json"));
    assert!(alpha_archive.exists());
    assert!(
        !beta_archive.exists(),
        "--repo alpha must never touch beta's archive"
    );

    let _ = fs::remove_dir_all(&dir);
}

/// Every emitted line deserializes back through okf-core's `CarryoverArchiveRow` — the
/// nested flatten over `Carryover`'s catch-all `extra` is the specific hazard okf-core's
/// own doc comment warns about, and a round-trip through the real type is the only thing
/// that catches a key collision between the row's own fields and an entry's unmodeled
/// ones.
#[test]
fn emitted_rows_round_trip_through_carryover_archive_row() {
    let dir = temp_dir("round-trip");
    write_brain_toml(&dir, &["alpha"]);

    let target = entry_value(
        "alpha-target",
        "Target with an unmodeled key.",
        Some(("legacy_field", serde_json::json!({"nested": "value"}))),
    );
    let kept = entry_value("alpha-kept", "Kept.", None);

    git_init(&dir);
    write_alpha_state(&dir, vec![target, kept.clone()]);
    git_commit_all(&dir, "chore: seed alpha carryover");
    write_alpha_state(&dir, vec![kept]);
    git_commit_all(&dir, "chore: drop alpha-target");

    let plan: HistoryWalkPlan =
        enumerate_historical_removals(&dir, None).expect("walk should succeed");
    assert_eq!(plan.removals.len(), 1);
    let report = run_backfill(&plan, false).expect("run should succeed");
    assert!(report.succeeded());

    let archive_path = archive_path_for(&dir.join("repos/alpha/planning/state.json"));
    let content = fs::read_to_string(&archive_path).unwrap();
    let lines: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 1);

    let row: okf_core::CarryoverArchiveRow = serde_json::from_str(lines[0])
        .expect("emitted line must deserialize as CarryoverArchiveRow");
    assert_eq!(row.entry.slug, "alpha-target");
    assert!(row.reconstructed);
    assert_eq!(row.reason, okf_core::DisposalReason::Withdrawn);
    assert!(
        row.entry.extra.contains_key("legacy_field"),
        "the unmodeled key must survive the round-trip in entry.extra, got: {:#?}",
        row.entry.extra
    );
    // The five archive-level keys must never leak into entry.extra (the exact hazard
    // okf-core's own doc comment on `CarryoverArchiveRow` warns about).
    for key in [
        "disposed_at",
        "reason",
        "reconstructed",
        "evidence",
        "amends",
    ] {
        assert!(
            !row.entry.extra.contains_key(key),
            "archive-level key {key:?} leaked into entry.extra"
        );
    }

    let _ = fs::remove_dir_all(&dir);
}
