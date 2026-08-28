//! Integration tests for `mev carryover --trajectory` (`MV.16.F`, task 4) — the weekly
//! `carryover-archive.jsonl` outflow trajectory built by
//! [`mev::brain::carryover::build_trajectory`].
//!
//! Fixture helpers mirror `tests/it/brain_carryover_archive_outflow.rs` (MV.16.E task 4):
//! a scratch `<root>/repos/<repo_slug>/planning/carryover-archive.jsonl` per repo, written
//! by hand as JSONL, plus a matching `(StateSource, StateFile)` pair. Every test injects an
//! explicit `today` string — no test reads the system clock, since a clock-driven expected
//! output changes every week and would be deleted by whoever it wakes at 3am.
//!
//! Cases:
//!   1. Week bucketing: rows on known dates land in the right `YYYY-Www` buckets.
//!   2. Exactly `weeks` rows are emitted; a zero-disposal week is present, not omitted.
//!   3. Observed/reconstructed split is preserved per week.
//!   4. `before_window`: a row dated before the first emitted week is counted there,
//!      excluded from every week row, and folded into the first row's cumulative.
//!   5. COHERENCE GATE: for a fixture archive entirely inside the window, the last week's
//!      cumulative equals `--audit`'s `archive_outflow.rows_total`; appending one more
//!      in-window row moves both numbers together.
//!   6. An unparseable `disposed_at` increments `undated`, stays out of every bucket, and
//!      still counts in `rows_total`.
//!   7. `--repo` scoping: a two-repo fixture where `repo_filter` selects one repo's archive
//!      only, and the trajectory total matches that repo's audit total.
//!   8. Misuse: the four conflicting flag combinations exit non-zero, driven through the
//!      built binary.

use std::fs;
use std::path::{Path, PathBuf};

use mev::brain::carryover::{CarryoverReport, archive_path_for, audit_carryover, build_trajectory};
use mev::brain::state::{StateFile, StateSource};

// ---------------------------------------------------------------------------
// Helpers (mirrors tests/it/brain_carryover_archive_outflow.rs)
// ---------------------------------------------------------------------------

