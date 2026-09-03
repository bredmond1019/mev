//! Fixture-driven integration tests for struct- and frontmatter-level validation (Phase 1, Block C).
//!
//! Each test writes real files to a temp-dir fixture tree, calls [`mev::validate_file`] (or
//! [`mev::validate`] for full-pipeline checks), and asserts on diagnostic locators and severities.
//!
//! Coverage:
//! - Good module `.json` + path `metadata.json` + `.mdx` → zero error diagnostics.
//! - Broken module `.json` variants: missing `duration`; bad `difficulty` enum; non-kebab `id`;
//!   malformed `duration`; empty `sections[]`; section missing `id`.
//! - Broken path `metadata.json` variant: missing `modules`.
//! - Broken `.mdx` variants: missing frontmatter block; missing required frontmatter key;
//!   malformed YAML in frontmatter.

use std::fs;
use std::path::{Path, PathBuf};

use mev::{FileKind, Locale, validate_file};
use mev_learn_ai::Severity;

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

/// Create a fresh temp dir with a unique per-test suffix.
fn temp_dir(suffix: &str) -> PathBuf {
    let dir = mev::testsupport::unique_temp_dir(&format!("mev-meta-integ-{suffix}"));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Write `contents` to `root/rel`, creating parent dirs as needed.
fn write_file(root: &Path, rel: &str, contents: &[u8]) -> PathBuf {
    let full = root.join(rel);
    fs::create_dir_all(full.parent().unwrap()).unwrap();
    fs::write(&full, contents).unwrap();
    full
}

/// Build a [`mev::ContentFile`] pointing at `path` with `rel` as its relative locator.
fn content_file(path: PathBuf, rel: &str, kind: FileKind) -> mev::ContentFile {
    mev::ContentFile {
        path,
        rel: PathBuf::from(rel),
        kind,
        path_id: "demo".to_string(),
        module_id: None,
        locale: Locale::En,
    }
}

/// Convenience: collect diagnostic locators from a slice.
fn locators(diags: &[mev::Diagnostic]) -> Vec<&str> {
    diags.iter().map(|d| d.locator.as_str()).collect()
}

// ---------------------------------------------------------------------------
// Fixture bodies
// ---------------------------------------------------------------------------

/// A fully-valid learn-module `.json` body.
const GOOD_MODULE_JSON: &str = r#"{
  "metadata": {
    "id": "intro-to-mcp",
    "pathId": "mcp-fundamentals",
    "title": "What is MCP?",
    "description": "An introduction to MCP.",
    "duration": "30 minutes",
    "type": "concept",
    "difficulty": "beginner",
    "order": 1,
    "objectives": ["Understand MCP"],
    "tags": ["mcp"],
    "version": "1.0.0",
    "lastUpdated": "2025-06-20",
    "author": "extra key tolerated"
  },
  "sections": [
    { "id": "overview", "type": "content", "order": 1, "title": "Overview" }
  ]
}"#;

/// A fully-valid path `metadata.json` body.
const GOOD_PATH_META_JSON: &str = r#"{
  "id": "mcp-fundamentals",
  "title": "MCP Fundamentals",
  "description": "A learning path covering MCP.",
  "level": "Intermediate",
  "duration": "4 hours",
  "version": "1.0.0",
  "lastUpdated": "2025-06-20",
  "topics": ["mcp", "agents"],
  "modules": ["intro-to-mcp"],
  "author": "Brandon J. Redmond"
}"#;

/// A fully-valid `.mdx` file body.
const GOOD_MDX: &str = "---\ntitle: Introduction to MCP\ndescription: A short intro.\nduration: 30 minutes\ndifficulty: beginner\nlastUpdated: \"2025-06-20\"\n---\n\n# Body\n";

// ---------------------------------------------------------------------------
// 1. Good fixtures produce zero error diagnostics
// ---------------------------------------------------------------------------

