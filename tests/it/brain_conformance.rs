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

/// Serialize `value` as pretty JSON and write it to `root/rel`.
fn write_json(root: &Path, rel: &str, value: &serde_json::Value) {
    write_file(root, rel, &serde_json::to_string_pretty(value).unwrap());
}

/// Create a file at `root/rel` (creating parent dirs as needed) with `content`.
fn write_file(root: &Path, rel: &str, content: &str) {
    let full = root.join(rel);
    fs::create_dir_all(full.parent().unwrap()).unwrap();
    fs::write(&full, content.as_bytes()).unwrap();
}

/// Write the HQ brain `planning/state.json` with one backlog title (`"Ticket One"`) and
/// one epic (`alpha`, status `active`, `plan` pointing inside `core/planning/epics/`) — the
/// full corpus shape `backlog-parity` and `epics-index-parity` both read from.
fn write_hq_state_full(root: &Path) {
    let state = serde_json::json!({
        "repo": "hq",
        "kind": "brain",
        "updated": "2026-08-03",
        "focus": { "now": [], "next": [], "blocked": [] },
        "backlog": [
            {
                "slug": "ticket-one",
                "title": "Ticket One",
                "repo": "cross-repo",
                "type": "improvement",
                "status": "idea"
            }
        ],
        "epics": [
            {
                "slug": "alpha",
                "title": "Alpha Epic",
                "status": "active",
                "plan": "core/planning/epics/alpha.md"
            }
        ]
    });
    write_json(root, "planning/state.json", &state);
}

/// Write `planning/backlog.md` whose `## Active` title matches the JSON side
/// (`"Ticket One"`) — `## Superseded`/`## Shipped` are present but empty to exercise the
/// section filter without affecting parity.
fn write_backlog_md_matching(root: &Path) {
    write_file(
        root,
        "planning/backlog.md",
        "## Active\n\n### [2026-08-01] Ticket One\nbody\n\n## Promoted\n\n## Superseded\n\n## Shipped\n",
    );
}

/// Write `core/planning/epics/index.md` with one row matching the `alpha` epic.
fn write_epics_index_matching(root: &Path) {
    write_file(
        root,
        "core/planning/epics/index.md",
        "| Doc | Epic | Status | Repos |\n|---|---|---|---|\n\
         | [alpha.md](alpha.md) | **Alpha Epic** | `active` | `mev` |\n",
    );
}

/// Write the per-epic doc the `alpha` epic's `plan` points at.
fn write_epic_doc(root: &Path) {
    write_file(root, "core/planning/epics/alpha.md", "# Alpha Epic\n");
}

/// Build the full corpus fixture: `brain.toml` + HQ `state.json` (`backlog[]` + `epics[]`)
/// plus `planning/backlog.md`, `core/planning/epics/index.md`, and the per-epic doc —
/// every side both disk-backed checks read is present and matching.
fn write_full_clean_fixture(root: &Path) {
    write_brain_toml(root);
    write_hq_state_full(root);
    write_backlog_md_matching(root);
    write_epics_index_matching(root);
    write_epic_doc(root);
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

// ---------------------------------------------------------------------------
// Full corpus fixture — clean fixture, seeded drift, single-check filter, tallies.
// ---------------------------------------------------------------------------

/// Names of every registered check whose `reads_live_checkout` property is set — i.e. it
/// compares against the *running checkout's own source tree* (a `build.rs`-stamped path)
/// rather than purely against the fixture passed in, and so can legitimately report
/// `Drift` for reasons that have nothing to do with the fixture under test (an
/// uncommitted worktree, a checkout path the CI runner stamped differently, etc). Derived
/// from the registry's own `reads_live_checkout` field — never a hard-coded name list —
/// so the next check with this property is excluded automatically instead of failing
/// these fixture assertions the day it lands.
fn live_checkout_check_names() -> std::collections::HashSet<&'static str> {
    mev::all_checks()
        .into_iter()
        .filter(|c| c.reads_live_checkout)
        .map(|c| c.name)
        .collect()
}

