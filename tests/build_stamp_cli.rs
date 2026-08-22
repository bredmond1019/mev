//! Integration tests for `mev --build-stamp` (Task 1 of
//! `MV.ticket.toolchain-freshness-covers-the-writer`).
//!
//! Drives the built binary directly (`CARGO_BIN_EXE_mev`) rather than the library function,
//! because the acceptance criteria are about the CLI surface itself: exactly one JSON line
//! on stdout, and short-circuiting before any subcommand runs (no subcommand is even given
//! here — that would fail clap's normal "subcommand required" parsing if the flag did not
//! intercept first).

use std::process::Command;

#[test]
fn build_stamp_emits_one_parseable_json_line_with_exact_keys() {
    let output = Command::new(env!("CARGO_BIN_EXE_mev"))
        .arg("--build-stamp")
        .output()
        .expect("failed to run mev --build-stamp");

    assert!(
        output.status.success(),
        "mev --build-stamp should exit 0; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout must be valid UTF-8");
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(
        lines.len(),
        1,
        "expected exactly one non-empty stdout line, got: {stdout:?}"
    );

    let parsed: serde_json::Value =
        serde_json::from_str(lines[0]).expect("stdout line must be valid JSON");
    let obj = parsed
        .as_object()
        .expect("build-stamp JSON must be an object");

    assert_eq!(
        obj.len(),
        3,
        "build-stamp JSON must have exactly 3 keys, got: {obj:?}"
    );
    assert!(obj.contains_key("git_sha"));
    assert!(obj.contains_key("dirty"));
    assert!(obj.contains_key("source_dir"));

    // dirty must be a JSON boolean or the literal string "unknown" — never anything else.
    match &obj["dirty"] {
        serde_json::Value::Bool(_) => {}
        serde_json::Value::String(s) => assert_eq!(s, "unknown"),
        other => panic!("dirty must be a bool or the string \"unknown\", got: {other:?}"),
    }
}

#[test]
fn build_stamp_short_circuits_with_no_subcommand_given() {
    // No subcommand is passed at all. If --build-stamp did not intercept before clap's
    // subcommand requirement, this would fail with a "required argument" parse error
    // instead of succeeding.
    let output = Command::new(env!("CARGO_BIN_EXE_mev"))
        .arg("--build-stamp")
        .output()
        .expect("failed to run mev --build-stamp");

    assert!(output.status.success());
}