/// All three file types with valid content → zero error diagnostics.
#[test]
fn good_module_json_fixture_is_clean() {
    let dir = temp_dir("good-module-json");
    let path = write_file(
        &dir,
        "paths/demo/modules/01-intro.json",
        GOOD_MODULE_JSON.as_bytes(),
    );
    let cf = content_file(
        path,
        "paths/demo/modules/01-intro.json",
        FileKind::LearnModuleJson,
    );

    let diags = validate_file(&cf);
    assert!(diags.is_empty(), "expected no diagnostics, got: {diags:?}");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn good_path_metadata_json_fixture_is_clean() {
    let dir = temp_dir("good-path-meta");
    let path = write_file(
        &dir,
        "paths/demo/metadata.json",
        GOOD_PATH_META_JSON.as_bytes(),
    );
    let cf = content_file(path, "paths/demo/metadata.json", FileKind::PathMetadataJson);

    let diags = validate_file(&cf);
    assert!(diags.is_empty(), "expected no diagnostics, got: {diags:?}");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn good_mdx_fixture_is_clean() {
    let dir = temp_dir("good-mdx");
    let path = write_file(&dir, "paths/demo/modules/01-intro.mdx", GOOD_MDX.as_bytes());
    let cf = content_file(path, "paths/demo/modules/01-intro.mdx", FileKind::ModuleMdx);

    let diags = validate_file(&cf);
    assert!(diags.is_empty(), "expected no diagnostics, got: {diags:?}");

    let _ = fs::remove_dir_all(&dir);
}

/// Full pipeline through `validate()`: a tree with all three good files → no errors.
#[test]
fn good_full_tree_via_validate_has_no_errors() {
    let dir = temp_dir("good-full-tree");

    write_file(
        &dir,
        "paths/demo/metadata.json",
        GOOD_PATH_META_JSON.as_bytes(),
    );
    write_file(
        &dir,
        "paths/demo/modules/01-intro.json",
        GOOD_MODULE_JSON.as_bytes(),
    );
    write_file(&dir, "paths/demo/modules/01-intro.mdx", GOOD_MDX.as_bytes());

    let report = mev::validate(&dir).unwrap();
    assert!(
        !report.is_failure(),
        "expected no errors for a valid tree, got: {:?}",
        report.diagnostics
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 2. Broken module `.json` variants
// ---------------------------------------------------------------------------

/// Missing `duration` field → exactly one error with locator `metadata.duration`.
#[test]
fn module_json_missing_duration_emits_locator() {
    let body = GOOD_MODULE_JSON.replace("\"duration\": \"30 minutes\",\n", "");
    let dir = temp_dir("mod-missing-duration");
    let path = write_file(&dir, "paths/demo/modules/01-intro.json", body.as_bytes());
    let cf = content_file(
        path,
        "paths/demo/modules/01-intro.json",
        FileKind::LearnModuleJson,
    );

    let diags = validate_file(&cf);
    assert_eq!(diags.len(), 1, "expected 1 diagnostic, got: {diags:?}");
    assert_eq!(diags[0].locator, "metadata.duration");
    assert_eq!(diags[0].severity, Severity::Error);
    assert_eq!(diags[0].message, "missing required field");

    let _ = fs::remove_dir_all(&dir);
}

/// Bad `difficulty` enum value → exactly one error with locator `metadata.difficulty`.
#[test]
fn module_json_bad_difficulty_enum_emits_locator() {
    let body = GOOD_MODULE_JSON.replace("\"beginner\"", "\"expert\"");
    let dir = temp_dir("mod-bad-difficulty");
    let path = write_file(&dir, "paths/demo/modules/01-intro.json", body.as_bytes());
    let cf = content_file(
        path,
        "paths/demo/modules/01-intro.json",
        FileKind::LearnModuleJson,
    );

    let diags = validate_file(&cf);
    assert_eq!(diags.len(), 1, "expected 1 diagnostic, got: {diags:?}");
    assert_eq!(diags[0].locator, "metadata.difficulty");
    assert_eq!(diags[0].severity, Severity::Error);

    let _ = fs::remove_dir_all(&dir);
}

/// Non-kebab-case `id` → exactly one error with locator `metadata.id`.
#[test]
fn module_json_non_kebab_id_emits_locator() {
    let body = GOOD_MODULE_JSON.replace("\"intro-to-mcp\"", "\"IntroToMCP\"");
    let dir = temp_dir("mod-non-kebab-id");
    let path = write_file(&dir, "paths/demo/modules/01-intro.json", body.as_bytes());
    let cf = content_file(
        path,
        "paths/demo/modules/01-intro.json",
        FileKind::LearnModuleJson,
    );

    let diags = validate_file(&cf);
    assert_eq!(diags.len(), 1, "expected 1 diagnostic, got: {diags:?}");
    assert_eq!(diags[0].locator, "metadata.id");
    assert_eq!(diags[0].severity, Severity::Error);

    let _ = fs::remove_dir_all(&dir);
}

/// Malformed `duration` string → exactly one error with locator `metadata.duration`.
#[test]
fn module_json_malformed_duration_emits_locator() {
    let body = GOOD_MODULE_JSON.replace("\"30 minutes\"", "\"half an hour\"");
    let dir = temp_dir("mod-bad-duration");
    let path = write_file(&dir, "paths/demo/modules/01-intro.json", body.as_bytes());
    let cf = content_file(
        path,
        "paths/demo/modules/01-intro.json",
        FileKind::LearnModuleJson,
    );

    let diags = validate_file(&cf);
    assert_eq!(diags.len(), 1, "expected 1 diagnostic, got: {diags:?}");
    assert_eq!(diags[0].locator, "metadata.duration");
    assert_eq!(diags[0].severity, Severity::Error);

    let _ = fs::remove_dir_all(&dir);
}

/// Empty `sections[]` → exactly one error with locator `sections`.
#[test]
fn module_json_empty_sections_emits_locator() {
    let body = GOOD_MODULE_JSON.replace(
        "\"sections\": [\n    { \"id\": \"overview\", \"type\": \"content\", \"order\": 1, \"title\": \"Overview\" }\n  ]",
        "\"sections\": []",
    );
    let dir = temp_dir("mod-empty-sections");
    let path = write_file(&dir, "paths/demo/modules/01-intro.json", body.as_bytes());
    let cf = content_file(
        path,
        "paths/demo/modules/01-intro.json",
        FileKind::LearnModuleJson,
    );

    let diags = validate_file(&cf);
    assert_eq!(diags.len(), 1, "expected 1 diagnostic, got: {diags:?}");
    assert_eq!(diags[0].locator, "sections");
    assert_eq!(diags[0].severity, Severity::Error);

    let _ = fs::remove_dir_all(&dir);
}

/// A section missing `id` → exactly one error with locator `sections[0].id`.
#[test]
fn module_json_section_missing_id_emits_indexed_locator() {
    let body = GOOD_MODULE_JSON.replace("\"id\": \"overview\", ", "");
    let dir = temp_dir("mod-section-no-id");
    let path = write_file(&dir, "paths/demo/modules/01-intro.json", body.as_bytes());
    let cf = content_file(
        path,
        "paths/demo/modules/01-intro.json",
        FileKind::LearnModuleJson,
    );

    let diags = validate_file(&cf);
    assert_eq!(diags.len(), 1, "expected 1 diagnostic, got: {diags:?}");
    assert_eq!(diags[0].locator, "sections[0].id");
    assert_eq!(diags[0].severity, Severity::Error);

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 3. Broken path `metadata.json` variants
// ---------------------------------------------------------------------------

/// Missing `modules` field → exactly one error with locator `modules`.
#[test]
fn path_metadata_missing_modules_emits_locator() {
    let body = GOOD_PATH_META_JSON.replace("  \"modules\": [\"intro-to-mcp\"],\n", "");
    let dir = temp_dir("path-meta-no-modules");
    let path = write_file(&dir, "paths/demo/metadata.json", body.as_bytes());
    let cf = content_file(path, "paths/demo/metadata.json", FileKind::PathMetadataJson);

    let diags = validate_file(&cf);
    assert_eq!(diags.len(), 1, "expected 1 diagnostic, got: {diags:?}");
    assert_eq!(diags[0].locator, "modules");
    assert_eq!(diags[0].severity, Severity::Error);
    assert_eq!(diags[0].message, "missing required field");

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 4. Broken `.mdx` variants
// ---------------------------------------------------------------------------

/// An `.mdx` file with no frontmatter block → one error with locator `frontmatter`.
#[test]
fn mdx_missing_frontmatter_block_emits_error() {
    let contents = b"# Just body text, no frontmatter block\n";
    let dir = temp_dir("mdx-no-frontmatter");
    let path = write_file(&dir, "paths/demo/modules/01-intro.mdx", contents);
    let cf = content_file(path, "paths/demo/modules/01-intro.mdx", FileKind::ModuleMdx);

    let diags = validate_file(&cf);
    assert_eq!(diags.len(), 1, "expected 1 diagnostic, got: {diags:?}");
    assert_eq!(diags[0].locator, "frontmatter");
    assert_eq!(diags[0].severity, Severity::Error);
    assert!(
        diags[0].message.contains("missing or unterminated"),
        "unexpected message: {}",
        diags[0].message
    );

    let _ = fs::remove_dir_all(&dir);
}

/// An `.mdx` file missing a required frontmatter key (`duration`) → one error with locator
/// `frontmatter.duration`.
#[test]
fn mdx_missing_required_frontmatter_key_emits_locator() {
    let contents = "---\ntitle: T\ndescription: D\ndifficulty: beginner\nlastUpdated: \"2025-06-20\"\n---\nbody\n";
    let dir = temp_dir("mdx-no-duration");
    let path = write_file(&dir, "paths/demo/modules/01-intro.mdx", contents.as_bytes());
    let cf = content_file(path, "paths/demo/modules/01-intro.mdx", FileKind::ModuleMdx);

    let diags = validate_file(&cf);
    assert_eq!(diags.len(), 1, "expected 1 diagnostic, got: {diags:?}");
    assert_eq!(diags[0].locator, "frontmatter.duration");
    assert_eq!(diags[0].severity, Severity::Error);
    assert_eq!(diags[0].message, "missing required field");

    let _ = fs::remove_dir_all(&dir);
}

/// An `.mdx` file with malformed YAML in the frontmatter → one error with locator `frontmatter`.
#[test]
fn mdx_malformed_yaml_frontmatter_emits_error() {
    let contents = "---\ntitle: [unclosed bracket\n---\nbody\n";
    let dir = temp_dir("mdx-bad-yaml");
    let path = write_file(&dir, "paths/demo/modules/01-intro.mdx", contents.as_bytes());
    let cf = content_file(path, "paths/demo/modules/01-intro.mdx", FileKind::ModuleMdx);

    let diags = validate_file(&cf);
    assert_eq!(diags.len(), 1, "expected 1 diagnostic, got: {diags:?}");
    assert_eq!(diags[0].locator, "frontmatter");
    assert_eq!(diags[0].severity, Severity::Error);
    assert!(
        diags[0].message.contains("malformed YAML"),
        "unexpected message: {}",
        diags[0].message
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 5. Confirm Block B and smoke tests are unaffected (via validate())
// ---------------------------------------------------------------------------

/// An empty tree still produces a clean report (smoke / Block B regression).
#[test]
fn empty_tree_still_clean() {
    let dir = temp_dir("empty-regression");
    let report = mev::validate(&dir).unwrap();
    assert_eq!(report.error_count(), 0);
    assert_eq!(report.warning_count(), 0);
    assert!(!report.is_failure());
    let _ = fs::remove_dir_all(&dir);
}

/// The `locators` helper is correct (sanity check for the helper itself).
#[test]
fn locators_helper_extracts_correctly() {
    let diags = vec![
        mev::Diagnostic::error("a.json", "metadata.id", "missing required field"),
        mev::Diagnostic::error("a.json", "metadata.duration", "missing required field"),
    ];
    let locs = locators(&diags);
    assert!(locs.contains(&"metadata.id"));
    assert!(locs.contains(&"metadata.duration"));
}
