//! Integration tests for `mev::brain::last_touched::derive_last_touched` — the
//! corpus-wide SDLC-artifact recency derivation (Phase 10, Block MV.10.D, Task 3).
//!
//! Mirrors the fixture-construction style of `tests/brain_state.rs` /
//! `tests/brain_block_graph.rs`: a temp HQ-root fixture with three leaf repos, each
//! reproducing one of the three real spec-folder naming conventions the block
//! definition measured against the live corpus:
//!
//! - `engine-rs` (`prefix = "EN"`) — full-ID folder `EN.7.C-materialize-harvest-gate`,
//!   plus a block whose folder sits under `planning/archive/`, plus a never-started
//!   block with no folder at all.
//! - `bastion-web` (`prefix = "BW"`) — bare-ID folder `BW.9.A`, plus a block with two
//!   matching folders carrying different `updated_at` values, plus a never-started
//!   block.
//! - `mev` (`prefix = "MV"`) — prefix-stripped folder `10.C-emit-block-graph-cli`,
//!   plus a block whose folder holds both `sdlc-flow-state.json` and
//!   `sdlc-task-state.json` with different `updated_at` values, plus a never-started
//!   block.
//!
//! The last test exercises the exact consumption path bastion's `BA.11.S` will use:
//! `discover_state_files` + `load_state` + `derive_rollup`, joined to
//! `derive_last_touched`'s map by `"{repo}:{id}"`.

use std::fs;
use std::path::Path;
use std::time::SystemTime;

use mev::brain::config::{BrainConfig, load_brain_config};
use mev::brain::last_touched::derive_last_touched;
use mev::brain::state::{
    StateFile, StateGraph, StateSource, TierScope, derive_rollup, discover_state_files, load_state,
};
use mev::{BlockGraphScope, block_graph_brain};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn write_file(root: &Path, rel: &str, content: &str) {
    let full = root.join(rel);
    fs::create_dir_all(full.parent().unwrap()).unwrap();
    fs::write(&full, content.as_bytes()).unwrap();
}

fn write_json(root: &Path, rel: &str, value: &serde_json::Value) {
    write_file(root, rel, &serde_json::to_string_pretty(value).unwrap());
}

fn temp_dir(suffix: &str) -> std::path::PathBuf {
    let dir = mev::testsupport::unique_temp_dir(&format!("mev-brain-last-touched-{suffix}"));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Write a `<spec_folder>/sdlc/<name>` state-file fixture with only `updated_at` set
/// (the rest of the module's schema is irrelevant to `derive_last_touched`).
fn write_state_artifact(root: &Path, spec_folder_rel: &str, name: &str, updated_at: &str) {
    write_json(
        root,
        &format!("{spec_folder_rel}/sdlc/{name}"),
        &serde_json::json!({ "updated_at": updated_at }),
    );
}

/// Same as [`write_state_artifact`] but also sets `status`, for the
/// `LastTouched.status` coverage tests (Task 1 of `ticket-reconcile-failed-consumer`).
fn write_state_artifact_with_status(
    root: &Path,
    spec_folder_rel: &str,
    name: &str,
    updated_at: &str,
    status: &str,
) {
    write_json(
        root,
        &format!("{spec_folder_rel}/sdlc/{name}"),
        &serde_json::json!({ "updated_at": updated_at, "status": status }),
    );
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
slug = "engine-rs"
tier = "core"
prefix = "EN"
repo_path = "engine-rs"
status_file = "engine-rs/planning/status.md"
cache_doc = "docs/projects/engine-rs.md"
heading = "Engine"

[[repos]]
slug = "bastion-web"
tier = "core"
prefix = "BW"
repo_path = "bastion-web"
status_file = "bastion-web/planning/status.md"
cache_doc = "docs/projects/bastion-web.md"
heading = "Bastion Web"

[[repos]]
slug = "mev"
tier = "core"
prefix = "MV"
repo_path = "mev"
status_file = "mev/planning/status.md"
cache_doc = "docs/projects/mev.md"
heading = "mev"
"#;
    fs::write(root.join("brain.toml"), toml.as_bytes()).unwrap();
}

fn write_hq_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "hq",
        "kind": "brain",
        "updated": "2026-07-29",
        "focus": { "now": [], "next": [], "blocked": [] },
        "repos": [],
        "cross_repo": [],
        "tiers": []
    });
    write_json(root, "planning/state.json", &state);
}

