//! Integration tests — end-to-end `validate_brain_links` over a temp brain fixture.
//!
//! Each test builds a temporary HQ-root fixture that mirrors a realistic company-brain
//! layout: `brain.toml` + docs in `planning/` and `docs/`, exercising each E_LINK_*
//! diagnostic code and the clean-corpus (zero-error) baseline.
//!
//! Tests exercise:
//! - Clean corpus (no dead links) → 0 errors
//! - Dead relative markdown link → E_LINK_DEAD_MARKDOWN
//! - Dead file:// URI → E_LINK_DEAD_FILE_URI
//! - Dangling [[wikilink]] → E_LINK_DANGLING_WIKILINK
//! - .brain-moves-pending entry drives E_LINK_MOVED_REFERENCE
//! - Missing .brain-moves-pending → no diagnostics
//! - External / anchor links are never flagged
//! - JSON envelope is well-formed

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
    let dir = mev::testsupport::unique_temp_dir(&format!("mev-brain-links-{suffix}"));
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

/// A minimal OKF doc with a `doc_id` that links to `target` (a relative markdown link).
fn doc_linking_to(doc_id: &str, target: &str) -> String {
    format!(
        "---\ntype: Reference\ntitle: {doc_id}\ndescription: Integration test doc.\ndoc_id: {doc_id}\n---\n\nSee [link]({target}) for details.\n"
    )
}

/// A minimal OKF doc with a `doc_id` and a `[[wikilink]]`.
fn doc_with_wikilink(doc_id: &str, slug: &str) -> String {
    format!(
        "---\ntype: Reference\ntitle: {doc_id}\ndescription: Integration test doc.\ndoc_id: {doc_id}\n---\n\nSee [[{slug}]] for details.\n"
    )
}

/// A minimal OKF doc with a `doc_id` and no outgoing links.
fn clean_doc(doc_id: &str) -> String {
    format!(
        "---\ntype: Reference\ntitle: {doc_id}\ndescription: Integration test doc.\ndoc_id: {doc_id}\n---\n\nBody text with no links.\n"
    )
}

// ---------------------------------------------------------------------------
// 1. Clean corpus — 0 errors
// ---------------------------------------------------------------------------

