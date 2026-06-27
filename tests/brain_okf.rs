//! Integration tests for the brain OKF frontmatter validator (Phase 2, Block H).
//!
//! Writes `.md` fixtures to a temp dir and drives them through [`BrainValidator`]
//! (via the [`ContentValidator`] trait) or directly through [`validate_md_file`] on
//! an [`MdFile`].  Mirrors the style of `tests/meta.rs` and `tests/brain_crawl.rs`.
//!
//! After Task 3, all `validate_md_file` calls require a `&BrainConfig`.
//! Tests load the standard fixture from `tests/fixtures/brain.toml` via
//! `fixture_config()` so vocabulary lookups resolve correctly.

use std::path::PathBuf;

use mev::ContentValidator;
use mev::brain::config::{BrainConfig, load_brain_config};
use mev::{BrainValidator, MdFile, Severity, validate_md_file};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn temp_dir(suffix: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mev-brain-okf-it-{suffix}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_md(dir: &PathBuf, name: &str, content: &str) -> MdFile {
    let path = dir.join(name);
    std::fs::write(&path, content).unwrap();
    MdFile {
        path,
        rel: PathBuf::from(name),
        stem: name.trim_end_matches(".md").to_string(),
    }
}

/// Load the standard test fixture (`tests/fixtures/brain.toml`).
fn fixture_config() -> BrainConfig {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("brain.toml");
    load_brain_config(&path).expect("fixture brain.toml must parse")
}

/// A fully-valid OKF frontmatter body.
///
/// Uses `project: brain` — the `brain` slug is present in `tests/fixtures/brain.toml`
/// and satisfies the config-driven project vocab check.
fn good_okf_body() -> &'static str {
    "---\n\
type: Decision\n\
title: My Decision\n\
description: A one-line summary.\n\
doc_id: my-decision\n\
layer: [brain]\n\
project: brain\n\
status: active\n\
keywords: [rust, cli, validation]\n\
related: [context]\n\
---\n\n# Body\n"
}

// ---------------------------------------------------------------------------
// validate_md_file — direct unit-style integration tests
// ---------------------------------------------------------------------------

