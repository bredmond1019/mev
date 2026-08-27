//! Integration tests for the `mev emit-block-graph` CLI subcommand (Phase 10, Block
//! MV.10.C, Task 4).
//!
//! Drives the built binary directly (`CARGO_BIN_EXE_mev`), the same pattern
//! `tests/doc_cli.rs` uses, so the CLI's flag parsing, scope-flag validation, and exit
//! codes are all genuinely exercised — not just the `mev::block_graph_brain` library
//! function (that is covered by `tests/brain_block_graph.rs`).
//!
//! Fixture shape mirrors `tests/brain_block_graph.rs`'s multi-repo brain fixture:
//! - HQ root (`"hq"`) with an `epics[]` registry (`"epic-x"`, spanning `alpha`/`gamma`)
//!   and `tiers[]` pointing at the `"core"` (`alpha`, `beta`) and `"portfolio"`
//!   (`gamma`) tier sub-brains.
//! - `alpha`: `A1` (open, epic member), `A2` (**closed**), `A3` (open, cross-repo
//!   `depends_on` on `gamma:G1` — a fan-in edge), `A4` (open, no deps).
//! - `beta`: `B1` (open).
//! - `gamma` (tier `portfolio`): `G1` (**closed**, epic member — the fan-in node, with
//!   two dependents: `alpha:A3` and `gamma:G2`), `G2` (open, `depends_on` on `G1`).

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use walkdir::WalkDir;

// ---------------------------------------------------------------------------
// Fixture helpers
// ---------------------------------------------------------------------------

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
slug = "hq"
tier = "_root"
repo_path = "."
status_file = "planning/status.md"
cache_doc = "docs/projects/hq.md"
heading = "HQ"

[[repos]]
slug = "core"
tier = "_root"
repo_path = "core"
status_file = "core/planning/status.md"
cache_doc = "docs/projects/core.md"
heading = "Core Tier"

[[repos]]
slug = "alpha"
tier = "core"
repo_path = "alpha"
status_file = "alpha/planning/status.md"
cache_doc = "docs/projects/alpha.md"
heading = "Alpha"

[[repos]]
slug = "beta"
tier = "core"
repo_path = "beta"
status_file = "beta/planning/status.md"
cache_doc = "docs/projects/beta.md"
heading = "Beta"

[[repos]]
slug = "portfolio"
tier = "_root"
repo_path = "portfolio"
status_file = "portfolio/planning/status.md"
cache_doc = "docs/projects/portfolio.md"
heading = "Portfolio Tier"

[[repos]]
slug = "gamma"
tier = "portfolio"
repo_path = "gamma"
status_file = "gamma/planning/status.md"
cache_doc = "docs/projects/gamma.md"
heading = "Gamma"
"#;
    fs::write(root.join("brain.toml"), toml.as_bytes()).unwrap();
}

fn write_hq_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "hq",
        "kind": "brain",
        "updated": "2026-07-28",
        "focus": { "now": [], "next": [], "blocked": [] },
        "repos": [],
        "cross_repo": [],
        "tiers": [
            { "tier": "core", "rollup": "core/planning/state.json", "summary": null },
            { "tier": "portfolio", "rollup": "portfolio/planning/state.json", "summary": null }
        ],
        "epics": [
            {
                "slug": "epic-x",
                "title": "Epic X",
                "description": "Cross-repo initiative spanning alpha and gamma.",
                "status": "active",
                "plan": null,
                "repos": ["alpha", "gamma"]
            }
        ]
    });
    write_json(root, "planning/state.json", &state);
}

fn write_core_tier_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "core",
        "kind": "brain",
        "updated": "2026-07-28",
        "focus": { "now": [], "next": [], "blocked": [] },
        "repos": [],
        "cross_repo": []
    });
    write_json(root, "core/planning/state.json", &state);
}

fn write_portfolio_tier_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "portfolio",
        "kind": "brain",
        "updated": "2026-07-28",
        "focus": { "now": [], "next": [], "blocked": [] },
        "repos": [],
        "cross_repo": []
    });
    write_json(root, "portfolio/planning/state.json", &state);
}

/// `alpha` — `A1` (open, epic member), `A2` (**closed**), `A3` (open, cross-repo dep
/// on `gamma:G1`), `A4` (open, no deps).
fn write_alpha_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "alpha",
        "kind": "project",
        "updated": "2026-07-28",
        "focus": { "now": [], "next": [], "blocked": [] },
        "tracks": [
            {
                "title": "Phase 1",
                "blocks": [
                    { "id": "A1", "title": "Alpha 1", "status": "open", "epics": ["epic-x"] },
                    { "id": "A2", "title": "Alpha 2", "status": "closed" },
                    {
                        "id": "A3",
                        "title": "Alpha 3",
                        "status": "open",
                        "depends_on": [
                            { "type": "block", "repo": "gamma", "id": "G1" }
                        ]
                    },
                    { "id": "A4", "title": "Alpha 4", "status": "open" }
                ]
            }
        ]
    });
    write_json(root, "alpha/planning/state.json", &state);
}

