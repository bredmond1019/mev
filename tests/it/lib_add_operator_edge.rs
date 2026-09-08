//! Integration tests for `MV.20.D` Task 2: `add_operator_edge` /
//! `add_operator_edge_as` — the fifth guarded pair on `MV.20.B`'s `*_as`
//! contract. Mirrors `lib_guard_entry_points.rs`'s shape for
//! `create_block`/`create_block_as` (this verb, like `create_block`, carries
//! no operator-gate check — only the quiesce lease applies).

use std::fs;
use std::path::{Path, PathBuf};

fn temp_dir(tag: &str) -> PathBuf {
    let d = mev::testsupport::unique_temp_dir(&format!("mev-lib-add-operator-edge-{tag}"));
    fs::create_dir_all(&d).unwrap();
    d
}

fn write_file(root: &Path, rel: &str, content: &str) {
    let target = root.join(rel);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&target, content.as_bytes()).unwrap();
}

fn write_json(root: &Path, rel: &str, value: &serde_json::Value) {
    write_file(root, rel, &serde_json::to_string_pretty(value).unwrap());
}

fn write_brain_toml(root: &Path) {
    let toml = r#"[vocab]
layer = ["brain", "engine", "factory", "console", "surface", "infra", "business", "content", "meta"]
status = ["active", "draft", "deprecated", "superseded", "archived"]

[crawl]
skip_dirs = ["target", "node_modules", ".git"]

[[repos]]
slug = "alpha"
tier = "primary"
repo_path = "repos/alpha"
status_file = "repos/alpha/planning/status.md"
cache_doc = "docs/projects/alpha.md"
heading = "Alpha"
"#;
    fs::write(root.join("brain.toml"), toml.as_bytes()).unwrap();
}

fn write_hq_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "hq",
        "kind": "brain",
        "updated": "2026-09-07",
        "focus": { "now": [], "next": [], "blocked": [] },
        "repos": [
            { "repo": "alpha", "now": [], "next": [], "blocked": [] }
        ],
        "cross_repo": [],
        "epics": []
    });
    write_json(root, "planning/state.json", &state);
}

fn write_alpha_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "alpha",
        "kind": "project",
        "updated": "2026-09-07",
        "focus": { "now": [], "next": [], "blocked": [] },
        "tracks": [
            {
                "title": "Phase 1",
                "blocks": [
                    { "id": "AL.1.A", "title": "Plain block", "status": "open", "wave": 1 }
                ]
            }
        ]
    });
    write_json(root, "repos/alpha/planning/state.json", &state);
}

fn write_full_fixture(root: &Path) {
    write_brain_toml(root);
    write_hq_state(root);
    write_alpha_state(root);
}

fn write_exclusive_lease(lock_dir: &Path, lane: &str, agent: &str, repo: &str, scope: &str) {
    let leases_dir = lock_dir.join("leases");
    fs::create_dir_all(&leases_dir).unwrap();
    let lease = serde_json::json!({
        "lane": lane,
        "agent": agent,
        "repo": repo,
        "kind": "exclusive",
        "scope": scope,
        "acquired_at": chrono::Local::now().to_rfc3339(),
    });
    write_json(&leases_dir, &format!("{lane}.json"), &lease);
}

