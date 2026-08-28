//! Integration tests for `mev carryover --would-block` over a temp-dir corpus fixture —
//! `MV.16.A`, Task 5.
//!
//! Drives the same public building blocks `src/main.rs`'s `run_carryover_would_block`
//! composes (discovery -> load -> `evaluate_carryover` -> `build_lane_residency_index` ->
//! `compute_would_block_report` -> the two renderers), since the CLI driver itself lives
//! in the `mev` binary and is not reachable from an integration test crate.
//!
//! Cases (mirroring the task's counted/not-counted matrix):
//!   - an edge onto an `open`, lane-resident target -> blocking, lane-resident
//!   - an edge with an empty `repo` resolving against the owning entry's own repo, onto an
//!     `open` target in NO lane -> blocking, not lane-resident
//!   - an edge onto a `closed` target -> not counted
//!   - an edge onto a `wontfix` target -> not counted
//!   - an edge onto an unresolvable target -> reported `unresolvable`, not counted
//!   - one `external`, one `operator`, one `approval` edge -> each its own row, none
//!     counted
//!   - `--repo` composition: filtering to one repo narrows the rows to that repo's entries
//!   - no-write proof: every file under the fixture corpus (every `state.json`, every
//!     `carryover-archive.jsonl`, and the lane record) is byte-identical before and after
//!     a full compute+render run
//!   - the blocking-count matrix from the task spec, made mechanical: 1 -> 0 -> 2

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use mev::brain::carryover::{
    BlockedByEdgeType, EdgeBlockVerdict, LaneResidencyIndex, build_lane_residency_index,
    compute_would_block_report, render_would_block_json, render_would_block_table,
};
use mev::brain::config::find_brain_config;
use mev::brain::state::{StateSource, discover_state_files, load_state};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn temp_dir(suffix: &str) -> PathBuf {
    let dir = mev::testsupport::unique_temp_dir(&format!("mev-carryover-would-block-it-{suffix}"));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_json(root: &Path, rel: &str, value: &serde_json::Value) {
    let full = root.join(rel);
    fs::create_dir_all(full.parent().unwrap()).unwrap();
    fs::write(
        &full,
        serde_json::to_string_pretty(value).unwrap().as_bytes(),
    )
    .unwrap();
}

fn write_raw(root: &Path, rel: &str, content: &str) {
    let full = root.join(rel);
    fs::create_dir_all(full.parent().unwrap()).unwrap();
    fs::write(&full, content.as_bytes()).unwrap();
}

/// Write `brain.toml` registering every slug in `repos` as a leaf `[[repos]]` entry —
/// mirrors `tests/brain_carryover_dispose.rs`'s helper of the same shape.
fn write_brain_toml(root: &Path, repos: &[&str]) {
    let mut toml = String::from(
        r#"[vocab]
layer = ["brain", "engine", "factory", "console", "surface", "infra", "business", "content", "meta"]
status = ["active", "draft", "deprecated", "superseded", "archived"]

[crawl]
skip_dirs = ["target", "node_modules", ".git"]

"#,
    );
    for slug in repos {
        toml.push_str(&format!(
            r#"[[repos]]
slug = "{slug}"
tier = "primary"
repo_path = "repos/{slug}"
status_file = "repos/{slug}/planning/status.md"
cache_doc = "docs/projects/{slug}.md"
heading = "{slug}"

"#
        ));
    }
    fs::write(root.join("brain.toml"), toml.as_bytes()).unwrap();
}

/// `mev`'s state: one open, lane-resident target (`MV.1.A`) plus the block-status rows
/// every fixture entry below resolves against (`MV.2.B` closed).
fn mev_state_value() -> serde_json::Value {
    serde_json::json!({
        "repo": "mev",
        "kind": "project",
        "updated": "2026-08-24",
        "focus": { "now": [], "next": [], "blocked": [] },
        "tracks": [
            {
                "title": "Phase 1",
                "blocks": [
                    { "id": "MV.1.A", "title": "mev block A", "status": "open" },
                    { "id": "MV.2.B", "title": "mev block B", "status": "closed" }
                ]
            }
        ],
        "carryover": [
            {
                "slug": "onto-open-lane-resident",
                "scope": { "repo": "mev" },
                "kind": "deferred",
                "text": "Waiting on other's block A landing.",
                "blocks": [
                    { "type": "block", "repo": "other", "id": "OT.1.A" }
                ],
                "clears_when": "OT.1.A lands",
                "created": "2026-06-01"
            },
            {
                "slug": "onto-open-no-lane-empty-repo",
                "scope": { "repo": "mev" },
                "kind": "deferred",
                "text": "Waiting on mev's own block A landing — authored with an empty repo on the edge.",
                "blocks": [
                    { "type": "block", "repo": "", "id": "MV.1.A" }
                ],
                "clears_when": "MV.1.A lands",
                "created": "2026-06-01"
            },
            {
                "slug": "onto-closed",
                "scope": { "repo": "mev" },
                "kind": "deferred",
                "text": "Waiting on other's block B landing — already closed.",
                "blocks": [
                    { "type": "block", "repo": "other", "id": "OT.2.A" }
                ],
                "clears_when": "OT.2.A lands",
                "created": "2026-06-01"
            },
            {
                "slug": "onto-wontfix",
                "scope": { "repo": "mev" },
                "kind": "deferred",
                "text": "Waiting on other's block C — marked wontfix.",
                "blocks": [
                    { "type": "block", "repo": "other", "id": "OT.3.A" }
                ],
                "clears_when": "OT.3.A lands",
                "created": "2026-06-01"
            },
            {
                "slug": "onto-unresolvable",
                "scope": { "repo": "mev" },
                "kind": "deferred",
                "text": "Waiting on a block that does not exist anywhere in the corpus.",
                "blocks": [
                    { "type": "block", "repo": "other", "id": "OT.99.Z" }
                ],
                "clears_when": "OT.99.Z lands",
                "created": "2026-06-01"
            },
            {
                "slug": "external-edge",
                "scope": { "repo": "mev" },
                "kind": "deferred",
                "text": "Waiting on a vendor API.",
                "blocks": [
                    { "type": "external", "what": "vendor API access" }
                ],
                "clears_when": "vendor grants access",
                "created": "2026-06-01"
            },
            {
                "slug": "operator-edge",
                "scope": { "repo": "mev" },
                "kind": "deferred",
                "text": "Needs an operator session.",
                "blocks": [
                    {
                        "type": "operator",
                        "slug": "some-session",
                        "exit": "planning/some-session/exit.md",
                        "start": "/begin-session some-session"
                    }
                ],
                "clears_when": "the operator session closes",
                "created": "2026-06-01"
            },
            {
                "slug": "approval-edge",
                "scope": { "repo": "mev" },
                "kind": "deferred",
                "text": "Needs an approval.",
                "blocks": [
                    {
                        "type": "approval",
                        "slug": "some-approval",
                        "what": "ship the thing",
                        "digest": "deadbeef"
                    }
                ],
                "clears_when": "the approval lands",
                "created": "2026-06-01"
            }
        ]
    })
}

fn other_state_value() -> serde_json::Value {
    serde_json::json!({
        "repo": "other",
        "kind": "project",
        "updated": "2026-08-24",
        "focus": { "now": [], "next": [], "blocked": [] },
        "tracks": [
            {
                "title": "Phase 1",
                "blocks": [
                    { "id": "OT.1.A", "title": "other block A", "status": "open" },
                    { "id": "OT.2.A", "title": "other block B", "status": "closed" },
                    { "id": "OT.3.A", "title": "other block C", "status": "wontfix" }
                ]
            }
        ],
        "carryover": []
    })
}

fn lane_json(lane: &str, roadmap: &str, blocks: &[(&str, &str)]) -> String {
    // blocks: (repo, id)
    let blocks_json: Vec<String> = blocks
        .iter()
        .map(|(repo, id)| {
            format!(r#"{{"id":"{id}","origin_roadmap":"{roadmap}","repo":"{repo}"}}"#)
        })
        .collect();
    format!(
        r#"{{"lane":"{lane}","roadmap":"{roadmap}","blocks":[{}]}}"#,
        blocks_json.join(",")
    )
}

fn write_fixture(dir: &Path) {
    write_brain_toml(dir, &["mev", "other"]);
    write_json(dir, "repos/mev/planning/state.json", &mev_state_value());
    write_json(dir, "repos/other/planning/state.json", &other_state_value());
    // Only OT.1.A is lane-resident; MV.1.A deliberately sits in NO lane record, so the
    // empty-repo edge exercises the "open target in no lane" branch.
    write_raw(
        dir,
        "planning/roadmaps/alpha/lane-substrate.json",
        &lane_json("substrate", "alpha", &[("other", "OT.1.A")]),
    );
}

/// Replicates `src/main.rs`'s `run_carryover_would_block` pipeline using only the public
/// API surface available to an integration test: discover -> load -> evaluate ->
/// lane-residency index -> compute the report. Returns the report plus the loaded
/// entries, for assertions that want to inspect the corpus alongside the report.
fn run_would_block_pipeline(
    root: &Path,
    repo_filter: Option<&str>,
) -> mev::brain::carryover::WouldBlockReport {
    let config = find_brain_config(root).expect("brain.toml should load");
    let (sources, _diags) = discover_state_files(root, &config);

    let mut loaded: Vec<(StateSource, mev::brain::state::StateFile)> = Vec::new();
    for src in &sources {
        let file = load_state(&src.abs_path).expect("fixture state.json should parse");
        loaded.push((src.clone(), file));
    }

    let mut status_map: HashMap<String, Option<String>> = HashMap::new();
    let mut repo_paths: HashMap<String, PathBuf> = HashMap::new();
    for (src, file) in &loaded {
        for track in &file.tracks {
            for block in &track.blocks {
                let key = format!("{}:{}", src.repo_slug, block.id);
                status_map.insert(key, block.status.clone());
            }
        }
        repo_paths.insert(
            src.repo_slug.clone(),
            root.join("repos").join(&src.repo_slug),
        );
    }

    let report = mev::evaluate_carryover(
        &loaded,
        &status_map,
        root,
        &repo_paths,
        "2026-08-24",
        &config.attention,
        repo_filter,
        false,
        mev::COMMAND_EXEC_TIMEOUT,
    );

    let (lane_index, lane_diags): (LaneResidencyIndex, _) = build_lane_residency_index(root);
    assert!(
        lane_diags.is_empty(),
        "expected no lane diagnostics, got {lane_diags:?}"
    );

    compute_would_block_report(&report.entries, &status_map, &lane_index)
}

/// Snapshot every regular file under `root`, keyed by its path relative to `root`, so a
/// later snapshot can be compared for byte-identity. This is what actually proves
/// "writes nothing" — not the absence of a write flag, but the bytes themselves.
fn snapshot_files(root: &Path) -> HashMap<PathBuf, Vec<u8>> {
    let mut out = HashMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let rel = path.strip_prefix(root).unwrap().to_path_buf();
                out.insert(rel, fs::read(&path).unwrap());
            }
        }
    }
    out
}

