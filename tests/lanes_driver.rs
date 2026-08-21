//! Integration tests for `mev::lanes_brain` — the `mev lanes` driver seam.
//!
//! The pieces this driver assembles (`compute_frontier`, `segment_statuses_with_slots`,
//! `lane_leverage`, `discover_live_runs`) each carry their own unit tests in
//! `src/brain/availability.rs`. What has no coverage below this file is the **assembly**:
//! config -> state discovery -> lane files -> block graph -> frontier -> statuses ->
//! leverage -> artifact. That seam holds one load-bearing line — the block-graph scope is
//! built with `max_nodes: usize::MAX` and passed through `ensure_untruncated` — which is
//! `MV.13.B`'s whole point: a frontier computed over a truncated graph silently drops gates
//! and looks correct while being wrong. A regression there would pass every unit test in
//! the module.
//!
//! Tests:
//!   1. A two-segment lane over a real on-disk corpus produces one status per segment, with
//!      a live head reported `startable` and an all-closed segment reported `done`.
//!   2. The artifact carries a parseable `derived_at` and `degraded: false` when no fleet
//!      lock store exists.

use std::fs;
use std::path::Path;

/// Minimal `brain.toml` registering the two leaf repos the fixture lane file spans.
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

[[repos]]
slug = "beta"
tier = "primary"
repo_path = "repos/beta"
status_file = "repos/beta/planning/status.md"
cache_doc = "docs/projects/beta.md"
heading = "Beta"
"#;
    fs::write(root.join("brain.toml"), toml.as_bytes()).unwrap();
}

fn write_file(root: &Path, rel: &str, content: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content.as_bytes()).unwrap();
}

fn write_json(root: &Path, rel: &str, value: &serde_json::Value) {
    write_file(root, rel, &serde_json::to_string_pretty(value).unwrap());
}

/// One HQ brain file plus two leaf repos. `alpha` owns a finished segment (both blocks
/// closed); `beta` owns a live one whose head has no unmet dependency of any kind.
fn write_corpus(root: &Path) {
    write_brain_toml(root);

    write_json(
        root,
        "planning/state.json",
        &serde_json::json!({
            "repo": "hq",
            "kind": "brain",
            "updated": "2026-08-17",
            "focus": { "now": [], "next": [], "blocked": [] },
            "repos": [],
            "cross_repo": []
        }),
    );

    write_json(
        root,
        "repos/alpha/planning/state.json",
        &serde_json::json!({
            "repo": "alpha",
            "kind": "project",
            "updated": "2026-08-17",
            "focus": { "now": [], "next": [], "blocked": [] },
            "repos": [],
            "cross_repo": [],
            "tracks": [{
                "title": "Phase 1",
                "blocks": [
                    { "id": "AL.1.A", "title": "First", "status": "closed" },
                    { "id": "AL.1.B", "title": "Second", "status": "closed" }
                ]
            }]
        }),
    );

    write_json(
        root,
        "repos/beta/planning/state.json",
        &serde_json::json!({
            "repo": "beta",
            "kind": "project",
            "updated": "2026-08-17",
            "focus": { "now": [], "next": [], "blocked": [] },
            "repos": [],
            "cross_repo": [],
            "tracks": [{
                "title": "Phase 1",
                "blocks": [
                    { "id": "BE.1.A", "title": "Live head", "status": "open" }
                ]
            }]
        }),
    );

    // Ownership changes between AL.1.B and BE.1.A, so this is two segments, not one.
    write_json(
        root,
        "planning/roadmaps/fixture/lane-only.json",
        &serde_json::json!({
            "lane": "only",
            "roadmap": "fixture",
            "blocks": [
                { "id": "AL.1.A", "origin_roadmap": "fixture", "repo": "alpha" },
                { "id": "AL.1.B", "origin_roadmap": "fixture", "repo": "alpha" },
                { "id": "BE.1.A", "origin_roadmap": "fixture", "repo": "beta" }
            ]
        }),
    );
}

#[test]
fn lanes_brain_reports_one_status_per_segment_including_the_finished_one() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write_corpus(root);

    let artifact = mev::lanes_brain(root).expect("driver runs over a well-formed corpus");

    let mut seen: Vec<(String, usize, String, Option<String>)> = artifact
        .segments
        .iter()
        .map(|s| {
            (
                s.status.repo.clone(),
                s.status.segment,
                format!("{:?}", s.status.availability),
                s.status.head.clone(),
            )
        })
        .collect();
    seen.sort();

    assert_eq!(
        seen.len(),
        2,
        "one status per segment, finished ones included — got {seen:?}"
    );

    let alpha = seen
        .iter()
        .find(|(repo, ..)| repo == "alpha")
        .expect("the all-closed alpha segment must still appear");
    assert_eq!(
        alpha.2, "Done",
        "a segment whose blocks are all closed reports Done, never absence"
    );
    assert!(
        alpha.3.is_none(),
        "a Done segment has no live head: {:?}",
        alpha.3
    );

    let beta = seen
        .iter()
        .find(|(repo, ..)| repo == "beta")
        .expect("the live beta segment must appear");
    assert_eq!(beta.2, "Startable");
    assert_eq!(beta.3.as_deref(), Some("beta:BE.1.A"));
}

#[test]
fn lanes_brain_stamps_a_parseable_derived_at() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write_corpus(root);

    let artifact = mev::lanes_brain(root).expect("driver runs over a well-formed corpus");

    assert!(
        chrono::DateTime::parse_from_rfc3339(&artifact.derived_at).is_ok(),
        "derived_at must be RFC 3339 so a consumer can tell how stale the answer is: {}",
        artifact.derived_at
    );
}

/// Pins the degraded contract in both directions.
///
/// An **absent** `.fleet-locks` directory currently reports `degraded: true` — "we could not
/// tell", not "there is capacity". That is what `MV.13.C` task 3 specified, and the honest
/// direction of the two: `degraded` never causes a hold, so over-reporting it costs a caller
/// nothing but under-reporting it would let `held-slot` read as authoritative when it was
/// never computed. Worth revisiting whether *absent* should be distinguished from
/// *unreadable* — the registry only exists once some lane has registered, so absence is
/// arguably knowable rather than unknown — but that is a behaviour change, not a test fix,
/// and it is recorded as an open item rather than made here.
#[test]
fn lanes_brain_reports_degraded_only_when_the_lock_store_is_unreadable() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write_corpus(root);

    let absent = mev::lanes_brain(root).expect("driver runs over a well-formed corpus");
    assert!(
        absent.degraded,
        "an absent .fleet-locks store reports degraded — 'could not tell', never 'has capacity'"
    );

    fs::create_dir_all(root.join(".fleet-locks")).unwrap();
    let present = mev::lanes_brain(root).expect("driver runs over a well-formed corpus");
    assert!(
        !present.degraded,
        "a readable (if empty) lock store is a real answer, not a degraded one"
    );
    assert!(
        present
            .segments
            .iter()
            .all(|s| format!("{:?}", s.status.availability) != "HeldSlot"),
        "an empty lock store holds nobody on a slot"
    );
}