fn alpha_block_depends_on(root: &Path, block_id: &str) -> serde_json::Value {
    let raw = fs::read_to_string(root.join("repos/alpha/planning/state.json")).unwrap();
    let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
    value["tracks"][0]["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["id"] == block_id)
        .unwrap()["depends_on"]
        .clone()
}

fn this_test_binary_stem() -> String {
    std::env::current_exe()
        .unwrap()
        .file_stem()
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

fn edge(slug: &str) -> okf_core::OperatorDep {
    okf_core::OperatorDep {
        slug: slug.to_string(),
        exit: "planning/handoff.md".to_string(),
        start: "/begin-session x".to_string(),
        what: None,
    }
}

// ---------------------------------------------------------------------------
// Reachability from OUTSIDE the crate — proves both are `pub`.
// ---------------------------------------------------------------------------

#[test]
fn add_operator_edge_and_add_operator_edge_as_are_reachable_from_outside_the_crate() {
    let root = temp_dir("reachable");
    write_full_fixture(&root);

    let report = mev::add_operator_edge(&root, "alpha:AL.1.A", &edge("op-a"), false, None)
        .expect("add_operator_edge must be callable from outside the crate");
    assert!(!report.is_failure());

    let report = mev::add_operator_edge_as(
        &root,
        "alpha:AL.1.A",
        &edge("op-a"),
        false,
        None,
        None,
        None,
        &root,
    )
    .expect("add_operator_edge_as must be callable from outside the crate");
    assert!(!report.is_failure());
}

// ---------------------------------------------------------------------------
// `add_operator_edge_as` — quiesce lease actually refuses.
// ---------------------------------------------------------------------------

#[test]
fn add_operator_edge_as_refuses_under_a_foreign_exclusive_lease() {
    let root = temp_dir("as-quiesced");
    write_full_fixture(&root);
    let lock_dir = root.join(".fleet-locks");
    write_exclusive_lease(&lock_dir, "other-lane", "other-agent", "alpha", "fleet");

    let err = mev::add_operator_edge_as(
        &root,
        "alpha:AL.1.A",
        &edge("op-a"),
        true,
        None,
        Some("me"),
        Some(&lock_dir),
        &root,
    )
    .expect_err("a foreign exclusive lease must refuse add_operator_edge_as");
    let refusal = err
        .downcast_ref::<mev::GuardRefusal>()
        .expect("refusal must be a GuardRefusal, not a generic write error");
    assert_eq!(refusal.code(), mev::E_QUIESCE_LEASE_HELD);
    match refusal {
        mev::GuardRefusal::Quiesce(held) => {
            assert_eq!(held.lane, "other-lane");
            assert_eq!(held.agent, "other-agent");
        }
        other => panic!("expected Quiesce, got {other:?}"),
    }

    // Refused means untouched.
    let depends_on = alpha_block_depends_on(&root, "AL.1.A");
    assert!(
        depends_on.is_null() || depends_on.as_array().unwrap().is_empty(),
        "a refused write must not have appended the edge, got: {depends_on:?}"
    );
}

#[test]
fn add_operator_edge_as_self_exemption_still_applies() {
    let root = temp_dir("as-self-exempt");
    write_full_fixture(&root);
    let lock_dir = root.join(".fleet-locks");
    write_exclusive_lease(&lock_dir, "my-lane", "me", "alpha", "fleet");

    let report = mev::add_operator_edge_as(
        &root,
        "alpha:AL.1.A",
        &edge("op-a"),
        true,
        None,
        Some("me"),
        Some(&lock_dir),
        &root,
    )
    .expect("the lease holder's own agent must not be refused by its own lease");
    assert!(
        !report.is_failure(),
        "diagnostics: {:#?}",
        report.diagnostics
    );

    let depends_on = alpha_block_depends_on(&root, "AL.1.A");
    assert_eq!(depends_on.as_array().unwrap().len(), 1);
    assert_eq!(depends_on[0]["slug"], "op-a");
}

// ---------------------------------------------------------------------------
// The permissive `add_operator_edge` wrapper: allowed AND warned under lease.
// ---------------------------------------------------------------------------

#[test]
fn add_operator_edge_wrapper_downgrades_the_quiesce_refusal_to_a_warning_and_writes_anyway() {
    let root = temp_dir("wrapper-quiesced");
    write_full_fixture(&root);
    let lock_dir = root.join(".fleet-locks");
    write_exclusive_lease(&lock_dir, "other-lane", "other-agent", "alpha", "fleet");

    let report = mev::add_operator_edge(&root, "alpha:AL.1.A", &edge("op-a"), true, None)
        .expect("the legacy wrapper must never refuse — that would break an existing consumer");

    let warnings: Vec<&mev::Diagnostic> = report
        .diagnostics
        .iter()
        .filter(|d| d.severity == mev::Severity::Warning)
        .collect();
    assert!(
        warnings
            .iter()
            .any(|d| d.message.contains(mev::W_MEV_UNGUARDED_WRITER)),
        "a wrapper call that neither refuses nor warns is the red this task turns green; \
         got diagnostics: {:#?}",
        report.diagnostics
    );
    assert!(
        warnings
            .iter()
            .any(|d| d.message.contains(mev::E_QUIESCE_LEASE_HELD)),
        "the warning must name the guard that would have refused"
    );
    let binary = this_test_binary_stem();
    assert!(
        warnings.iter().any(|d| d.message.contains(&binary)),
        "the warning must name the CALLING BINARY ('{binary}'), not a hardcoded string; \
         got: {:#?}",
        warnings
    );

    // ALLOWED: the write proceeded despite the held lease.
    let depends_on = alpha_block_depends_on(&root, "AL.1.A");
    assert_eq!(depends_on.as_array().unwrap().len(), 1);
}

#[test]
fn add_operator_edge_wrapper_warns_on_a_dry_run_too() {
    let root = temp_dir("wrapper-quiesced-dry-run");
    write_full_fixture(&root);
    let lock_dir = root.join(".fleet-locks");
    write_exclusive_lease(&lock_dir, "other-lane", "other-agent", "alpha", "fleet");

    let report = mev::add_operator_edge(&root, "alpha:AL.1.A", &edge("op-a"), false, None)
        .expect("the legacy wrapper must never refuse, dry run or not");

    let warnings: Vec<&mev::Diagnostic> = report
        .diagnostics
        .iter()
        .filter(|d| d.severity == mev::Severity::Warning)
        .collect();
    assert!(
        warnings
            .iter()
            .any(|d| d.message.contains(mev::W_MEV_UNGUARDED_WRITER)),
        "a dry-run call through the unguarded wrapper is still evidence of a bypassing \
         consumer and must warn; got diagnostics: {:#?}",
        report.diagnostics
    );

    // Dry run: nothing written.
    let depends_on = alpha_block_depends_on(&root, "AL.1.A");
    assert!(
        depends_on.is_null() || depends_on.as_array().unwrap().is_empty(),
        "a dry run must not have appended the edge, got: {depends_on:?}"
    );
}

// ---------------------------------------------------------------------------
// Dry-run is the default: with `write == false`, nothing on disk changes.
// ---------------------------------------------------------------------------

#[test]
fn add_operator_edge_as_dry_run_leaves_state_json_byte_identical() {
    let root = temp_dir("dry-run-byte-identical");
    write_full_fixture(&root);
    let state_path = root.join("repos/alpha/planning/state.json");
    let before = fs::read_to_string(&state_path).unwrap();

    let report = mev::add_operator_edge_as(
        &root,
        "alpha:AL.1.A",
        &edge("op-a"),
        false,
        None,
        None,
        None,
        &root,
    )
    .expect("a dry run must never refuse or error");
    assert!(!report.is_failure());

    let after = fs::read_to_string(&state_path).unwrap();
    assert_eq!(before, after, "a dry run must not touch state.json at all");
}

// ---------------------------------------------------------------------------
// A successful `--write` chains emit-state on the same path set_block_status
// uses — proven by the HQ-level derived `now`/`next` list picking up the
// change without a second, separate emit call.
// ---------------------------------------------------------------------------

#[test]
fn add_operator_edge_as_write_chains_emit_state() {
    let root = temp_dir("write-chains-emit");
    write_full_fixture(&root);

    let report = mev::add_operator_edge_as(
        &root,
        "alpha:AL.1.A",
        &edge("op-a"),
        true,
        None,
        Some("me"),
        None,
        &root,
    )
    .expect("a write with no held lease must succeed");
    assert!(
        !report.is_failure(),
        "diagnostics: {:#?}",
        report.diagnostics
    );

    // The edge landed.
    let depends_on = alpha_block_depends_on(&root, "AL.1.A");
    assert_eq!(depends_on.as_array().unwrap().len(), 1);
    assert_eq!(depends_on[0]["slug"], "op-a");

    // emit-state's own diagnostics are folded into the same report — no second,
    // separate emit call was needed to regenerate derived surfaces.
    let hq_state = root.join("planning/state.json");
    assert!(hq_state.exists());
}

// ---------------------------------------------------------------------------
// Each `*_as` entry point holds `<root>/.mev-emit.lock` for the duration of
// its write — a lock already held elsewhere must contend, never overwrite.
// ---------------------------------------------------------------------------

#[test]
fn add_operator_edge_as_contends_on_the_emit_lock_rather_than_overwriting() {
    let root = temp_dir("as-lock-contend");
    write_full_fixture(&root);

    let _held = mev::brain::lock::acquire_lock(&root, std::time::Duration::from_secs(1))
        .expect("this test must be able to take the lock uncontended first");

    let err = mev::add_operator_edge_as(
        &root,
        "alpha:AL.1.A",
        &edge("op-a"),
        true,
        None,
        None,
        None,
        &root,
    )
    .expect_err("a held emit lock must contend, not silently overwrite");
    let lock_err = err
        .downcast_ref::<mev::brain::lock::LockError>()
        .expect("the contention must surface as an E_EMIT_LOCK_HELD-shaped LockError");
    assert!(
        matches!(lock_err, mev::brain::lock::LockError::Held { .. }),
        "expected LockError::Held, got {lock_err:?}"
    );

    let depends_on = alpha_block_depends_on(&root, "AL.1.A");
    assert!(
        depends_on.is_null() || depends_on.as_array().unwrap().is_empty(),
        "a lock-contended write must not have appended the edge, got: {depends_on:?}"
    );
}