/// An all-clean full corpus fixture (matching `backlog[]`/`backlog.md` and
/// `epics[]`/index.md sides) reports zero drift. `project-cache-watermark` has no
/// matching inputs in this fixture and is expected to be `not-evaluable`, never `drift`.
///
/// Checks with `reads_live_checkout: true` (currently `toolchain-freshness` and
/// `sibling-rule-coverage`) are excluded from this assertion: they don't read the fixture
/// at all — they compare the *running test binary's* compiled-in build stamp / source
/// text against the live state of the real `core/mev` source tree (see
/// `src/brain/conformance/toolchain.rs` and `sibling.rs`), so they legitimately report
/// `Drift` whenever `cargo test` runs from an uncommitted mev worktree or a checkout the
/// runner stamped at a different path. That's a property of the checkout running the
/// test, not of this fixture — see `ConformanceCheck::reads_live_checkout`'s doc comment.
#[test]
fn full_fixture_reports_zero_drift() {
    let dir = temp_dir("full-clean");
    write_full_clean_fixture(&dir);

    let report = mev::conformance(&dir, None).expect("conformance should not error");
    let live_checkout_checks = live_checkout_check_names();

    let non_live_checkout_drift = report
        .results
        .iter()
        .filter(|r| {
            !live_checkout_checks.contains(r.name.as_str())
                && r.outcome.status == mev::CheckStatus::Drift
        })
        .count();

    assert_eq!(
        non_live_checkout_drift, 0,
        "clean full corpus fixture should report zero drift outside live-checkout checks, got: {:#?}",
        report.results
    );

    let _ = fs::remove_dir_all(&dir);
}

/// Seeding one backlog title into `backlog.md` that has no `state.json backlog[]`
/// counterpart is detected as exactly one drifting check (`backlog-parity`), and the
/// offending title is named in its findings.
#[test]
fn seeded_backlog_title_drift_is_detected_and_named() {
    let dir = temp_dir("seeded-backlog-drift");
    write_full_clean_fixture(&dir);

    // Overwrite backlog.md with an extra markdown-only title that has no state.json
    // backlog[] counterpart.
    write_file(
        &dir,
        "planning/backlog.md",
        "## Active\n\n### [2026-08-01] Ticket One\nbody\n\n### [2026-08-02] Markdown Only Ticket\nbody\n\n## Promoted\n\n## Superseded\n\n## Shipped\n",
    );

    let report = mev::conformance(&dir, None).expect("conformance should not error");
    let live_checkout_checks = live_checkout_check_names();

    // Checks with `reads_live_checkout: true` are excluded here for the same reason as in
    // `full_fixture_reports_zero_drift`: they reflect the real mev checkout's build
    // provenance / source tree, not this fixture, and can independently report `Drift`
    // when `cargo test` runs from an uncommitted mev worktree or a differently-stamped
    // checkout.
    let drifting: Vec<_> = report
        .results
        .iter()
        .filter(|r| {
            !live_checkout_checks.contains(r.name.as_str())
                && r.outcome.status == mev::CheckStatus::Drift
        })
        .collect();

    assert_eq!(
        drifting.len(),
        1,
        "expected exactly one drifting check, got: {:#?}",
        report.results
    );
    assert_eq!(drifting[0].name, "backlog-parity");
    assert!(
        drifting[0]
            .outcome
            .findings
            .iter()
            .any(|f| f.contains("Markdown Only Ticket")),
        "backlog-parity findings should name the offending title, got: {:#?}",
        drifting[0].outcome.findings
    );

    let _ = fs::remove_dir_all(&dir);
}

/// `only = Some("epics-index-parity")` over the full corpus fixture runs exactly that one
/// check and it passes (both sides match).
#[test]
fn only_filter_over_full_fixture_runs_exactly_one_check() {
    let dir = temp_dir("full-only-filter");
    write_full_clean_fixture(&dir);

    let report = mev::conformance(&dir, Some("epics-index-parity"))
        .expect("conformance with a valid --check name should not error");

    assert_eq!(
        report.results.len(),
        1,
        "expected exactly one result, got: {:#?}",
        report.results
    );
    assert_eq!(report.results[0].name, "epics-index-parity");
    assert_eq!(report.results[0].outcome.status, mev::CheckStatus::Pass);

    let _ = fs::remove_dir_all(&dir);
}

/// The full corpus fixture's report tallies still sum to the number of results.
#[test]
fn full_fixture_tallies_sum_to_results_len() {
    let dir = temp_dir("full-tallies");
    write_full_clean_fixture(&dir);

    let report = mev::conformance(&dir, None).expect("conformance should not error");

    assert_eq!(
        report.pass_count + report.drift_count + report.not_evaluable_count,
        report.results.len(),
        "tallies must sum to the number of results, got report: {:#?}",
        report
    );

    let _ = fs::remove_dir_all(&dir);
}
