//! Integration tests for `mev check-consumers` (`ticket-consumer-compile-gate`, Task 3).
//!
//! Drives the built binary directly (`CARGO_BIN_EXE_mev`), the same pattern every other CLI
//! integration test in this crate uses. Exercises the CLI wiring end to end — real `git`,
//! real `cargo nextest run --no-run --locked` against fixture consumers that genuinely
//! path-depend on this checked-out mev — not the pure classifier (covered in
//! `src/consumers/mod.rs`'s unit tests) or discovery in isolation
//! (`src/brain/conformance/consumers.rs`'s unit tests).
//!
//! `--json` shape, `--consumer` narrowing, the unknown-slug error, and the
//! `Broken`-fails/`LockfileStale`-and-friends-don't-fail exit contract are all covered here
//! because they only exist at this layer.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// This checked-out mev's own crate directory — the path every fixture consumer below
/// declares a `path` dependency on, so `discover_mev_consumers` (which matches by
/// canonicalized path against the SAME constant, compiled into the binary under test) finds
/// them as real consumers.
fn real_mev_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn temp_dir(suffix: &str) -> PathBuf {
    let dir = mev::testsupport::unique_temp_dir(&format!("mev-check-consumers-cli-{suffix}"));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn run_git(dir: &Path, args: &[&str]) {
    let status = mev::testsupport::git_command()
        .arg("-C")
        .arg(dir)
        .args(args)
        .status()
        .expect("git must be on PATH for this test");
    assert!(status.success(), "git {args:?} failed in {dir:?}");
}

/// Write a `brain.toml` at `root` registering one `[[repos]]` entry per `(slug, path)` pair.
/// `path` is an absolute path, so `root.join(path)` (what `discover_mev_consumers` computes)
/// resolves straight to it regardless of `root`.
fn write_brain_toml(root: &Path, repos: &[(&str, &Path)]) {
    let mut toml = String::from(
        "[vocab]\n\
         layer = [\"brain\", \"engine\", \"factory\", \"console\", \"surface\", \"infra\", \"business\", \"content\", \"meta\"]\n\
         status = [\"active\", \"draft\", \"deprecated\", \"superseded\", \"archived\"]\n\n\
         [crawl]\n\
         skip_dirs = [\"target\", \"node_modules\", \".git\"]\n\n",
    );
    for (slug, path) in repos {
        toml.push_str(&format!(
            "[[repos]]\nslug = \"{slug}\"\ntier = \"secondary\"\nrepo_path = \"{}\"\nstatus_file = \"planning/status.md\"\ncache_doc = \"\"\nheading = \"{slug}\"\n\n",
            path.display()
        ));
    }
    fs::write(root.join("brain.toml"), toml).unwrap();
}

/// Build a fixture consumer crate at `dir`: a `Cargo.toml` with a path dependency on the real
/// mev crate, `src/lib.rs` set from `lib_body`, a generated + committed `Cargo.lock` (so the
/// later `--locked` run has something valid to check against), all inside a fresh git repo
/// (so it reads as clean — the dirty short-circuit is covered elsewhere, not here).
fn fixture_consumer(dir: &Path, crate_name: &str, lib_body: &str) {
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(
        dir.join("Cargo.toml"),
        format!(
            "[package]\nname = \"{crate_name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nmev = {{ path = \"{}\" }}\n",
            real_mev_dir().display()
        ),
    )
    .unwrap();
    fs::write(dir.join("src/lib.rs"), lib_body).unwrap();

    let status = Command::new("cargo")
        .arg("generate-lockfile")
        .current_dir(dir)
        .status()
        .expect("cargo generate-lockfile must run");
    assert!(
        status.success(),
        "cargo generate-lockfile failed for {dir:?}"
    );

    run_git(dir, &["init", "-q"]);
    run_git(dir, &["config", "user.email", "test@example.com"]);
    run_git(dir, &["config", "user.name", "Test"]);
    run_git(dir, &["add", "-A"]);
    run_git(dir, &["commit", "-q", "-m", "init"]);
}

/// A consumer whose test targets compile cleanly against the working mev.
fn pass_fixture(dir: &Path, crate_name: &str) {
    fixture_consumer(dir, crate_name, "pub fn hello() -> i32 { 1 }\n");
}

/// A consumer whose test target has a genuine type error — classifies as `Broken`.
fn broken_fixture(dir: &Path, crate_name: &str) {
    fixture_consumer(
        dir,
        crate_name,
        "pub fn hello() -> i32 { 1 }\n\n\
         #[cfg(test)]\n\
         mod tests {\n\
         \x20   #[test]\n\
         \x20   fn broken() {\n\
         \x20       let _x: i32 = \"not an int\";\n\
         \x20   }\n\
         }\n",
    );
}

/// A consumer whose `Cargo.lock` is stale relative to `Cargo.toml` (an extra dependency added
/// after the lock was generated, then committed together) — classifies as `LockfileStale`,
/// not `Broken`, when run under `--locked`.
fn stale_lockfile_fixture(dir: &Path, crate_name: &str) {
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(
        dir.join("Cargo.toml"),
        format!(
            "[package]\nname = \"{crate_name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nmev = {{ path = \"{}\" }}\n",
            real_mev_dir().display()
        ),
    )
    .unwrap();
    fs::write(dir.join("src/lib.rs"), "pub fn hello() -> i32 { 1 }\n").unwrap();

    let status = Command::new("cargo")
        .arg("generate-lockfile")
        .current_dir(dir)
        .status()
        .expect("cargo generate-lockfile must run");
    assert!(
        status.success(),
        "cargo generate-lockfile failed for {dir:?}"
    );

    // Add a dependency AFTER the lock was generated, without regenerating it, so the
    // committed Cargo.lock is stale relative to the committed Cargo.toml.
    let mut cargo_toml = fs::read_to_string(dir.join("Cargo.toml")).unwrap();
    cargo_toml.push_str("once_cell = \"1\"\n");
    fs::write(dir.join("Cargo.toml"), cargo_toml).unwrap();

    run_git(dir, &["init", "-q"]);
    run_git(dir, &["config", "user.email", "test@example.com"]);
    run_git(dir, &["config", "user.name", "Test"]);
    run_git(dir, &["add", "-A"]);
    run_git(dir, &["commit", "-q", "-m", "init"]);
}

fn run_check_consumers(root: &Path, extra_args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_mev"))
        .arg("check-consumers")
        .arg(root)
        .args(extra_args)
        .output()
        .expect("mev check-consumers must run")
}

#[test]
fn json_output_carries_one_entry_per_consumer() {
    let root = temp_dir("json-one-entry");
    let consumer_dir = temp_dir("json-one-entry-consumer");
    pass_fixture(&consumer_dir, "fixture-pass-json");
    write_brain_toml(&root, &[("fixture-pass-json", &consumer_dir)]);

    let output = run_check_consumers(&root, &["--json"]);
    assert!(
        output.status.success(),
        "expected success for a passing consumer, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("expected valid JSON, got error {e}: {stdout}"));
    let results = parsed.as_array().expect("JSON output must be an array");
    assert_eq!(
        results.len(),
        1,
        "expected exactly one consumer entry: {results:?}"
    );
    assert_eq!(results[0]["slug"], "fixture-pass-json");
    assert_eq!(results[0]["outcome"]["outcome"], "pass");
}

#[test]
fn consumer_flag_runs_exactly_one() {
    let root = temp_dir("consumer-flag");
    let dir_a = temp_dir("consumer-flag-a");
    let dir_b = temp_dir("consumer-flag-b");
    pass_fixture(&dir_a, "fixture-pass-a");
    pass_fixture(&dir_b, "fixture-pass-b");
    write_brain_toml(
        &root,
        &[("fixture-pass-a", &dir_a), ("fixture-pass-b", &dir_b)],
    );

    let output = run_check_consumers(&root, &["--json", "--consumer", "fixture-pass-a"]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    let results = parsed.as_array().unwrap();
    assert_eq!(results.len(), 1, "expected exactly one result: {results:?}");
    assert_eq!(results[0]["slug"], "fixture-pass-a");
}

#[test]
fn unknown_consumer_slug_errors_listing_valid_slugs() {
    let root = temp_dir("unknown-slug");
    let dir_a = temp_dir("unknown-slug-a");
    pass_fixture(&dir_a, "fixture-pass-known");
    write_brain_toml(&root, &[("fixture-pass-known", &dir_a)]);

    let output = run_check_consumers(&root, &["--consumer", "does-not-exist"]);

    assert!(
        !output.status.success(),
        "expected failure for an unknown --consumer slug"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("does-not-exist"),
        "error should name the unknown slug: {stderr}"
    );
    assert!(
        stderr.contains("fixture-pass-known"),
        "error should list the valid slugs: {stderr}"
    );
}

#[test]
fn broken_consumer_fails_the_run_with_error_details() {
    let root = temp_dir("broken");
    let dir = temp_dir("broken-consumer");
    broken_fixture(&dir, "fixture-broken");
    write_brain_toml(&root, &[("fixture-broken", &dir)]);

    let output = run_check_consumers(&root, &["--json"]);

    assert!(
        !output.status.success(),
        "a Broken consumer must fail the run (non-zero exit)"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("expected valid JSON, got error {e}: {stdout}"));
    let results = parsed.as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["slug"], "fixture-broken");
    assert_eq!(results[0]["outcome"]["outcome"], "broken");
    let errors = results[0]["outcome"]["errors"]
        .as_array()
        .expect("Broken outcome must carry an errors array");
    assert!(
        !errors.is_empty(),
        "Broken outcome must name at least one error"
    );
}

#[test]
fn lockfile_stale_consumer_exits_zero_and_is_not_reported_broken() {
    let root = temp_dir("lockfile-stale");
    let dir = temp_dir("lockfile-stale-consumer");
    stale_lockfile_fixture(&dir, "fixture-stale");
    write_brain_toml(&root, &[("fixture-stale", &dir)]);

    let output = run_check_consumers(&root, &["--json"]);

    assert!(
        output.status.success(),
        "LockfileStale must NOT fail the run (that is the whole point of distinguishing it \
         from Broken), stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("expected valid JSON, got error {e}: {stdout}"));
    let results = parsed.as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["slug"], "fixture-stale");
    assert_eq!(
        results[0]["outcome"]["outcome"], "lockfile_stale",
        "must be classified lockfile_stale, never broken: {results:?}"
    );
}

#[test]
fn passing_consumer_exits_zero() {
    let root = temp_dir("pass-exit-zero");
    let dir = temp_dir("pass-exit-zero-consumer");
    pass_fixture(&dir, "fixture-pass-exit-zero");
    write_brain_toml(&root, &[("fixture-pass-exit-zero", &dir)]);

    let output = run_check_consumers(&root, &[]);

    assert!(
        output.status.success(),
        "a Pass-only run must exit zero, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
