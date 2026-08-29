//! Fixture-backed tests for the `W_BLOCK_*` block-record checks
//! (`MV.ticket.block-record-validation`, Task 4).
//!
//! Unlike `src/brain/block.rs`'s inline unit tests (tasks 1–2, which build
//! `BlockRecordFile`s from in-memory strings or `tempdir()` scratch files),
//! these tests run [`discover_block_records`] + [`check_block_record`]
//! against real fixture trees committed under `tests/fixtures/blocks/`,
//! matching the fixture-tree precedent set by `tests/it/master_plan_fixtures.rs`
//! (`tests/fixtures/master-plan/`). Each fixture directory stands in for one
//! repo root: a `planning/blocks/<id>.json` sits inside it exactly as
//! [`discover_block_records`] expects to find it.
//!
//! Fixtures, one per `W_BLOCK_*` code plus two controls:
//!   - `known-good/` — every field populated, filename matches id, spec_dir
//!     canonical, id present in the driving known-ids set: must produce
//!     **zero** diagnostics.
//!   - `no-blocks-dir/` — a repo root with a `planning/` dir but no
//!     `planning/blocks/` subdirectory: must produce zero diagnostics and no
//!     error (the common state across the fleet today).
//!   - `missing-why/`, `missing-description/`, `missing-out-of-scope/`,
//!     `spec-dir-mismatch/`, `filename-id-mismatch/`, `unknown-id/`,
//!     `operator-edge-incomplete/` — each fixture is otherwise well-formed
//!     and deliberately breaks exactly one of the seven checks, so each
//!     assertion is on that fixture's own code, by code, never by message
//!     text.

use mev::brain::block::{
    W_BLOCK_FILENAME_ID_MISMATCH, W_BLOCK_MISSING_DESCRIPTION, W_BLOCK_MISSING_OUT_OF_SCOPE,
    W_BLOCK_MISSING_WHY, W_BLOCK_OPERATOR_EDGE_INCOMPLETE, W_BLOCK_SPEC_DIR_MISMATCH,
    W_BLOCK_UNKNOWN_ID, check_block_record, discover_block_records,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Root of the fixture tree checked into `tests/fixtures/blocks/`.
fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/blocks")
}

/// Run the checks for every `planning/blocks/*.json` record under
/// `<fixture_root>/<name>/`, using `known_ids` as the driving known-block-id
/// set, and return the flattened list of diagnostic codes.
fn codes_for_fixture(name: &str, known_ids: &HashSet<String>) -> Vec<String> {
    let repo_root = fixture_root().join(name);
    let files = discover_block_records(&repo_root);
    files
        .iter()
        .flat_map(|file| {
            assert!(
                file.parsed.is_ok(),
                "fixture '{name}' must parse cleanly, got: {:?}",
                file.parsed.as_ref().err()
            );
            check_block_record(file, known_ids)
        })
        .map(|diag| diag.locator.clone())
        .collect()
}

/// The single record id authored inside `<fixture_root>/<name>/planning/blocks/`,
/// read straight off disk so the "known ids" set below always matches
/// whatever the fixture actually declares.
fn declared_id(name: &str) -> String {
    let repo_root = fixture_root().join(name);
    let files = discover_block_records(&repo_root);
    assert_eq!(
        files.len(),
        1,
        "fixture '{name}' must contain exactly one block record"
    );
    files[0]
        .parsed
        .as_ref()
        .expect("fixture record must parse")
        .id
        .clone()
}

#[test]
fn known_good_fixture_produces_zero_diagnostics() {
    let mut known_ids = HashSet::new();
    known_ids.insert(declared_id("known-good"));

    let codes = codes_for_fixture("known-good", &known_ids);
    assert!(
        codes.is_empty(),
        "known-good fixture must produce zero diagnostics, got: {codes:?}"
    );
}

#[test]
fn no_blocks_dir_fixture_produces_zero_diagnostics_and_no_error() {
    // No planning/blocks/ subdirectory at all — discover_block_records must
    // yield an empty Vec, never an error, so there is nothing to check.
    let repo_root = fixture_root().join("no-blocks-dir");
    let files = discover_block_records(&repo_root);
    assert!(
        files.is_empty(),
        "a repo with no planning/blocks/ dir must yield no records, got: {files:?}"
    );
}

