//! Integration tests — end-to-end `validate_brain_graph` over a multi-unit fixture.
//!
//! Each test builds a temporary HQ-root fixture that mirrors a realistic company-brain
//! layout: `brain.toml` registering three units (`brain` → `.`, `core` → `core`,
//! `mev` → `core/mev`), with OKF-clean docs placed to exercise each scope.
//!
//! These tests exercise:
//! - Clean corpus (no violations) → 0 errors
//! - Same `doc_id` under two scopes → 0 errors (different canonical ids)
//! - Two nodes in one scope with the same `doc_id` → exactly one `E_GRAPH_DUPLICATE_DOC_ID`
//! - Cross-scope `related:` that resolves → 0; after target removal → `E_GRAPH_DANGLING_RELATED`
//! - `related:` entry pointing at a leaf (no `doc_id`) → exactly one `W_GRAPH_LEAF_TARGET`
//! - JSON round-trip: a graph diagnostic appears in the `--json` envelope

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
    let dir = mev::testsupport::unique_temp_dir(&format!("mev-brain-graph-{suffix}"));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Write a standard `brain.toml` that registers three units:
/// - `brain`  → `.`         (HQ root)
/// - `core`   → `core`      (a tier dir)
/// - `mev`    → `core/mev`  (a sub-project)
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

[[repos]]
slug = "core"
tier = "tier"
repo_path = "core"
status_file = ""
cache_doc = ""
heading = "Core"

[[repos]]
slug = "mev"
tier = "primary"
repo_path = "core/mev"
status_file = "planning/status.md"
cache_doc = "docs/projects/mev.md"
heading = "mev"
"#;
    fs::write(root.join("brain.toml"), toml.as_bytes()).unwrap();
}

/// A minimal valid OKF doc with an explicit `doc_id`.
fn okf_with_doc_id(doc_id: &str) -> String {
    format!(
        "---\ntype: Reference\ntitle: {doc_id}\ndescription: Integration test doc.\ndoc_id: {doc_id}\n---\n\nBody.\n"
    )
}

/// A minimal valid OKF doc with an explicit `doc_id` and `related` entries.
fn okf_with_doc_id_and_related(doc_id: &str, related: &[&str]) -> String {
    let related_list: String = related.iter().map(|r| format!("  - {r}\n")).collect();
    format!(
        "---\ntype: Reference\ntitle: {doc_id}\ndescription: Integration test doc.\ndoc_id: {doc_id}\nrelated:\n{related_list}---\n\nBody.\n"
    )
}

/// A minimal valid OKF doc **without** a `doc_id` (so it becomes a leaf).
const OKF_NO_DOC_ID: &str = "---\ntype: Reference\ntitle: Leaf Doc\ndescription: A leaf doc with no doc_id.\n---\n\nBody.\n";

// ---------------------------------------------------------------------------
// 1. Clean corpus — 0 errors
// ---------------------------------------------------------------------------

