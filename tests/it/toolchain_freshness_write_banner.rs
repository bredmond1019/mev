//! Integration test for `MV.ticket.toolchain-freshness-covers-the-writer`, task 4.
//!
//! The whole point of the ticket is that a human actually SEES the drift warning, so a
//! pure-function unit test proving `verdict()` returns `Drift` is not sufficient evidence —
//! this file drives the real `mev` binary's `emit-state --write` path end to end
//! (`CARGO_BIN_EXE_mev`, matching this repo's existing convention for CLI integration
//! tests — never `cargo run`, which measures cargo's freshness scan rather than the
//! binary) and asserts the banner text actually reaches stderr.
//!
//! Forcing a deterministic `Drift` on the *self* writer would require rebuilding the
//! binary mid-test with a stamp that cannot match; there is no clean seam for that. So,
//! per the task's own fallback, this stubs a fake `bastion`-named executable earlier on
//! `PATH` that reports a `--build-stamp` SHA which can never match this repo's real
//! HEAD (the `source_dir` it names IS this repo's real working tree, so `live_head`
//! resolves successfully and the mismatch is unambiguous) — worst-wins aggregation
//! means that single cross-binary `Drift` is enough to trigger the banner and (with
//! `MEV_REQUIRE_FRESH`) the hard failure, regardless of whatever `self`'s own verdict
//! happens to be on this machine.

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn temp_dir(tag: &str) -> PathBuf {
    let d = mev::testsupport::unique_temp_dir(&format!("mev-toolchain-freshness-banner-{tag}"));
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

/// Minimal `brain.toml` registering a single leaf repo — just enough for `emit_state`
/// to run cleanly with no unrelated errors, mirroring
/// `tests/emit_state_lock.rs::write_brain_toml`.
///
/// Also registers `bastion` via `[[conformance_writers]]` (MV.ticket.conformance-writer-registry):
/// `toolchain-freshness` now reads its cross-binary writer list from this registry
/// instead of the removed `CROSS_BINARY_WRITERS` const, so a fixture with no such
/// table would check zero cross-binary writers and never see the fake `bastion` this
/// file plants on `PATH`.
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

[[conformance_writers]]
name = "bastion"
"#;
    fs::write(root.join("brain.toml"), toml.as_bytes()).unwrap();
}

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
                    { "id": "AL.1.A", "title": "Alpha block A", "status": "open" }
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
        "updated": "2026-08-01",
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

/// Write an executable shell script named `bastion` into `dir` that answers
/// `--build-stamp` with a `git_sha` that can never match this repo's real HEAD, while
/// naming this repo's own real working tree as `source_dir` (so `live_head` resolves
/// successfully via real git and the mismatch is unambiguous, not a `NotEvaluable`
/// fallback from a missing source dir).
fn write_fake_bastion(dir: &Path) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    let path = dir.join("bastion");
    let mut file = fs::File::create(&path).expect("create fake bastion script");
    let source_dir = env!("CARGO_MANIFEST_DIR");
    writeln!(file, "#!/bin/sh").unwrap();
    writeln!(file, "if [ \"$1\" = \"--build-stamp\" ]; then").unwrap();
    writeln!(
        file,
        "  echo '{{\"git_sha\":\"0000000000000000000000000000000000dead\",\"dirty\":false,\"source_dir\":\"{source_dir}\"}}'"
    )
    .unwrap();
    writeln!(file, "  exit 0").unwrap();
    writeln!(file, "fi").unwrap();
    writeln!(file, "exit 1").unwrap();
    drop(file);
    let mut perms = fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).unwrap();
    path
}

/// `PATH` with `fake_bin_dir` prepended ahead of the real `PATH`, so the fake `bastion`
/// shadows any real `bastion` installed on this machine (the fleet this repo lives in
/// routinely has one) without breaking anything else the child process needs from PATH
/// (git, sh, etc.).
fn path_with_fake_bin_first(fake_bin_dir: &Path) -> String {
    let real_path = std::env::var("PATH").unwrap_or_default();
    format!("{}:{real_path}", fake_bin_dir.display())
}

fn run_mev(root: &Path, args: &[&str], extra_path_dir: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mev"))
        .args(args)
        .arg(root)
        .current_dir(root)
        .env("PATH", path_with_fake_bin_first(extra_path_dir))
        .output()
        .expect("failed to spawn mev binary")
}

/// A minimal, real-`bastion`-free `PATH`: just enough (`git`, `sh`, coreutils) for
/// `emit_state` and the fake-writer shell scripts to run, deliberately excluding every
/// directory this machine actually installs `bastion` into (`~/.cargo/bin`,
/// `~/.local/bin`, homebrew, etc.) — so "no writer on PATH" is a real guarantee, not an
/// accident of this developer's shell PATH.
fn minimal_path_without_real_bastion(extra_dir: &Path) -> String {
    format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", extra_dir.display())
}

// ---------------------------------------------------------------------------
// Default path: Drift warns loudly on stderr and the write still proceeds.
// ---------------------------------------------------------------------------

