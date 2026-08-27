//! Integration tests for `mev carryover --dispose` (and `--dispose --dry-run`) over a
//! temp-dir corpus fixture — `MV.ticket.carryover-dispose`, Task 5.
//!
//! Drives the same public building blocks `src/main.rs`'s `run_carryover_dispose` composes
//! (discovery → load-with-errors → evaluate → `compute_disposal_plan` → `run_dispose` /
//! `dispose_repo`), since the CLI driver itself lives in the `mev` binary and is not
//! reachable from an integration test crate.
//!
//! Cases (one per Task 5 acceptance-criteria letter):
//!   (a) a CLEARED entry is removed from `carryover[]` and a matching archive row is
//!       appended, carrying the entry verbatim plus `disposed_at`, `reason: cleared`,
//!       `reconstructed: false`, `evidence`.
//!   (b) the rest of `state.json` is byte-identical apart from the removed element.
//!   (c) an ACTIONABLE and a NOT-EVALUABLE entry both survive.
//!   (d) a repo whose `state.json` fails to parse is named in the output and BOTH its
//!       files are unmodified, while a sibling repo is still disposed in the same run.
//!   (e) a `command_exits_zero` entry without `--allow-exec` is NotEvaluable and survives
//!       `--dispose`.
//!   (f) `--dispose --dry-run` prints the identical disposal list and leaves every
//!       `state.json` and `carryover-archive.jsonl` byte-identical.
//!   (g) a `state.json` containing em dashes and other non-ASCII round-trips unchanged.
//!   (h) the printed output contains the full text of each disposed entry and a
//!       `git commit -o` pathspec naming both written files.
//!   Shown-failing atomicity case: a write failure between the two writes leaves no
//!       removed-but-unarchived entry — `state.json` is unchanged and well-formed.

