//! Integration tests for the `validate_brain` public function and `JsonReport` envelope.
//!
//! Verifies that `validate_brain` honors the crawl skip-list end-to-end (nested-git sub-dirs
//! are pruned), that OKF violations are detected, and that the JSON envelope serializes
//! correctly with the expected keys and counts.
//!
//! Task 4: `validate_brain` now resolves `brain.toml` via walk-up. All tests that need a
//! real validation pass (no spurious `E_CONFIG_NOT_FOUND`) must write a minimal `brain.toml`
//! to the temp directory before calling `validate_brain`.

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

/// Write a minimal `brain.toml` to `root` so that `find_brain_config` finds it immediately.
///
/// The vocab matches the standard controlled sets; project slugs include the common ones used
/// in tests. Tests that need custom vocab must write their own `brain.toml` instead.
fn write_brain_toml(root: &Path) {
    write_brain_toml_with_layers(
        root,
        &[
            "brain", "engine", "factory", "console", "surface", "infra", "business", "content",
            "meta",
        ],
    );
}

/// Write a `brain.toml` to `root` with a custom `layer` list (and standard status + repos).
fn write_brain_toml_with_layers(root: &Path, layers: &[&str]) {
    let layer_toml = layers
        .iter()
        .map(|s| format!("\"{s}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let toml = format!(
        r#"[vocab]
layer = [{layer_toml}]
status = ["active", "draft", "deprecated", "superseded", "archived"]

[crawl]
skip_dirs = ["target", "node_modules", ".git"]

[[repos]]
slug = "brain"
tier = "primary"
repo_path = "."
status_file = "planning/status.md"
cache_doc = "docs/projects/brain.md"
heading = "Brain"

[[repos]]
slug = "mev"
tier = "primary"
repo_path = "core/mev"
status_file = "planning/status.md"
cache_doc = "docs/projects/mev.md"
heading = "mev"
"#
    );
    fs::write(root.join("brain.toml"), toml.as_bytes()).unwrap();
}

/// Minimal valid OKF frontmatter.
const VALID_OKF: &str =
    "---\ntype: Reference\ntitle: Test Doc\ndescription: A test doc.\n---\n\nBody text.\n";

/// OKF frontmatter with a deliberate violation: missing required `title` field.
const MISSING_TITLE: &str =
    "---\ntype: Reference\ndescription: A test doc without a title.\n---\n\nBody text.\n";

// ---------------------------------------------------------------------------
// 1. validate_brain detects OKF violations in a corpus-member .md file
// ---------------------------------------------------------------------------

#[test]
fn validate_brain_detects_missing_title() {
    let dir = temp_dir("missing-title");

    write_brain_toml(&dir);
    // Corpus members live under planning/ or docs/ — place the file there.
    write_file(&dir, "planning/doc.md", MISSING_TITLE);

    let report = mev::validate_brain(&dir).expect("validate_brain should not error");

    // Confirm no E_CONFIG_NOT_FOUND error masked the real validation.
    let config_errs: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.locator == "E_CONFIG_NOT_FOUND")
        .collect();
    assert!(
        config_errs.is_empty(),
        "unexpected E_CONFIG_NOT_FOUND: {config_errs:#?}"
    );

    assert!(
        report.error_count() > 0,
        "expected at least one error for missing title, got: {:#?}",
        report.diagnostics
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 2. validate_brain excludes non-corpus files (corpus membership rule end-to-end)
// ---------------------------------------------------------------------------

#[test]
fn validate_brain_skips_nested_git_subdir() {
    let dir = temp_dir("nested-git-skip");

    write_brain_toml(&dir);

    // A corpus member under planning/ with a violation — must be flagged.
    write_file(&dir, "planning/root.md", MISSING_TITLE);

    // A .md nested under a non-planning/docs path — not a corpus member; must NOT be validated.
    let sub = dir.join("sub-project");
    fs::create_dir_all(sub.join(".git")).unwrap();
    write_file(&dir, "sub-project/README.md", MISSING_TITLE);

    let report = mev::validate_brain(&dir).expect("validate_brain should not error");

    // sub-project/README.md is not a corpus member — must not appear in diagnostics.
    let files_flagged: std::collections::HashSet<_> = report
        .diagnostics
        .iter()
        .map(|d| d.file.to_string_lossy().to_string())
        .collect();

    for file in &files_flagged {
        assert!(
            !file.contains("sub-project"),
            "non-corpus sub-project file must be excluded, but found diagnostic for: {file}"
        );
    }

    // The planning/root.md must have produced at least one diagnostic.
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

    write_brain_toml(&dir);
    // Corpus member under planning/ — file is crawled and validated.
    write_file(&dir, "planning/valid.md", VALID_OKF);

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

    write_brain_toml(&dir);
    // Corpus member under planning/ so the violation is actually detected.
    write_file(&dir, "planning/doc.md", MISSING_TITLE);

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

    write_brain_toml(&dir);
    // Corpus member under planning/ so a diagnostic is actually produced.
    write_file(&dir, "planning/bad.md", MISSING_TITLE);

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

// ---------------------------------------------------------------------------
// 6. E_CONFIG_NOT_FOUND: validate_brain surfaces a fatal error when brain.toml
//    is not found by walk-up (no brain.toml in any ancestor of an isolated temp dir).
//
//    NOTE: On a developer machine the temp dir's ancestor chain may eventually reach
//    the portfolio root which contains a real brain.toml — in that case no E_CONFIG_NOT_FOUND
//    is emitted. The assertion here is primarily a no-panic smoke test: validate_brain must
//    return Ok regardless.
// ---------------------------------------------------------------------------

#[test]
fn validate_brain_no_panic_when_brain_toml_may_be_absent() {
    // Create a dir without brain.toml — walk-up may or may not find one.
    let dir = temp_dir("no-brain-toml-ancestor");
    // Do NOT call write_brain_toml.

    // Must not panic — validate_brain always returns Ok.
    let result = mev::validate_brain(&dir);
    assert!(
        result.is_ok(),
        "validate_brain must return Ok, got: {result:?}"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 7. Config-flip: a config-only change to brain.toml flips a validation result
//    without any Rust source change.
//    This is the direct acceptance-criterion test for Task 4.
// ---------------------------------------------------------------------------

#[test]
fn validate_brain_config_flip_changes_result() {
    let dir = temp_dir("config-flip");

    // --- Step 1: brain.toml with custom-layer in the vocabulary. ---
    write_brain_toml_with_layers(&dir, &["custom-layer"]);

    // A .md file that uses the custom layer value — placed under planning/ (corpus member).
    let doc_content = "---\ntype: Reference\ntitle: Custom Layer Doc\ndescription: Tests config-flip.\nlayer: [custom-layer]\n---\n\nBody.\n";
    write_file(&dir, "planning/doc.md", doc_content);

    let report_with_layer = mev::validate_brain(&dir).expect("validate_brain should not error");

    // No layer errors or config errors expected.
    let layer_errors_before: Vec<_> = report_with_layer
        .diagnostics
        .iter()
        .filter(|d| d.locator == "layer" || d.locator == "E_CONFIG_NOT_FOUND")
        .collect();
    assert!(
        layer_errors_before.is_empty(),
        "expected no layer/config errors when custom-layer is in vocab, got: {layer_errors_before:#?}"
    );

    // --- Step 2: Rewrite brain.toml WITHOUT custom-layer (config-only change, no source edit). ---
    write_brain_toml_with_layers(&dir, &["brain", "engine"]); // custom-layer removed

    let report_without_layer = mev::validate_brain(&dir).expect("validate_brain should not error");

    // Now a layer error for 'custom-layer' must appear.
    let layer_errors_after: Vec<_> = report_without_layer
        .diagnostics
        .iter()
        .filter(|d| d.locator == "layer")
        .collect();
    assert!(
        !layer_errors_after.is_empty(),
        "expected a layer error after removing custom-layer from vocab; got: {:#?}",
        report_without_layer.diagnostics
    );
    assert!(
        layer_errors_after
            .iter()
            .any(|d| d.message.contains("custom-layer")),
        "expected layer error message to mention 'custom-layer'; got: {layer_errors_after:#?}"
    );

    let _ = fs::remove_dir_all(&dir);
}
