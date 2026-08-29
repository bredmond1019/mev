//! Integration tests for `mev::blocks_brain` — the `mev blocks` driver seam
//! (`MV.ticket.query-verb-leverage-chain-and-filters`, Task 3).
//!
//! `brain::query::select`/`block_cone`/`same_repo_chain` each carry their own fixture
//! tests in `src/brain/query.rs` over hand-built `BlockInfo`/`DepEdge` graphs. What has
//! no coverage below this file is the **assembly**: config -> state discovery -> the
//! real `StateGraph` -> `BlockInfo`/`DepEdge` adaptation -> the filter/derivation calls.
//! That seam is exactly where defect 2 in the block record's `why` lives — a `--repo`
//! flag that is *accepted* but silently ignored, so this file's first test is the
//! direct regression guard: `--repo` alone must narrow the result set against a real
//! on-disk corpus, not just against a hand-built `Vec<BlockInfo>`.

use std::fs;
use std::path::Path;

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

/// Two repos, `alpha` (3 blocks) and `beta` (2 blocks), one cross-repo-shaped chain
/// (`AL.1.A` blocked by nothing, `AL.1.B` blocked by `AL.1.A`, `AL.1.C` blocked by
/// `AL.1.B`) so `alpha` alone has a startable head plus a same-repo chain to walk, and
/// `beta` has an independent startable head at a different priority.
fn write_corpus(root: &Path) {
    write_brain_toml(root);

    write_json(
        root,
        "planning/state.json",
        &serde_json::json!({
            "repo": "hq",
            "kind": "brain",
            "updated": "2026-08-29",
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
            "updated": "2026-08-29",
            "focus": { "now": [], "next": [], "blocked": [] },
            "repos": [],
            "cross_repo": [],
            "tracks": [{
                "title": "Phase 1",
                "blocks": [
                    { "id": "AL.1.A", "title": "Head", "status": "open", "priority": 1 },
                    {
                        "id": "AL.1.B", "title": "Second", "status": "open", "priority": 1,
                        "depends_on": [{"type": "block", "repo": "alpha", "id": "AL.1.A"}]
                    },
                    {
                        "id": "AL.1.C", "title": "Third", "status": "open", "priority": 1,
                        "depends_on": [{"type": "block", "repo": "alpha", "id": "AL.1.B"}]
                    }
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
            "updated": "2026-08-29",
            "focus": { "now": [], "next": [], "blocked": [] },
            "repos": [],
            "cross_repo": [],
            "tracks": [{
                "title": "Phase 1",
                "blocks": [
                    { "id": "BE.1.A", "title": "Live head", "status": "open", "priority": 2 }
                ]
            }]
        }),
    );
}

#[test]
fn repo_filter_narrows_the_result_set_on_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write_corpus(root);

    let unfiltered = mev::blocks_brain(root, &mev::BlockQuery::default(), false, false)
        .expect("driver runs over a well-formed corpus");
    assert_eq!(
        unfiltered.blocks.len(),
        4,
        "3 alpha + 1 beta block; got {:?}",
        unfiltered.blocks
    );

    let query = mev::BlockQuery {
        repo: Some("alpha".to_string()),
        ..Default::default()
    };
    let filtered = mev::blocks_brain(root, &query, false, false)
        .expect("driver runs over a well-formed corpus");

    assert!(
        filtered.blocks.len() < unfiltered.blocks.len(),
        "--repo alpha must narrow the result set on its own, with no second flag \
         required — the direct regression guard for the emit-block-graph silent-ignore \
         defect (block record `why` #2); unfiltered={:?} filtered={:?}",
        unfiltered.blocks,
        filtered.blocks
    );
    assert_eq!(filtered.blocks.len(), 3, "all three alpha blocks, no more");
    assert!(filtered.blocks.iter().all(|k| k.starts_with("alpha:")));
}

#[test]
fn repo_startable_and_max_priority_compose() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write_corpus(root);

    // beta:BE.1.A has priority 2 and is startable; excluded by max_priority=1.
    // alpha:AL.1.A has priority 1 and is startable; alpha:AL.1.B/C are blocked.
    let query = mev::BlockQuery {
        startable: Some(true),
        max_priority: Some(1),
        ..Default::default()
    };
    let result = mev::blocks_brain(root, &query, false, false)
        .expect("driver runs over a well-formed corpus");

    assert_eq!(
        result.blocks,
        vec!["alpha:AL.1.A".to_string()],
        "repo-agnostic filters (startable AND max_priority) must both hold at once; \
         got {:?}",
        result.blocks
    );

    // Adding --repo beta on top must now exclude alpha:AL.1.A too — three filters
    // composing together, not just two.
    let query = mev::BlockQuery {
        repo: Some("beta".to_string()),
        startable: Some(true),
        max_priority: Some(1),
        ..Default::default()
    };
    let result = mev::blocks_brain(root, &query, false, false)
        .expect("driver runs over a well-formed corpus");
    assert!(
        result.blocks.is_empty(),
        "repo AND startable AND max_priority together must exclude beta:BE.1.A \
         (priority 2 > max_priority 1); got {:?}",
        result.blocks
    );
}

#[test]
fn leverage_reports_the_live_cone_for_the_startable_alpha_head() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write_corpus(root);

    let query = mev::BlockQuery {
        repo: Some("alpha".to_string()),
        startable: Some(true),
        ..Default::default()
    };
    let result = mev::blocks_brain(root, &query, true, false)
        .expect("driver runs over a well-formed corpus");

    assert_eq!(result.blocks, vec!["alpha:AL.1.A".to_string()]);
    let cone = result
        .cones
        .get("alpha:AL.1.A")
        .expect("the only selected block is startable, so it must carry a cone");
    assert_eq!(
        cone.live_count(),
        2,
        "AL.1.A's transitive cone is AL.1.B and AL.1.C, both live; got {cone:?}"
    );
    assert!(result.chains.is_empty(), "chains not requested");
}

#[test]
fn chain_reports_the_same_repo_run_from_the_startable_alpha_head() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write_corpus(root);

    let query = mev::BlockQuery {
        repo: Some("alpha".to_string()),
        startable: Some(true),
        ..Default::default()
    };
    let result = mev::blocks_brain(root, &query, false, true)
        .expect("driver runs over a well-formed corpus");

    let chain = result
        .chains
        .get("alpha:AL.1.A")
        .expect("the only selected block is startable, so it must carry a chain");
    assert_eq!(
        chain,
        &vec![
            "alpha:AL.1.A".to_string(),
            "alpha:AL.1.B".to_string(),
            "alpha:AL.1.C".to_string()
        ],
        "the same-repo run from AL.1.A must walk all three alpha blocks in order; \
         got {chain:?}"
    );
    assert!(result.cones.is_empty(), "cones not requested");
}