use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use mev::brain::carryover::{
    CarryoverLane, DisposalCandidate, archive_path_for, compute_disposal_plan, dispose_repo,
    render_commit_pathspec, render_dispose_preamble, render_dispose_summary, run_dispose,
};
use mev::brain::config::find_brain_config;
use mev::brain::state::{StateFile, StateSource, discover_state_files, load_state};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn temp_dir(suffix: &str) -> PathBuf {
    let dir = mev::testsupport::unique_temp_dir(&format!("mev-carryover-dispose-it-{suffix}"));
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

fn write_raw(root: &Path, rel: &str, content: &str) {
    let full = root.join(rel);
    fs::create_dir_all(full.parent().unwrap()).unwrap();
    fs::write(&full, content.as_bytes()).unwrap();
}

/// Write `brain.toml` registering every slug in `repos` as a leaf `[[repos]]` entry.
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

/// `alpha`'s state: a cleared block-ref entry, an actionable block-ref entry, and a
/// prose-only not-evaluable entry — mirroring `tests/brain_carryover.rs`'s fixture shape.
/// Also carries an em dash and other non-ASCII in `text`, so a disposal run's untouched
/// entries double as case (g)'s round-trip check.
fn alpha_state_value() -> serde_json::Value {
    serde_json::json!({
        "repo": "alpha",
        "kind": "project",
        "updated": "2026-08-01",
        "focus": { "now": [], "next": [], "blocked": [] },
        "tracks": [
            {
                "title": "Phase 1",
                "blocks": [
                    { "id": "AL.1.A", "title": "Alpha block A", "status": "open" }
                ]
            }
        ],
        "carryover": [
            {
                "slug": "alpha-cleared",
                "scope": { "repo": "alpha" },
                "kind": "known_issue",
                "text": "Alpha was waiting on beta's block A landing — an em dash lives right here.",
                "related": [
                    { "type": "block", "repo": "beta", "id": "BE.1.A" }
                ],
                "clears_when": "BE.1.A lands",
                "created": "2026-06-01"
            },
            {
                "slug": "alpha-actionable",
                "scope": { "repo": "alpha" },
                "kind": "deferred",
                "text": "Alpha is waiting on beta's block B landing.",
                "clears_when": "BE.1.B lands",
                "created": "2026-06-01"
            },
            {
                "slug": "alpha-prose-only",
                "scope": { "repo": "alpha" },
                "kind": "constraint",
                "text": "Needs a human to review the approach manually.",
                "clears_when": "a human reviews this manually and signs off",
                "created": "2026-06-01"
            }
        ]
    })
}

fn beta_state_value() -> serde_json::Value {
    serde_json::json!({
        "repo": "beta",
        "kind": "project",
        "updated": "2026-08-01",
        "focus": { "now": [], "next": [], "blocked": [] },
        "tracks": [
            {
                "title": "Phase 1",
                "blocks": [
                    { "id": "BE.1.A", "title": "Beta block A", "status": "closed" },
                    { "id": "BE.1.B", "title": "Beta block B", "status": "open" }
                ]
            }
        ],
        "carryover": []
    })
}

fn write_alpha_beta_fixture(root: &Path) {
    write_brain_toml(root, &["alpha", "beta"]);
    write_json(
        root,
        "repos/alpha/planning/state.json",
        &alpha_state_value(),
    );
    write_json(root, "repos/beta/planning/state.json", &beta_state_value());
}

/// Run the discovery → load(-with-errors) → evaluate pipeline exactly as
/// `src/main.rs`'s `load_and_evaluate_carryover_corpus_for_dispose` does, using only the
/// public API surface available to an integration test.
#[allow(clippy::type_complexity)]
fn load_and_evaluate_for_dispose(
    root: &Path,
    allow_exec: bool,
) -> (
    Vec<(StateSource, StateFile)>,
    Vec<(String, String)>,
    mev::CarryoverReport,
) {
    let config = find_brain_config(root).expect("brain.toml should load");
    let (sources, _diags) = discover_state_files(root, &config);

    let mut loaded: Vec<(StateSource, StateFile)> = Vec::new();
    let mut load_errors: Vec<(String, String)> = Vec::new();
    for src in &sources {
        match load_state(&src.abs_path) {
            Ok(file) => loaded.push((src.clone(), file)),
            Err(e) => load_errors.push((src.repo_slug.clone(), e.to_string())),
        }
    }

    let mut status_map: HashMap<String, Option<String>> = HashMap::new();
    for (src, file) in &loaded {
        for track in &file.tracks {
            for block in &track.blocks {
                let key = format!("{}:{}", src.repo_slug, block.id);
                status_map.insert(key, block.status.clone());
            }
        }
    }

    let mut repo_paths: HashMap<String, PathBuf> = HashMap::new();
    for repo in &config.repos {
        let repo_root = if repo.repo_path == "." || repo.repo_path.is_empty() {
            root.to_path_buf()
        } else {
            root.join(&repo.repo_path)
        };
        repo_paths.insert(repo.slug.clone(), repo_root);
    }

    let report = mev::evaluate_carryover(
        &loaded,
        &status_map,
        root,
        &repo_paths,
        "2026-08-22",
        &config.attention,
        None,
        allow_exec,
    );

    (loaded, load_errors, report)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// (a) + (b) + (c): a CLEARED entry is removed and archived with the right archive-row
/// fields; the ACTIONABLE and NOT-EVALUABLE entries survive; the rest of `state.json` is
/// byte-identical apart from the removed element.
#[test]
fn cleared_entry_is_archived_and_removed_others_survive_state_untouched_otherwise() {
    let dir = temp_dir("basic");
    write_alpha_beta_fixture(&dir);

    let alpha_state_path = dir.join("repos/alpha/planning/state.json");
    let original_state_bytes = fs::read_to_string(&alpha_state_path).unwrap();

    let (loaded, load_errors, report) = load_and_evaluate_for_dispose(&dir, false);
    assert!(load_errors.is_empty(), "no repo should fail to load here");

    let plan = compute_disposal_plan(&report, &loaded, &load_errors);
    assert_eq!(
        plan.candidates.len(),
        1,
        "expected exactly one Cleared candidate, got: {:#?}",
        plan.candidates
    );
    assert_eq!(plan.candidates[0].slug, "alpha-cleared");
    assert_eq!(plan.candidates[0].repo, "alpha");

    let dispose_report = run_dispose(&plan, &loaded, "2026-08-22", false);
    assert!(
        dispose_report.succeeded(),
        "expected a clean run, got failures: {:#?}",
        dispose_report.failures
    );

    // The disposed entry is gone from state.json; the other two survive.
    let new_state: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&alpha_state_path).unwrap()).unwrap();
    let slugs: Vec<&str> = new_state["carryover"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["slug"].as_str().unwrap())
        .collect();
    assert_eq!(slugs, vec!["alpha-actionable", "alpha-prose-only"]);

    // (b) byte-identical apart from the removed element: reconstruct the expected file by
    // removing exactly that element from the loaded `StateFile` struct (the same typed
    // model the write path serializes) and re-serializing the same way (pretty + trailing
    // newline), then compare bytes. This is what actually pins "no re-indentation, no key
    // reordering, no escape churn" — a generic `serde_json::Value` round-trip would
    // silently re-sort keys and drop `#[serde(default)]` fields, masking exactly the
    // regressions this criterion exists to catch.
    let (_, alpha_file_for_expected) = loaded
        .iter()
        .find(|(src, _)| src.repo_slug == "alpha")
        .expect("alpha should have loaded")
        .clone();
    let mut expected_file = alpha_file_for_expected;
    expected_file
        .carryover
        .retain(|c| c.slug != "alpha-cleared");
    let mut expected_bytes = serde_json::to_string_pretty(&expected_file).unwrap();
    expected_bytes.push('\n');
    assert_eq!(
        fs::read_to_string(&alpha_state_path).unwrap(),
        expected_bytes,
        "state.json should differ from the original by exactly the removed element"
    );
    let _ = original_state_bytes;

    // (a) archive row carries the entry verbatim plus the disposal fields.
    let archive_path = dir.join("repos/alpha/planning/carryover-archive.jsonl");
    let archive_content = fs::read_to_string(&archive_path).unwrap();
    let lines: Vec<&str> = archive_content.lines().collect();
    assert_eq!(lines.len(), 1, "expected exactly one archived row");
    let row: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(row["slug"], "alpha-cleared");
    assert_eq!(row["reason"], "cleared");
    assert_eq!(row["reconstructed"], false);
    assert_eq!(row["disposed_at"], "2026-08-22");
    assert!(
        row["evidence"].as_str().is_some_and(|e| !e.is_empty()),
        "evidence should name the clearing predicate, got: {row:#?}"
    );
    assert_eq!(
        row["text"],
        "Alpha was waiting on beta's block A landing \u{2014} an em dash lives right here.",
        "the archived entry should carry the original text verbatim"
    );

    let _ = fs::remove_dir_all(&dir);
}