fn row_by_owner<'a>(
    report: &'a mev::brain::carryover::WouldBlockReport,
    owner: &str,
) -> &'a mev::brain::carryover::WouldBlockRow {
    report
        .rows
        .iter()
        .find(|r| r.owner == owner)
        .unwrap_or_else(|| panic!("expected a row for owner {owner}, got {:#?}", report.rows))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// The full counted/not-counted matrix, in one fixture: open+lane-resident blocks;
/// empty-repo-onto-own-open-not-in-any-lane blocks (not lane-resident); closed, wontfix,
/// and unresolvable targets are all reported but excluded from the blocking count;
/// external/operator/approval edges are each their own row with no node target.
#[test]
fn would_block_report_covers_the_full_counted_not_counted_matrix() {
    let dir = temp_dir("matrix");
    write_fixture(&dir);

    let report = run_would_block_pipeline(&dir, None);

    assert_eq!(report.summary.total_edges, 8);
    assert_eq!(report.summary.blocking, 2, "the two open targets");
    assert_eq!(report.summary.closed, 1);
    assert_eq!(report.summary.wontfix, 1);
    assert_eq!(report.summary.unresolvable, 1);
    assert_eq!(
        report.summary.no_node_target, 3,
        "external + operator + approval"
    );

    let lane_resident_row = row_by_owner(&report, "mev:onto-open-lane-resident");
    assert_eq!(lane_resident_row.verdict, EdgeBlockVerdict::Blocking);
    assert!(lane_resident_row.lane_resident);
    assert_eq!(
        lane_resident_row.lanes,
        vec!["alpha/lane-substrate.json".to_string()]
    );

    let empty_repo_row = row_by_owner(&report, "mev:onto-open-no-lane-empty-repo");
    assert_eq!(empty_repo_row.target_key.as_deref(), Some("mev:MV.1.A"));
    assert_eq!(empty_repo_row.verdict, EdgeBlockVerdict::Blocking);
    assert!(
        !empty_repo_row.lane_resident,
        "MV.1.A sits in no lane record"
    );

    let closed_row = row_by_owner(&report, "mev:onto-closed");
    assert_eq!(closed_row.verdict, EdgeBlockVerdict::Closed);

    let wontfix_row = row_by_owner(&report, "mev:onto-wontfix");
    assert_eq!(wontfix_row.verdict, EdgeBlockVerdict::Wontfix);

    let unresolvable_row = row_by_owner(&report, "mev:onto-unresolvable");
    assert_eq!(unresolvable_row.verdict, EdgeBlockVerdict::Unresolvable);
    assert_eq!(
        unresolvable_row.target_key.as_deref(),
        Some("other:OT.99.Z")
    );

    let external_row = row_by_owner(&report, "mev:external-edge");
    assert_eq!(external_row.edge_type, BlockedByEdgeType::External);
    assert_eq!(external_row.verdict, EdgeBlockVerdict::NoNodeTarget);
    assert_eq!(external_row.target_key, None);

    let operator_row = row_by_owner(&report, "mev:operator-edge");
    assert_eq!(operator_row.edge_type, BlockedByEdgeType::Operator);
    assert_eq!(operator_row.verdict, EdgeBlockVerdict::NoNodeTarget);
    assert_eq!(operator_row.target_key, None);

    let approval_row = row_by_owner(&report, "mev:approval-edge");
    assert_eq!(approval_row.edge_type, BlockedByEdgeType::Approval);
    assert_eq!(approval_row.verdict, EdgeBlockVerdict::NoNodeTarget);
    assert_eq!(approval_row.target_key, None);

    let _ = fs::remove_dir_all(&dir);
}

