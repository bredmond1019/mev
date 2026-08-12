//! Integration tests for `mev approve <slug> --digest <d>` and `mev reject <slug>`
//! (ticket `ticket-operator-edge-graph`, task 8).
//!
//! Mirrors `tests/close_operator_gate.rs`'s fixture-helper shape and
//! hold-a-lock-then-assert-refusal pattern. All fixtures are `tempfile`-adjacent
//! temp-dir based — the live corpus at `~/Dev/agentic-portfolio` is never touched.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use mev::brain::lock::acquire_lock;

const LOCK_FILE_NAME: &str = ".mev-emit.lock";
const REVIEWED_DIGEST: &str = "sha256:abc123";
const WRONG_DIGEST: &str = "sha256:def456";

fn temp_dir(tag: &str) -> PathBuf {
    let d = mev::testsupport::unique_temp_dir(&format!("mev-approve-reject-cli-{tag}"));
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

[[repos]]
slug = "beta"
tier = "primary"
repo_path = "repos/beta"
status_file = "repos/beta/planning/status.md"
cache_doc = "docs/projects/beta.md"
heading = "Beta"
"#;
    fs::write(root.join("brain.toml"), toml.as_bytes()).unwrap();
}

/// Two blocks in alpha and one in beta, all three gated on the same approval slug
/// (`dev-to-cta-sweep`), each carrying [`REVIEWED_DIGEST`] as the reviewed digest —
/// the shared-identity fixture proving a single call clears every one of them,
/// fleet-wide, in one pass.
fn write_alpha_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "alpha",
        "kind": "project",
        "updated": "2026-08-01",
        "focus": { "now": [], "next": [], "blocked": [] },
        "tracks": [
            {
                "title": "Phase 1",
                "blocks": [
                    {
                        "id": "AL.1.A",
                        "title": "Alpha block A",
                        "status": "open",
                        "wave": 1,
                        "depends_on": [
                            {
                                "type": "approval",
                                "slug": "dev-to-cta-sweep",
                                "what": "Approve the Dev.to CTA sweep dry-run diffs",
                                "digest": REVIEWED_DIGEST
                            }
                        ]
                    },
                    {
                        "id": "AL.1.B",
                        "title": "Alpha block B",
                        "status": "open",
                        "wave": 1,
                        "depends_on": [
                            {
                                "type": "approval",
                                "slug": "dev-to-cta-sweep",
                                "what": "Approve the Dev.to CTA sweep dry-run diffs",
                                "digest": REVIEWED_DIGEST
                            }
                        ]
                    }
                ]
            }
        ]
    });
    write_file(
        root,
        "repos/alpha/planning/state.json",
        &serde_json::to_string_pretty(&state).unwrap(),
    );
}

fn write_beta_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "beta",
        "kind": "project",
        "updated": "2026-08-01",
        "focus": { "now": [], "next": [], "blocked": [] },
        "tracks": [
            {
                "title": "Phase 1",
                "blocks": [
                    {
                        "id": "BE.1.A",
                        "title": "Beta block A",
                        "status": "open",
                        "wave": 1,
                        "depends_on": [
                            {
                                "type": "approval",
                                "slug": "dev-to-cta-sweep",
                                "what": "Approve the Dev.to CTA sweep dry-run diffs",
                                "digest": REVIEWED_DIGEST
                            }
                        ]
                    }
                ]
            }
        ]
    });
    write_file(
        root,
        "repos/beta/planning/state.json",
        &serde_json::to_string_pretty(&state).unwrap(),
    );
}

fn write_brain_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "hq",
        "kind": "brain",
        "updated": "2026-06-29",
        "focus": { "now": [], "next": [], "blocked": [] },
        "repos": [
            { "repo": "alpha", "now": [], "next": [], "blocked": [] },
            { "repo": "beta", "now": [], "next": [], "blocked": [] }
        ],
        "cross_repo": [],
        "epics": []
    });
    write_file(
        root,
        "planning/state.json",
        &serde_json::to_string_pretty(&state).unwrap(),
    );
}