/// A clean multi-scope fixture with cross-scope `related:` that all resolve must
/// produce a report with zero errors (leaf warnings are tolerated, but there are none here).
#[test]
fn clean_corpus_has_zero_errors() {
    let dir = temp_dir("clean");
    write_brain_toml(&dir);

    // brain-scope: two nodes, one referencing the other
    write_file(
        &dir,
        "planning/alpha.md",
        &okf_with_doc_id_and_related("alpha", &["beta"]),
    );
    write_file(&dir, "planning/beta.md", &okf_with_doc_id("beta"));

    // mev-scope: a node cross-referenced from brain
    write_file(&dir, "core/mev/docs/gamma.md", &okf_with_doc_id("gamma"));
    // brain-scope references across scope (qualified)
    write_file(
        &dir,
        "planning/cross.md",
        &okf_with_doc_id_and_related("cross", &["mev:gamma"]),
    );

    let report = mev::validate_brain_graph(&dir).expect("validate_brain_graph must not error");
    let errors: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.severity == mev::Severity::Error && d.locator != "E_CONFIG_NOT_FOUND")
        .collect();
    assert!(
        errors.is_empty(),
        "expected 0 errors for clean corpus, got: {errors:#?}"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 2. Same doc_id in two scopes — 0 errors (distinct canonical ids)
// ---------------------------------------------------------------------------

/// `mev:knowledge` and `brain:knowledge` are different canonical ids; this must not
/// trigger `E_GRAPH_DUPLICATE_DOC_ID`.
#[test]
fn same_doc_id_different_scopes_no_duplicate_error() {
    let dir = temp_dir("diff-scopes");
    write_brain_toml(&dir);

    write_file(&dir, "planning/knowledge.md", &okf_with_doc_id("knowledge"));
    write_file(
        &dir,
        "core/mev/docs/knowledge.md",
        &okf_with_doc_id("knowledge"),
    );

    let report = mev::validate_brain_graph(&dir).expect("validate_brain_graph must not error");
    let dup_errors: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.locator == "E_GRAPH_DUPLICATE_DOC_ID")
        .collect();
    assert!(
        dup_errors.is_empty(),
        "same doc_id in different scopes must not be flagged: {dup_errors:#?}"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 3. Two nodes in one scope with the same doc_id — exactly one duplicate error
// ---------------------------------------------------------------------------

#[test]
fn two_nodes_same_canonical_id_produces_one_duplicate_error() {
    let dir = temp_dir("dup-id");
    write_brain_toml(&dir);

    // Both files are in the `brain` scope; both author the same `doc_id`.
    write_file(&dir, "planning/dup-a.md", &okf_with_doc_id("shared-id"));
    write_file(&dir, "docs/dup-b.md", &okf_with_doc_id("shared-id"));

    let report = mev::validate_brain_graph(&dir).expect("validate_brain_graph must not error");
    let dup_errors: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.locator == "E_GRAPH_DUPLICATE_DOC_ID")
        .collect();
    assert_eq!(
        dup_errors.len(),
        1,
        "expected exactly one E_GRAPH_DUPLICATE_DOC_ID, got: {:#?}",
        report.diagnostics
    );
    assert!(
        dup_errors[0].message.contains("brain:shared-id"),
        "duplicate error must name the canonical id: {:?}",
        dup_errors[0].message
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 4a. Cross-scope related that resolves → 0 errors
// ---------------------------------------------------------------------------

#[test]
fn cross_scope_related_that_resolves_is_ok() {
    let dir = temp_dir("cross-scope-ok");
    write_brain_toml(&dir);

    // brain-scope file referencing a mev-scope node (qualified)
    write_file(
        &dir,
        "planning/referrer.md",
        &okf_with_doc_id_and_related("referrer", &["mev:target-node"]),
    );
    write_file(
        &dir,
        "core/mev/docs/target.md",
        &okf_with_doc_id("target-node"),
    );

    let report = mev::validate_brain_graph(&dir).expect("validate_brain_graph must not error");
    let errors: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| {
            d.severity == mev::Severity::Error
                && (d.locator == "E_GRAPH_DANGLING_RELATED"
                    || d.locator == "E_GRAPH_DUPLICATE_DOC_ID")
        })
        .collect();
    assert!(
        errors.is_empty(),
        "cross-scope related that resolves must produce no errors: {errors:#?}"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 4b. Cross-scope related after target deleted → exactly one E_GRAPH_DANGLING_RELATED
// ---------------------------------------------------------------------------

#[test]
fn cross_scope_related_after_target_removed_is_dangling() {
    let dir = temp_dir("cross-scope-dangling");
    write_brain_toml(&dir);

    write_file(
        &dir,
        "planning/referrer.md",
        &okf_with_doc_id_and_related("referrer", &["mev:removed-target"]),
    );
    // Do NOT write the mev-scope target file — it was "deleted" / never existed.

    let report = mev::validate_brain_graph(&dir).expect("validate_brain_graph must not error");

    let dangling: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.severity == mev::Severity::Error && d.locator == "E_GRAPH_DANGLING_RELATED")
        .collect();
    assert_eq!(
        dangling.len(),
        1,
        "expected exactly one dangling-related error, got: {:#?}",
        report.diagnostics
    );
    assert!(
        dangling[0].message.contains("removed-target"),
        "dangling error must name the missing target: {:?}",
        dangling[0].message
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 5. related: entry pointing at a leaf → exactly one W_GRAPH_LEAF_TARGET, 0 errors
// ---------------------------------------------------------------------------

#[test]
fn related_to_leaf_produces_warning_and_no_error() {
    let dir = temp_dir("leaf-target");
    write_brain_toml(&dir);

    // `leaf-doc` is used as the related: value and matches the stem of the leaf file.
    write_file(
        &dir,
        "planning/referrer.md",
        &okf_with_doc_id_and_related("referrer", &["leaf-doc"]),
    );
    // The target file exists but has no `doc_id` — it is a leaf.
    write_file(&dir, "planning/leaf-doc.md", OKF_NO_DOC_ID);

    let report = mev::validate_brain_graph(&dir).expect("validate_brain_graph must not error");

    let warnings: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.severity == mev::Severity::Warning && d.locator == "W_GRAPH_LEAF_TARGET")
        .collect();
    // Errors that are NOT OKF schema violations (which we don't care about here)
    let graph_errors: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| {
            d.severity == mev::Severity::Error
                && (d.locator == "E_GRAPH_DANGLING_RELATED"
                    || d.locator == "E_GRAPH_DUPLICATE_DOC_ID")
        })
        .collect();

    assert_eq!(
        warnings.len(),
        1,
        "expected exactly one W_GRAPH_LEAF_TARGET warning, got: {:#?}",
        report.diagnostics
    );
    assert!(
        warnings[0].message.contains("leaf-doc"),
        "warning must name the leaf: {:?}",
        warnings[0].message
    );
    assert!(
        graph_errors.is_empty(),
        "leaf target must not produce an error: {graph_errors:#?}"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 6. JSON round-trip — graph diagnostic appears in the envelope
// ---------------------------------------------------------------------------

/// Verify that when `--json` output is produced (via `JsonReport`), a graph diagnostic
/// (here: a dangling-related error) survives the serialization round-trip.
#[test]
fn json_round_trip_includes_graph_diagnostic() {
    let dir = temp_dir("json-rt");
    write_brain_toml(&dir);

    // A referrer with a dangling related entry so we have a graph diagnostic.
    write_file(
        &dir,
        "planning/referrer.md",
        &okf_with_doc_id_and_related("referrer", &["does-not-exist"]),
    );

    let report = mev::validate_brain_graph(&dir).expect("validate_brain_graph must not error");
    let json_report = mev::JsonReport::new("brain", &dir, &report);
    let json_str = json_report
        .to_json()
        .expect("JSON serialization must succeed");

    // The JSON envelope must contain the graph diagnostic vocabulary.
    assert!(
        json_str.contains("does-not-exist") || json_str.contains("dangling"),
        "JSON envelope must include the graph diagnostic detail, got:\n{json_str}"
    );
    // Round-trip: deserialize and check counts.
    let parsed: serde_json::Value = serde_json::from_str(&json_str).expect("JSON must be valid");
    let errors = parsed["errors"].as_u64().unwrap_or(0);
    assert!(
        errors >= 1,
        "JSON envelope must report at least one error for dangling related: {json_str}"
    );

    let _ = fs::remove_dir_all(&dir);
}
