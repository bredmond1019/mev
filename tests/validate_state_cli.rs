//! Integration tests for `mev validate-state <path>`
//! (`ticket-reference-container-validation`, Task 5).
//!
//! Drives the built binary directly (`CARGO_BIN_EXE_mev`), the same pattern every other
//! CLI integration test in this crate uses (see `tests/check_consumers_cli.rs`). Covers:
//!
//! - the red-first fixture reproducing the exact live 2026-08-13 incident shape —
//!   `scope` authored as a plain string and `related` as bare slug strings instead of
//!   the typed `CarryoverScope`/`BlockedBy` objects — asserting the command exits
//!   non-zero and names both the entry's `slug` and the offending field;
//! - a clean file exits zero;
//! - a wall-clock assertion on a representative real-sized file, measured against the
//!   release build (`cargo run --release --`), never the installed `~/.cargo/bin/mev`
//!   (a separate copy that does not auto-track locally-authored commits).
//!
//! Before `diagnose_malformed_state_shape` existed, the incident fixture below still
//! exited non-zero (a bare `E_STATE_MALFORMED_JSON` from the raw `serde_json::Error`),
//! but the message carried only a line/column — never the entry's `slug` or which
//! field was wrong. The assertion on message content (not just the exit code) is what
//! pins the new behaviour; run against the pre-Task-5 binary it fails on the
//! slug/field content checks while still passing the bare exit-code check.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

fn temp_dir(suffix: &str) -> PathBuf {
    let dir = mev::testsupport::unique_temp_dir(&format!("mev-validate-state-cli-{suffix}"));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_json(path: &Path, value: &serde_json::Value) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, serde_json::to_string_pretty(value).unwrap()).unwrap();
}

fn run_validate_state(path: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_mev"))
        .arg("validate-state")
        .arg(path)
        .output()
        .expect("mev validate-state must run")
}

#[test]
fn incident_shape_exits_nonzero_naming_slug_and_field() {
    let dir = temp_dir("incident-shape");
    let path = dir.join("planning/state.json");
    // The exact live 2026-08-13 incident shape: `scope` written as a plain string
    // and `related` written as bare slug strings, instead of a `CarryoverScope`
    // object and `BlockedBy` objects respectively.
    write_json(
        &path,
        &serde_json::json!({
            "repo": "sample",
            "kind": "project",
            "updated": "2026-08-13",
            "carryover": [
                {
                    "slug": "bad-shape-entry",
                    "scope": "core",
                    "kind": "deferred",
                    "text": "a caveat",
                    "related": ["some-other-slug", "another-slug"]
                }
            ]
        }),
    );

    let output = run_validate_state(&path);
    assert!(
        !output.status.success(),
        "expected non-zero exit for the incident-shape fixture, stdout: {}, stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("bad-shape-entry"),
        "expected the offending entry's slug in the output: {stdout}"
    );
    assert!(
        stdout.contains("scope"),
        "expected the offending field 'scope' named in the output: {stdout}"
    );
}

#[test]
fn clean_file_exits_zero() {
    let dir = temp_dir("clean-file");
    let path = dir.join("planning/state.json");
    write_json(
        &path,
        &serde_json::json!({
            "repo": "sample",
            "kind": "project",
            "updated": "2026-08-13",
            "tracks": [
                {
                    "title": "Phase 1",
                    "blocks": [
                        {"id": "P.1.A", "title": "First block", "status": "open"}
                    ]
                }
            ],
            "carryover": [
                {
                    "slug": "clean-entry",
                    "scope": {"repo": "sample"},
                    "kind": "deferred",
                    "text": "a caveat",
                    "created": "2026-08-01"
                }
            ],
            "reference": [
                {
                    "slug": "clean-reference",
                    "scope": {"repo": "sample"},
                    "class": "trap",
                    "text": "a lesson",
                    "created": "2026-08-01"
                }
            ]
        }),
    );

    let output = run_validate_state(&path);
    assert!(
        output.status.success(),
        "expected zero exit for a clean state.json, stdout: {}, stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn missing_file_exits_nonzero() {
    let dir = temp_dir("missing-file");
    let path = dir.join("planning/state.json");

    let output = run_validate_state(&path);
    assert!(
        !output.status.success(),
        "expected non-zero exit for a missing file, stdout: {}, stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Wall-clock assertion measured against the **release** build via `cargo run
/// --release --`, per Task 5's un-gateable AC 11 — the installed `~/.cargo/bin/mev`
/// is a separate copy that does not auto-track locally-authored commits, so it is
/// never a valid stand-in for "does this repo's current source run fast enough".
///
/// First invocation is untimed (lets `cargo run` build/link if the release artifact
/// is stale or absent); the timed run is the second invocation only.
#[test]
fn validate_state_completes_fast_on_a_representative_file_release_build() {
    let dir = temp_dir("wall-clock");
    let path = dir.join("planning/state.json");

    // A representative real-sized file: enough tracks/blocks/carryover/reference
    // entries that a per-file schema pass has real work to do, without being an
    // artificial stress fixture.
    let tracks: Vec<serde_json::Value> = (0..20)
        .map(|t| {
            let blocks: Vec<serde_json::Value> = (0..10)
                .map(|b| {
                    serde_json::json!({
                        "id": format!("P.{t}.{b}"),
                        "title": format!("Block {t}-{b}"),
                        "status": "open"
                    })
                })
                .collect();
            serde_json::json!({"title": format!("Phase {t}"), "blocks": blocks})
        })
        .collect();
    let carryover: Vec<serde_json::Value> = (0..30)
        .map(|i| {
            serde_json::json!({
                "slug": format!("carryover-{i}"),
                "scope": {"repo": "sample"},
                "kind": "deferred",
                "text": "a caveat",
                "created": "2026-08-01"
            })
        })
        .collect();
    let reference: Vec<serde_json::Value> = (0..30)
        .map(|i| {
            serde_json::json!({
                "slug": format!("reference-{i}"),
                "scope": {"repo": "sample"},
                "class": "trap",
                "text": "a lesson",
                "created": "2026-08-01"
            })
        })
        .collect();

    write_json(
        &path,
        &serde_json::json!({
            "repo": "sample",
            "kind": "project",
            "updated": "2026-08-13",
            "tracks": tracks,
            "carryover": carryover,
            "reference": reference
        }),
    );

    let manifest_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let run_release = |args: &[&str]| -> std::process::Output {
        Command::new("cargo")
            .arg("run")
            .arg("--release")
            .arg("--manifest-path")
            .arg(&manifest_path)
            .arg("--quiet")
            .arg("--")
            .args(args)
            .output()
            .expect("cargo run --release must run")
    };

    // Untimed warm-up: builds the release binary if it isn't already fresh.
    let warmup = run_release(&["validate-state", path.to_str().unwrap()]);
    assert!(
        warmup.status.success(),
        "expected zero exit for the representative fixture, stdout: {}, stderr: {}",
        String::from_utf8_lossy(&warmup.stdout),
        String::from_utf8_lossy(&warmup.stderr)
    );

    let started = Instant::now();
    let timed = run_release(&["validate-state", path.to_str().unwrap()]);
    let elapsed = started.elapsed();

    assert!(
        timed.status.success(),
        "expected zero exit for the representative fixture, stdout: {}, stderr: {}",
        String::from_utf8_lossy(&timed.stdout),
        String::from_utf8_lossy(&timed.stderr)
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "mev validate-state took {elapsed:?} on a representative file — expected well \
         under 2s so it's cheap enough to run after every manual state.json write"
    );
}