/// `--would-block --repo <slug>` narrows the rows to that repo's entries — composes with
/// the existing `--repo` filter exactly as `--json` does.
#[test]
fn would_block_report_composes_with_repo_filter() {
    let dir = temp_dir("repo-filter");
    write_fixture(&dir);

    let report = run_would_block_pipeline(&dir, Some("other"));

    assert!(
        report.rows.is_empty(),
        "other authors no carryover[] entries of its own, so filtering to it should yield \
         no rows, got {:#?}",
        report.rows
    );

    let mev_only = run_would_block_pipeline(&dir, Some("mev"));
    assert_eq!(mev_only.summary.total_edges, 8);

    let _ = fs::remove_dir_all(&dir);
}

/// No-write proof: every file under the fixture corpus — every `state.json`, the lane
/// record, and `brain.toml` — is byte-identical before and after a full compute+render
/// run, including a run that also serializes both the human table and the JSON output.
/// `carryover-archive.jsonl` must not be created at all (`--would-block` never disposes).
#[test]
fn would_block_run_writes_nothing_bytes_identical_before_and_after() {
    let dir = temp_dir("no-write");
    write_fixture(&dir);

    let before = snapshot_files(&dir);

    let report = run_would_block_pipeline(&dir, None);
    let _ = render_would_block_table(&report);
    let _ = render_would_block_json(&report).unwrap();

    let after = snapshot_files(&dir);

    assert_eq!(
        before, after,
        "--would-block must never modify any file in the corpus"
    );
    assert!(
        !dir.join("repos/mev/planning/carryover-archive.jsonl")
            .exists(),
        "--would-block must never create a disposal archive"
    );
    assert!(
        !dir.join("repos/other/planning/carryover-archive.jsonl")
            .exists()
    );

    let _ = fs::remove_dir_all(&dir);
}