/// `engine-rs` — `EN.7.C-materialize-harvest-gate` (full-ID folder), `EN.5.A-closed`
/// (archived folder under `planning/archive/`), `EN.9.Z-never-started` (no folder).
fn write_engine_rs_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "engine-rs",
        "kind": "project",
        "updated": "2026-07-29",
        "focus": { "now": [], "next": [], "blocked": [] },
        "tracks": [
            {
                "title": "Phase 7",
                "blocks": [
                    { "id": "EN.7.C-materialize-harvest-gate", "title": "Harvest gate" },
                    { "id": "EN.5.A-closed", "title": "Closed block" },
                    { "id": "EN.9.Z-never-started", "title": "Never started" }
                ]
            }
        ]
    });
    write_json(root, "engine-rs/planning/state.json", &state);
}

/// `bastion-web` — `BW.9.A` (bare-ID folder), `BW.2.A` (two matching folders, newest
/// wins), `BW.6.G-ghost` (no folder).
fn write_bastion_web_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "bastion-web",
        "kind": "project",
        "updated": "2026-07-29",
        "focus": { "now": [], "next": [], "blocked": [] },
        "tracks": [
            {
                "title": "Phase 9",
                "blocks": [
                    { "id": "BW.9.A", "title": "Bare-ID block" },
                    { "id": "BW.2.A", "title": "Two-folder block" },
                    { "id": "BW.6.G-ghost", "title": "Never started" }
                ]
            }
        ]
    });
    write_json(root, "bastion-web/planning/state.json", &state);
}

/// `mev` — `MV.10.C` (prefix-stripped folder), `MV.3.B-slug` (two state-file kinds,
/// newest wins), `MV.8.Z-ghost` (no folder).
fn write_mev_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "mev",
        "kind": "project",
        "updated": "2026-07-29",
        "tracks": [
            {
                "title": "Phase 10",
                "blocks": [
                    { "id": "MV.10.C", "title": "Prefix-stripped block" },
                    { "id": "MV.3.B-slug", "title": "Two-kind block" },
                    { "id": "MV.8.Z-ghost", "title": "Never started" }
                ]
            }
        ]
    });
    write_json(root, "mev/planning/state.json", &state);
}

/// Build the full fixture corpus reproducing all three real folder conventions plus
/// the archive, multi-folder, and multi-kind resolution rules in one tree.
fn write_fixture(root: &Path) {
    write_brain_toml(root);
    write_hq_state(root);
    write_engine_rs_state(root);
    write_bastion_web_state(root);
    write_mev_state(root);

    // engine-rs: full-ID folder.
    write_state_artifact(
        root,
        "engine-rs/planning/EN.7.C-materialize-harvest-gate",
        "sdlc-task-state.json",
        "2026-07-10T09:00:00Z",
    );
    // engine-rs: archived folder.
    write_state_artifact(
        root,
        "engine-rs/planning/archive/EN.5.A-closed",
        "sdlc-flow-state.json",
        "2026-03-01T12:00:00Z",
    );

    // bastion-web: bare-ID folder.
    write_state_artifact(
        root,
        "bastion-web/planning/BW.9.A",
        "sdlc-run-state.json",
        "2026-06-15T08:30:00Z",
    );
    // bastion-web: two matching folders, different timestamps (newest wins).
    write_state_artifact(
        root,
        "bastion-web/planning/BW.2.A-attempt1",
        "sdlc-task-state.json",
        "2026-05-01T00:00:00Z",
    );
    write_state_artifact(
        root,
        "bastion-web/planning/BW.2.A-final",
        "sdlc-task-state.json",
        "2026-07-20T18:00:00Z",
    );

    // mev: prefix-stripped folder.
    write_state_artifact(
        root,
        "mev/planning/10.C-emit-block-graph-cli",
        "sdlc-task-state.json",
        "2026-07-25T14:00:00Z",
    );
    // mev: one folder, two state-file kinds, different timestamps (newest wins).
    write_state_artifact(
        root,
        "mev/planning/MV.3.B-slug",
        "sdlc-flow-state.json",
        "2026-04-01T00:00:00Z",
    );
    write_state_artifact(
        root,
        "mev/planning/MV.3.B-slug",
        "sdlc-task-state.json",
        "2026-07-28T23:00:00Z",
    );
}