/// (d): a repo whose `state.json` fails to parse is named among `load_errors` and both of
/// its files are left untouched; a sibling repo in the same run is still disposed.
#[test]
fn malformed_sibling_repo_is_skipped_others_still_disposed() {
    let dir = temp_dir("malformed-sibling");
    write_brain_toml(&dir, &["alpha", "beta", "gamma"]);
    write_json(
        &dir,
        "repos/alpha/planning/state.json",
        &alpha_state_value(),
    );
    write_json(&dir, "repos/beta/planning/state.json", &beta_state_value());
    write_raw(
        &dir,
        "repos/gamma/planning/state.json",
        "{ this is not valid json",
    );

    let gamma_state_path = dir.join("repos/gamma/planning/state.json");
    let gamma_original = fs::read_to_string(&gamma_state_path).unwrap();
    let gamma_archive_path = dir.join("repos/gamma/planning/carryover-archive.jsonl");

    let (loaded, load_errors, report) = load_and_evaluate_for_dispose(&dir, false);
    assert!(
        load_errors.iter().any(|(repo, _)| repo == "gamma"),
        "gamma's parse failure should be reported, got: {load_errors:#?}"
    );
    assert!(
        loaded.iter().all(|(src, _)| src.repo_slug != "gamma"),
        "gamma must not appear among successfully-loaded repos"
    );

    let plan = compute_disposal_plan(&report, &loaded, &load_errors);
    assert!(
        plan.skipped.iter().any(|s| s.repo == "gamma"),
        "gamma should be recorded as SKIPPED, got: {:#?}",
        plan.skipped
    );
    // alpha still contributed its cleared candidate.
    assert!(plan.candidates.iter().any(|c| c.repo == "alpha"));

    let dispose_report = run_dispose(&plan, &loaded, "2026-08-22", false);
    assert!(dispose_report.succeeded());
    assert!(
        dispose_report
            .writes
            .iter()
            .any(|w| w.repo == "alpha" && w.written),
        "alpha should still be disposed in the same run"
    );

    // gamma's files are untouched.
    assert_eq!(
        fs::read_to_string(&gamma_state_path).unwrap(),
        gamma_original,
        "gamma's state.json must be left byte-identical"
    );
    assert!(
        !gamma_archive_path.exists(),
        "gamma's archive file must not be created"
    );

    let summary = render_dispose_summary(&dispose_report);
    assert!(
        summary.contains("gamma") && summary.contains("SKIPPED"),
        "summary should name gamma as SKIPPED, got:\n{summary}"
    );

    let _ = fs::remove_dir_all(&dir);
}

