//! Integration tests for `mev::conformance` — the `mev conformance` driver
//! (`MV.ticket.conformance-check-registry` — Task 6).
//!
//! Exercises the public driver end to end: `find_brain_config` → `discover_state_files` →
//! `load_state` → `ConformanceCtx` → `run_checks`, over a minimal temp-dir fixture. The
//! four seed checks each have their own focused unit tests in
//! `src/brain/conformance/*.rs`; these tests cover only the driver's own responsibilities:
//! wiring the registry together, the `--check` narrowing + error path, and the drift-count
//! → exit-code contract the CLI layer relies on.

use std::fs;
use std::path::Path;

/// Make a fresh uniquely-named temp dir for a test and return its path.
fn temp_dir(suffix: &str) -> std::path::PathBuf {
    let dir = mev::testsupport::unique_temp_dir(&format!("mev-brain-conformance-it-{suffix}"));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Write a minimal `brain.toml` with no `[[repos]]` entries — enough for `find_brain_config`
/// to resolve, but light enough that the disk-backed checks (`backlog-parity`,
/// `epics-index-parity`, `project-cache-watermark`) all land on `not-evaluable` (their
/// inputs are absent), which is exactly the "missing input" contract the driver must not
/// mistake for drift.
fn write_brain_toml(root: &Path) {
    let toml = r#"[vocab]
layer = ["brain", "engine", "factory", "console", "surface", "infra", "business", "content", "meta"]
status = ["active", "draft", "deprecated", "superseded", "archived"]

[crawl]
skip_dirs = ["target", "node_modules", ".git"]
"#;
    fs::write(root.join("brain.toml"), toml.as_bytes()).unwrap();
}

#[test]
fn conformance_runs_every_registered_check() {
    let dir = temp_dir("all-checks");
    write_brain_toml(&dir);

    let report = mev::conformance(&dir, None).expect("conformance should not error");

    assert_eq!(
        report.results.len(),
        mev::all_checks().len(),
        "driver should run every registered check when --check is not given"
    );
    let names: Vec<&str> = report.results.iter().map(|r| r.name.as_str()).collect();
    assert!(names.contains(&"backlog-parity"));
    assert!(names.contains(&"epics-index-parity"));
    assert!(names.contains(&"project-cache-watermark"));
    assert!(names.contains(&"toolchain-freshness"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn conformance_check_flag_narrows_to_one_check() {
    let dir = temp_dir("narrow");
    write_brain_toml(&dir);

    let report =
        mev::conformance(&dir, Some("backlog-parity")).expect("conformance should not error");

    assert_eq!(report.results.len(), 1);
    assert_eq!(report.results[0].name, "backlog-parity");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn conformance_unknown_check_name_errors_naming_valid_checks() {
    let dir = temp_dir("unknown-check");
    write_brain_toml(&dir);

    let err = mev::conformance(&dir, Some("does-not-exist")).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("does-not-exist"));
    assert!(msg.contains("backlog-parity"));
    assert!(msg.contains("epics-index-parity"));
    assert!(msg.contains("project-cache-watermark"));
    assert!(msg.contains("toolchain-freshness"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn conformance_missing_inputs_report_not_evaluable_never_drift() {
    let dir = temp_dir("missing-inputs");
    write_brain_toml(&dir);

    let report = mev::conformance(&dir, None).expect("conformance should not error");

    // backlog.md and the epics index are both absent from this minimal fixture — a
    // missing input must land on not-evaluable, never be misreported as drift.
    let backlog = report
        .results
        .iter()
        .find(|r| r.name == "backlog-parity")
        .expect("backlog-parity should be present");
    assert_eq!(backlog.outcome.status, mev::CheckStatus::NotEvaluable);

    let epics = report
        .results
        .iter()
        .find(|r| r.name == "epics-index-parity")
        .expect("epics-index-parity should be present");
    assert_eq!(epics.outcome.status, mev::CheckStatus::NotEvaluable);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn conformance_tallies_sum_to_results_len() {
    let dir = temp_dir("tallies");
    write_brain_toml(&dir);

    let report = mev::conformance(&dir, None).expect("conformance should not error");

    assert_eq!(
        report.pass_count + report.drift_count + report.not_evaluable_count,
        report.results.len()
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn conformance_errors_when_brain_toml_is_missing() {
    let dir = temp_dir("no-brain-toml");
    // No brain.toml written — this must surface as a hard configuration error, matching
    // block_graph_brain / carryover_sweep's contract.
    let err = mev::conformance(&dir, None).unwrap_err();
    assert!(err.to_string().contains("brain.toml"));

    let _ = fs::remove_dir_all(&dir);
}
