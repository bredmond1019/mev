//! Integration tests for `mev close-operator-gate <slug> --exit-verified`
//! (ticket `ticket-operator-edge-graph`, task 7).
//!
//! Mirrors `tests/epic_lock.rs`'s fixture-helper shape and hold-a-lock-then-assert-
//! refusal pattern. All fixtures are `tempfile`-adjacent temp-dir based — the live
//! corpus at `~/Dev/agentic-portfolio` is never touched.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use mev::brain::lock::acquire_lock;

const LOCK_FILE_NAME: &str = ".mev-emit.lock";

fn temp_dir(tag: &str) -> PathBuf {
    let d = mev::testsupport::unique_temp_dir(&format!("mev-close-operator-gate-cli-{tag}"));
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

/// Two blocks in alpha and one in beta, all three gated on the same operator slug
/// (`session-mac-mini`) — the shared-identity fixture that proves a single call
/// clears every one of them, fleet-wide, in one pass.
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
                                "type": "operator",
                                "slug": "session-mac-mini",
                                "exit": "planning/handoff.md",
                                "start": "/begin-session mac-mini"
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
                                "type": "operator",
                                "slug": "session-mac-mini",
                                "exit": "planning/handoff.md",
                                "start": "/begin-session mac-mini"
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
                                "type": "operator",
                                "slug": "session-mac-mini",
                                "exit": "planning/handoff.md",
                                "start": "/begin-session mac-mini"
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
            "{} must be byte-identical after a refused close-operator-gate",
            path.display()
        );
    }
}

// ---------------------------------------------------------------------------
// (1) Without --exit-verified: non-zero exit, changes nothing, no lock taken.
// ---------------------------------------------------------------------------

#[test]
fn without_exit_verified_it_refuses_and_changes_nothing() {
    let dir = temp_dir("no-exit-verified");
    write_fixture(&dir);
    let before = read_state_files(&dir);

    let output = run_mev(&dir, &["close-operator-gate", "session-mac-mini"]);

    assert!(
        !output.status.success(),
        "must refuse without --exit-verified; status: {:?}",
        output.status
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("E_OPERATOR_GATE_NOT_VERIFIED"),
        "stderr must carry E_OPERATOR_GATE_NOT_VERIFIED; stderr: {stderr}"
    );
    assert!(
        !dir.join(LOCK_FILE_NAME).exists(),
        "refusing before --exit-verified must never take the lock"
    );

    assert_state_files_unchanged(&before);
    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// (2) Unknown slug: non-zero exit with a clear message, changes nothing.
// ---------------------------------------------------------------------------

#[test]
fn unknown_slug_refuses_with_a_clear_error_and_changes_nothing() {
    let dir = temp_dir("unknown-slug");
    write_fixture(&dir);
    let before = read_state_files(&dir);

    let output = run_mev(
        &dir,
        &["close-operator-gate", "no-such-slug", "--exit-verified"],
    );

    assert!(
        !output.status.success(),
        "an unknown slug must exit non-zero; status: {:?}",
        output.status
    );
    // Diagnostics from a successfully-returned Report print to stdout, not stderr
    // (mirrors report_doc's shape for every other authored-state writer).
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("E_OPERATOR_GATE_UNKNOWN"),
        "stdout must carry E_OPERATOR_GATE_UNKNOWN; stdout: {stdout}"
    );

    assert_state_files_unchanged(&before);
    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// (3) close-operator-gate --exit-verified is refused while another writer
//     holds the lock, and every state.json stays byte-identical.
// ---------------------------------------------------------------------------

#[test]
fn write_is_refused_while_the_lock_is_held() {
    let dir = temp_dir("lock-refused");
    write_fixture(&dir);
    let before = read_state_files(&dir);

    let _held = acquire_lock(&dir, Duration::from_secs(5)).expect("test process should acquire");

    let output = run_mev(
        &dir,
        &["close-operator-gate", "session-mac-mini", "--exit-verified"],
    );

    assert!(
        !output.status.success(),
        "close-operator-gate must be refused while the lock is held; status: {:?}",
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
// (4) Once the lock is free and --exit-verified is passed, every edge with
//     the slug is removed fleet-wide (2 blocks in alpha, 1 in beta), and the
//     lockfile is released afterward.
// ---------------------------------------------------------------------------

#[test]
fn succeeds_and_clears_every_matching_edge_fleet_wide() {
    let dir = temp_dir("succeeds");
    write_fixture(&dir);

    let output = run_mev(
        &dir,
        &["close-operator-gate", "session-mac-mini", "--exit-verified"],
    );

    assert!(
        output.status.success(),
        "close-operator-gate --exit-verified must succeed once the lock is free; status: {:?}, stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !dir.join(LOCK_FILE_NAME).exists(),
        "the lockfile must be released again after a successful close-operator-gate completes"
    );

    let alpha_state: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(dir.join("repos/alpha/planning/state.json")).unwrap(),
    )
    .unwrap();
    for block in alpha_state["tracks"][0]["blocks"].as_array().unwrap() {
        assert_eq!(
            block["depends_on"].as_array().unwrap().len(),
            0,
            "alpha block {:?} must have its operator edge removed",
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
        "beta block must have its operator edge removed too"
    );

    let _ = fs::remove_dir_all(&dir);
}