/// (e): a `command_exits_zero` entry without `--allow-exec` is NotEvaluable and is never
/// selected for disposal, even under `--dispose`.
#[test]
fn command_exits_zero_without_allow_exec_survives_dispose() {
    let dir = temp_dir("command-exec");
    write_brain_toml(&dir, &["delta"]);
    let state = serde_json::json!({
        "repo": "delta",
        "kind": "project",
        "updated": "2026-08-01",
        "focus": { "now": [], "next": [], "blocked": [] },
        "tracks": [],
        "carryover": [
            {
                "slug": "delta-command-exec",
                "scope": { "repo": "delta" },
                "kind": "deferred",
                "text": "Clears once the check script exits 0.",
                "clears_when": { "type": "command_exits_zero", "command": "true" },
                "created": "2026-06-01"
            }
        ]
    });
    write_json(&dir, "repos/delta/planning/state.json", &state);

    // allow_exec = false, matching `--dispose` without `--allow-exec`.
    let (loaded, load_errors, report) = load_and_evaluate_for_dispose(&dir, false);
    let entry = report
        .entries
        .iter()
        .find(|e| e.slug == "delta-command-exec")
        .expect("entry should be present");
    assert_eq!(
        entry.lane,
        CarryoverLane::NotEvaluable,
        "without --allow-exec, a command_exits_zero entry must stay NotEvaluable"
    );

    let plan = compute_disposal_plan(&report, &loaded, &load_errors);
    assert!(
        plan.candidates.is_empty(),
        "a command_exits_zero entry without --allow-exec must never be disposal-eligible, got: {:#?}",
        plan.candidates
    );

    let _ = fs::remove_dir_all(&dir);
}

