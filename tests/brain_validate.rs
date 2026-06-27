//! Integration tests for the `validate_brain` public function and `JsonReport` envelope.
//!
//! Verifies that `validate_brain` honors the crawl skip-list end-to-end (nested-git sub-dirs
//! are pruned), that OKF violations are detected, and that the JSON envelope serializes
//! correctly with the expected keys and counts.

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

/// Make a fresh uniquely-named temp dir for a test.
fn temp_dir(suffix: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("mev-brain-validate-{suffix}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Minimal valid OKF frontmatter.
const VALID_OKF: &str =
    "---\ntype: Reference\ntitle: Test Doc\ndescription: A test doc.\n---\n\nBody text.\n";

/// OKF frontmatter with a deliberate violation: missing required `title` field.
const MISSING_TITLE: &str =
    "---\ntype: Reference\ndescription: A test doc without a title.\n---\n\nBody text.\n";

// ---------------------------------------------------------------------------
// 1. validate_brain detects OKF violations in a root-level .md file
// ---------------------------------------------------------------------------

#[test]
fn validate_brain_detects_missing_title() {
    let dir = temp_dir("missing-title");

    write_file(&dir, "doc.md", MISSING_TITLE);

    let report = mev::validate_brain(&dir).expect("validate_brain should not error");

    assert!(
        report.error_count() > 0,
        "expected at least one error for missing title, got: {:#?}",
        report.diagnostics
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 2. validate_brain skips nested-git sub-dirs (crawl skip-list honored end-to-end)
// ---------------------------------------------------------------------------

#[test]
fn validate_brain_skips_nested_git_subdir() {
    let dir = temp_dir("nested-git-skip");

    // Root-level file with a violation — must be flagged.
    write_file(&dir, "root.md", MISSING_TITLE);

    // Nested-git sub-project: has its own .git marker.
    let sub = dir.join("sub-project");
    fs::create_dir_all(sub.join(".git")).unwrap();
    // A .md inside the sub-project — must NOT be validated.
    write_file(&dir, "sub-project/README.md", MISSING_TITLE);

    let report = mev::validate_brain(&dir).expect("validate_brain should not error");

    // Only the root file should be flagged; the nested file is skipped.
    // Count distinct files referenced in diagnostics.
    let files_flagged: std::collections::HashSet<_> = report
        .diagnostics
        .iter()
        .map(|d| d.file.to_string_lossy().to_string())
        .collect();

    for file in &files_flagged {
        assert!(
            !file.contains("sub-project"),
            "nested-git sub-dir must be skipped, but found diagnostic for: {file}"
        );
    }

    // The root file must have produced at least one diagnostic.
    assert!(
        !files_flagged.is_empty(),
        "expected diagnostics for the root file"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 3. validate_brain returns no errors for a valid OKF file
// ---------------------------------------------------------------------------

#[test]
fn validate_brain_no_errors_for_valid_file() {
    let dir = temp_dir("valid-okf");

    write_file(&dir, "valid.md", VALID_OKF);

    let report = mev::validate_brain(&dir).expect("validate_brain should not error");

    assert_eq!(
        report.error_count(),
        0,
        "expected zero errors for valid OKF file, got: {:#?}",
        report.diagnostics
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 4. JsonReport serializes to valid JSON with expected keys and counts
// ---------------------------------------------------------------------------

#[test]
fn json_report_valid_json_with_expected_keys() {
    let dir = temp_dir("json-envelope");

    write_file(&dir, "doc.md", MISSING_TITLE);

    let report = mev::validate_brain(&dir).expect("validate_brain should not error");
    let envelope = mev::JsonReport::new("brain", &dir, &report);
    let json_str = envelope.to_json().expect("to_json should not fail");

    // Must parse as valid JSON.
    let value: serde_json::Value =
        serde_json::from_str(&json_str).expect("envelope must be valid JSON");

    // Check required keys.
    assert!(value.get("validator").is_some(), "missing 'validator' key");
    assert!(value.get("root").is_some(), "missing 'root' key");
    assert!(value.get("errors").is_some(), "missing 'errors' key");
    assert!(value.get("warnings").is_some(), "missing 'warnings' key");
    assert!(
        value.get("diagnostics").is_some(),
        "missing 'diagnostics' key"
    );

    // validator must be "brain".
    assert_eq!(
        value["validator"].as_str().unwrap(),
        "brain",
        "validator must be 'brain'"
    );

    // errors/warnings must match the report's counts.
    assert_eq!(
        value["errors"].as_u64().unwrap() as usize,
        report.error_count(),
        "errors count mismatch"
    );
    assert_eq!(
        value["warnings"].as_u64().unwrap() as usize,
        report.warning_count(),
        "warnings count mismatch"
    );

    // diagnostics array length must match.
    let diag_array = value["diagnostics"].as_array().unwrap();
    assert_eq!(
        diag_array.len(),
        report.diagnostics.len(),
        "diagnostics length mismatch"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 5. JsonReport: Severity serializes as lowercase strings
// ---------------------------------------------------------------------------

#[test]
fn json_report_severity_is_lowercase() {
    let dir = temp_dir("severity-lowercase");

    write_file(&dir, "bad.md", MISSING_TITLE);

    let report = mev::validate_brain(&dir).expect("validate_brain should not error");
    assert!(
        !report.diagnostics.is_empty(),
        "need at least one diagnostic to check severity serialization"
    );

    let envelope = mev::JsonReport::new("brain", &dir, &report);
    let json_str = envelope.to_json().expect("to_json should not fail");
    let value: serde_json::Value = serde_json::from_str(&json_str).unwrap();

    let diags = value["diagnostics"].as_array().unwrap();
    for diag in diags {
        let sev = diag["severity"].as_str().unwrap();
        assert!(
            sev == "error" || sev == "warning",
            "severity must be 'error' or 'warning', got: {sev}"
        );
    }

    let _ = fs::remove_dir_all(&dir);
}
