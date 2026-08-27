//! Integration tests — end-to-end `validate_brain_structure` over a temp brain fixture.
//!
//! Each test builds a temporary HQ-root fixture that mirrors a realistic company-brain
//! layout: `brain.toml` + docs in `planning/`, exercising the bidirectional
//! `index.md` <-> directory structural coverage check (D17 / CLAUDE.md Standing Rule 7).
//!
//! Tests exercise:
//! - Clean tree with a correct `index.md` -> 0 E_STRUCT_* diagnostics, exit 0
//! - An orphan corpus file not listed in its `index.md` -> E_STRUCT_ORPHAN_FILE, exit 1
//! - An `index.md` row pointing at a deleted/nonexistent file -> E_STRUCT_DANGLING_ROW, exit 1
//! - The `--json` envelope carries the E_STRUCT_* codes

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
    let dir = mev::testsupport::unique_temp_dir(&format!("mev-brain-structure-{suffix}"));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Write a minimal `brain.toml` with one `brain` unit at the root.
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

/// A minimal OKF `index.md` doc listing rows that link to `targets` (relative paths).
fn index_doc_listing(targets: &[&str]) -> String {
    let mut body = String::from(
        "---\ntype: Index\ntitle: Planning Index\ndescription: Integration test index.\ndoc_id: planning-index\n---\n\n",
    );
    for target in targets {
        body.push_str(&format!("- [entry]({target})\n"));
    }
    body
}

/// A minimal OKF doc with no outgoing links.
fn clean_doc(doc_id: &str) -> String {
    format!(
        "---\ntype: Reference\ntitle: {doc_id}\ndescription: Integration test doc.\ndoc_id: {doc_id}\n---\n\nBody text with no links.\n"
    )
}

// ---------------------------------------------------------------------------
// 1. Clean tree with a correct index.md -> 0 E_STRUCT_* diagnostics, exit 0
// ---------------------------------------------------------------------------

