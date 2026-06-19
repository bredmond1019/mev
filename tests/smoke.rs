//! Phase 0 smoke test: the library surface is wired up and an empty/clean tree passes.
//! Real fixture-driven checks arrive with Phase 1 (crawl/parse/validate).

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