#[test]
fn clean_corpus_has_zero_link_errors() {
    let dir = temp_dir("clean");
    write_brain_toml(&dir);

    // Create two docs where the link resolves to an existing file.
    write_file(&dir, "planning/target.md", &clean_doc("target-doc"));
    write_file(
        &dir,
        "planning/referrer.md",
        &doc_linking_to("referrer-doc", "target.md"),
    );

    let report = mev::validate_brain_links(&dir).expect("validate_brain_links must not error");
    let link_errors: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.locator.starts_with("E_LINK_"))
        .collect();
    assert!(
        link_errors.is_empty(),
        "expected 0 link errors for clean corpus, got: {link_errors:#?}"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 2. Dead relative markdown link → E_LINK_DEAD_MARKDOWN
// ---------------------------------------------------------------------------

#[test]
fn dead_markdown_link_is_flagged() {
    let dir = temp_dir("dead-md");
    write_brain_toml(&dir);

    // Reference a file that does NOT exist.
    write_file(
        &dir,
        "planning/referrer.md",
        &doc_linking_to("referrer-doc", "../docs/nonexistent.md"),
    );

    let report = mev::validate_brain_links(&dir).expect("validate_brain_links must not error");
    let dead_md: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.locator == "E_LINK_DEAD_MARKDOWN")
        .collect();
    assert_eq!(
        dead_md.len(),
        1,
        "expected 1 E_LINK_DEAD_MARKDOWN, got: {:#?}",
        report.diagnostics
    );
    assert!(
        dead_md[0].message.contains("nonexistent.md"),
        "diagnostic must name the missing target: {:?}",
        dead_md[0].message
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 3. Dead file:// URI → E_LINK_DEAD_FILE_URI
// ---------------------------------------------------------------------------

#[test]
fn dead_file_uri_is_flagged() {
    let dir = temp_dir("dead-file-uri");
    write_brain_toml(&dir);

    // Reference an absolute path that definitely does not exist.
    let content = "---\ntype: Reference\ntitle: Test\ndescription: Test doc.\ndoc_id: test-doc\n---\n\nOpen [doc](file:///tmp/mev-nonexistent-777/gone.md) now.\n";
    write_file(&dir, "planning/test.md", content);

    let report = mev::validate_brain_links(&dir).expect("validate_brain_links must not error");
    let dead_uri: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.locator == "E_LINK_DEAD_FILE_URI")
        .collect();
    assert_eq!(
        dead_uri.len(),
        1,
        "expected 1 E_LINK_DEAD_FILE_URI, got: {:#?}",
        report.diagnostics
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 4. Dangling [[wikilink]] → E_LINK_DANGLING_WIKILINK
// ---------------------------------------------------------------------------

#[test]
fn dangling_wikilink_is_flagged() {
    let dir = temp_dir("dangling-wiki");
    write_brain_toml(&dir);

    // Wikilink to a slug that does not exist as a doc_id in the corpus.
    write_file(
        &dir,
        "planning/referrer.md",
        &doc_with_wikilink("referrer-doc", "nonexistent-slug"),
    );

    let report = mev::validate_brain_links(&dir).expect("validate_brain_links must not error");
    let dangling: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.locator == "E_LINK_DANGLING_WIKILINK")
        .collect();
    assert_eq!(
        dangling.len(),
        1,
        "expected 1 E_LINK_DANGLING_WIKILINK, got: {:#?}",
        report.diagnostics
    );
    assert!(
        dangling[0].message.contains("nonexistent-slug"),
        "diagnostic must name the dangling slug: {:?}",
        dangling[0].message
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 4b. [[wikilink]] to a known doc_id passes
// ---------------------------------------------------------------------------

#[test]
fn known_wikilink_passes() {
    let dir = temp_dir("known-wiki");
    write_brain_toml(&dir);

    // The target doc exists with doc_id = "target-doc".
    write_file(&dir, "planning/target.md", &clean_doc("target-doc"));
    // Referrer wikilinks to "target-doc".
    write_file(
        &dir,
        "planning/referrer.md",
        &doc_with_wikilink("referrer-doc", "target-doc"),
    );

    let report = mev::validate_brain_links(&dir).expect("validate_brain_links must not error");
    let dangling: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.locator == "E_LINK_DANGLING_WIKILINK")
        .collect();
    assert!(
        dangling.is_empty(),
        "expected no dangling-wikilink errors for known slug, got: {dangling:#?}"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 5. .brain-moves-pending entry → E_LINK_MOVED_REFERENCE
// ---------------------------------------------------------------------------

#[test]
fn moves_pending_entry_flags_stale_reference() {
    let dir = temp_dir("moved-ref");
    write_brain_toml(&dir);

    // A doc that links to the (now moved) path.
    write_file(
        &dir,
        "planning/referrer.md",
        &doc_linking_to("referrer-doc", "../docs/old-doc.md"),
    );

    // Write .brain-moves-pending listing the moved path.
    fs::write(
        dir.join(".brain-moves-pending"),
        b"2026-06-30 docs/old-doc.md\n",
    )
    .unwrap();

    let report = mev::validate_brain_links(&dir).expect("validate_brain_links must not error");
    let moved: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.locator == "E_LINK_MOVED_REFERENCE")
        .collect();
    assert_eq!(
        moved.len(),
        1,
        "expected 1 E_LINK_MOVED_REFERENCE, got: {:#?}",
        report.diagnostics
    );
    assert!(
        moved[0].message.contains("old-doc.md"),
        "diagnostic must name the moved path: {:?}",
        moved[0].message
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 6. Missing .brain-moves-pending → no E_LINK_MOVED_REFERENCE
// ---------------------------------------------------------------------------

#[test]
fn missing_moves_pending_produces_no_diagnostics() {
    let dir = temp_dir("no-pending");
    write_brain_toml(&dir);

    // A doc with a dead link (will get E_LINK_DEAD_MARKDOWN) but no .brain-moves-pending.
    write_file(
        &dir,
        "planning/page.md",
        "---\ntype: Reference\ntitle: Page\ndescription: A page.\ndoc_id: page\n---\n\nBody only, no links.\n",
    );

    let report = mev::validate_brain_links(&dir).expect("validate_brain_links must not error");
    let moved: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.locator == "E_LINK_MOVED_REFERENCE")
        .collect();
    assert!(
        moved.is_empty(),
        "expected no E_LINK_MOVED_REFERENCE when no .brain-moves-pending, got: {moved:#?}"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 7. External / anchor links are never flagged
// ---------------------------------------------------------------------------

#[test]
fn external_and_anchor_links_never_flagged() {
    let dir = temp_dir("ext-anchor");
    write_brain_toml(&dir);

    let content = concat!(
        "---\ntype: Reference\ntitle: Ext\ndescription: External links doc.\ndoc_id: ext-doc\n---\n\n",
        "See [external](https://example.com), ",
        "[mail](mailto:foo@example.com), ",
        "[tel](tel:+15555555555), ",
        "[anchor](#section), ",
        "[proto-rel](//cdn.example.com/script.js).\n"
    );
    write_file(&dir, "planning/ext.md", content);

    let report = mev::validate_brain_links(&dir).expect("validate_brain_links must not error");
    let link_errors: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.locator.starts_with("E_LINK_"))
        .collect();
    assert!(
        link_errors.is_empty(),
        "external/anchor links must never be flagged, got: {link_errors:#?}"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 8. JSON envelope is well-formed
// ---------------------------------------------------------------------------

#[test]
fn json_envelope_is_well_formed() {
    let dir = temp_dir("json-rt");
    write_brain_toml(&dir);

    // A dead link so we have an E_LINK_DEAD_MARKDOWN in the report.
    write_file(
        &dir,
        "planning/referrer.md",
        &doc_linking_to("referrer-doc", "../docs/nonexistent.md"),
    );

    let report = mev::validate_brain_links(&dir).expect("validate_brain_links must not error");
    let envelope = mev::JsonReport::new("brain", &dir, &report);
    let json_str = envelope.to_json().expect("JSON serialization must succeed");

    // Must be parseable JSON.
    let parsed: serde_json::Value =
        serde_json::from_str(&json_str).expect("envelope must be valid JSON");

    // Top-level fields must be present.
    assert!(
        parsed.get("validator").is_some(),
        "JSON envelope must have 'validator' field"
    );
    assert!(
        parsed.get("errors").is_some(),
        "JSON envelope must have 'errors' field"
    );
    assert!(
        parsed.get("diagnostics").is_some(),
        "JSON envelope must have 'diagnostics' field"
    );

    // Must report at least one error (the dead link).
    let errors = parsed["errors"].as_u64().unwrap_or(0);
    assert!(
        errors >= 1,
        "JSON envelope must report at least one error: {json_str}"
    );

    // The diagnostic detail must appear in the serialized output.
    assert!(
        json_str.contains("nonexistent.md") || json_str.contains("E_LINK_DEAD_MARKDOWN"),
        "JSON envelope must include the link diagnostic detail: {json_str}"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 9. CLI dispatch precedence — `--links` outranks `--state`
// ---------------------------------------------------------------------------

/// `--links` takes the highest precedence in the `validate-brain` dispatch ladder.
/// When both `--links` and `--state` are passed, the link-integrity pass must run
/// (and the state pass must not be selected instead). The link pass is the only one
/// that emits `E_LINK_*` locators, so seeing `E_LINK_DEAD_MARKDOWN` proves `--links`
/// won the dispatch.
#[test]
fn links_flag_outranks_state_in_dispatch() {
    let dir = temp_dir("dispatch-precedence");
    write_brain_toml(&dir);

    // A doc with a dead relative markdown link — only the link pass flags this.
    write_file(
        &dir,
        "planning/referrer.md",
        &doc_linking_to("referrer-doc", "../docs/nonexistent.md"),
    );

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_mev"))
        .args(["validate-brain", "--links", "--state"])
        .arg(&dir)
        .output()
        .expect("failed to spawn mev binary");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("E_LINK_DEAD_MARKDOWN"),
        "`--links --state` must run the link pass (highest precedence); stdout: {stdout}"
    );

    let _ = fs::remove_dir_all(&dir);
}