fn write_fixture(root: &Path) {
    write_brain_toml(root);
    write_brain_state(root);
    write_alpha_state(root);
    write_beta_state(root);
}

fn run_mev(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_mev"))
        .args(args)
        .arg(root)
        .current_dir(root)
        .output()
        .expect("failed to spawn mev binary")
}

fn read_state_files(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let rels = [
        "planning/state.json",
        "repos/alpha/planning/state.json",
        "repos/beta/planning/state.json",
    ];
    rels.iter()
        .map(|rel| {
            let p = root.join(rel);
            (p.clone(), fs::read(&p).unwrap())
        })
        .collect()
}

fn assert_state_files_unchanged(before: &[(PathBuf, Vec<u8>)]) {
    for (path, bytes_before) in before {
        let bytes_after = fs::read(path).unwrap();
        assert_eq!(
            &bytes_after,
            bytes_before,
            "{} must be byte-identical after a refused approve/reject",
            path.display()
        );
    }
}

// ---------------------------------------------------------------------------
// approve: matching digest clears every edge fleet-wide.
// ---------------------------------------------------------------------------

#[test]
fn approve_with_matching_digest_clears_every_matching_edge_fleet_wide() {
    let dir = temp_dir("approve-match");
    write_fixture(&dir);

    let output = run_mev(
        &dir,
        &["approve", "dev-to-cta-sweep", "--digest", REVIEWED_DIGEST],
    );

    assert!(
        output.status.success(),
        "approve with a matching digest must succeed; status: {:?}, stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !dir.join(LOCK_FILE_NAME).exists(),
        "the lockfile must be released again after a successful approve completes"
    );

    let alpha_state: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(dir.join("repos/alpha/planning/state.json")).unwrap(),
    )
    .unwrap();
    for block in alpha_state["tracks"][0]["blocks"].as_array().unwrap() {
        assert_eq!(
            block["depends_on"].as_array().unwrap().len(),
            0,
            "alpha block {:?} must have its approval edge cleared",
            block["id"]
        );
    }

    let beta_state: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(dir.join("repos/beta/planning/state.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        beta_state["tracks"][0]["blocks"][0]["depends_on"]
            .as_array()
            .unwrap()
            .len(),
        0,
        "beta block must have its approval edge cleared too"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// approve: digest mismatch refuses, changes nothing, and alarms distinctly.
// ---------------------------------------------------------------------------

#[test]
fn approve_with_mismatched_digest_refuses_changes_nothing_and_alarms() {
    let dir = temp_dir("approve-mismatch");
    write_fixture(&dir);
    let before = read_state_files(&dir);

    let output = run_mev(
        &dir,
        &["approve", "dev-to-cta-sweep", "--digest", WRONG_DIGEST],
    );

    assert!(
        !output.status.success(),
        "a digest mismatch must exit non-zero; status: {:?}",
        output.status
    );

    // Diagnostics from a successfully-returned Report print to stdout, not stderr
    // (mirrors report_doc's shape for every other authored-state writer).
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("E_APPROVAL_DIGEST_MISMATCH"),
        "stdout must carry the distinct E_APPROVAL_DIGEST_MISMATCH alarm code, not a generic \
         refusal message; stdout: {stdout}"
    );

    // The re-queue: the edge must still be present afterward, unmet.
    assert_state_files_unchanged(&before);

    let alpha_state: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(dir.join("repos/alpha/planning/state.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        alpha_state["tracks"][0]["blocks"][0]["depends_on"]
            .as_array()
            .unwrap()
            .len(),
        1,
        "a digest mismatch must re-queue, not clear, the approval edge"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// approve: unknown slug refuses with a clear error, changes nothing.
// ---------------------------------------------------------------------------

#[test]
fn approve_unknown_slug_refuses_with_a_clear_error_and_changes_nothing() {
    let dir = temp_dir("approve-unknown");
    write_fixture(&dir);
    let before = read_state_files(&dir);

    let output = run_mev(
        &dir,
        &["approve", "no-such-slug", "--digest", REVIEWED_DIGEST],
    );

    assert!(
        !output.status.success(),
        "an unknown slug must exit non-zero; status: {:?}",
        output.status
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("E_APPROVAL_UNKNOWN"),
        "stdout must carry E_APPROVAL_UNKNOWN; stdout: {stdout}"
    );

    assert_state_files_unchanged(&before);
    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// approve: refused while another writer holds the lock.
// ---------------------------------------------------------------------------

#[test]
fn approve_is_refused_while_the_lock_is_held() {
    let dir = temp_dir("approve-lock-refused");
    write_fixture(&dir);
    let before = read_state_files(&dir);

    let _held = acquire_lock(&dir, Duration::from_secs(5)).expect("test process should acquire");

    let output = run_mev(
        &dir,
        &["approve", "dev-to-cta-sweep", "--digest", REVIEWED_DIGEST],
    );

    assert!(
        !output.status.success(),
        "approve must be refused while the lock is held; status: {:?}",
        output.status
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("E_EMIT_LOCK_HELD"),
        "stderr must carry the E_EMIT_LOCK_HELD diagnostic; stderr: {stderr}"
    );

    assert_state_files_unchanged(&before);

    drop(_held);
    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// reject: clears every matching edge fleet-wide, regardless of digest, and
// records the rejection (the write note, always surfaced).
// ---------------------------------------------------------------------------

#[test]
fn reject_clears_every_matching_edge_fleet_wide_and_records_the_rejection() {
    let dir = temp_dir("reject-succeeds");
    write_fixture(&dir);

    let output = run_mev(&dir, &["reject", "dev-to-cta-sweep"]);

    assert!(
        output.status.success(),
        "reject must succeed once the lock is free; status: {:?}, stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !dir.join(LOCK_FILE_NAME).exists(),
        "the lockfile must be released again after a successful reject completes"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("reject 'dev-to-cta-sweep'"),
        "the rejection must be recorded in the write note; stdout: {stdout}"
    );

    let alpha_state: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(dir.join("repos/alpha/planning/state.json")).unwrap(),
    )
    .unwrap();
    for block in alpha_state["tracks"][0]["blocks"].as_array().unwrap() {
        assert_eq!(
            block["depends_on"].as_array().unwrap().len(),
            0,
            "alpha block {:?} must have its approval edge removed",
            block["id"]
        );
    }

    let beta_state: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(dir.join("repos/beta/planning/state.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        beta_state["tracks"][0]["blocks"][0]["depends_on"]
            .as_array()
            .unwrap()
            .len(),
        0,
        "beta block must have its approval edge removed too"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// reject: unknown slug refuses with a clear error, changes nothing.
// ---------------------------------------------------------------------------

#[test]
fn reject_unknown_slug_refuses_with_a_clear_error_and_changes_nothing() {
    let dir = temp_dir("reject-unknown");
    write_fixture(&dir);
    let before = read_state_files(&dir);

    let output = run_mev(&dir, &["reject", "no-such-slug"]);

    assert!(
        !output.status.success(),
        "an unknown slug must exit non-zero; status: {:?}",
        output.status
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("E_APPROVAL_UNKNOWN"),
        "stdout must carry E_APPROVAL_UNKNOWN; stdout: {stdout}"
    );

    assert_state_files_unchanged(&before);
    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// reject: refused while another writer holds the lock.
// ---------------------------------------------------------------------------

#[test]
fn reject_is_refused_while_the_lock_is_held() {
    let dir = temp_dir("reject-lock-refused");
    write_fixture(&dir);
    let before = read_state_files(&dir);

    let _held = acquire_lock(&dir, Duration::from_secs(5)).expect("test process should acquire");

    let output = run_mev(&dir, &["reject", "dev-to-cta-sweep"]);

    assert!(
        !output.status.success(),
        "reject must be refused while the lock is held; status: {:?}",
        output.status
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("E_EMIT_LOCK_HELD"),
        "stderr must carry the E_EMIT_LOCK_HELD diagnostic; stderr: {stderr}"
    );

    assert_state_files_unchanged(&before);

    drop(_held);
    let _ = fs::remove_dir_all(&dir);
}
