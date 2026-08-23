//! Integration tests for the `mev graph-findings` CLI subcommand
//! (`MV.ticket.graph-derived-carryover-findings`, Task 4).
//!
//! Drives the built binary directly (`CARGO_BIN_EXE_mev`), the same pattern
//! `tests/emit_block_graph_cli.rs` uses, so the CLI's flag parsing, the
//! non-zero-on-findings exit convention, and `--json` are all genuinely exercised —
//! not just the `mev::graph_findings_report` library function.
//!
//! One repo (`acme`, `repo_path = "."`, i.e. the fixture root itself) carries both
//! detector shapes: a `planning/roadmaps/alpha/lane-substrate.json` lane record, and a
//! `.claude/commands/example.md` that references `scripts/render_spec.py`.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn write_file(root: &Path, rel: &str, content: &str) {
    let full = root.join(rel);
    fs::create_dir_all(full.parent().unwrap()).unwrap();
    fs::write(&full, content.as_bytes()).unwrap();
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
slug = "acme"
tier = "_root"
repo_path = "."
status_file = "planning/status.md"
cache_doc = "docs/projects/acme.md"
heading = "Acme"
"#;
    write_file(root, "brain.toml", toml);
}

/// `planning/state.json` registering exactly one block, `ACME.1.A`.
fn write_state_with_registered_block(root: &Path) {
    let state = serde_json::json!({
        "repo": "acme",
        "kind": "brain",
        "updated": "2026-08-22",
        "focus": { "now": [], "next": [], "blocked": [] },
        "tracks": [
            {
                "title": "Phase 1",
                "blocks": [
                    { "id": "ACME.1.A", "title": "Block One", "status": "open" }
                ]
            }
        ]
    });
    write_json(root, "planning/state.json", &state);
}

/// A lane record naming `block_id` (repo `acme`) — registered or not depending on
/// what `write_state_with_registered_block` (or its absence) put in `state.json`.
fn write_lane_naming_block(root: &Path, block_id: &str) {
    let lane = serde_json::json!({
        "lane": "substrate",
        "roadmap": "alpha",
        "blocks": [
            { "id": block_id, "origin_roadmap": "alpha", "repo": "acme" }
        ]
    });
    write_json(root, "planning/roadmaps/alpha/lane-substrate.json", &lane);
}

/// A command file referencing `scripts/render_spec.py` — the block record's own
/// referenced-path-absent example.
fn write_command_referencing_render_spec(root: &Path) {
    write_file(
        root,
        ".claude/commands/example.md",
        "Run `scripts/render_spec.py --check` before committing.\n",
    );
}

fn write_render_spec_script(root: &Path) {
    write_file(root, "scripts/render_spec.py", "# exists\n");
}

/// Fully clean corpus: the lane's block is registered, and the referenced script
/// exists — zero findings expected from either detector (the positive control).
fn build_clean_fixture() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path();
    write_brain_toml(dir);
    write_state_with_registered_block(dir);
    write_lane_naming_block(dir, "ACME.1.A");
    write_command_referencing_render_spec(dir);
    write_render_spec_script(dir);
    tmp
}

/// Corpus with a planted `unregistered-lane-block` finding: the lane names
/// `ACME.9.Z`, which `state.json` never registers. The referenced script still
/// exists, so this exercises exactly one detector class.
fn build_planted_fixture() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path();
    write_brain_toml(dir);
    write_state_with_registered_block(dir);
    write_lane_naming_block(dir, "ACME.9.Z");
    write_command_referencing_render_spec(dir);
    write_render_spec_script(dir);
    tmp
}

fn run_mev(args: &[&str], cwd: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mev"))
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("failed to spawn mev binary")
}

#[test]
fn clean_corpus_exits_zero() {
    let tmp = build_clean_fixture();
    let output = run_mev(&["graph-findings", "."], tmp.path());
    assert!(
        output.status.success(),
        "expected exit 0 on a clean corpus; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn planted_unregistered_lane_block_exits_nonzero() {
    let tmp = build_planted_fixture();
    let output = run_mev(&["graph-findings", "."], tmp.path());
    assert!(
        !output.status.success(),
        "expected non-zero exit on a corpus with a planted unregistered lane block; \
         stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("unregistered-lane-block"),
        "expected the unregistered-lane-block class to be named in the report: {stdout}"
    );
}

#[test]
fn json_flag_emits_parseable_report_on_the_clean_corpus() {
    let tmp = build_clean_fixture();
    let output = run_mev(&["graph-findings", ".", "--json"], tmp.path());
    assert!(
        output.status.success(),
        "expected exit 0 on a clean corpus; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|err| {
        panic!(
            "stdout did not parse as JSON: {err}; stdout: {}",
            String::from_utf8_lossy(&output.stdout)
        )
    });
    assert_eq!(value["total"], 0);
    assert_eq!(value["unregistered_lane_block"], 0);
    assert_eq!(value["referenced_path_absent"], 0);
    assert!(value["findings"].as_array().unwrap().is_empty());
}

#[test]
fn json_flag_on_planted_corpus_reports_the_finding_with_a_stable_id() {
    let tmp = build_planted_fixture();
    let output = run_mev(&["graph-findings", ".", "--json"], tmp.path());
    assert!(
        !output.status.success(),
        "expected non-zero exit; stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("--json output must parse");
    assert_eq!(value["total"], 1);
    assert_eq!(value["unregistered_lane_block"], 1);
    assert_eq!(value["referenced_path_absent"], 0);

    let findings = value["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0]["detector"], "unregistered-lane-block");
    assert_eq!(findings[0]["repo"], "acme");
    assert_eq!(findings[0]["subject"], "acme:ACME.9.Z");
    assert_eq!(
        findings[0]["finding_id"],
        mev::finding_id(mev::DetectorClass::UnregisteredLaneBlock, "acme:ACME.9.Z")
    );
}

#[test]
fn write_without_the_flag_is_a_report_only_and_is_still_rejected_with_it() {
    // --write is not yet implemented in this task (task 5 of the same spec) -- assert
    // it errors clearly rather than silently doing nothing, per the block record's
    // "never leave a flag that silently does nothing" instruction.
    let tmp = build_clean_fixture();
    let output = run_mev(&["graph-findings", ".", "--write"], tmp.path());
    assert!(
        !output.status.success(),
        "expected --write to fail loudly since it is not yet implemented"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--write") && stderr.contains("not yet implemented"),
        "expected a clear not-yet-implemented error, got: {stderr}"
    );
}
