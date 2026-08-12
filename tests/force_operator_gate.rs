//! Integration tests for `mev set-block-status ... --force-operator-gate`
//! (ticket `ticket-operator-edge-graph`, task 9).
//!
//! Per D71, `--force-operator-gate` is the only override that starts a block
//! (`set-block-status <key> in_progress --write`) while it still carries an unmet
//! `operator` `depends_on` edge, and that override is human-only: mev refuses the
//! flag outright whenever stdin is not a TTY. These tests pin both halves of that
//! floor — the gate itself, and the flag's non-TTY refusal — plus the no-flag,
//! no-gate happy path so a plain `set-block-status` is unaffected.
//!
//! Mirrors `tests/close_operator_gate.rs`'s fixture-helper shape. All fixtures are
//! `tempfile`-adjacent temp-dir based — the live corpus at `~/Dev/agentic-portfolio`
//! is never touched.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const LOCK_FILE_NAME: &str = ".mev-emit.lock";

fn temp_dir(tag: &str) -> PathBuf {
    let d = mev::testsupport::unique_temp_dir(&format!("mev-force-operator-gate-cli-{tag}"));
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
"#;
    fs::write(root.join("brain.toml"), toml.as_bytes()).unwrap();
}

/// `AL.1.A` carries an unmet operator edge; `AL.1.B` carries none — the paired
/// gated/ungated fixture every test below picks between.
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
                        "title": "Alpha block A (operator-gated)",
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
                        "title": "Alpha block B (ungated)",
                        "status": "open",
                        "wave": 1
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
}

/// Runs `mev` with stdin explicitly nulled (never a TTY), matching an agent's
/// or CI's non-interactive invocation — the exact shape `--force-operator-gate`
/// must refuse.
fn run_mev(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_mev"))
        .args(args)
        .arg(root)
        .current_dir(root)
        .stdin(Stdio::null())
        .output()
        .expect("failed to spawn mev binary")
}

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
            "{} must be byte-identical after a refused write",
            path.display()
        );
    }
}

// ---------------------------------------------------------------------------
// (1) --force-operator-gate on non-TTY stdin: non-zero exit, an error (not a
//     warning), and no write — the core assertion this task exists to pin.
// ---------------------------------------------------------------------------

#[test]
fn force_operator_gate_on_non_tty_stdin_is_refused_as_an_error() {
    let dir = temp_dir("non-tty-refused");
    write_fixture(&dir);
    let before = read_state_files(&dir);

    let output = run_mev(
        &dir,
        &[
            "set-block-status",
            "alpha:AL.1.A",
            "in_progress",
            "--write",
            "--force-operator-gate",
        ],
    );

    assert!(
        !output.status.success(),
        "--force-operator-gate on non-TTY stdin must exit non-zero; status: {:?}",
        output.status
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("error") && stderr.contains("E_FORCE_OPERATOR_GATE_NOT_TTY"),
        "refusal must be surfaced as an error carrying E_FORCE_OPERATOR_GATE_NOT_TTY, not a \
         warning; stderr: {stderr}"
    );
    assert!(
        !dir.join(LOCK_FILE_NAME).exists(),
        "refusing the flag before touching anything must never take the lock"
    );

    assert_state_files_unchanged(&before);
    let _ = fs::remove_dir_all(&dir);
}

