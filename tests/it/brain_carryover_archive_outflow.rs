//! Integration tests for `read_archive_outflow` (`mev carryover --audit`'s archive
//! disposition pass) — MV.16.E, Task 4.
//!
//! Each test builds its own `planning/carryover-archive.jsonl` fixture(s) under
//! `tempfile` and a matching `StateSource` list; no test reads the real corpus or the
//! developer's home directory.
//!
//! Cases:
//!   1. per-reason counts summed across both columns equal the archive's line count.
//!   2. `reconstructed: true` lands in the reconstructed column; an absent key lands in
//!      observed (`#[serde(default)]`).
//!   3. SHOWN FAILING CASE: an archive holding exactly one `reason: "superseded"` row
//!      reports it under `superseded` and leaves `cleared` at zero.
//!   4. Absent `carryover-archive.jsonl`: zero rows, `archives_missing == 1`, no
//!      malformed lines.
//!   5. A malformed line is named with path + 1-based line number, and the surrounding
//!      valid rows are still counted.
//!   6. `--repo` scoping: a two-repo fixture where `repo_filter` of one repo reads only
//!      that repo's archive.
//!   7. Window: a row whose `disposed_at` is outside `window_days` counts toward
//!      `rows_total` but not `rows_in_window`.

use std::fs;
use std::path::{Path, PathBuf};