/// Discover + load every `planning/state.json` under `root`, returning the pairs
/// `derive_last_touched` (and `derive_rollup`) expect.
fn discover_and_load(root: &Path, config: &BrainConfig) -> Vec<(StateSource, StateFile)> {
    let (sources, _diags) = discover_state_files(root, config);
    sources
        .into_iter()
        .filter_map(|src| {
            let file = load_state(&src.abs_path).ok()?;
            Some((src, file))
        })
        .collect()
}

/// Recursively snapshot every file's relative path + modified time under `root`, for
/// the read-only (nothing-written) assertion.
fn snapshot(root: &Path) -> Vec<(std::path::PathBuf, SystemTime)> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<(std::path::PathBuf, SystemTime)>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, root, out);
            } else if let Ok(meta) = entry.metadata()
                && let Ok(modified) = meta.modified()
            {
                out.push((path.strip_prefix(root).unwrap().to_path_buf(), modified));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn full_id_folder_resolves_to_its_updated_at() {
    let dir = temp_dir("full-id");
    write_fixture(&dir);
    let config = load_brain_config(&dir.join("brain.toml")).expect("brain.toml must parse");

    let loaded = discover_and_load(&dir, &config);
    let map = derive_last_touched(&dir, &config, &loaded);

    assert_eq!(
        map.get("engine-rs:EN.7.C-materialize-harvest-gate")
            .map(|lt| lt.updated_at.as_str()),
        Some("2026-07-10T09:00:00Z")
    );
}

#[test]
fn bare_id_folder_resolves_to_its_updated_at() {
    let dir = temp_dir("bare-id");
    write_fixture(&dir);
    let config = load_brain_config(&dir.join("brain.toml")).expect("brain.toml must parse");

    let loaded = discover_and_load(&dir, &config);
    let map = derive_last_touched(&dir, &config, &loaded);

    assert_eq!(
        map.get("bastion-web:BW.9.A")
            .map(|lt| lt.updated_at.as_str()),
        Some("2026-06-15T08:30:00Z")
    );
}

#[test]
fn prefix_stripped_folder_resolves_to_its_updated_at() {
    let dir = temp_dir("prefix-stripped");
    write_fixture(&dir);
    let config = load_brain_config(&dir.join("brain.toml")).expect("brain.toml must parse");

    let loaded = discover_and_load(&dir, &config);
    let map = derive_last_touched(&dir, &config, &loaded);

    assert_eq!(
        map.get("mev:MV.10.C").map(|lt| lt.updated_at.as_str()),
        Some("2026-07-25T14:00:00Z")
    );
}

#[test]
fn never_started_blocks_are_absent_and_key_set_is_exact() {
    let dir = temp_dir("never-started");
    write_fixture(&dir);
    let config = load_brain_config(&dir.join("brain.toml")).expect("brain.toml must parse");

    let loaded = discover_and_load(&dir, &config);
    let map = derive_last_touched(&dir, &config, &loaded);

    assert!(!map.contains_key("engine-rs:EN.9.Z-never-started"));
    assert!(!map.contains_key("bastion-web:BW.6.G-ghost"));
    assert!(!map.contains_key("mev:MV.8.Z-ghost"));

    let mut keys: Vec<&str> = map.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "bastion-web:BW.2.A",
            "bastion-web:BW.9.A",
            "engine-rs:EN.5.A-closed",
            "engine-rs:EN.7.C-materialize-harvest-gate",
            "mev:MV.10.C",
            "mev:MV.3.B-slug",
        ],
        "the map's key set must contain exactly the blocks with a resolvable SDLC run \
         — no fabricated entries for the never-started blocks"
    );
}

#[test]
fn archived_folder_is_counted() {
    let dir = temp_dir("archived");
    write_fixture(&dir);
    let config = load_brain_config(&dir.join("brain.toml")).expect("brain.toml must parse");

    let loaded = discover_and_load(&dir, &config);
    let map = derive_last_touched(&dir, &config, &loaded);

    assert_eq!(
        map.get("engine-rs:EN.5.A-closed")
            .map(|lt| lt.updated_at.as_str()),
        Some("2026-03-01T12:00:00Z")
    );
}