#[test]
fn good_okf_doc_is_clean() {
    let dir = temp_dir("good-doc");
    let mf = write_md(&dir, "status.md", good_okf_body());
    let cfg = fixture_config();
    let diags = validate_md_file(&mf, &cfg);
    assert!(diags.is_empty(), "expected no diagnostics, got: {diags:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn missing_required_fields_emit_errors() {
    // A document with no frontmatter fields at all — type, title, description must all error.
    let dir = temp_dir("missing-required");
    let mf = write_md(&dir, "bare.md", "---\nextra: tolerated\n---\nbody\n");
    let cfg = fixture_config();
    let diags = validate_md_file(&mf, &cfg);
    let locators: Vec<&str> = diags.iter().map(|d| d.locator.as_str()).collect();
    for expected in ["type", "title", "description"] {
        assert!(
            locators.contains(&expected),
            "expected locator '{expected}' in {locators:?}"
        );
    }
    assert!(
        diags.iter().all(|d| d.severity == Severity::Error),
        "all diagnostics should be errors, got: {diags:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bad_layer_member_emits_error() {
    let dir = temp_dir("bad-layer");
    let body = "---\ntype: T\ntitle: T\ndescription: D\nlayer: [unknown-layer]\n---\nbody\n";
    let mf = write_md(&dir, "doc.md", body);
    let cfg = fixture_config();
    let diags = validate_md_file(&mf, &cfg);
    assert_eq!(diags.len(), 1, "got: {diags:?}");
    assert_eq!(diags[0].severity, Severity::Error);
    assert_eq!(diags[0].locator, "layer");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bad_project_emits_error() {
    let dir = temp_dir("bad-project");
    let body = "---\ntype: T\ntitle: T\ndescription: D\nproject: not-a-project\n---\nbody\n";
    let mf = write_md(&dir, "doc.md", body);
    let cfg = fixture_config();
    let diags = validate_md_file(&mf, &cfg);
    assert_eq!(diags.len(), 1, "got: {diags:?}");
    assert_eq!(diags[0].severity, Severity::Error);
    assert_eq!(diags[0].locator, "project");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bad_status_emits_error() {
    let dir = temp_dir("bad-status");
    let body = "---\ntype: T\ntitle: T\ndescription: D\nstatus: unknown-status\n---\nbody\n";
    let mf = write_md(&dir, "doc.md", body);
    let cfg = fixture_config();
    let diags = validate_md_file(&mf, &cfg);
    assert_eq!(diags.len(), 1, "got: {diags:?}");
    assert_eq!(diags[0].severity, Severity::Error);
    assert_eq!(diags[0].locator, "status");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn non_kebab_doc_id_emits_error() {
    let dir = temp_dir("bad-doc-id");
    let body = "---\ntype: T\ntitle: T\ndescription: D\ndoc_id: BadId_123\n---\nbody\n";
    let mf = write_md(&dir, "doc.md", body);
    let cfg = fixture_config();
    let diags = validate_md_file(&mf, &cfg);
    assert_eq!(diags.len(), 1, "got: {diags:?}");
    assert_eq!(diags[0].severity, Severity::Error);
    assert_eq!(diags[0].locator, "doc_id");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn keywords_too_few_emits_warning() {
    let dir = temp_dir("keywords-low");
    let body = "---\ntype: T\ntitle: T\ndescription: D\nkeywords: [one]\n---\nbody\n";
    let mf = write_md(&dir, "doc.md", body);
    let cfg = fixture_config();
    let diags = validate_md_file(&mf, &cfg);
    assert_eq!(diags.len(), 1, "got: {diags:?}");
    assert_eq!(diags[0].severity, Severity::Warning);
    assert_eq!(diags[0].locator, "keywords");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn keywords_too_many_emits_warning() {
    let dir = temp_dir("keywords-high");
    let body =
        "---\ntype: T\ntitle: T\ndescription: D\nkeywords: [a, b, c, d, e, f, g, h]\n---\nbody\n";
    let mf = write_md(&dir, "doc.md", body);
    let cfg = fixture_config();
    let diags = validate_md_file(&mf, &cfg);
    assert_eq!(diags.len(), 1, "got: {diags:?}");
    assert_eq!(diags[0].severity, Severity::Warning);
    assert_eq!(diags[0].locator, "keywords");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn no_frontmatter_emits_single_error() {
    let dir = temp_dir("no-fm");
    let mf = write_md(&dir, "doc.md", "# Heading only\n");
    let cfg = fixture_config();
    let diags = validate_md_file(&mf, &cfg);
    assert_eq!(diags.len(), 1, "got: {diags:?}");
    assert_eq!(diags[0].severity, Severity::Error);
    assert_eq!(diags[0].locator, "frontmatter");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn malformed_yaml_emits_single_error() {
    let dir = temp_dir("bad-yaml");
    let mf = write_md(&dir, "doc.md", "---\ntitle: [unclosed\n---\nbody\n");
    let cfg = fixture_config();
    let diags = validate_md_file(&mf, &cfg);
    assert_eq!(diags.len(), 1, "got: {diags:?}");
    assert_eq!(diags[0].severity, Severity::Error);
    assert_eq!(diags[0].locator, "frontmatter");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// BrainValidator (ContentValidator trait) — end-to-end
// ---------------------------------------------------------------------------

#[test]
fn brain_validator_run_clean_tree_returns_empty_report() {
    let dir = temp_dir("brain-run-clean");
    // Write one good OKF doc (uses project: brain, which is in the fixture config).
    write_md(&dir, "status.md", good_okf_body());

    let report = BrainValidator::new(fixture_config()).run(&dir);
    assert!(
        report.diagnostics.is_empty(),
        "expected empty report for valid doc, got: {:?}",
        report.diagnostics
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn brain_validator_run_violation_tree_returns_errors() {
    let dir = temp_dir("brain-run-violations");
    // Write one doc that is missing all required fields.
    write_md(&dir, "bare.md", "---\nextra: tolerated\n---\nbody\n");

    let report = BrainValidator::new(BrainConfig::default()).run(&dir);
    assert!(
        report.is_failure(),
        "expected failure for missing required fields, got: {:?}",
        report.diagnostics
    );
    let locators: Vec<&str> = report
        .diagnostics
        .iter()
        .map(|d| d.locator.as_str())
        .collect();
    for expected in ["type", "title", "description"] {
        assert!(
            locators.contains(&expected),
            "expected locator '{expected}' in {locators:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn brain_validator_run_mixed_tree_collects_all_diagnostics() {
    let dir = temp_dir("brain-run-mixed");
    // Good doc (project: brain is in the fixture config).
    write_md(&dir, "good.md", good_okf_body());
    // Bad doc — missing title.
    write_md(&dir, "bad.md", "---\ntype: T\ndescription: D\n---\nbody\n");

    let report = BrainValidator::new(fixture_config()).run(&dir);
    // Should have exactly one error: missing title on bad.md.
    assert_eq!(report.error_count(), 1, "got: {:?}", report.diagnostics);
    assert_eq!(report.diagnostics[0].locator, "title");
    assert_eq!(
        report.diagnostics[0].file,
        PathBuf::from("bad.md"),
        "diagnostic should point at bad.md"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn brain_validator_prunes_nested_git_repos() {
    // Confirms that Block G's nested-git pruning integrates end-to-end with BrainValidator.
    let dir = temp_dir("brain-pruning");
    // A nested sub-project (contains .git) — its .md files must be skipped.
    let subproject = dir.join("subproject");
    std::fs::create_dir_all(subproject.join(".git")).unwrap();
    std::fs::write(
        subproject.join("README.md"),
        "# no frontmatter — would fail if read",
    )
    .unwrap();
    // One valid .md at the root.
    write_md(&dir, "root.md", good_okf_body());

    let report = BrainValidator::new(fixture_config()).run(&dir);
    assert!(
        report.diagnostics.is_empty(),
        "nested-git docs must be pruned; got: {:?}",
        report.diagnostics
    );
    let _ = std::fs::remove_dir_all(&dir);
}