/// Same refusal in dry-run mode (no --write): the TTY check is not conditioned
/// on --write, because the flag itself — coming from a non-interactive caller —
/// is the failure mode being closed, independent of whether a write was asked
/// for.
#[test]
fn force_operator_gate_on_non_tty_stdin_is_refused_even_without_write() {
    let dir = temp_dir("non-tty-refused-dry-run");
    write_fixture(&dir);
    let before = read_state_files(&dir);

    let output = run_mev(
        &dir,
        &[
            "set-block-status",
            "alpha:AL.1.A",
            "in_progress",
            "--force-operator-gate",
        ],
    );

    assert!(
        !output.status.success(),
        "--force-operator-gate on non-TTY stdin must be refused even in dry-run mode; status: {:?}",
        output.status
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("E_FORCE_OPERATOR_GATE_NOT_TTY"),
        "stderr must carry E_FORCE_OPERATOR_GATE_NOT_TTY; stderr: {stderr}"
    );

    assert_state_files_unchanged(&before);
    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// (2) Without --force-operator-gate, starting a block with an unmet operator
//     edge is refused — the enforcement half of the floor.
// ---------------------------------------------------------------------------

#[test]
fn starting_an_operator_gated_block_without_the_flag_is_refused() {
    let dir = temp_dir("gated-no-flag");
    write_fixture(&dir);
    let before = read_state_files(&dir);

    let output = run_mev(
        &dir,
        &["set-block-status", "alpha:AL.1.A", "in_progress", "--write"],
    );

    assert!(
        !output.status.success(),
        "starting an operator-gated block without --force-operator-gate must exit non-zero; \
         status: {:?}",
        output.status
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("E_BLOCK_OPERATOR_GATED"),
        "stderr must carry E_BLOCK_OPERATOR_GATED; stderr: {stderr}"
    );
    assert!(
        !dir.join(LOCK_FILE_NAME).exists(),
        "refusing before the write must never take the lock"
    );

    assert_state_files_unchanged(&before);
    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// (3) No priority threshold or alternate condition bypasses the gate: passing
//     a bogus/unrelated flag combination never starts the block either.
// ---------------------------------------------------------------------------

#[test]
fn no_alternate_condition_bypasses_the_gate() {
    let dir = temp_dir("no-bypass");
    write_fixture(&dir);
    let before = read_state_files(&dir);

    // Setting to a different (non-in_progress) status is unaffected by the gate —
    // the gate only guards *starting* the block — but attempting the actual
    // start again, twice, must refuse identically both times: there is no
    // retry-based, count-based, or otherwise implicit bypass.
    for _ in 0..2 {
        let output = run_mev(
            &dir,
            &["set-block-status", "alpha:AL.1.A", "in_progress", "--write"],
        );
        assert!(
            !output.status.success(),
            "the operator gate must refuse every attempt, not just the first; status: {:?}",
            output.status
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("E_BLOCK_OPERATOR_GATED"),
            "stderr must carry E_BLOCK_OPERATOR_GATED on every attempt; stderr: {stderr}"
        );
    }

    assert_state_files_unchanged(&before);
    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// (4) No regression: an ungated block starts normally with no flag at all,
//     and a gated block can still be moved to a non-"in_progress" status
//     (e.g. "deferred") without needing --force-operator-gate.
// ---------------------------------------------------------------------------

#[test]
fn ungated_block_starts_normally_without_the_flag() {
    let dir = temp_dir("ungated-happy-path");
    write_fixture(&dir);

    let output = run_mev(
        &dir,
        &["set-block-status", "alpha:AL.1.B", "in_progress", "--write"],
    );

    assert!(
        output.status.success(),
        "an ungated block must start normally with no flag; status: {:?}, stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    let state: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(dir.join("repos/alpha/planning/state.json")).unwrap(),
    )
    .unwrap();
    let blocks = state["tracks"][0]["blocks"].as_array().unwrap();
    let block_b = blocks
        .iter()
        .find(|b| b["id"] == "AL.1.B")
        .expect("AL.1.B must still exist");
    assert_eq!(block_b["status"], "in_progress");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn operator_gated_block_can_still_be_moved_to_a_non_in_progress_status() {
    let dir = temp_dir("gated-non-start-status");
    write_fixture(&dir);

    let output = run_mev(
        &dir,
        &["set-block-status", "alpha:AL.1.A", "deferred", "--write"],
    );

    assert!(
        output.status.success(),
        "moving an operator-gated block to a non-'in_progress' status must not need \
         --force-operator-gate; status: {:?}, stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    let state: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(dir.join("repos/alpha/planning/state.json")).unwrap(),
    )
    .unwrap();
    let blocks = state["tracks"][0]["blocks"].as_array().unwrap();
    let block_a = blocks
        .iter()
        .find(|b| b["id"] == "AL.1.A")
        .expect("AL.1.A must still exist");
    assert_eq!(block_a["status"], "deferred");

    let _ = fs::remove_dir_all(&dir);
}