#[test]
fn newest_wins_across_two_matching_folders() {
    let dir = temp_dir("newest-folders");
    write_fixture(&dir);
    let config = load_brain_config(&dir.join("brain.toml")).expect("brain.toml must parse");

    let loaded = discover_and_load(&dir, &config);
    let map = derive_last_touched(&dir, &config, &loaded);

    assert_eq!(
        map.get("bastion-web:BW.2.A")
            .map(|lt| lt.updated_at.as_str()),
        Some("2026-07-20T18:00:00Z"),
        "newest updated_at across BW.2.A-attempt1 and BW.2.A-final must win"
    );
}

#[test]
fn newest_wins_across_state_file_kinds() {
    let dir = temp_dir("newest-kinds");
    write_fixture(&dir);
    let config = load_brain_config(&dir.join("brain.toml")).expect("brain.toml must parse");

    let loaded = discover_and_load(&dir, &config);
    let map = derive_last_touched(&dir, &config, &loaded);

    assert_eq!(
        map.get("mev:MV.3.B-slug").map(|lt| lt.updated_at.as_str()),
        Some("2026-07-28T23:00:00Z"),
        "newest updated_at across sdlc-flow-state.json and sdlc-task-state.json must win"
    );
}

#[test]
fn map_is_deterministic_across_consecutive_calls() {
    let dir = temp_dir("determinism");
    write_fixture(&dir);
    let config = load_brain_config(&dir.join("brain.toml")).expect("brain.toml must parse");

    let loaded = discover_and_load(&dir, &config);
    let first = derive_last_touched(&dir, &config, &loaded);
    let second = derive_last_touched(&dir, &config, &loaded);

    assert_eq!(first, second);
}

#[test]
fn derivation_is_read_only() {
    let dir = temp_dir("read-only");
    write_fixture(&dir);
    let config = load_brain_config(&dir.join("brain.toml")).expect("brain.toml must parse");
    let loaded = discover_and_load(&dir, &config);

    let before = snapshot(&dir);
    let _map = derive_last_touched(&dir, &config, &loaded);
    let after = snapshot(&dir);

    assert_eq!(
        before, after,
        "derive_last_touched must not create, modify, or delete any file on disk"
    );
}

/// Exercises the exact consumption path bastion's `BA.11.S` will use:
/// `discover_state_files` + `load_state` + `derive_rollup`, joined to
/// `derive_last_touched`'s map by `"{repo}:{id}"`. This is the one-derivation
/// cross-check, run mev-side so `BA.11.S` inherits it rather than re-proving it.
///
/// **Deviation (recorded in the spec's Amendment Log):** the task description frames
/// this as a cross-check against "the block-graph export"'s `last_touched` field, but
/// that field is threaded onto `BlockGraphNode` by Task 4, which has not landed as of
/// this task (Task 3 depends only on Task 2). The join is instead verified against
/// `derive_last_touched` computed directly over the same `loaded` corpus — the
/// strongest check available without Task 4's field, and the exact equality Task 4's
/// own node-population code must preserve when it lands.
#[test]
fn rollup_consumption_path_joins_to_the_last_touched_map() {
    let dir = temp_dir("consumption-path");
    write_fixture(&dir);
    let config = load_brain_config(&dir.join("brain.toml")).expect("brain.toml must parse");

    let loaded = discover_and_load(&dir, &config);
    let last_touched = derive_last_touched(&dir, &config, &loaded);

    let rollups = derive_rollup(
        &TierScope::All,
        &config,
        &[],
        &StateGraph::default(),
        &loaded,
    );

    // Independently recompute the map so the join is checked against a second
    // derivation over the same inputs, not against itself.
    let expected = derive_last_touched(&dir, &config, &loaded);

    let mut checked_present = 0;
    let mut checked_absent = 0;

    for rollup in &rollups {
        for block in rollup
            .now
            .iter()
            .chain(rollup.next.iter())
            .chain(rollup.blocked.iter())
            .chain(rollup.deferred.iter())
        {
            let key = format!("{}:{}", rollup.repo, block.id);
            match (last_touched.get(&key), expected.get(&key)) {
                (Some(got), Some(want)) => {
                    assert_eq!(got, want, "joined value for {key} must agree");
                    checked_present += 1;
                }
                (None, None) => {
                    checked_absent += 1;
                }
                (got, want) => {
                    panic!("presence mismatch for {key}: last_touched={got:?}, expected={want:?}")
                }
            }
        }
    }

    // `derive_rollup`'s Block-derived focus lists (now/next/blocked/deferred) are only
    // populated for open/in-progress/blocked/deferred blocks in this fixture (none of
    // the fixture blocks carry an explicit `status`, so `derive_focus` buckets them as
    // `now`), so at minimum every resolvable block in the fixture must have been
    // joined and agreed.
    assert!(
        checked_present >= 6,
        "expected at least 6 resolvable blocks to join through the rollup path, got {checked_present}"
    );
    let _ = checked_absent;
}