/// The task's "shown failing" signal, made mechanical: a fixture corpus with one edge
/// onto an open, lane-resident target has a blocking count of 1; removing that edge
/// drops it to 0; adding a second such edge raises it to 2.
#[test]
fn would_block_blocking_count_matrix_one_zero_two() {
    let dir = temp_dir("count-matrix");
    write_brain_toml(&dir, &["mev", "other"]);
    write_json(
        &dir,
        "repos/other/planning/state.json",
        &serde_json::json!({
            "repo": "other",
            "kind": "project",
            "updated": "2026-08-24",
            "focus": { "now": [], "next": [], "blocked": [] },
            "tracks": [{
                "title": "Phase 1",
                "blocks": [{ "id": "OT.1.A", "title": "other block A", "status": "open" }]
            }],
            "carryover": []
        }),
    );
    write_raw(
        &dir,
        "planning/roadmaps/alpha/lane-substrate.json",
        &lane_json("substrate", "alpha", &[("other", "OT.1.A")]),
    );

    // one edge -> blocking count 1
    write_json(
        &dir,
        "repos/mev/planning/state.json",
        &serde_json::json!({
            "repo": "mev",
            "kind": "project",
            "updated": "2026-08-24",
            "focus": { "now": [], "next": [], "blocked": [] },
            "tracks": [{ "title": "Phase 1", "blocks": [] }],
            "carryover": [{
                "slug": "one-edge",
                "scope": { "repo": "mev" },
                "kind": "deferred",
                "text": "One edge onto other's block A.",
                "blocks": [{ "type": "block", "repo": "other", "id": "OT.1.A" }],
                "clears_when": "OT.1.A lands",
                "created": "2026-06-01"
            }]
        }),
    );
    let report_one = run_would_block_pipeline(&dir, None);
    assert_eq!(report_one.summary.blocking, 1);

    // zero edges -> blocking count 0
    write_json(
        &dir,
        "repos/mev/planning/state.json",
        &serde_json::json!({
            "repo": "mev",
            "kind": "project",
            "updated": "2026-08-24",
            "focus": { "now": [], "next": [], "blocked": [] },
            "tracks": [{ "title": "Phase 1", "blocks": [] }],
            "carryover": []
        }),
    );
    let report_zero = run_would_block_pipeline(&dir, None);
    assert_eq!(report_zero.summary.blocking, 0);

    // two edges (two distinct entries, same target) -> blocking count 2
    write_json(
        &dir,
        "repos/mev/planning/state.json",
        &serde_json::json!({
            "repo": "mev",
            "kind": "project",
            "updated": "2026-08-24",
            "focus": { "now": [], "next": [], "blocked": [] },
            "tracks": [{ "title": "Phase 1", "blocks": [] }],
            "carryover": [
                {
                    "slug": "edge-one",
                    "scope": { "repo": "mev" },
                    "kind": "deferred",
                    "text": "First edge onto other's block A.",
                    "blocks": [{ "type": "block", "repo": "other", "id": "OT.1.A" }],
                    "clears_when": "OT.1.A lands",
                    "created": "2026-06-01"
                },
                {
                    "slug": "edge-two",
                    "scope": { "repo": "mev" },
                    "kind": "deferred",
                    "text": "Second edge onto other's block A.",
                    "blocks": [{ "type": "block", "repo": "other", "id": "OT.1.A" }],
                    "clears_when": "OT.1.A lands",
                    "created": "2026-06-01"
                }
            ]
        }),
    );
    let report_two = run_would_block_pipeline(&dir, None);
    assert_eq!(report_two.summary.blocking, 2);

    let _ = fs::remove_dir_all(&dir);
}
