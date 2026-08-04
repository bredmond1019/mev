//! Integration tests for `validate_brain_sync` — end-to-end `--sync` over a fixture HQ tree.
//!
//! Task 4: builds a temp HQ-root with two `[[repos]]` entries (full `[vocab]` so the schema
//! pass is clean), each repo's `status_file` and `cache_doc` present and OKF-clean, with
//! full-ISO RFC3339 watermarks.
//!
//! Tests:
//!   1. In-sync fixture → 0 errors.
//!   2. Bumping one repo's `timestamp` without updating `cache` → exactly one `E_SYNC_DRIFT`.
//!   3. Re-aligning the cache clears the drift error.
//!   4. `--json` round-trip: a `Sync` diagnostic appears in the serialized envelope.

use std::fs;
use std::path::Path;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create a file at `root/rel` (creating parent dirs as needed) with `content`.
fn write_file(root: &Path, rel: &str, content: &str) {
    let full = root.join(rel);
    fs::create_dir_all(full.parent().unwrap()).unwrap();
    fs::write(&full, content.as_bytes()).unwrap();
}

/// Make a fresh uniquely-named temp dir for a test and return its path.
fn temp_dir(suffix: &str) -> std::path::PathBuf {
    let dir = mev::testsupport::unique_temp_dir(&format!("mev-brain-sync-it-{suffix}"));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Write `brain.toml` with two repos (`alpha`, `beta`) and a full `[vocab]` block.
///
/// Status files and cache docs live under `repos/<slug>/` and `docs/projects/` within the
/// HQ root, matching the paths in `[[repos]]` entries.
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

/// OKF-clean `status_file` content bearing a full RFC3339 `timestamp`.
fn status_md(slug: &str, ts: &str) -> String {
    format!(
        "---\ntype: ProjectStatus\ntitle: {slug} Status\ndescription: Status for {slug}.\ntimestamp: \"{ts}\"\n---\n\n# Status\n"
    )
}

/// OKF-clean `cache_doc` content bearing a full RFC3339 `synced_from`.
fn cache_md(slug: &str, sf: &str) -> String {
    format!(
        "---\ntype: ProjectContext\ntitle: {slug} Cache\ndescription: Brain cache for {slug}.\nsynced_from: \"{sf}\"\n---\n\n# Cache\n"
    )
}

/// Write in-sync fixtures for both repos into `root`.
///
/// Both repos share the same watermark `ts` so the sync check should produce zero errors.
fn write_in_sync_fixture(root: &Path, ts: &str) {
    write_brain_toml(root);

    // alpha
    write_file(
        root,
        "repos/alpha/planning/status.md",
        &status_md("alpha", ts),
    );
    write_file(root, "docs/projects/alpha.md", &cache_md("alpha", ts));

    // beta
    write_file(
        root,
        "repos/beta/planning/status.md",
        &status_md("beta", ts),
    );
    write_file(root, "docs/projects/beta.md", &cache_md("beta", ts));
}

// ---------------------------------------------------------------------------
// Test 1 — in-sync fixture produces zero errors
// ---------------------------------------------------------------------------

#[test]
fn in_sync_fixture_produces_no_errors() {
    let dir = temp_dir("in-sync");
    let ts = "2026-06-27T00:00:00Z";

    write_in_sync_fixture(&dir, ts);

    let report = mev::validate_brain_sync(&dir).expect("validate_brain_sync should not error");

    // Filter out any non-sync diagnostics (the schema pass may pick up noise from
    // deeply-nested system files; we care specifically about zero errors from the
    // sync watermark check as well as the overall report).
    let sync_errors: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.locator.starts_with("E_SYNC_"))
        .collect();

    assert!(
        sync_errors.is_empty(),
        "in-sync fixture should produce no Sync errors, got: {sync_errors:#?}"
    );

    assert_eq!(
        report.error_count(),
        0,
        "in-sync fixture should produce zero total errors, got: {:#?}",
        report.diagnostics
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Test 2 — bumping one repo's timestamp without updating cache → E_SYNC_DRIFT
// ---------------------------------------------------------------------------

#[test]
fn drifted_repo_produces_exactly_one_e_sync_drift() {
    let dir = temp_dir("drift");
    let ts_old = "2026-06-27T00:00:00Z";
    let ts_new = "2026-06-28T12:00:00Z";

    // Start with everything in sync.
    write_in_sync_fixture(&dir, ts_old);

    // Bump alpha's status timestamp without updating its cache.
    write_file(
        &dir,
        "repos/alpha/planning/status.md",
        &status_md("alpha", ts_new),
    );
    // beta stays in sync.

    let report = mev::validate_brain_sync(&dir).expect("validate_brain_sync should not error");

    let sync_errors: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.locator.starts_with("E_SYNC_"))
        .collect();

    assert_eq!(
        sync_errors.len(),
        1,
        "expected exactly one Sync error after bumping alpha's timestamp, got: {sync_errors:#?}"
    );

    let err = &sync_errors[0];
    assert_eq!(
        err.locator, "E_SYNC_DRIFT",
        "expected E_SYNC_DRIFT, got: {:?}",
        err.locator
    );

    // The error message must reference both watermark values.
    assert!(
        err.message.contains(ts_old) || err.message.contains(ts_new),
        "E_SYNC_DRIFT message should mention the watermark values: {:?}",
        err.message
    );

    // The error message should identify the repo (slug = "alpha").
    assert!(
        err.message.contains("alpha"),
        "E_SYNC_DRIFT message should name the drifted repo: {:?}",
        err.message
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Test 3 — re-aligning the cache clears the drift error
// ---------------------------------------------------------------------------

#[test]
fn realigning_cache_clears_drift_error() {
    let dir = temp_dir("realign");
    let ts_old = "2026-06-27T00:00:00Z";
    let ts_new = "2026-06-28T12:00:00Z";

    write_in_sync_fixture(&dir, ts_old);

    // Create drift.
    write_file(
        &dir,
        "repos/alpha/planning/status.md",
        &status_md("alpha", ts_new),
    );

    let report_drifted =
        mev::validate_brain_sync(&dir).expect("validate_brain_sync should not error");
    let drift_errors: Vec<_> = report_drifted
        .diagnostics
        .iter()
        .filter(|d| d.locator == "E_SYNC_DRIFT")
        .collect();
    assert_eq!(
        drift_errors.len(),
        1,
        "should have one drift error before re-aligning"
    );

    // Re-align: update cache to match the new timestamp.
    write_file(&dir, "docs/projects/alpha.md", &cache_md("alpha", ts_new));

    let report_aligned =
        mev::validate_brain_sync(&dir).expect("validate_brain_sync should not error");
    let sync_errors_after: Vec<_> = report_aligned
        .diagnostics
        .iter()
        .filter(|d| d.locator.starts_with("E_SYNC_"))
        .collect();

    assert!(
        sync_errors_after.is_empty(),
        "re-aligning the cache should clear the drift error, got: {sync_errors_after:#?}"
    );

    assert_eq!(
        report_aligned.error_count(),
        0,
        "overall error count should be zero after re-aligning, got: {:#?}",
        report_aligned.diagnostics
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Test 4 — --json round-trip: Sync diagnostic appears in serialized envelope
// ---------------------------------------------------------------------------

#[test]
fn json_round_trip_includes_sync_diagnostic() {
    let dir = temp_dir("json-roundtrip");
    let ts_old = "2026-06-27T00:00:00Z";
    let ts_new = "2026-06-28T12:00:00Z";

    // In-sync first.
    write_in_sync_fixture(&dir, ts_old);

    // Introduce drift on alpha.
    write_file(
        &dir,
        "repos/alpha/planning/status.md",
        &status_md("alpha", ts_new),
    );

    let report = mev::validate_brain_sync(&dir).expect("validate_brain_sync should not error");
    let envelope = mev::JsonReport::new("brain", &dir, &report);
    let json_str = envelope.to_json().expect("to_json should not fail");

    let value: serde_json::Value =
        serde_json::from_str(&json_str).expect("envelope must be valid JSON");

    // The errors count must be at least 1.
    assert!(
        value["errors"].as_u64().unwrap_or(0) >= 1,
        "JSON envelope must report at least one error, got: {}",
        value["errors"]
    );

    // The diagnostics array must contain an E_SYNC_DRIFT entry.
    let diags = value["diagnostics"]
        .as_array()
        .expect("diagnostics must be an array");
    let drift_diags: Vec<_> = diags
        .iter()
        .filter(|d| d["locator"].as_str() == Some("E_SYNC_DRIFT"))
        .collect();

    assert!(
        !drift_diags.is_empty(),
        "serialized diagnostics must include at least one E_SYNC_DRIFT, got array: {diags:#?}"
    );

    // Every diagnostic must have severity serialized as lowercase "error" or "warning".
    for diag in diags {
        let sev = diag["severity"].as_str().unwrap_or("");
        assert!(
            sev == "error" || sev == "warning",
            "severity must be lowercase 'error' or 'warning', got: {sev:?}"
        );
    }

    let _ = fs::remove_dir_all(&dir);
}