// ---------------------------------------------------------------------------
// Task 1 (ticket-reconcile-failed-consumer): `LastTouched.status` coverage.
// ---------------------------------------------------------------------------

/// Minimal single-block fixture (not the shared `write_fixture` corpus) purpose-built
/// for the `status`-threading tests below: one repo, one block.
fn write_single_block_fixture(root: &Path) {
    write_brain_toml(root);
    write_hq_state(root);
    let state = serde_json::json!({
        "repo": "mev",
        "kind": "project",
        "updated": "2026-08-01",
        "tracks": [
            {
                "title": "Phase 10",
                "blocks": [ { "id": "MV.10.C", "title": "Status-threading block" } ]
            }
        ]
    });
    write_json(root, "mev/planning/state.json", &state);
}

#[test]
fn winning_files_status_is_returned_when_two_folders_have_different_timestamps() {
    let dir = temp_dir("status-winner");
    write_single_block_fixture(&dir);

    // Older folder: status "done". Newer folder: status "reconcile_failed". The
    // NEWER folder's status must win, tracked alongside its own updated_at — never a
    // status borrowed from the older (losing) file.
    write_state_artifact_with_status(
        &dir,
        "mev/planning/MV.10.C-first-attempt",
        "sdlc-task-state.json",
        "2026-06-01T00:00:00Z",
        "done",
    );
    write_state_artifact_with_status(
        &dir,
        "mev/planning/MV.10.C-final",
        "sdlc-task-state.json",
        "2026-07-10T00:00:00Z",
        "reconcile_failed",
    );

    let config = load_brain_config(&dir.join("brain.toml")).expect("brain.toml must parse");
    let loaded = discover_and_load(&dir, &config);
    let map = derive_last_touched(&dir, &config, &loaded);

    let entry = map.get("mev:MV.10.C").expect("block must resolve");
    assert_eq!(entry.updated_at, "2026-07-10T00:00:00Z");
    assert_eq!(
        entry.status.as_deref(),
        Some("reconcile_failed"),
        "status must come from the SAME file that won on updated_at, not the older loser"
    );
}

#[test]
fn a_file_with_updated_at_and_no_status_yields_none() {
    let dir = temp_dir("status-none");
    write_single_block_fixture(&dir);

    // write_state_artifact (no status field) — the plain helper used everywhere else.
    write_state_artifact(
        &dir,
        "mev/planning/MV.10.C",
        "sdlc-task-state.json",
        "2026-07-01T00:00:00Z",
    );

    let config = load_brain_config(&dir.join("brain.toml")).expect("brain.toml must parse");
    let loaded = discover_and_load(&dir, &config);
    let map = derive_last_touched(&dir, &config, &loaded);

    let entry = map.get("mev:MV.10.C").expect("block must resolve");
    assert_eq!(entry.updated_at, "2026-07-01T00:00:00Z");
    assert_eq!(
        entry.status, None,
        "a state file with no status field must yield status: None, never a substituted default"
    );
}

#[test]
fn a_block_with_no_state_file_is_absent_from_the_map_entirely() {
    let dir = temp_dir("status-absent");
    write_single_block_fixture(&dir);
    // No sdlc/ artifact written at all for MV.10.C.

    let config = load_brain_config(&dir.join("brain.toml")).expect("brain.toml must parse");
    let loaded = discover_and_load(&dir, &config);
    let map = derive_last_touched(&dir, &config, &loaded);

    assert!(
        !map.contains_key("mev:MV.10.C"),
        "a block with no resolvable SDLC run must have no map entry at all — not present \
         with a None status, absent entirely"
    );
}