/// `beta` — `B1` (open, no deps).
fn write_beta_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "beta",
        "kind": "project",
        "updated": "2026-07-28",
        "focus": { "now": [], "next": [], "blocked": [] },
        "tracks": [
            {
                "title": "Phase 1",
                "blocks": [
                    { "id": "B1", "title": "Beta 1", "status": "open" }
                ]
            }
        ]
    });
    write_json(root, "beta/planning/state.json", &state);
}

/// `gamma` (tier `portfolio`) — `G1` (**closed**, epic member, the fan-in node with
/// two dependents: `alpha:A3` and `gamma:G2`), `G2` (open, `depends_on` on `G1`).
fn write_gamma_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "gamma",
        "kind": "project",
        "updated": "2026-07-28",
        "focus": { "now": [], "next": [], "blocked": [] },
        "tracks": [
            {
                "title": "Phase 1",
                "blocks": [
                    { "id": "G1", "title": "Gamma 1", "status": "closed", "epics": ["epic-x"] },
                    {
                        "id": "G2",
                        "title": "Gamma 2",
                        "status": "open",
                        "depends_on": [
                            { "type": "block", "repo": "gamma", "id": "G1" }
                        ]
                    }
                ]
            }
        ]
    });
    write_json(root, "gamma/planning/state.json", &state);
}

/// Build the complete fixture in a fresh tempdir and return it (kept alive for the
/// caller's whole test — dropping it removes the directory).
fn build_fixture() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    write_brain_toml(dir);
    write_hq_state(dir);
    write_core_tier_state(dir);
    write_portfolio_tier_state(dir);
    write_alpha_state(dir);
    write_beta_state(dir);
    write_gamma_state(dir);
    tmp
}

fn run_mev(args: &[&str], cwd: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mev"))
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("failed to spawn mev binary")
}