#[test]
fn clean_tree_has_zero_structure_errors() {
    let dir = temp_dir("clean");
    write_brain_toml(&dir);

    // index.md lists both siblings; no orphans, no dangling rows.
    write_file(
        &dir,
        "planning/index.md",
        &index_doc_listing(&["alpha.md", "beta.md"]),
    );
    write_file(&dir, "planning/alpha.md", &clean_doc("alpha-doc"));
    write_file(&dir, "planning/beta.md", &clean_doc("beta-doc"));

    let report =
        mev::validate_brain_structure(&dir).expect("validate_brain_structure must not error");
    let struct_diags: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.locator.starts_with("E_STRUCT_"))
        .collect();
    assert!(
        struct_diags.is_empty(),
        "expected 0 E_STRUCT_* diagnostics for clean tree, got: {struct_diags:#?}"
    );
    assert!(
        !report.is_failure(),
        "clean tree must not be a failure: {:#?}",
        report.diagnostics
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 2. Orphan corpus file not listed in its index.md -> E_STRUCT_ORPHAN_FILE
// ---------------------------------------------------------------------------

#[test]
fn orphan_file_is_flagged() {
    let dir = temp_dir("orphan");
    write_brain_toml(&dir);

    // index.md only lists alpha.md; beta.md is an orphan.
    write_file(&dir, "planning/index.md", &index_doc_listing(&["alpha.md"]));
    write_file(&dir, "planning/alpha.md", &clean_doc("alpha-doc"));
    write_file(&dir, "planning/beta.md", &clean_doc("beta-doc"));

    let report =
        mev::validate_brain_structure(&dir).expect("validate_brain_structure must not error");
    let orphans: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.locator == "E_STRUCT_ORPHAN_FILE")
        .collect();
    assert_eq!(
        orphans.len(),
        1,
        "expected 1 E_STRUCT_ORPHAN_FILE, got: {:#?}",
        report.diagnostics
    );
    assert!(
        orphans[0].file.to_string_lossy().contains("beta.md"),
        "orphan diagnostic must be located at the orphan file: {:?}",
        orphans[0].file
    );
    assert!(
        report.is_failure(),
        "an orphan file must cause report.is_failure() -> true"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 3. index.md row pointing at a deleted/nonexistent file -> E_STRUCT_DANGLING_ROW
// ---------------------------------------------------------------------------

#[test]
fn dangling_row_is_flagged() {
    let dir = temp_dir("dangling");
    write_brain_toml(&dir);

    // index.md references gone.md, which does not exist on disk.
    write_file(&dir, "planning/index.md", &index_doc_listing(&["gone.md"]));

    let report =
        mev::validate_brain_structure(&dir).expect("validate_brain_structure must not error");
    let dangling: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.locator == "E_STRUCT_DANGLING_ROW")
        .collect();
    assert_eq!(
        dangling.len(),
        1,
        "expected 1 E_STRUCT_DANGLING_ROW, got: {:#?}",
        report.diagnostics
    );
    assert!(
        dangling[0].file.to_string_lossy().contains("index.md"),
        "dangling-row diagnostic must be located at the index.md: {:?}",
        dangling[0].file
    );
    assert!(
        dangling[0].message.contains("gone.md"),
        "diagnostic must name the missing target: {:?}",
        dangling[0].message
    );
    assert!(
        report.is_failure(),
        "a dangling row must cause report.is_failure() -> true"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 4. --json envelope carries the E_STRUCT_* codes
// ---------------------------------------------------------------------------

#[test]
fn json_envelope_carries_struct_codes() {
    let dir = temp_dir("json-rt");
    write_brain_toml(&dir);

    // Both an orphan and a dangling row in the same tree.
    write_file(&dir, "planning/index.md", &index_doc_listing(&["gone.md"]));
    write_file(&dir, "planning/orphan.md", &clean_doc("orphan-doc"));

    let report =
        mev::validate_brain_structure(&dir).expect("validate_brain_structure must not error");
    let envelope = mev::JsonReport::new("brain", &dir, &report);
    let json_str = envelope.to_json().expect("JSON serialization must succeed");

    let parsed: serde_json::Value =
        serde_json::from_str(&json_str).expect("envelope must be valid JSON");
    assert!(
        parsed.get("validator").is_some(),
        "JSON envelope must have 'validator' field"
    );
    assert!(
        parsed.get("errors").is_some(),
        "JSON envelope must have 'errors' field"
    );
    let errors = parsed["errors"].as_u64().unwrap_or(0);
    assert!(
        errors >= 2,
        "JSON envelope must report both struct errors: {json_str}"
    );
    assert!(
        json_str.contains("E_STRUCT_ORPHAN_FILE"),
        "JSON envelope must include E_STRUCT_ORPHAN_FILE: {json_str}"
    );
    assert!(
        json_str.contains("E_STRUCT_DANGLING_ROW"),
        "JSON envelope must include E_STRUCT_DANGLING_ROW: {json_str}"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 5. CLI end-to-end: --structure exits 1 on findings, 0 on a clean tree
// ---------------------------------------------------------------------------

#[test]
fn cli_structure_flag_end_to_end() {
    // 5a. Dirty tree -> exit 1, stdout mentions E_STRUCT_ORPHAN_FILE.
    let dirty = temp_dir("cli-dirty");
    write_brain_toml(&dirty);
    write_file(&dirty, "planning/index.md", &index_doc_listing(&[]));
    write_file(&dirty, "planning/orphan.md", &clean_doc("orphan-doc"));

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_mev"))
        .args(["validate-brain", "--structure"])
        .arg(&dirty)
        .output()
        .expect("failed to spawn mev binary");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("E_STRUCT_ORPHAN_FILE"),
        "`--structure` must report the orphan file; stdout: {stdout}"
    );
    assert!(
        !output.status.success(),
        "dirty tree must exit non-zero; status: {:?}",
        output.status
    );

    // 5b. Clean tree -> exit 0.
    let clean = temp_dir("cli-clean");
    write_brain_toml(&clean);
    write_file(
        &clean,
        "planning/index.md",
        &index_doc_listing(&["alpha.md"]),
    );
    write_file(&clean, "planning/alpha.md", &clean_doc("alpha-doc"));

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_mev"))
        .args(["validate-brain", "--structure"])
        .arg(&clean)
        .output()
        .expect("failed to spawn mev binary");
    assert!(
        output.status.success(),
        "clean tree must exit 0; status: {:?}, stdout: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout)
    );

    let _ = fs::remove_dir_all(&dirty);
    let _ = fs::remove_dir_all(&clean);
}