/// (f): `--dispose --dry-run` produces the identical disposal list and writes nothing.
#[test]
fn dry_run_reports_identical_plan_and_writes_nothing() {
    let dir = temp_dir("dry-run");
    write_alpha_beta_fixture(&dir);

    let alpha_state_path = dir.join("repos/alpha/planning/state.json");
    let alpha_archive_path = dir.join("repos/alpha/planning/carryover-archive.jsonl");
    let original_state = fs::read_to_string(&alpha_state_path).unwrap();

    let (loaded, load_errors, report) = load_and_evaluate_for_dispose(&dir, false);
    let plan = compute_disposal_plan(&report, &loaded, &load_errors);

    let real_report = run_dispose(&plan, &loaded, "2026-08-22", false);
    // Reset the fixture and re-run under dry-run to compare the disposal list on a clean
    // slate, since the real run above already mutated it.
    let _ = fs::remove_dir_all(&dir);
    let dir = temp_dir("dry-run-2");
    write_alpha_beta_fixture(&dir);
    let alpha_state_path = dir.join("repos/alpha/planning/state.json");
    let alpha_archive_path2 = dir.join("repos/alpha/planning/carryover-archive.jsonl");
    let original_state2 = fs::read_to_string(&alpha_state_path).unwrap();

    let (loaded2, load_errors2, report2) = load_and_evaluate_for_dispose(&dir, false);
    let plan2 = compute_disposal_plan(&report2, &loaded2, &load_errors2);
    assert_eq!(
        plan.candidates.len(),
        plan2.candidates.len(),
        "dry-run plan should select the same candidates as a real run"
    );

    let dry_report = run_dispose(&plan2, &loaded2, "2026-08-22", true);
    assert!(dry_report.succeeded());
    assert_eq!(
        dry_report
            .writes
            .iter()
            .map(|w| w.disposed.len())
            .sum::<usize>(),
        real_report
            .writes
            .iter()
            .map(|w| w.disposed.len())
            .sum::<usize>(),
        "dry-run should report the same disposal counts as the real run"
    );
    for write in &dry_report.writes {
        assert!(
            !write.written,
            "dry-run must never mark a repo as written, got: {write:#?}"
        );
    }

    assert_eq!(
        fs::read_to_string(&alpha_state_path).unwrap(),
        original_state2,
        "dry-run must leave state.json byte-identical"
    );
    assert!(
        !alpha_archive_path2.exists(),
        "dry-run must never create the archive file"
    );

    // Sanity: original_state captured before the earlier real run is unused beyond
    // demonstrating the real run indeed differs from the untouched original.
    assert_ne!(
        original_state,
        fs::read_to_string(&alpha_archive_path).unwrap_or_default()
    );

    let _ = fs::remove_dir_all(&dir);
}

/// (g): a `state.json` with em dashes / non-ASCII in an untouched entry round-trips
/// unchanged — no `\uXXXX` escape churn — after a disposal run touches a *different*
/// entry in the same file.
#[test]
fn non_ascii_in_surviving_entries_round_trips_unchanged() {
    let dir = temp_dir("non-ascii");
    write_alpha_beta_fixture(&dir);
    let alpha_state_path = dir.join("repos/alpha/planning/state.json");

    let (loaded, load_errors, report) = load_and_evaluate_for_dispose(&dir, false);
    let plan = compute_disposal_plan(&report, &loaded, &load_errors);
    let dispose_report = run_dispose(&plan, &loaded, "2026-08-22", false);
    assert!(dispose_report.succeeded());

    let new_content = fs::read_to_string(&alpha_state_path).unwrap();
    assert!(
        !new_content.contains("\\u2014"),
        "em dash must not be escaped as \\u2014, got a snippet of:\n{}",
        &new_content
            .lines()
            .find(|l| l.contains("2014"))
            .unwrap_or("<not found>")
    );

    // Note: the disposed entry (which carried the em dash in this fixture) moved to the
    // archive file, which must itself preserve the literal character too.
    let archive_content =
        fs::read_to_string(dir.join("repos/alpha/planning/carryover-archive.jsonl")).unwrap();
    assert!(
        archive_content.contains('\u{2014}'),
        "archive row should preserve the literal em dash, got:\n{archive_content}"
    );
    assert!(!archive_content.contains("\\u2014"));

    let _ = fs::remove_dir_all(&dir);
}