use mev::brain::carryover::{ArchiveOutflow, archive_path_for, read_archive_outflow};
use mev::brain::state::{StateFile, StateSource};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn temp_dir(suffix: &str) -> PathBuf {
    let dir = mev::testsupport::unique_temp_dir(&format!("mev-carryover-archive-outflow-{suffix}"));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Build one `(StateSource, StateFile)` pair for `repo_slug`, with `abs_path` pointing
/// at `<root>/repos/<repo_slug>/planning/state.json` (so `archive_path_for` derives
/// `<root>/repos/<repo_slug>/planning/carryover-archive.jsonl`, matching wherever the
/// test writes the archive fixture).
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// (1) per-reason counts summed across both columns equal the archive's line count.
#[test]
fn per_reason_totals_sum_to_line_count() {
    let dir = temp_dir("totals");
    let lines = vec![
        row("a1", "cleared", "2026-08-20", Some(false)),
        row("a2", "cleared", "2026-08-20", Some(true)),
        row("a3", "withdrawn", "2026-08-20", Some(false)),
    ];
    write_archive(&dir, "alpha", &lines);

    let files = vec![source_for(&dir, "alpha")];
    let outflow = read_archive_outflow(&files, "2026-08-22", 30, None);

    assert_eq!(outflow.rows_total, 3);
    let sum: usize = outflow.per_reason.values().map(|s| s.total()).sum();
    assert_eq!(sum, outflow.rows_total);

    let _ = fs::remove_dir_all(&dir);
}

/// (2) `reconstructed: true` lands in the reconstructed column; an absent key lands in
/// observed.
#[test]
fn reconstructed_flag_splits_the_column_absent_key_is_observed() {
    let dir = temp_dir("split");
    let lines = vec![
        row("b1", "cleared", "2026-08-20", Some(true)),
        row("b2", "cleared", "2026-08-20", None),
    ];
    write_archive(&dir, "alpha", &lines);

    let files = vec![source_for(&dir, "alpha")];
    let outflow = read_archive_outflow(&files, "2026-08-22", 30, None);

    let split = outflow
        .per_reason
        .get("cleared")
        .expect("cleared entry should exist");
    assert_eq!(split.reconstructed, 1, "row with reconstructed:true");
    assert_eq!(
        split.observed, 1,
        "row with the key absent defaults to observed"
    );

    let _ = fs::remove_dir_all(&dir);
}

/// (3) SHOWN FAILING CASE: an archive holding exactly one `reason: "superseded"` row
/// reports it under `superseded` and leaves `cleared` at zero.
#[test]
fn superseded_row_does_not_move_the_cleared_count() {
    let dir = temp_dir("superseded");
    let lines = vec![row("c1", "superseded", "2026-08-20", Some(false))];
    write_archive(&dir, "alpha", &lines);

    let files = vec![source_for(&dir, "alpha")];
    let outflow = read_archive_outflow(&files, "2026-08-22", 30, None);

    let superseded = outflow
        .per_reason
        .get("superseded")
        .expect("superseded entry should exist");
    assert_eq!(superseded.total(), 1);

    let cleared_count = outflow
        .per_reason
        .get("cleared")
        .map(|s| s.total())
        .unwrap_or(0);
    assert_eq!(
        cleared_count, 0,
        "a superseded row must never be counted as cleared"
    );

    let _ = fs::remove_dir_all(&dir);
}

/// (4) Absent `carryover-archive.jsonl`: zero rows, `archives_missing == 1`, no
/// diagnostic beyond that, and no malformed lines.
#[test]
fn absent_archive_yields_zero_rows_and_is_reported_missing() {
    let dir = temp_dir("absent");
    // No archive written for `alpha` at all — not even the directory.
    fs::create_dir_all(dir.join("repos/alpha/planning")).unwrap();

    let files = vec![source_for(&dir, "alpha")];
    let outflow: ArchiveOutflow = read_archive_outflow(&files, "2026-08-22", 30, None);

    assert_eq!(outflow.rows_total, 0);
    assert_eq!(outflow.archives_missing, 1);
    assert_eq!(outflow.archives_read, 0);
    assert!(outflow.malformed_lines.is_empty());

    let _ = fs::remove_dir_all(&dir);
}

/// (5) A malformed line is named with its file path and 1-based line number and
/// skipped; the surrounding valid rows are still counted.
#[test]
fn malformed_line_is_named_and_skipped_valid_rows_still_counted() {
    let dir = temp_dir("malformed");
    let (src, _) = source_for(&dir, "alpha");
    let archive_path = archive_path_for(&src.abs_path);
    fs::create_dir_all(archive_path.parent().unwrap()).unwrap();
    let content = format!(
        "{}\n{}\n{}\n",
        row("d1", "cleared", "2026-08-20", Some(false)),
        "{ this is not valid json",
        row("d2", "withdrawn", "2026-08-20", Some(false)),
    );
    fs::write(&archive_path, content.as_bytes()).unwrap();

    let files = vec![source_for(&dir, "alpha")];
    let outflow = read_archive_outflow(&files, "2026-08-22", 30, None);

    assert_eq!(
        outflow.rows_total, 2,
        "the two valid rows are still counted"
    );
    assert_eq!(outflow.malformed_lines.len(), 1);
    let expected = format!("{}:2", archive_path.display());
    assert_eq!(outflow.malformed_lines[0], expected);

    let _ = fs::remove_dir_all(&dir);
}

/// (6) `--repo` scoping: a two-repo fixture where a `repo_filter` of one repo reads
/// only that repo's archive.
#[test]
fn repo_filter_scopes_which_archives_are_read() {
    let dir = temp_dir("repo-scope");
    write_archive(
        &dir,
        "alpha",
        &[row("e1", "cleared", "2026-08-20", Some(false))],
    );
    write_archive(
        &dir,
        "beta",
        &[
            row("e2", "cleared", "2026-08-20", Some(false)),
            row("e3", "withdrawn", "2026-08-20", Some(false)),
        ],
    );

    let files = vec![source_for(&dir, "alpha"), source_for(&dir, "beta")];

    let outflow_all = read_archive_outflow(&files, "2026-08-22", 30, None);
    assert_eq!(outflow_all.rows_total, 3);
    assert_eq!(outflow_all.archives_read, 2);

    let outflow_alpha_only = read_archive_outflow(&files, "2026-08-22", 30, Some("alpha"));
    assert_eq!(outflow_alpha_only.rows_total, 1);
    assert_eq!(outflow_alpha_only.archives_read, 1);
    assert_eq!(outflow_alpha_only.archives_missing, 0);

    let _ = fs::remove_dir_all(&dir);
}

/// (7) A row whose `disposed_at` is outside `window_days` counts toward `rows_total`
/// but not `rows_in_window`.
#[test]
fn row_outside_window_counts_toward_total_not_window() {
    let dir = temp_dir("window");
    let lines = vec![
        row("f1", "cleared", "2026-08-20", Some(false)), // 2 days before today, in window
        row("f2", "cleared", "2026-01-01", Some(false)), // far outside window
    ];
    write_archive(&dir, "alpha", &lines);

    let files = vec![source_for(&dir, "alpha")];
    let outflow = read_archive_outflow(&files, "2026-08-22", 30, None);

    assert_eq!(outflow.rows_total, 2);
    assert_eq!(
        outflow.rows_in_window, 1,
        "only the row disposed inside the 30-day window should count"
    );

    let _ = fs::remove_dir_all(&dir);
}
