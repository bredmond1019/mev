//! Integration tests for the advisory lock on the epic-mutation commands
//! (ticket `ticket-epic-mutation-lock`, task 2).
//!
//! `defer-epic`, `resume-epic`, and `sync-epics` all chain into `emit_state`
//! on `--write`, exactly like `emit-state` and `set-block-status` do, and now
//! take the same `.mev-emit.lock` advisory lock before touching anything.
//! This file mirrors `tests/emit_state_lock.rs`'s fixture-helper shape and
//! hold-a-lock-then-assert-refusal pattern, extended with one epic ("demo")
//! and one open member block so the positive (lock-released) path actually
//! has something to mutate.
//!
//! All fixtures are `tempfile`-adjacent temp-dir based (mirroring
//! `emit_state_lock.rs`'s own `std::env::temp_dir()` helper) — the live
//! corpus at `~/Dev/agentic-portfolio` is never touched.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use mev::brain::lock::acquire_lock;

const LOCK_FILE_NAME: &str = ".mev-emit.lock";

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("mev-epic-lock-cli-{tag}"));
    let _ = fs::remove_dir_all(&d);
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

/// Minimal `brain.toml` registering a single leaf repo ("alpha"), mirroring
/// `tests/emit_state_lock.rs::write_brain_toml`.
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

/// Alpha's `state.json`, with one block ("AL.1.A", status `open`) that
/// belongs to the "demo" epic — the member `defer-epic demo --write` cascades
/// into a `deferred` status, and `resume-epic`/`sync-epics` reverse it.
fn write_alpha_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "alpha",
        "kind": "project",
        "updated": "2026-07-04",
        "focus": {
            "now": [{ "id": "AL.1.A", "title": "Alpha block A", "status": "open" }],
            "next": [],
            "blocked": []
        },
        "tracks": [
            {
                "title": "Phase 1",
                "blocks": [
                    { "id": "AL.1.A", "title": "Alpha block A", "status": "open", "wave": 1, "epics": ["demo"] }
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

/// HQ `state.json`, carrying the "demo" epic registry entry that
/// `write_alpha_state`'s member block references.
fn write_brain_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "hq",
        "kind": "brain",
        "updated": "2026-06-29",
        "focus": { "now": [], "next": [], "blocked": [] },
        "repos": [
            { "repo": "alpha", "now": [], "next": [], "blocked": [] }
        ],
        "cross_repo": [],
        "epics": [
            { "slug": "demo", "title": "Demo", "status": "active" }
        ]
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
}

fn run_mev(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_mev"))
        .args(args)
        .arg(root)
        .current_dir(root)
        .output()
        .expect("failed to spawn mev binary")
}

/// Byte-read every `state.json` under `repos/alpha` and the HQ root, keyed by
/// their relative path, so callers can assert nothing moved at all.
fn read_state_files(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let rels = ["planning/state.json", "repos/alpha/planning/state.json"];
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
            "{} must be byte-identical after a refused --write",
            path.display()
        );
    }
}

// ---------------------------------------------------------------------------
// (1) defer-epic --write is refused while another writer holds the lock, and
//     every state.json in the fixture stays byte-identical.
// ---------------------------------------------------------------------------

#[test]
fn defer_epic_write_is_refused_while_the_lock_is_held() {
    let dir = temp_dir("defer-refused");
    write_fixture(&dir);

    let before = read_state_files(&dir);

    let _held = acquire_lock(&dir, Duration::from_secs(5)).expect("test process should acquire");

    let output = run_mev(&dir, &["defer-epic", "demo", "--write"]);

    assert!(
        !output.status.success(),
        "defer-epic --write must be refused while the lock is held; status: {:?}",
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
// (2) sync-epics --write is refused the same way — it reaches run_epic_status
//     via the `slug: None` arm and a different `label` branch.
// ---------------------------------------------------------------------------

#[test]
fn sync_epics_write_is_refused_while_the_lock_is_held() {
    let dir = temp_dir("sync-refused");
    write_fixture(&dir);

    let before = read_state_files(&dir);

    let _held = acquire_lock(&dir, Duration::from_secs(5)).expect("test process should acquire");

    let output = run_mev(&dir, &["sync-epics", "--write"]);

    assert!(
        !output.status.success(),
        "sync-epics --write must be refused while the lock is held; status: {:?}",
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
// (3) A dry-run (no --write) takes no lock and is unaffected by one already
//     held — it must still exit 0.
// ---------------------------------------------------------------------------

#[test]
fn epic_dry_run_takes_no_lock_and_is_unaffected_by_a_held_one() {
    let dir = temp_dir("dry-run-unaffected");
    write_fixture(&dir);

    let _held = acquire_lock(&dir, Duration::from_secs(5)).expect("test process should acquire");
    let held_contents_before = fs::read_to_string(dir.join(LOCK_FILE_NAME)).unwrap();

    let output = run_mev(&dir, &["defer-epic", "demo"]);

    assert!(
        output.status.success(),
        "dry-run defer-epic must succeed even while another process holds the write lock; \
         status: {:?}, stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    let held_contents_after = fs::read_to_string(dir.join(LOCK_FILE_NAME)).unwrap();
    assert_eq!(
        held_contents_before, held_contents_after,
        "dry-run must not acquire, modify, or release the lock at all"
    );

    drop(_held);
    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// (4) Once the lock is free, defer-epic --write succeeds and mutates, and no
//     lock file is left behind afterward (Drop fires on the success path too).
// ---------------------------------------------------------------------------

#[test]
fn epic_write_succeeds_once_the_lock_is_released() {
    let dir = temp_dir("succeeds-after-release");
    write_fixture(&dir);

    let output = run_mev(&dir, &["defer-epic", "demo", "--write"]);
    assert!(
        output.status.success(),
        "defer-epic --write must succeed while no lock is held; status: {:?}, stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !dir.join(LOCK_FILE_NAME).exists(),
        "the lockfile must be released again after a successful --write completes"
    );

    let alpha_state: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(dir.join("repos/alpha/planning/state.json")).unwrap(),
    )
    .unwrap();
    let block_status = alpha_state["tracks"][0]["blocks"][0]["status"]
        .as_str()
        .unwrap();
    assert_eq!(
        block_status, "deferred",
        "defer-epic --write must have parked the member block"
    );

    let hq_state: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.join("planning/state.json")).unwrap())
            .unwrap();
    let epic_status = hq_state["epics"][0]["status"].as_str().unwrap();
    assert_eq!(
        epic_status, "paused",
        "defer-epic --write must have paused the epic registry entry"
    );

    let _ = fs::remove_dir_all(&dir);
}