/// (h): the printed preamble contains each disposed entry's full text, and the summary
/// carries a `git commit -o` pathspec naming both written files.
#[test]
fn printed_output_carries_full_text_and_commit_pathspec() {
    let dir = temp_dir("printed-output");
    write_alpha_beta_fixture(&dir);

    let (loaded, load_errors, report) = load_and_evaluate_for_dispose(&dir, false);
    let plan = compute_disposal_plan(&report, &loaded, &load_errors);

    let preamble = render_dispose_preamble(&plan);
    assert!(
        preamble.contains("alpha-cleared"),
        "preamble should name the disposed slug, got:\n{preamble}"
    );
    assert!(
        preamble.contains("Alpha was waiting on beta's block A landing"),
        "preamble should contain the entry's full text, got:\n{preamble}"
    );

    let dispose_report = run_dispose(&plan, &loaded, "2026-08-22", false);
    let pathspec = render_commit_pathspec(&dispose_report).expect("expected a commit pathspec");
    assert!(pathspec.starts_with("git commit -o "));
    let alpha_state_path = dir.join("repos/alpha/planning/state.json");
    let alpha_archive_path = dir.join("repos/alpha/planning/carryover-archive.jsonl");
    assert!(pathspec.contains(&alpha_state_path.display().to_string()));
    assert!(pathspec.contains(&alpha_archive_path.display().to_string()));

    let summary = render_dispose_summary(&dispose_report);
    assert!(summary.contains(&pathspec));

    let _ = fs::remove_dir_all(&dir);
}

/// Shown-failing atomicity case: a write failure between the two writes (simulated by
/// making the repo's `planning/` directory read-only so the second write — `state.json` —
/// cannot be staged) must leave NO removed-but-unarchived entry: `state.json` unchanged
/// and well-formed, with the archive write best-effort reverted to its prior (nonexistent)
/// state.
#[test]
fn write_failure_between_the_two_writes_leaves_no_orphaned_removal() {
    let dir = temp_dir("atomicity");
    write_alpha_beta_fixture(&dir);

    let alpha_state_path = dir.join("repos/alpha/planning/state.json");
    let alpha_planning_dir = dir.join("repos/alpha/planning");
    let alpha_archive_path = archive_path_for(&alpha_state_path);
    let original_state = fs::read_to_string(&alpha_state_path).unwrap();

    let (loaded, load_errors, report) = load_and_evaluate_for_dispose(&dir, false);
    let plan = compute_disposal_plan(&report, &loaded, &load_errors);
    let alpha_candidates: Vec<DisposalCandidate> = plan
        .candidates
        .iter()
        .filter(|c| c.repo == "alpha")
        .cloned()
        .collect();
    assert!(!alpha_candidates.is_empty());

    let (alpha_source, alpha_file) = loaded
        .iter()
        .find(|(src, _)| src.repo_slug == "alpha")
        .expect("alpha should be loaded");

    // Make the planning dir read-only so the second write (state.json's temp-file stage,
    // which lives in the same directory) fails with a permission error, after the first
    // write (the archive) has already succeeded.
    let mut perms = fs::metadata(&alpha_planning_dir).unwrap().permissions();
    let original_mode = perms.mode();
    perms.set_mode(0o555);
    fs::set_permissions(&alpha_planning_dir, perms).unwrap();

    let result = dispose_repo(
        alpha_source,
        alpha_file,
        &alpha_candidates,
        &alpha_archive_path,
        "2026-08-22",
        false,
    );

    // Restore permissions immediately so cleanup can proceed regardless of the assertion
    // outcome below.
    let mut restore = fs::metadata(&alpha_planning_dir).unwrap().permissions();
    restore.set_mode(original_mode);
    fs::set_permissions(&alpha_planning_dir, restore).unwrap();

    assert!(
        result.is_err(),
        "expected the second write to fail under a read-only planning dir"
    );

    // state.json must be exactly as it was before the run: no entry silently removed.
    assert_eq!(
        fs::read_to_string(&alpha_state_path).unwrap(),
        original_state,
        "state.json must be left unchanged when the run fails partway through"
    );
    // It must still parse as valid state — never left malformed mid-write.
    let _: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&alpha_state_path).unwrap())
            .expect("state.json must remain well-formed JSON after a failed dispose");

    // The archive write is best-effort reverted: since the archive file did not exist
    // before this run, it must not exist afterward either.
    assert!(
        !alpha_archive_path.exists(),
        "archive file should be reverted to its prior (nonexistent) state, found: {}",
        alpha_archive_path.display()
    );

    let _ = fs::remove_dir_all(&dir);
}