fn temp_dir(suffix: &str) -> PathBuf {
    let dir = mev::testsupport::unique_temp_dir(&format!("mev-carryover-trajectory-{suffix}"));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Build one `(StateSource, StateFile)` pair for `repo_slug`, with `abs_path` pointing at
/// `<root>/repos/<repo_slug>/planning/state.json` so `archive_path_for` derives
/// `<root>/repos/<repo_slug>/planning/carryover-archive.jsonl`.
fn source_for(root: &Path, repo_slug: &str) -> (StateSource, StateFile) {
    let abs_path = root
        .join("repos")
        .join(repo_slug)
        .join("planning/state.json");
    (
        StateSource {
            repo_slug: repo_slug.to_string(),
            abs_path,
            expected_kind: "project",
        },
        StateFile::default(),
    )
}

/// Write `lines` (already-serialized JSON strings, one per archive row) to
/// `<root>/repos/<repo_slug>/planning/carryover-archive.jsonl`.
fn write_archive(root: &Path, repo_slug: &str, lines: &[String]) {
    let (src, _) = source_for(root, repo_slug);
    let archive_path = archive_path_for(&src.abs_path);
    fs::create_dir_all(archive_path.parent().unwrap()).unwrap();
    let mut content = lines.join("\n");
    if !lines.is_empty() {
        content.push('\n');
    }
    fs::write(&archive_path, content.as_bytes()).unwrap();
}

/// Append one more archive line to an already-written archive.
fn append_archive(root: &Path, repo_slug: &str, line: String) {
    let (src, _) = source_for(root, repo_slug);
    let archive_path = archive_path_for(&src.abs_path);
    let mut content = fs::read_to_string(&archive_path).unwrap_or_default();
    content.push_str(&line);
    content.push('\n');
    fs::write(&archive_path, content.as_bytes()).unwrap();
}

/// A well-formed archive row as a JSON string. `disposed_at` and `reconstructed` are
/// caller-supplied since the tests key on both.
fn row(slug: &str, reason: &str, disposed_at: &str, reconstructed_field: Option<bool>) -> String {
    let mut value = serde_json::json!({
        "slug": slug,
        "scope": { "repo": "alpha", "tier": null, "cross_repo": null },
        "kind": "deferred",
        "text": format!("{slug} text"),
        "created": "2026-06-01",
        "disposed_at": disposed_at,
        "reason": reason,
    });
    if let Some(reconstructed) = reconstructed_field {
        value["reconstructed"] = serde_json::json!(reconstructed);
    }
    value.to_string()
}

/// Write `brain.toml` with a single leaf repo (`alpha`), enough for
/// `find_brain_root`/CLI misuse checks (which run before any corpus load).
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// (1) Week bucketing: rows on known dates land in the right `YYYY-Www` buckets.
/// 2026-08-24 is in ISO week 2026-W35; 2026-08-17 is in ISO week 2026-W34.
#[test]
fn week_bucketing_places_rows_in_their_iso_week() {
    let dir = temp_dir("bucketing");
    let lines = vec![
        row("a1", "cleared", "2026-08-24", Some(false)),
        row("a2", "cleared", "2026-08-17", Some(false)),
    ];
    write_archive(&dir, "alpha", &lines);

    let files = vec![source_for(&dir, "alpha")];
    let report = build_trajectory(&files, "2026-08-24", 4, None);

    assert_eq!(report.weeks.len(), 4);
    assert_eq!(report.weeks.last().unwrap().iso_week, "2026-W35");
    assert_eq!(report.weeks.last().unwrap().observed, 1);
    let w34 = report
        .weeks
        .iter()
        .find(|w| w.iso_week == "2026-W34")
        .expect("2026-W34 row should exist");
    assert_eq!(w34.observed, 1);

    let _ = fs::remove_dir_all(&dir);
}

/// (2) Exactly `weeks` rows are emitted; a week with zero disposals is present as a zero
/// row, not omitted.
#[test]
fn exactly_weeks_rows_emitted_including_zero_disposal_weeks() {
    let dir = temp_dir("zero-weeks");
    let lines = vec![row("b1", "cleared", "2026-08-24", Some(false))];
    write_archive(&dir, "alpha", &lines);

    let files = vec![source_for(&dir, "alpha")];
    let report = build_trajectory(&files, "2026-08-24", 4, None);

    assert_eq!(report.weeks.len(), 4);
    let zero_weeks = report.weeks.iter().filter(|w| w.total() == 0).count();
    assert_eq!(zero_weeks, 3, "three weeks should carry zero disposals");

    let _ = fs::remove_dir_all(&dir);
}

/// (3) Observed/reconstructed split is preserved per week.
#[test]
fn observed_and_reconstructed_split_is_preserved_per_week() {
    let dir = temp_dir("split");
    let lines = vec![
        row("c1", "cleared", "2026-08-24", Some(false)),
        row("c2", "cleared", "2026-08-24", Some(true)),
    ];
    write_archive(&dir, "alpha", &lines);

    let files = vec![source_for(&dir, "alpha")];
    let report = build_trajectory(&files, "2026-08-24", 1, None);

    let week = &report.weeks[0];
    assert_eq!(week.observed, 1);
    assert_eq!(week.reconstructed, 1);
    assert_ne!(
        week.observed,
        week.total(),
        "reconstructed rows must never be folded into the observed column"
    );

    let _ = fs::remove_dir_all(&dir);
}

/// (4) `before_window`: a row dated before the first emitted week is counted there,
/// excluded from every week row, and included in the first row's cumulative.
#[test]
fn rows_before_the_window_are_counted_and_excluded_from_week_rows() {
    let dir = temp_dir("before-window");
    let lines = vec![
        row("d1", "cleared", "2026-01-01", Some(false)), // far before the window
        row("d2", "cleared", "2026-08-24", Some(false)),
    ];
    write_archive(&dir, "alpha", &lines);

    let files = vec![source_for(&dir, "alpha")];
    let report = build_trajectory(&files, "2026-08-24", 1, None);

    assert_eq!(report.before_window, 1);
    assert_eq!(
        report.weeks[0].total(),
        1,
        "the before-window row must not appear in any week bucket"
    );
    assert_eq!(
        report.weeks[0].cumulative, 2,
        "the first row's cumulative must include before_window"
    );

    let _ = fs::remove_dir_all(&dir);
}

/// (5) COHERENCE GATE: for a fixture archive entirely inside the window, the last week's
/// cumulative equals `--audit`'s `archive_outflow.rows_total`. Appending one more in-window
/// row moves both numbers together. This is the block's headline criterion.
#[test]
fn coherence_gate_last_cumulative_matches_audit_rows_total_and_moves_together() {
    let dir = temp_dir("coherence");
    let lines = vec![
        row("e1", "cleared", "2026-08-24", Some(false)),
        row("e2", "withdrawn", "2026-08-17", Some(false)),
    ];
    write_archive(&dir, "alpha", &lines);

    let files = vec![source_for(&dir, "alpha")];

    let trajectory_before = build_trajectory(&files, "2026-08-24", 8, None);
    let empty_report = CarryoverReport::default();
    let audit_before = audit_carryover(&files, &empty_report, "2026-08-24", 3650, None);

    let cumulative_before = trajectory_before.weeks.last().unwrap().cumulative;
    let audit_total_before = audit_before.archive_outflow.rows_total;
    assert_eq!(
        cumulative_before, audit_total_before,
        "trajectory's last cumulative must equal --audit's archive row total \
         when the window covers the whole archive"
    );

    // Dispose one more row into the fixture, entirely inside the window.
    append_archive(
        &dir,
        "alpha",
        row("e3", "cleared", "2026-08-24", Some(false)),
    );

    let trajectory_after = build_trajectory(&files, "2026-08-24", 8, None);
    let audit_after = audit_carryover(&files, &empty_report, "2026-08-24", 3650, None);

    let cumulative_after = trajectory_after.weeks.last().unwrap().cumulative;
    let audit_total_after = audit_after.archive_outflow.rows_total;

    assert_eq!(cumulative_after, cumulative_before + 1);
    assert_eq!(audit_total_after, audit_total_before + 1);
    assert_eq!(
        cumulative_after, audit_total_after,
        "both numbers must move together after the new disposal"
    );

    let _ = fs::remove_dir_all(&dir);
}

/// (6) An unparseable `disposed_at` increments `undated`, stays out of every bucket, and
/// still counts in `rows_total`.
#[test]
fn undated_row_is_excluded_from_buckets_but_counted_in_rows_total() {
    let dir = temp_dir("undated");
    let lines = vec![
        row("f1", "cleared", "2026-08-24", Some(false)),
        row("f2", "cleared", "not-a-date", Some(false)),
    ];
    write_archive(&dir, "alpha", &lines);

    let files = vec![source_for(&dir, "alpha")];
    let report = build_trajectory(&files, "2026-08-24", 1, None);

    assert_eq!(report.rows_total, 2);
    assert_eq!(report.undated, 1);
    assert_eq!(report.weeks[0].total(), 1);
    assert_eq!(report.weeks.last().unwrap().cumulative, 1);

    let _ = fs::remove_dir_all(&dir);
}

/// (7) `--repo` scoping: a two-repo fixture where `repo_filter` of one repo selects only
/// that repo's archive, and the trajectory total matches that repo's audit total.
#[test]
fn repo_filter_scopes_which_archives_are_read() {
    let dir = temp_dir("repo-scope");
    write_archive(
        &dir,
        "alpha",
        &[row("g1", "cleared", "2026-08-24", Some(false))],
    );
    write_archive(
        &dir,
        "beta",
        &[
            row("g2", "cleared", "2026-08-24", Some(false)),
            row("g3", "withdrawn", "2026-08-17", Some(false)),
        ],
    );

    let files = vec![source_for(&dir, "alpha"), source_for(&dir, "beta")];

    let trajectory_all = build_trajectory(&files, "2026-08-24", 4, None);
    assert_eq!(trajectory_all.rows_total, 3);
    assert_eq!(trajectory_all.archives_read, 2);

    let trajectory_alpha = build_trajectory(&files, "2026-08-24", 4, Some("alpha"));
    let empty_report = CarryoverReport::default();
    let audit_alpha = audit_carryover(&files, &empty_report, "2026-08-24", 3650, Some("alpha"));

    assert_eq!(trajectory_alpha.rows_total, 1);
    assert_eq!(trajectory_alpha.archives_read, 1);
    assert_eq!(
        trajectory_alpha.rows_total, audit_alpha.archive_outflow.rows_total,
        "the repo-scoped trajectory total must match that repo's audit total"
    );

    let _ = fs::remove_dir_all(&dir);
}

/// (8) Misuse: `--trajectory` combined with any of `--audit` / `--dispose` / `--backfill`
/// / `--would-block` exits non-zero, driven through the built binary the way the existing
/// CLI tests in `tests/it/` do.
#[test]
fn trajectory_combined_with_conflicting_flags_exits_non_zero() {
    let dir = temp_dir("misuse");
    write_brain_toml(&dir);

    for conflicting_flag in ["--audit", "--dispose", "--backfill", "--would-block"] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_mev"))
            .arg("carryover")
            .arg(&dir)
            .arg("--trajectory")
            .arg(conflicting_flag)
            .output()
            .expect("mev carryover should run");

        assert!(
            !output.status.success(),
            "--trajectory combined with {conflicting_flag} should exit non-zero"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("--trajectory"),
            "misuse message should name --trajectory, got: {stderr}"
        );
    }

    let _ = fs::remove_dir_all(&dir);
}
