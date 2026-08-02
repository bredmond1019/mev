//! Integration tests for the `emit-state --write` advisory lock's CLI wiring
//! (ticket `ticket-emit-state-scope-and-lock`, task 5).
//!
//! `src/brain/lock.rs` already carries unit tests for the lock primitive
//! itself (acquire/release/reclaim, in-process). This file exercises the
//! *CLI* surface main.rs wires the lock into: that a concurrent `--write`
//! is refused with `E_EMIT_LOCK_HELD` and writes nothing, that a stale
//! lockfile (dead owning pid) is reclaimed rather than blocking the run
//! indefinitely, and that a dry-run neither takes the lock nor is affected
//! by one already held.
//!
//! To make "two concurrent `--write`s, one wins" deterministic instead of a
//! timing-dependent race (a real second binary would very likely just wait
//! out the ~50ms it takes the first to finish and then succeed too), the
//! "first" writer is simulated by holding the lock directly via
//! `mev::brain::lock::acquire_lock` in this test process for longer than
//! `DEFAULT_LOCK_TIMEOUT`, then observing the real CLI invocation refuse to
//! run concurrently with it.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use mev::brain::lock::{DEFAULT_LOCK_TIMEOUT, acquire_lock};

const LOCK_FILE_NAME: &str = ".mev-emit.lock";

fn temp_dir(tag: &str) -> PathBuf {
    let d = mev::testsupport::unique_temp_dir(&format!("mev-emit-state-lock-cli-{tag}"));
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
/// the fixture `tests/brain_emit.rs::write_from_main_tree_is_unchanged_regression`
/// uses — just enough for `emit_state` to run cleanly with no errors.
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

fn write_alpha_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "alpha",
        "kind": "project",
        "updated": "2026-07-04",
        "focus": {
            "now": [{ "id": "AL.1.A", "title": "Alpha block A", "status": "in_progress" }],
            "next": [],
            "blocked": []
        },
        "tracks": [
            {
                "title": "Phase 1",
                "blocks": [
                    { "id": "AL.1.A", "title": "Alpha block A", "status": "in_progress" },
                    { "id": "AL.1.B", "title": "Alpha block B", "status": "open" }
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

fn write_brain_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "hq",
        "kind": "brain",
        "updated": "2026-06-29",
        "focus": { "now": [], "next": [], "blocked": [] },
        "repos": [
            { "repo": "alpha", "now": [], "next": [], "blocked": [] }
        ],
        "cross_repo": []
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

// ---------------------------------------------------------------------------
// (a) A concurrent `--write` is refused with E_EMIT_LOCK_HELD and writes
//     nothing while another writer holds the lock.
// ---------------------------------------------------------------------------

#[test]
fn concurrent_write_is_refused_with_lock_held_and_writes_nothing() {
    let dir = temp_dir("refused");
    write_fixture(&dir);

    let alpha_state_path = dir.join("repos/alpha/planning/state.json");
    let brain_state_path = dir.join("planning/state.json");
    let alpha_before = fs::read(&alpha_state_path).unwrap();
    let brain_before = fs::read(&brain_state_path).unwrap();

    // Simulate an in-progress `--write` by holding the lock directly for
    // longer than the CLI's DEFAULT_LOCK_TIMEOUT, so the concurrent CLI
    // invocation below deterministically times out rather than racing.
    let _held = acquire_lock(&dir, Duration::from_secs(5)).expect("test process should acquire");

    let output = run_mev(&dir, &["emit-state", "--write"]);

    assert!(
        !output.status.success(),
        "concurrent emit-state --write must exit non-zero while the lock is held; status: {:?}",
        output.status
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("E_EMIT_LOCK_HELD"),
        "stderr must carry the E_EMIT_LOCK_HELD diagnostic; stderr: {stderr}"
    );

    // Nothing was written by the refused invocation.
    assert_eq!(
        fs::read(&alpha_state_path).unwrap(),
        alpha_before,
        "alpha state.json must be unchanged when --write is refused for lock contention"
    );
    assert_eq!(
        fs::read(&brain_state_path).unwrap(),
        brain_before,
        "brain state.json must be unchanged when --write is refused for lock contention"
    );

    drop(_held);
    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Once the lock is free again, a subsequent --write succeeds normally —
// confirming the earlier refusal was contention, not a broken write path.
// ---------------------------------------------------------------------------

#[test]
fn write_succeeds_once_the_lock_is_released() {
    let dir = temp_dir("succeeds-after-release");
    write_fixture(&dir);

    {
        let _held =
            acquire_lock(&dir, Duration::from_millis(200)).expect("test process should acquire");
        // Lock held and released before the CLI ever runs.
    }

    let output = run_mev(&dir, &["emit-state", "--write"]);
    assert!(
        output.status.success(),
        "emit-state --write must succeed once the lock is free; status: {:?}, stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !dir.join(LOCK_FILE_NAME).exists(),
        "the lockfile must be released again after a successful --write completes"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// (b) A lockfile whose owning pid is gone is reclaimed rather than blocking
//     the run indefinitely.
// ---------------------------------------------------------------------------

#[test]
fn stale_lock_from_dead_pid_is_reclaimed_not_blocked() {
    let dir = temp_dir("stale-reclaimed");
    write_fixture(&dir);

    // A pid essentially guaranteed not to be alive.
    fs::write(dir.join(LOCK_FILE_NAME), "999999999").unwrap();

    let started = Instant::now();
    let output = run_mev(&dir, &["emit-state", "--write"]);
    let elapsed = started.elapsed();

    assert!(
        output.status.success(),
        "emit-state --write must reclaim a stale lock and succeed; status: {:?}, stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        elapsed < DEFAULT_LOCK_TIMEOUT,
        "a stale lock must be reclaimed promptly, not by waiting out the full timeout; elapsed: {elapsed:?}"
    );
    assert!(
        !dir.join(LOCK_FILE_NAME).exists(),
        "the lockfile must be released again after the reclaiming run completes"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// (c) A dry-run (no --write) neither takes the lock nor is blocked by one
//     already held.
// ---------------------------------------------------------------------------

#[test]
fn dry_run_takes_no_lock_and_is_unaffected_by_a_held_one() {
    let dir = temp_dir("dry-run-unaffected");
    write_fixture(&dir);

    let _held = acquire_lock(&dir, Duration::from_secs(5)).expect("test process should acquire");
    let held_contents_before = fs::read_to_string(dir.join(LOCK_FILE_NAME)).unwrap();

    let started = Instant::now();
    let output = run_mev(&dir, &["emit-state"]);
    let elapsed = started.elapsed();

    assert!(
        output.status.success(),
        "dry-run emit-state must succeed even while another process holds the write lock; status: {:?}, stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        elapsed < DEFAULT_LOCK_TIMEOUT,
        "dry-run must not wait on the lock at all; elapsed: {elapsed:?}"
    );

    // The dry-run never touched the lockfile: it still names this test
    // process's own held lock, untouched.
    let held_contents_after = fs::read_to_string(dir.join(LOCK_FILE_NAME)).unwrap();
    assert_eq!(
        held_contents_before, held_contents_after,
        "dry-run must not acquire, modify, or release the lock at all"
    );

    drop(_held);
    let _ = fs::remove_dir_all(&dir);
}
