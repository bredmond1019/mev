//! Integration tests for `mev::carryover_sweep` — end-to-end over a temp-dir corpus fixture.
//!
//! `MV.ticket.carryover-sweep-command` — Task 4.
//!
//! Builds a temp HQ-root fixture with two leaf repos (`alpha`, `beta`), where `alpha` carries
//! `carryover[]` entries that reference blocks living in the *other* repo (`beta`), exercising
//! cross-repo reference resolution end to end through the public driver: `find_brain_config` →
//! `discover_state_files` → `load_state` → status-map construction → `evaluate_carryover`.
//!
//! Tests:
//!   1. A `clears_when` naming a closed block in another repo → `Cleared`.
//!   2. A `clears_when` naming an open block in another repo → `Actionable`, with that block
//!      named among the unmet refs.
//!   3. A prose-only `clears_when` (no resolvable block or path token) → `NotEvaluable` /
//!      `Prose`.
//!   4. `--repo` filtering returns only that repo's entries.
//!   5. `total == cleared + actionable + not_evaluable`, and equals the sum of `entries.len()`.

use std::fs;
use std::path::Path;

use mev::brain::carryover::{CarryoverLane, CarryoverRef, NotEvaluableReason};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Make a fresh uniquely-named temp dir for a test and return its path.
fn temp_dir(suffix: &str) -> std::path::PathBuf {
    let dir = mev::testsupport::unique_temp_dir(&format!("mev-brain-carryover-it-{suffix}"));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Write `brain.toml` with two leaf repos (alpha, beta) and a standard `[vocab]` block.
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

/// Serialize `value` as pretty JSON and write it to `root/rel`.
fn write_json(root: &Path, rel: &str, value: &serde_json::Value) {
    let full = root.join(rel);
    fs::create_dir_all(full.parent().unwrap()).unwrap();
    fs::write(
        &full,
        serde_json::to_string_pretty(value).unwrap().as_bytes(),
    )
    .unwrap();
}

/// Write beta's leaf state: one closed block (`BE.1.A`) and one open block (`BE.1.B`).
/// Beta carries no `carryover[]` entries of its own — only alpha's entries reference it.
fn write_beta_state(root: &Path) {
    let state = serde_json::json!({
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
    });
    write_json(root, "repos/beta/planning/state.json", &state);
}

/// Write alpha's leaf state: three `carryover[]` entries —
///   - `alpha-cleared-cross-repo`: `related[]` names beta's closed `BE.1.A` → Cleared.
///   - `alpha-actionable-cross-repo`: prose names beta's open `BE.1.B` → Actionable.
///   - `alpha-prose-only`: no resolvable block/path token → NotEvaluable(Prose).
fn write_alpha_state(root: &Path) {
    let state = serde_json::json!({
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
                "slug": "alpha-cleared-cross-repo",
                "scope": { "repo": "alpha" },
                "kind": "known_issue",
                "text": "Alpha was waiting on beta's block A landing.",
                "related": [
                    { "type": "block", "repo": "beta", "id": "BE.1.A" }
                ],
                "clears_when": "BE.1.A lands",
                "created": "2026-06-01"
            },
            {
                "slug": "alpha-actionable-cross-repo",
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
    });
    write_json(root, "repos/alpha/planning/state.json", &state);
}

/// Build the complete fixture: brain.toml + alpha leaf + beta leaf. No HQ-root
/// `planning/state.json` is written — discovery tolerates its absence (a
/// `W_STATE_FILE_MISSING` discovery diagnostic, which `carryover_sweep` ignores) and still
/// walks the leaf `[[repos]]` entries.
fn write_fixture(root: &Path) {
    write_brain_toml(root);
    write_alpha_state(root);
    write_beta_state(root);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn cross_repo_closed_block_lands_in_cleared() {
    let dir = temp_dir("cleared");
    write_fixture(&dir);

    let report = mev::carryover_sweep(&dir, None).expect("carryover_sweep should not error");

    let entry = report
        .entries
        .iter()
        .find(|e| e.slug == "alpha-cleared-cross-repo")
        .expect("alpha-cleared-cross-repo entry should be present");

    assert_eq!(entry.lane, CarryoverLane::Cleared);
    assert!(
        entry.refs.iter().any(|r| matches!(
            r,
            CarryoverRef::Block { key, satisfied } if key == "beta:BE.1.A" && *satisfied
        )),
        "expected a satisfied beta:BE.1.A block ref, got: {:#?}",
        entry.refs
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn cross_repo_open_block_lands_in_actionable_naming_the_unmet_block() {
    let dir = temp_dir("actionable");
    write_fixture(&dir);

    let report = mev::carryover_sweep(&dir, None).expect("carryover_sweep should not error");

    let entry = report
        .entries
        .iter()
        .find(|e| e.slug == "alpha-actionable-cross-repo")
        .expect("alpha-actionable-cross-repo entry should be present");

    assert_eq!(entry.lane, CarryoverLane::Actionable);
    assert!(
        entry.refs.iter().any(|r| matches!(
            r,
            CarryoverRef::Block { key, satisfied } if key == "beta:BE.1.B" && !*satisfied
        )),
        "expected an unsatisfied beta:BE.1.B block ref, got: {:#?}",
        entry.refs
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn prose_only_predicate_lands_in_not_evaluable_prose() {
    let dir = temp_dir("prose");
    write_fixture(&dir);

    let report = mev::carryover_sweep(&dir, None).expect("carryover_sweep should not error");

    let entry = report
        .entries
        .iter()
        .find(|e| e.slug == "alpha-prose-only")
        .expect("alpha-prose-only entry should be present");

    assert_eq!(entry.lane, CarryoverLane::NotEvaluable);
    assert!(
        entry.refs.is_empty(),
        "expected no refs, got: {:#?}",
        entry.refs
    );
    assert_eq!(entry.reason, Some(NotEvaluableReason::Prose));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn repo_filter_returns_only_that_repos_entries() {
    let dir = temp_dir("repo-filter");
    write_fixture(&dir);

    let report =
        mev::carryover_sweep(&dir, Some("alpha")).expect("carryover_sweep should not error");

    assert!(
        !report.entries.is_empty(),
        "expected alpha's entries to be present"
    );
    assert!(
        report.entries.iter().all(|e| e.repo == "alpha"),
        "expected only alpha entries with --repo alpha, got: {:#?}",
        report
            .entries
            .iter()
            .map(|e| (&e.repo, &e.slug))
            .collect::<Vec<_>>()
    );

    let beta_only =
        mev::carryover_sweep(&dir, Some("beta")).expect("carryover_sweep should not error");
    assert_eq!(
        beta_only.total, 0,
        "beta has no carryover[] entries of its own, expected total == 0, got: {:#?}",
        beta_only.entries
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn total_equals_sum_of_lanes_and_entries() {
    let dir = temp_dir("total");
    write_fixture(&dir);

    let report = mev::carryover_sweep(&dir, None).expect("carryover_sweep should not error");

    assert_eq!(
        report.total, 3,
        "expected all 3 alpha carryover entries, got: {report:#?}"
    );
    assert_eq!(
        report.total,
        report.cleared + report.actionable + report.not_evaluable,
        "total should equal the sum of the three lane counts"
    );
    assert_eq!(
        report.total,
        report.entries.len(),
        "total should equal entries.len()"
    );
    assert_eq!(report.cleared, 1);
    assert_eq!(report.actionable, 1);
    assert_eq!(report.not_evaluable, 1);

    let _ = fs::remove_dir_all(&dir);
}