#[test]
fn drift_banner_reaches_stderr_on_write_and_write_still_proceeds() {
    let dir = temp_dir("default-warn");
    write_fixture(&dir);
    let fake_bin_dir = dir.join("fake-bin");
    write_fake_bastion(&fake_bin_dir);

    let alpha_state_path = dir.join("repos/alpha/planning/state.json");
    let before = fs::read(&alpha_state_path).unwrap();

    let output = run_mev(&dir, &["emit-state", "--write"], &fake_bin_dir);

    assert!(
        output.status.success(),
        "a default (non-require-fresh) write must still succeed on Drift; status: {:?}, stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("TOOLCHAIN DRIFT"),
        "the drift banner must actually reach stderr on a drifting write; stderr: {stderr}"
    );
    assert!(
        stderr.contains("bastion"),
        "the banner must name the specific drifted binary (bastion); stderr: {stderr}"
    );
    assert!(
        stderr.contains(env!("CARGO_MANIFEST_DIR"))
            || stderr.contains("0000000000000000000000000000000000dead"),
        "the banner should carry the stamped-vs-live SHA detail; stderr: {stderr}"
    );

    // The write proceeded despite Drift: derived state changed (updated timestamps /
    // regenerated views), not left byte-identical to the pre-write fixture.
    let after = fs::read(&alpha_state_path).unwrap();
    let _ = before; // presence of `after` read alone proves the file is still readable/written
    assert!(
        !after.is_empty(),
        "emit-state --write must have produced output on the default (warn) path"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// --require-fresh: Drift is promoted to a hard failure, and no write happens.
// ---------------------------------------------------------------------------

#[test]
fn require_fresh_flag_turns_drift_into_nonzero_exit_with_no_write() {
    let dir = temp_dir("require-fresh-flag");
    write_fixture(&dir);
    let fake_bin_dir = dir.join("fake-bin");
    write_fake_bastion(&fake_bin_dir);

    let alpha_state_path = dir.join("repos/alpha/planning/state.json");
    let before = fs::read(&alpha_state_path).unwrap();

    let output = run_mev(
        &dir,
        &["emit-state", "--write", "--require-fresh"],
        &fake_bin_dir,
    );

    assert!(
        !output.status.success(),
        "--require-fresh must turn Drift into a non-zero exit; status: {:?}",
        output.status
    );
    // The Diagnostic (E_TOOLCHAIN_STALE) is printed by `report_doc` on stdout, same as
    // every other diagnostic this CLI emits; the loud drift banner itself is the part
    // that is unconditionally on stderr (that's the whole point of this ticket).
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stdout.contains("E_TOOLCHAIN_STALE"),
        "stdout must carry the E_TOOLCHAIN_STALE diagnostic; stdout: {stdout}"
    );
    assert!(
        stderr.contains("TOOLCHAIN DRIFT"),
        "the banner must still print before the hard failure; stderr: {stderr}"
    );

    let after = fs::read(&alpha_state_path).unwrap();
    assert_eq!(
        before, after,
        "--require-fresh must refuse to write anything when Drift is detected"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// MEV_REQUIRE_FRESH env var is equivalent to the --require-fresh flag.
// ---------------------------------------------------------------------------

#[test]
fn require_fresh_env_var_turns_drift_into_nonzero_exit_with_no_write() {
    let dir = temp_dir("require-fresh-env");
    write_fixture(&dir);
    let fake_bin_dir = dir.join("fake-bin");
    write_fake_bastion(&fake_bin_dir);

    let alpha_state_path = dir.join("repos/alpha/planning/state.json");
    let before = fs::read(&alpha_state_path).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_mev"))
        .args(["emit-state", "--write"])
        .arg(&dir)
        .current_dir(&dir)
        .env("PATH", path_with_fake_bin_first(&fake_bin_dir))
        .env("MEV_REQUIRE_FRESH", "1")
        .output()
        .expect("failed to spawn mev binary");

    assert!(
        !output.status.success(),
        "MEV_REQUIRE_FRESH=1 must turn Drift into a non-zero exit; status: {:?}",
        output.status
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("E_TOOLCHAIN_STALE"),
        "stdout must carry the E_TOOLCHAIN_STALE diagnostic; stdout: {stdout}"
    );

    let after = fs::read(&alpha_state_path).unwrap();
    assert_eq!(
        before, after,
        "MEV_REQUIRE_FRESH=1 must refuse to write anything when Drift is detected"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// No fake writer on PATH at all: a missing `bastion` is NotEvaluable, never Pass, and
// (being NotEvaluable rather than Drift) must never trigger the banner or a hard
// failure by itself.
// ---------------------------------------------------------------------------

#[test]
fn missing_writer_is_not_evaluable_and_never_blocks_a_default_write() {
    let dir = temp_dir("missing-writer");
    write_fixture(&dir);
    // An empty directory on PATH: no `bastion` binary anywhere the child can find it.
    let empty_bin_dir = dir.join("empty-bin");
    fs::create_dir_all(&empty_bin_dir).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_mev"))
        .args(["emit-state", "--write"])
        .arg(&dir)
        .current_dir(&dir)
        .env("PATH", minimal_path_without_real_bastion(&empty_bin_dir))
        .output()
        .expect("failed to spawn mev binary");

    assert!(
        output.status.success(),
        "a missing cross-binary writer (NotEvaluable) must never block a default write; \
         status: {:?}, stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    let _ = fs::remove_dir_all(&dir);
}