// ---------------------------------------------------------------------------
// Task 2 (ticket-reconcile-failed-consumer): `BlockGraphNode.reconcile_failed`
// coverage. Exercised through the public `block_graph_brain` driver (never the
// internal builder), matching `tests/brain_block_graph.rs`'s convention.
// ---------------------------------------------------------------------------

/// Unscoped scope — closed included, no boundary, unlimited `max_nodes` — the same
/// baseline `tests/brain_block_graph.rs::default_scope()` uses.
fn default_scope() -> BlockGraphScope {
    BlockGraphScope {
        tier: TierScope::All,
        epic: None,
        repo: None,
        include_closed: true,
        include_boundary: false,
        max_nodes: usize::MAX,
    }
}

#[test]
fn node_is_flagged_when_newest_run_ended_reconcile_failed() {
    let dir = temp_dir("reconcile-flag-true");
    write_single_block_fixture(&dir);
    write_state_artifact_with_status(
        &dir,
        "mev/planning/MV.10.C",
        "sdlc-task-state.json",
        "2026-07-10T00:00:00Z",
        "reconcile_failed",
    );

    let export = block_graph_brain(&dir, &default_scope()).expect("block graph export must build");
    let node = export
        .nodes
        .iter()
        .find(|n| n.key == "mev:MV.10.C")
        .expect("block must resolve");

    assert_eq!(
        node.reconcile_failed,
        Some(true),
        "a block whose newest run ended reconcile_failed must be flagged Some(true)"
    );
}

#[test]
fn node_is_not_flagged_when_newest_run_ended_done() {
    let dir = temp_dir("reconcile-flag-done");
    write_single_block_fixture(&dir);
    write_state_artifact_with_status(
        &dir,
        "mev/planning/MV.10.C",
        "sdlc-task-state.json",
        "2026-07-10T00:00:00Z",
        "done",
    );

    let export = block_graph_brain(&dir, &default_scope()).expect("block graph export must build");
    let node = export
        .nodes
        .iter()
        .find(|n| n.key == "mev:MV.10.C")
        .expect("block must resolve");

    assert_eq!(
        node.reconcile_failed,
        Some(false),
        "a block whose newest run ended done must be Some(false) — a run was found, just \
         not a reconcile_failed one"
    );
}

#[test]
fn node_is_none_when_no_state_file_exists() {
    let dir = temp_dir("reconcile-flag-none");
    write_single_block_fixture(&dir);
    // No sdlc/ artifact written at all for MV.10.C.

    let export = block_graph_brain(&dir, &default_scope()).expect("block graph export must build");
    let node = export
        .nodes
        .iter()
        .find(|n| n.key == "mev:MV.10.C")
        .expect("block must resolve");

    assert_eq!(
        node.reconcile_failed, None,
        "a block with no resolvable SDLC run must be None, never a sentinel false"
    );
}

#[test]
fn older_reconcile_failed_does_not_flag_a_node_whose_newer_run_ended_done() {
    let dir = temp_dir("reconcile-flag-winner-tracking");
    write_single_block_fixture(&dir);

    // Older folder: reconcile_failed. Newer folder: done. The NEWER file's status must
    // win — this is exactly the case Task 1's winner-tracking exists to get right, and
    // a regression here would mean status got detached from its own updated_at.
    write_state_artifact_with_status(
        &dir,
        "mev/planning/MV.10.C-first-attempt",
        "sdlc-task-state.json",
        "2026-06-01T00:00:00Z",
        "reconcile_failed",
    );
    write_state_artifact_with_status(
        &dir,
        "mev/planning/MV.10.C-final",
        "sdlc-task-state.json",
        "2026-07-10T00:00:00Z",
        "done",
    );

    let export = block_graph_brain(&dir, &default_scope()).expect("block graph export must build");
    let node = export
        .nodes
        .iter()
        .find(|n| n.key == "mev:MV.10.C")
        .expect("block must resolve");

    assert_eq!(
        node.reconcile_failed,
        Some(false),
        "the newer file's status (done) must win, not the older file's reconcile_failed"
    );
}
