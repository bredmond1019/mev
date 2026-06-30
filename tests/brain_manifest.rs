//! Integration tests for the manifest CLI driver (`manifest_brain`) and emitter.
//!
//! Each test builds a small temp-dir fixture with a `brain.toml` and a handful of `.md` files,
//! calls [`mev::manifest_brain`], and asserts on the returned [`mev::Manifest`].

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

/// Create a fresh temp dir (removing any leftovers from a prior run).
fn temp_dir(suffix: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("mev-brain-manifest-{suffix}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Write a minimal `brain.toml` registering one scope unit (`brain` → `.`).
fn write_brain_toml(root: &Path) {
    let toml = r#"[vocab]
layer = ["brain", "engine", "factory", "console", "surface", "infra", "business", "content", "meta"]
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
"#;
    fs::write(root.join("brain.toml"), toml.as_bytes()).unwrap();
}

/// A minimal OKF doc with the given `doc_id` and `title`.
fn okf_doc(doc_id: &str, title: &str) -> String {
    format!(
        "---\ntype: Reference\ntitle: {title}\ndescription: Test doc {doc_id}.\ndoc_id: {doc_id}\nproject: brain\nstatus: active\n---\n\nBody.\n"
    )
}

/// A markdown file with no frontmatter.
fn no_frontmatter_doc() -> &'static str {
    "# Just a heading\n\nNo frontmatter here.\n"
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Basic smoke test: entry count matches the file set, metadata fields round-trip.
#[test]
fn manifest_entry_count_and_fields() {
    let dir = temp_dir("entry-count");
    write_brain_toml(&dir);

    // Three corpus-member files: two with frontmatter, one without.
    write_file(
        &dir,
        "planning/status.md",
        &okf_doc("brain-status", "Brain Status"),
    );
    write_file(
        &dir,
        "docs/index.md",
        &okf_doc("brain-index", "Brain Index"),
    );
    write_file(&dir, "README.md", no_frontmatter_doc());

    let manifest = mev::manifest_brain(&dir).expect("manifest_brain must succeed");

    // All three corpus members appear.
    assert_eq!(
        manifest.entries.len(),
        3,
        "expected 3 entries, got {}: {:#?}",
        manifest.entries.len(),
        manifest.entries.iter().map(|e| &e.rel).collect::<Vec<_>>()
    );

    // Find the status entry.
    let status = manifest
        .entries
        .iter()
        .find(|e| e.rel.ends_with("status.md"))
        .expect("status.md must appear in manifest");
    assert_eq!(status.scope, "brain");
    assert_eq!(status.doc_id.as_deref(), Some("brain-status"));
    assert_eq!(status.title.as_deref(), Some("Brain Status"));

    // Find the docs/index entry.
    let index = manifest
        .entries
        .iter()
        .find(|e| e.rel.ends_with("index.md"))
        .expect("docs/index.md must appear in manifest");
    assert_eq!(index.doc_id.as_deref(), Some("brain-index"));
    assert_eq!(index.title.as_deref(), Some("Brain Index"));

    // README has no frontmatter — all metadata fields are None.
    let readme = manifest
        .entries
        .iter()
        .find(|e| e.rel == "README.md")
        .expect("README.md must appear in manifest");
    assert_eq!(readme.scope, "brain");
    assert!(
        readme.doc_id.is_none(),
        "README without frontmatter must have doc_id: null"
    );
    assert!(readme.title.is_none());
    assert!(readme.doc_type.is_none());

    let _ = fs::remove_dir_all(&dir);
}

/// A file without frontmatter produces `doc_id: null` in the serialised JSON.
#[test]
fn no_frontmatter_file_has_null_metadata_in_json() {
    let dir = temp_dir("null-meta");
    write_brain_toml(&dir);

    write_file(&dir, "README.md", no_frontmatter_doc());

    let manifest = mev::manifest_brain(&dir).expect("manifest_brain must succeed");
    let json = serde_json::to_string(&manifest).expect("manifest must serialize to JSON");

    // "doc_id":null must appear in the JSON output.
    assert!(
        json.contains("\"doc_id\":null"),
        "expected doc_id:null in JSON, got: {json}"
    );

    // Round-trip to confirm valid JSON.
    let _: serde_json::Value = serde_json::from_str(&json).expect("JSON must be valid");

    let _ = fs::remove_dir_all(&dir);
}

/// The manifest serialises to valid JSON with the top-level keys present.
#[test]
fn manifest_serializes_to_valid_json() {
    let dir = temp_dir("json-valid");
    write_brain_toml(&dir);

    write_file(&dir, "planning/status.md", &okf_doc("s-id", "Status"));
    write_file(&dir, "docs/arch.md", &okf_doc("a-id", "Architecture"));

    let manifest = mev::manifest_brain(&dir).expect("manifest_brain must succeed");
    let json = serde_json::to_string(&manifest).expect("must serialize");

    assert!(json.contains("\"version\""), "version key must be present");
    assert!(json.contains("\"root\""), "root key must be present");
    assert!(json.contains("\"entries\""), "entries key must be present");

    let value: serde_json::Value = serde_json::from_str(&json).expect("JSON must be valid");
    assert_eq!(value["version"], "1");
    assert!(value["entries"].is_array());
    assert_eq!(value["entries"].as_array().unwrap().len(), 2);

    let _ = fs::remove_dir_all(&dir);
}

/// `manifest_brain` returns an error when no `brain.toml` is found.
#[test]
fn manifest_brain_errors_without_brain_toml() {
    let dir = temp_dir("no-toml");
    // Intentionally do NOT write brain.toml.
    write_file(&dir, "planning/status.md", "no frontmatter");

    let result = mev::manifest_brain(&dir);
    assert!(
        result.is_err(),
        "expected Err when brain.toml is absent, got Ok"
    );
    let msg = format!("{:#}", result.unwrap_err());
    assert!(
        msg.contains("brain.toml"),
        "error message should mention brain.toml, got: {msg}"
    );

    let _ = fs::remove_dir_all(&dir);
}

/// Scope field on each entry matches the registry slug from `brain.toml`.
#[test]
fn manifest_entries_carry_correct_scope() {
    let dir = temp_dir("scope-check");
    write_brain_toml(&dir);

    write_file(&dir, "planning/status.md", &okf_doc("status", "Status"));
    write_file(&dir, "docs/index.md", &okf_doc("index", "Index"));

    let manifest = mev::manifest_brain(&dir).expect("manifest_brain must succeed");

    for entry in &manifest.entries {
        assert_eq!(
            entry.scope, "brain",
            "all entries under the single brain unit must have scope 'brain', got '{}'",
            entry.scope
        );
    }

    let _ = fs::remove_dir_all(&dir);
}