#[test]
fn missing_why_fixture_triggers_its_code() {
    let mut known_ids = HashSet::new();
    known_ids.insert(declared_id("missing-why"));

    let codes = codes_for_fixture("missing-why", &known_ids);
    assert!(
        codes.contains(&W_BLOCK_MISSING_WHY.to_string()),
        "expected {W_BLOCK_MISSING_WHY} in {codes:?}"
    );
}

#[test]
fn missing_description_fixture_triggers_its_code() {
    let mut known_ids = HashSet::new();
    known_ids.insert(declared_id("missing-description"));

    let codes = codes_for_fixture("missing-description", &known_ids);
    assert!(
        codes.contains(&W_BLOCK_MISSING_DESCRIPTION.to_string()),
        "expected {W_BLOCK_MISSING_DESCRIPTION} in {codes:?}"
    );
}

#[test]
fn missing_out_of_scope_fixture_triggers_its_code() {
    let mut known_ids = HashSet::new();
    known_ids.insert(declared_id("missing-out-of-scope"));

    let codes = codes_for_fixture("missing-out-of-scope", &known_ids);
    assert!(
        codes.contains(&W_BLOCK_MISSING_OUT_OF_SCOPE.to_string()),
        "expected {W_BLOCK_MISSING_OUT_OF_SCOPE} in {codes:?}"
    );
}

#[test]
fn spec_dir_mismatch_fixture_triggers_its_code() {
    let mut known_ids = HashSet::new();
    known_ids.insert(declared_id("spec-dir-mismatch"));

    let codes = codes_for_fixture("spec-dir-mismatch", &known_ids);
    assert!(
        codes.contains(&W_BLOCK_SPEC_DIR_MISMATCH.to_string()),
        "expected {W_BLOCK_SPEC_DIR_MISMATCH} in {codes:?}"
    );
}

#[test]
fn filename_id_mismatch_fixture_triggers_its_code() {
    let mut known_ids = HashSet::new();
    known_ids.insert(declared_id("filename-id-mismatch"));

    let codes = codes_for_fixture("filename-id-mismatch", &known_ids);
    assert!(
        codes.contains(&W_BLOCK_FILENAME_ID_MISMATCH.to_string()),
        "expected {W_BLOCK_FILENAME_ID_MISMATCH} in {codes:?}"
    );
}

#[test]
fn unknown_id_fixture_triggers_its_code() {
    // Deliberately empty — the driving known-ids set must NOT contain this
    // fixture's own id, which is exactly what W_BLOCK_UNKNOWN_ID checks for.
    let known_ids = HashSet::new();

    let codes = codes_for_fixture("unknown-id", &known_ids);
    assert!(
        codes.contains(&W_BLOCK_UNKNOWN_ID.to_string()),
        "expected {W_BLOCK_UNKNOWN_ID} in {codes:?}"
    );
}

#[test]
fn operator_edge_incomplete_fixture_triggers_its_code() {
    let mut known_ids = HashSet::new();
    known_ids.insert(declared_id("operator-edge-incomplete"));

    let codes = codes_for_fixture("operator-edge-incomplete", &known_ids);
    assert!(
        codes.contains(&W_BLOCK_OPERATOR_EDGE_INCOMPLETE.to_string()),
        "expected {W_BLOCK_OPERATOR_EDGE_INCOMPLETE} in {codes:?}"
    );
}

/// Every diagnostic emitted across every triggering fixture is warning
/// severity — this ticket ships warning-only, and Task 4's acceptance
/// criteria pin that at the fixture layer too, not just in the unit tests.
#[test]
fn every_fixture_diagnostic_is_warning_severity() {
    let fixtures_and_ids: &[(&str, Option<&str>)] = &[
        ("missing-why", None),
        ("missing-description", None),
        ("missing-out-of-scope", None),
        ("spec-dir-mismatch", None),
        ("filename-id-mismatch", None),
        ("unknown-id", None),
        ("operator-edge-incomplete", None),
    ];

    for (name, _) in fixtures_and_ids {
        let mut known_ids = HashSet::new();
        known_ids.insert(declared_id(name));

        let repo_root = fixture_root().join(name);
        let files = discover_block_records(&repo_root);
        for file in &files {
            for diag in check_block_record(file, &known_ids) {
                assert_eq!(
                    diag.severity,
                    mev::Severity::Warning,
                    "fixture '{name}' produced a non-warning diagnostic: {diag:?}"
                );
            }
        }
    }
}