fn stdout_json(output: &Output) -> serde_json::Value {
    assert!(
        output.status.success(),
        "expected exit 0; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|err| {
        panic!(
            "stdout did not parse as JSON: {err}; stdout: {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

fn node_keys(export: &serde_json::Value) -> Vec<String> {
    let mut keys: Vec<String> = export["nodes"]
        .as_array()
        .expect("nodes must be an array")
        .iter()
        .map(|n| n["key"].as_str().unwrap().to_string())
        .collect();
    keys.sort();
    keys
}

fn find_node<'a>(export: &'a serde_json::Value, key: &str) -> &'a serde_json::Value {
    export["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["key"] == key)
        .unwrap_or_else(|| panic!("node {key} not found in export"))
}

/// Recursive listing of every regular file under `root`, as `(relative path, contents)`
/// pairs, sorted by path — a true before/after snapshot, not tied to a fixed file list.
fn snapshot(root: &Path) -> Vec<(String, Vec<u8>)> {
    let mut entries: Vec<(String, Vec<u8>)> = WalkDir::new(root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| {
            let rel = e
                .path()
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .to_string();
            let bytes = fs::read(e.path()).unwrap();
            (rel, bytes)
        })
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn exits_zero_and_emits_parseable_json_with_version_one() {
    let tmp = build_fixture();
    let output = run_mev(&["emit-block-graph", "."], tmp.path());
    let export = stdout_json(&output);
    assert_eq!(export["version"], "1");
}

#[test]
fn pretty_output_is_multiline_and_parses_to_the_same_value_as_compact() {
    let tmp = build_fixture();

    let compact_output = run_mev(&["emit-block-graph", "."], tmp.path());
    let pretty_output = run_mev(&["emit-block-graph", "--pretty", "."], tmp.path());

    let compact_stdout = String::from_utf8_lossy(&compact_output.stdout).to_string();
    let pretty_stdout = String::from_utf8_lossy(&pretty_output.stdout).to_string();

    assert_eq!(
        compact_stdout.lines().count(),
        1,
        "compact output must be single-line"
    );
    assert!(
        pretty_stdout.lines().count() > 1,
        "pretty output must be multi-line, got: {pretty_stdout}"
    );

    let compact_value = stdout_json(&compact_output);
    let pretty_value = stdout_json(&pretty_output);
    assert_eq!(
        compact_value, pretty_value,
        "--pretty must parse to the same value as compact output"
    );
}

#[test]
fn scope_tier_limits_the_node_set_to_repos_in_that_tier() {
    let tmp = build_fixture();

    let hq_export = stdout_json(&run_mev(
        &["emit-block-graph", "--include-closed", "."],
        tmp.path(),
    ));
    let tier_export = stdout_json(&run_mev(
        &[
            "emit-block-graph",
            "--include-closed",
            "--scope",
            "tier",
            "--tier",
            "core",
            ".",
        ],
        tmp.path(),
    ));

    assert_ne!(
        node_keys(&hq_export),
        node_keys(&tier_export),
        "--scope tier must change the emitted node set"
    );
    // Only alpha/beta (tier "core") blocks should be present; gamma (tier
    // "portfolio") must be excluded.
    assert!(
        node_keys(&tier_export)
            .iter()
            .all(|k| k.starts_with("alpha:") || k.starts_with("beta:")),
        "tier-scoped export must contain only core-tier repos, got: {:?}",
        node_keys(&tier_export)
    );
}

#[test]
fn scope_repo_limits_the_node_set_to_that_repo() {
    let tmp = build_fixture();

    let hq_export = stdout_json(&run_mev(
        &["emit-block-graph", "--include-closed", "."],
        tmp.path(),
    ));
    let repo_export = stdout_json(&run_mev(
        &[
            "emit-block-graph",
            "--include-closed",
            "--scope",
            "repo",
            "--repo",
            "alpha",
            ".",
        ],
        tmp.path(),
    ));

    assert_ne!(node_keys(&hq_export), node_keys(&repo_export));
    assert!(
        node_keys(&repo_export)
            .iter()
            .all(|k| k.starts_with("alpha:")),
        "repo-scoped export must contain only alpha blocks, got: {:?}",
        node_keys(&repo_export)
    );
}

#[test]
fn scope_epic_projects_only_epic_member_blocks() {
    let tmp = build_fixture();

    let hq_export = stdout_json(&run_mev(
        &["emit-block-graph", "--include-closed", "."],
        tmp.path(),
    ));
    let epic_export = stdout_json(&run_mev(
        &[
            "emit-block-graph",
            "--include-closed",
            "--scope",
            "epic",
            "--epic",
            "epic-x",
            ".",
        ],
        tmp.path(),
    ));

    assert_ne!(node_keys(&hq_export), node_keys(&epic_export));
    let epic_keys = node_keys(&epic_export);
    assert!(
        epic_keys.contains(&"alpha:A1".to_string()),
        "epic-x member alpha:A1 must be present, got: {epic_keys:?}"
    );
    assert!(
        epic_keys.contains(&"gamma:G1".to_string()),
        "epic-x member gamma:G1 must be present, got: {epic_keys:?}"
    );
    assert!(
        !epic_keys.contains(&"beta:B1".to_string()),
        "beta:B1 is not an epic-x member and must be excluded, got: {epic_keys:?}"
    );
}

#[test]
fn include_closed_adds_closed_blocks() {
    let tmp = build_fixture();

    let without_closed = stdout_json(&run_mev(&["emit-block-graph", "."], tmp.path()));
    let with_closed = stdout_json(&run_mev(
        &["emit-block-graph", "--include-closed", "."],
        tmp.path(),
    ));

    let without_keys = node_keys(&without_closed);
    let with_keys = node_keys(&with_closed);

    assert!(
        !without_keys.contains(&"alpha:A2".to_string()),
        "closed alpha:A2 must be absent by default, got: {without_keys:?}"
    );
    assert!(
        with_keys.contains(&"alpha:A2".to_string()),
        "--include-closed must add closed alpha:A2, got: {with_keys:?}"
    );
    assert_ne!(without_keys, with_keys);
}

#[test]
fn include_boundary_adds_out_of_scope_boundary_nodes() {
    let tmp = build_fixture();

    // Scope to alpha only; without boundary, gamma:G1 (the cross-repo dependency
    // target of alpha:A3) must not appear. With boundary, it should show up flagged
    // `in_scope: false`.
    let without_boundary = stdout_json(&run_mev(
        &[
            "emit-block-graph",
            "--include-closed",
            "--scope",
            "repo",
            "--repo",
            "alpha",
            ".",
        ],
        tmp.path(),
    ));
    let with_boundary = stdout_json(&run_mev(
        &[
            "emit-block-graph",
            "--include-closed",
            "--include-boundary",
            "--scope",
            "repo",
            "--repo",
            "alpha",
            ".",
        ],
        tmp.path(),
    ));

    let without_keys = node_keys(&without_boundary);
    let with_keys = node_keys(&with_boundary);
    assert_ne!(
        without_keys, with_keys,
        "--include-boundary must change the node set"
    );

    assert!(
        !without_keys.contains(&"gamma:G1".to_string()),
        "gamma:G1 must not appear without --include-boundary, got: {without_keys:?}"
    );
    assert!(
        with_keys.contains(&"gamma:G1".to_string()),
        "gamma:G1 must appear as a boundary node with --include-boundary, got: {with_keys:?}"
    );
    let boundary_node = find_node(&with_boundary, "gamma:G1");
    assert_eq!(
        boundary_node["in_scope"], false,
        "boundary node must be flagged in_scope: false"
    );
}

#[test]
fn max_nodes_caps_the_list_and_sets_truncated() {
    let tmp = build_fixture();

    let full_export = stdout_json(&run_mev(
        &["emit-block-graph", "--include-closed", "."],
        tmp.path(),
    ));
    let full_count = full_export["nodes"].as_array().unwrap().len();
    assert!(
        full_count > 1,
        "fixture must have more than one node to cap"
    );
    assert!(
        !full_export["truncated"].as_bool().unwrap(),
        "untruncated export must report truncated: false"
    );

    let capped_export = stdout_json(&run_mev(
        &[
            "emit-block-graph",
            "--include-closed",
            "--max-nodes",
            "1",
            ".",
        ],
        tmp.path(),
    ));
    assert_eq!(
        capped_export["nodes"].as_array().unwrap().len(),
        1,
        "--max-nodes 1 must cap the node list at 1"
    );
    assert!(
        capped_export["truncated"].as_bool().unwrap(),
        "capped export must report truncated: true"
    );
}

#[test]
fn unknown_epic_exits_one_and_names_the_slug_on_stderr() {
    let tmp = build_fixture();
    let output = run_mev(
        &[
            "emit-block-graph",
            "--scope",
            "epic",
            "--epic",
            "no-such-epic",
            ".",
        ],
        tmp.path(),
    );
    assert!(
        !output.status.success(),
        "unknown --epic must exit non-zero"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("no-such-epic"),
        "stderr must name the unknown slug, got: {stderr}"
    );
}

#[test]
fn scope_epic_without_epic_flag_exits_one() {
    let tmp = build_fixture();
    let output = run_mev(&["emit-block-graph", "--scope", "epic", "."], tmp.path());
    assert!(
        !output.status.success(),
        "--scope epic without --epic must exit non-zero"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--epic"),
        "stderr must mention the missing --epic flag, got: {stderr}"
    );
}

#[test]
fn scope_repo_without_repo_flag_exits_one() {
    let tmp = build_fixture();
    let output = run_mev(&["emit-block-graph", "--scope", "repo", "."], tmp.path());
    assert!(
        !output.status.success(),
        "--scope repo without --repo must exit non-zero"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--repo"),
        "stderr must mention the missing --repo flag, got: {stderr}"
    );
}

#[test]
fn two_consecutive_runs_produce_byte_identical_stdout() {
    let tmp = build_fixture();
    let output1 = run_mev(&["emit-block-graph", "--include-closed", "."], tmp.path());
    let output2 = run_mev(&["emit-block-graph", "--include-closed", "."], tmp.path());

    assert!(output1.status.success() && output2.status.success());
    assert_eq!(
        output1.stdout, output2.stdout,
        "two consecutive runs over an unchanged corpus must produce byte-identical stdout"
    );
}

#[test]
fn nothing_is_written_to_disk() {
    let tmp = build_fixture();
    let before = snapshot(tmp.path());

    let output = run_mev(
        &["emit-block-graph", "--pretty", "--include-closed", "."],
        tmp.path(),
    );
    assert!(output.status.success());

    let after = snapshot(tmp.path());
    assert_eq!(
        before, after,
        "emit-block-graph must never create or modify any file on disk"
    );
}

#[test]
fn dependent_count_is_identical_between_unscoped_and_repo_scoped_export() {
    let tmp = build_fixture();

    // gamma:G1 is the fan-in node: alpha:A3 (cross-repo) and gamma:G2 both depend on
    // it, so its dependent_count must be 2 — and identical whether the export is
    // unscoped (hq) or scoped down to the repo that owns the node (gamma).
    let hq_export = stdout_json(&run_mev(
        &["emit-block-graph", "--include-closed", "."],
        tmp.path(),
    ));
    let repo_export = stdout_json(&run_mev(
        &[
            "emit-block-graph",
            "--include-closed",
            "--scope",
            "repo",
            "--repo",
            "gamma",
            ".",
        ],
        tmp.path(),
    ));

    let hq_node = find_node(&hq_export, "gamma:G1");
    let repo_node = find_node(&repo_export, "gamma:G1");

    assert_eq!(hq_node["dependent_count"], 2);
    assert_eq!(
        hq_node["dependent_count"], repo_node["dependent_count"],
        "dependent_count for gamma:G1 must be identical across scoped and unscoped exports"
    );
}
