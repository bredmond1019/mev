//! Integration tests for `MV.20.B` Task 2: the four identity-taking `*_as` entry
//! points (`emit_state_as`, `set_block_status_as`, `create_block_as`,
//! `close_operator_gate_as`) actually refuse under a held guard, while the
//! existing four public signatures (`emit_state`, `set_block_status`,
//! `create_block`, `close_operator_gate`) keep today's PERMISSIVE behaviour —
//! the same calls succeed, but emit `W_MEV_UNGUARDED_WRITER` naming the calling
//! binary. A wrapper call that neither refuses nor warns is the red this task
//! turns green.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

fn temp_dir(tag: &str) -> PathBuf {
    let d = mev::testsupport::unique_temp_dir(&format!("mev-lib-guard-entry-points-{tag}"));
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

/// One plain block, one operator-gated block — both still `open` so
/// `set_block_status_as`/`set_block_status` can move either to `in_progress`
/// and exercise the D71 gate.
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
                    { "id": "AL.1.A", "title": "Plain block", "status": "open", "wave": 1 },
                    {
                        "id": "AL.1.B",
                        "title": "Gated block",
                        "status": "open",
                        "wave": 1,
                        "depends_on": [
                            {
                                "type": "operator",
                                "slug": "op-slug",
                                "exit": "planning/handoff.md",
                                "start": "/begin-session x"
                            }
                        ]
                    }
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

fn alpha_block_status(root: &Path, block_id: &str) -> String {
    let raw = fs::read_to_string(root.join("repos/alpha/planning/state.json")).unwrap();
    let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
    value["tracks"][0]["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["id"] == block_id)
        .unwrap()["status"]
        .as_str()
        .unwrap()
        .to_string()
}

fn this_test_binary_stem() -> String {
    std::env::current_exe()
        .unwrap()
        .file_stem()
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

// ---------------------------------------------------------------------------
// `set_block_status_as` — the D71 operator gate actually refuses.
// ---------------------------------------------------------------------------

#[test]
fn set_block_status_as_refuses_an_operator_gated_block() {
    let root = temp_dir("sbs-as-gated");
    write_full_fixture(&root);

    let err = mev::set_block_status_as(
        &root,
        "alpha:AL.1.B",
        "in_progress",
        true,
        None,
        Some("me"),
        None,
        &root,
    )
    .expect_err("an unmet operator gate must refuse set_block_status_as");
    let refusal = err
        .downcast_ref::<mev::GuardRefusal>()
        .expect("refusal must be a GuardRefusal, not a generic write error");
    assert_eq!(refusal.code(), mev::E_BLOCK_OPERATOR_GATED);
    match refusal {
        mev::GuardRefusal::OperatorGate { key } => assert_eq!(key, "alpha:AL.1.B"),
        other => panic!("expected OperatorGate, got {other:?}"),
    }

    // Refused means untouched: the block must still be `open`, not `in_progress`.
    assert_eq!(alpha_block_status(&root, "AL.1.B"), "open");
}

#[test]
fn set_block_status_as_does_not_gate_a_plain_block() {
    let root = temp_dir("sbs-as-plain");
    write_full_fixture(&root);

    let report = mev::set_block_status_as(
        &root,
        "alpha:AL.1.A",
        "in_progress",
        true,
        None,
        Some("me"),
        None,
        &root,
    )
    .expect("a block with no operator gate must not be refused");
    assert!(
        !report.is_failure(),
        "expected no error diagnostics, got: {:#?}",
        report.diagnostics
    );
    assert_eq!(alpha_block_status(&root, "AL.1.A"), "in_progress");
}

// ---------------------------------------------------------------------------
// The old `set_block_status` wrapper: the SAME operator-gated call is ALLOWED
// and WARNED, not refused.
// ---------------------------------------------------------------------------

#[test]
fn set_block_status_wrapper_downgrades_the_gate_refusal_to_a_warning_and_writes_anyway() {
    let root = temp_dir("sbs-wrapper-gated");
    write_full_fixture(&root);

    let report = mev::set_block_status(&root, "alpha:AL.1.B", "in_progress", true, None)
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
            .any(|d| d.message.contains(mev::E_BLOCK_OPERATOR_GATED)),
        "the warning must name the guard that would have refused"
    );
    let binary = this_test_binary_stem();
    assert!(
        warnings.iter().any(|d| d.message.contains(&binary)),
        "the warning must name the CALLING BINARY (this test binary's own stem, '{binary}'), \
         not a hardcoded string; got: {:#?}",
        warnings
    );

    // ALLOWED: the write proceeded despite the gate — this is the permissive,
    // additive behaviour every existing in-process consumer depends on.
    assert_eq!(alpha_block_status(&root, "AL.1.B"), "in_progress");
}

// ---------------------------------------------------------------------------
// `emit_state_as` — a foreign fleet-scope exclusive lease actually refuses.
// ---------------------------------------------------------------------------

#[test]
fn emit_state_as_refuses_under_a_foreign_fleet_scope_lease() {
    let root = temp_dir("emit-as-quiesced");
    write_full_fixture(&root);
    let lock_dir = root.join(".fleet-locks");
    write_exclusive_lease(&lock_dir, "other-lane", "other-agent", "alpha", "fleet");

    let err = mev::emit_state_as(&root, true, None, Some("me"), Some(&lock_dir), &root)
        .expect_err("a foreign fleet-scope exclusive lease must refuse emit_state_as");
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
}

#[test]
fn emit_state_as_self_exemption_still_applies() {
    let root = temp_dir("emit-as-self-exempt");
    write_full_fixture(&root);
    let lock_dir = root.join(".fleet-locks");
    write_exclusive_lease(&lock_dir, "my-lane", "me", "alpha", "fleet");

    // The holder's own agent is never refused by its own lease, even fleet-scoped.
    let report = mev::emit_state_as(&root, true, None, Some("me"), Some(&lock_dir), &root)
        .expect("the lease holder's own agent must not be refused by its own lease");
    assert!(!report.is_failure());
}

// ---------------------------------------------------------------------------
// The old `emit_state` wrapper: the SAME quiesced call is ALLOWED and WARNED.
// ---------------------------------------------------------------------------

#[test]
fn emit_state_wrapper_downgrades_the_quiesce_refusal_to_a_warning_and_writes_anyway() {
    let root = temp_dir("emit-wrapper-quiesced");
    write_full_fixture(&root);
    let lock_dir = root.join(".fleet-locks");
    write_exclusive_lease(&lock_dir, "other-lane", "other-agent", "alpha", "fleet");

    // The legacy wrapper takes no `--lock-dir`, so it always resolves the
    // default `<root>/.fleet-locks` — write the lease there directly (already
    // done above) and call with no identity, exactly like bastion's
    // `mev::emit_state(&root, write, None)` call site.
    let report = mev::emit_state(&root, true, None)
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
}

// ---------------------------------------------------------------------------
// Additivity: `W_MEV_UNGUARDED_WRITER` is a stable, reachable public constant.
// ---------------------------------------------------------------------------

#[test]
fn unguarded_writer_warning_constant_is_public_and_stable() {
    assert_eq!(mev::W_MEV_UNGUARDED_WRITER, "W_MEV_UNGUARDED_WRITER");
}

// ---------------------------------------------------------------------------
// Each `*_as` entry point holds `<root>/.mev-emit.lock` for the duration of
// its write, releasing on the error path too — a lock already held elsewhere
// must contend, never silently overwrite.
// ---------------------------------------------------------------------------

#[test]
fn emit_state_as_contends_on_the_emit_lock_rather_than_overwriting() {
    let root = temp_dir("emit-as-lock-contend");
    write_full_fixture(&root);

    // Hold the lock ourselves first, exactly as a concurrent `--write` would.
    let _held = mev::brain::lock::acquire_lock(&root, Duration::from_secs(1))
        .expect("this test must be able to take the lock uncontended first");

    let err = mev::emit_state_as(&root, true, None, None, None, &root)
        .expect_err("a held emit lock must contend, not silently overwrite");
    let lock_err = err
        .downcast_ref::<mev::brain::lock::LockError>()
        .expect("the contention must surface as an E_EMIT_LOCK_HELD-shaped LockError");
    assert!(
        matches!(lock_err, mev::brain::lock::LockError::Held { .. }),
        "expected LockError::Held, got {lock_err:?}"
    );
}

#[test]
fn set_block_status_as_contends_on_the_emit_lock_rather_than_overwriting() {
    let root = temp_dir("sbs-as-lock-contend");
    write_full_fixture(&root);

    let _held = mev::brain::lock::acquire_lock(&root, Duration::from_secs(1))
        .expect("this test must be able to take the lock uncontended first");

    let err = mev::set_block_status_as(
        &root,
        "alpha:AL.1.A",
        "in_progress",
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
    // Refused-by-contention means untouched.
    assert_eq!(alpha_block_status(&root, "AL.1.A"), "open");
}
