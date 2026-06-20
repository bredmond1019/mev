//! Phase 0 smoke test: the library surface is wired up and an empty/clean tree passes.
//! Real fixture-driven checks arrive with Phase 1 (crawl/parse/validate).
//!
//! Task 5 wiring tests: `validate()` now dispatches struct/frontmatter checks via
//! `meta::validate_file` — these tests confirm the wiring by running `validate()` end-to-end.

use std::fs;

use mev::{Diagnostic, Severity};

#[test]
fn empty_tree_produces_clean_report() {
    let dir = std::env::temp_dir().join("mev-smoke-empty");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    let report = mev::validate(&dir).unwrap();

    assert_eq!(report.error_count(), 0);
    assert_eq!(report.warning_count(), 0);
    assert!(!report.is_failure());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn diagnostic_severity_drives_failure() {
    let err = Diagnostic::error("a.json", "metadata.id", "missing id");
    let warn = Diagnostic::warning("b.mdx", "", "no frontmatter");
    assert_eq!(err.severity, Severity::Error);
    assert_eq!(warn.severity, Severity::Warning);
}

// ---------------------------------------------------------------------------
// Task 5 wiring: validate() dispatches struct/frontmatter checks
// ---------------------------------------------------------------------------

/// A module `.json` with valid filename conventions but invalid JSON content must produce a
/// struct-level error through `validate()`, confirming `validate_file` is wired in.
#[test]
fn validate_surfaces_struct_errors_for_invalid_module_json() {
    let dir = std::env::temp_dir().join("mev-smoke-invalid-json");
    let _ = fs::remove_dir_all(&dir);

    // Build a minimal tree: valid filename, invalid JSON content.
    let module_dir = dir.join("paths/intro/modules");
    fs::create_dir_all(&module_dir).unwrap();
    fs::write(module_dir.join("01-intro.json"), b"{ not valid json }").unwrap();

    let report = mev::validate(&dir).unwrap();

    assert!(
        report.is_failure(),
        "expected at least one error for invalid module JSON, got: {:?}",
        report.diagnostics
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error && d.message.contains("invalid module JSON")),
        "expected 'invalid module JSON' error, got: {:?}",
        report.diagnostics
    );

    let _ = fs::remove_dir_all(&dir);
}

/// A valid module `.json` + path `metadata.json` tree must produce zero errors through
/// `validate()`, confirming the wiring does not introduce spurious diagnostics.
#[test]
fn validate_good_tree_has_no_errors() {
    let dir = std::env::temp_dir().join("mev-smoke-good-tree");
    let _ = fs::remove_dir_all(&dir);

    // Path metadata.json
    let path_dir = dir.join("paths/intro");
    fs::create_dir_all(&path_dir).unwrap();
    fs::write(
        path_dir.join("metadata.json"),
        br#"{
          "id": "intro",
          "title": "Introduction",
          "description": "A learning path.",
          "level": "beginner",
          "duration": "2 hours",
          "version": "1.0.0",
          "lastUpdated": "2025-06-20",
          "topics": ["basics"],
          "modules": ["01-intro"]
        }"#,
    )
    .unwrap();

    // Module .json
    let module_dir = path_dir.join("modules");
    fs::create_dir_all(&module_dir).unwrap();
    fs::write(
        module_dir.join("01-intro.json"),
        br#"{
          "metadata": {
            "id": "01-intro",
            "pathId": "intro",
            "title": "Introduction Module",
            "description": "A short intro.",
            "duration": "30 minutes",
            "type": "concept",
            "difficulty": "beginner",
            "order": 1,
            "objectives": ["Learn basics"],
            "tags": ["intro"],
            "version": "1.0.0",
            "lastUpdated": "2025-06-20"
          },
          "sections": [
            { "id": "overview", "type": "content", "order": 1 }
          ]
        }"#,
    )
    .unwrap();

    // Module .mdx
    fs::write(
        module_dir.join("01-intro.mdx"),
        b"---\ntitle: Introduction Module\ndescription: A short intro.\nduration: 30 minutes\ndifficulty: beginner\nlastUpdated: \"2025-06-20\"\n---\n\n# Body\n",
    )
    .unwrap();

    let report = mev::validate(&dir).unwrap();

    assert!(
        !report.is_failure(),
        "expected no errors for a valid tree, got: {:?}",
        report.diagnostics
    );

    let _ = fs::remove_dir_all(&dir);
}
