//! Integration tests for `MV.ticket.carryover-gating-reaches-derived-surfaces` task 2 —
//! wiring the `carryover_gating_from_config` builder (task 1) into the emit-state
//! writer (`plan_state_json`, `derive_rollup`, `derive_brain_focus`) and the
//! validate-brain focus-drift checker (`check_focus_drift`), so writer and checker can
//! never disagree at a commit boundary.
//!
//! Fixture: repos `mev` and `other`. `other` has open `OT.1.A` (lane-resident) and open
//! `OT.2.A` (in no lane). `mev` carries carryover `gate-open-lane-resident` with a
//! `blocks[]` edge onto `other:OT.1.A` and `gate-no-lane` onto `other:OT.2.A`. Every
//! "held" assertion below is a runtime inversion: the same fixture, `enforce_blocks`
//! flipped, and the assertion flips with it — so no committed-red test is needed and a
//! regression that silently stops gating is caught in both directions.
//!
//! Never touches the live corpus — every fixture is a fresh temp dir via
//! `mev::testsupport::unique_temp_dir`, matching `tests/it/brain_carryover_enforcement.rs`.

use std::fs;
use std::path::{Path, PathBuf};

use mev::brain::carryover::carryover_gating_from_config;
use mev::brain::config::{BrainConfig, find_brain_config};
use mev::brain::emit::plan_state_json;
use mev::brain::state::{
    StateFile, StateGraph, StateSource, TierScope, check_focus_drift, derive_brain_focus,
    derive_focus, derive_rollup, discover_state_files, load_state,
};

// ---------------------------------------------------------------------------
// Fixture helpers (mirrors tests/it/brain_carryover_enforcement.rs's shapes)
// ---------------------------------------------------------------------------

fn temp_dir(suffix: &str) -> PathBuf {
    let dir =
        mev::testsupport::unique_temp_dir(&format!("mev-carryover-enforcement-wiring-it-{suffix}"));
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

fn write_brain_toml(root: &Path, repos: &[&str], enforce_blocks: bool, max_gates_per_repo: usize) {
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
    toml.push_str(&format!(
        r#"[carryover]
enforce_blocks = {enforce_blocks}
max_gates_per_repo = {max_gates_per_repo}
"#
    ));
    fs::write(root.join("brain.toml"), toml.as_bytes()).unwrap();
}

fn lane_json(lane: &str, roadmap: &str, blocks: &[(&str, &str)]) -> String {
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

fn block(id: &str, status: Option<&str>) -> serde_json::Value {
    let mut v = serde_json::json!({ "id": id, "title": format!("block {id}") });
    if let Some(s) = status {
        v["status"] = serde_json::json!(s);
    }
    v
}

fn block_edge(repo: &str, id: &str) -> serde_json::Value {
    serde_json::json!({ "type": "block", "repo": repo, "id": id })
}

fn carryover_entry(
    slug: &str,
    blocks: &[serde_json::Value],
    enforce: Option<bool>,
) -> serde_json::Value {
    let mut v = serde_json::json!({
        "slug": slug,
        "scope": { "repo": "mev" },
        "kind": "deferred",
        "text": format!("fixture entry {slug}"),
        "blocks": blocks,
        "clears_when": "fixture never clears",
        "created": "2026-06-01"
    });
    if let Some(e) = enforce {
        v["enforce"] = serde_json::json!(e);
    }
    v
}

fn write_wiring_fixture(dir: &Path, enforce_blocks: bool, max_gates_per_repo: usize) {
    write_brain_toml(dir, &["mev", "other"], enforce_blocks, max_gates_per_repo);

    write_json(
        dir,
        "repos/other/planning/state.json",
        &serde_json::json!({
            "repo": "other",
            "kind": "project",
            "updated": "2026-09-12",
            "focus": { "now": [], "next": [], "blocked": [] },
            "tracks": [{
                "title": "Phase 1",
                "blocks": [
                    block("OT.1.A", None),  // open, lane-resident
                    block("OT.2.A", None),  // open, NO lane record at all
                ]
            }],
            "carryover": []
        }),
    );

    write_json(
        dir,
        "repos/mev/planning/state.json",
        &serde_json::json!({
            "repo": "mev",
            "kind": "project",
            "updated": "2026-09-12",
            "focus": { "now": [], "next": [], "blocked": [] },
            "tracks": [{ "title": "Phase 1", "blocks": [] }],
            "carryover": [
                carryover_entry("gate-open-lane-resident", &[block_edge("other", "OT.1.A")], None),
                carryover_entry("gate-no-lane", &[block_edge("other", "OT.2.A")], None),
            ]
        }),
    );

    // Only other:OT.1.A is lane-resident; other:OT.2.A deliberately sits in NO lane
    // record.
    write_raw(
        dir,
        "planning/roadmaps/wiring-roadmap/lane-substrate.json",
        &lane_json("substrate", "wiring-roadmap", &[("other", "OT.1.A")]),
    );

    // A `planning/blocks/OT.1.A.json` record so the lane-resident head passes lane
    // registration clause 2 — task-3 tests exercise `lanes_brain`, which would
    // otherwise report `HeldUnregistered` (outranking the carryover gate) regardless
    // of `enforce_blocks`, masking the very inversion these tests assert.
    write_json(
        dir,
        "repos/other/planning/blocks/OT.1.A.json",
        &serde_json::json!({ "id": "OT.1.A", "title": "block OT.1.A" }),
    );
}

#[allow(clippy::type_complexity)]
fn load_corpus(root: &Path) -> (Vec<(StateSource, StateFile)>, BrainConfig) {
    let config = find_brain_config(root).expect("brain.toml should load");
    let (sources, _diags) = discover_state_files(root, &config);
    let mut loaded: Vec<(StateSource, StateFile)> = Vec::new();
    for src in &sources {
        let file = load_state(&src.abs_path).expect("fixture state.json should parse");
        loaded.push((src.clone(), file));
    }
    (loaded, config)
}

fn find_action_content<'a>(plan: &'a mev::brain::emit::EmitPlan, repo_slug: &str) -> &'a str {
    plan.actions
        .iter()
        .find(|a| {
            a.path
                .to_string_lossy()
                .contains(&format!("repos/{repo_slug}/planning/state.json"))
        })
        .unwrap_or_else(|| panic!("expected an emit action rewriting {repo_slug}'s state.json"))
        .new_content
        .as_str()
}

fn focus_ids(file: &StateFile, lane: &str) -> Vec<String> {
    let blocks: &[mev::brain::state::Block] = match lane {
        "now" => &file.focus.now,
        "next" => &file.focus.next,
        "blocked" => &file.focus.blocked,
        "deferred" => &file.focus.deferred,
        _ => panic!("unknown lane {lane}"),
    };
    blocks.iter().map(|b| b.id.clone()).collect()
}

// ---------------------------------------------------------------------------
// (a) plan_state_json: enforcement on holds both targets in `blocked`, not `next`;
// flipping the flag off in the same test returns both to `next`.
// ---------------------------------------------------------------------------

#[test]
fn plan_state_json_holds_gated_blocks_and_releases_them_when_enforcement_is_off() {
    let dir = temp_dir("plan-on-off");
    write_wiring_fixture(&dir, true, 10);
    let (loaded, config_on) = load_corpus(&dir);

    let plan_on = plan_state_json(&loaded, &StateGraph::default(), &config_on);
    let other_content_on = find_action_content(&plan_on, "other");
    let other_file_on: StateFile = serde_json::from_str(other_content_on).unwrap();

    assert_eq!(
        focus_ids(&other_file_on, "blocked"),
        vec!["OT.1.A".to_string(), "OT.2.A".to_string()],
        "with enforcement on, both gated targets must be in focus.blocked"
    );
    assert!(
        focus_ids(&other_file_on, "next").is_empty(),
        "with enforcement on, gated targets must not be in focus.next, got {:?}",
        focus_ids(&other_file_on, "next")
    );

    // Flip the flag off in the same test — the assertion inverts.
    write_brain_toml(&dir, &["mev", "other"], false, 10);
    let config_off = find_brain_config(&dir).expect("brain.toml should reload");

    let plan_off = plan_state_json(&loaded, &StateGraph::default(), &config_off);
    let other_content_off = find_action_content(&plan_off, "other");
    let other_file_off: StateFile = serde_json::from_str(other_content_off).unwrap();

    assert!(
        focus_ids(&other_file_off, "blocked").is_empty(),
        "with enforcement off, no target should be held, got {:?}",
        focus_ids(&other_file_off, "blocked")
    );
    assert_eq!(
        focus_ids(&other_file_off, "next"),
        vec!["OT.1.A".to_string(), "OT.2.A".to_string()],
        "with enforcement off, both targets must return to focus.next"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// (b) derive_brain_focus / derive_rollup agree with the leaf focus.
// ---------------------------------------------------------------------------

#[test]
fn derive_rollup_and_derive_brain_focus_agree_with_leaf_focus_when_enforced() {
    let dir = temp_dir("rollup-brain-focus");
    write_wiring_fixture(&dir, true, 10);
    let (loaded, config) = load_corpus(&dir);
    let graph = StateGraph::default();

    // The leaf derivation, directly.
    let (other_src, other_file) = loaded
        .iter()
        .find(|(s, _)| s.repo_slug == "other")
        .expect("other's state.json should be loaded");
    let gating = carryover_gating_from_config(&config, &loaded);
    let leaf = derive_focus(other_src, other_file, &graph, &loaded, Some(&gating));
    assert_eq!(
        leaf.blocked
            .iter()
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>(),
        vec!["OT.1.A".to_string(), "OT.2.A".to_string()]
    );

    // derive_rollup must report the same held blocks for "other".
    let scope = TierScope::Tier("primary".to_string());
    let rollup = derive_rollup(&scope, &config, &[], &graph, &loaded);
    let other_rollup = rollup
        .iter()
        .find(|r| r.repo == "other")
        .expect("expected a rollup entry for other");
    let rollup_blocked: Vec<String> = other_rollup.blocked.iter().map(|b| b.id.clone()).collect();
    assert_eq!(
        rollup_blocked,
        vec!["OT.1.A".to_string(), "OT.2.A".to_string()],
        "derive_rollup must agree with the leaf derivation"
    );
    assert!(
        other_rollup.next.is_empty(),
        "gated targets must not appear in the rollup's next"
    );

    // derive_brain_focus's union must also hold them, tagged with repo "other".
    let (self_src, self_file) = (
        StateSource {
            repo_slug: "hq".to_string(),
            abs_path: dir.join("planning/state.json"),
            expected_kind: "brain",
        },
        StateFile {
            repo: "hq".to_string(),
            kind: "brain".to_string(),
            updated: "2026-09-12".to_string(),
            ..Default::default()
        },
    );
    let brain_focus = derive_brain_focus(&self_src, &self_file, &scope, &config, &graph, &loaded);
    let brain_blocked: Vec<(String, Option<String>)> = brain_focus
        .blocked
        .iter()
        .map(|b| (b.id.clone(), b.repo.clone()))
        .collect();
    assert_eq!(
        brain_blocked,
        vec![
            ("OT.1.A".to_string(), Some("other".to_string())),
            ("OT.2.A".to_string(), Some("other".to_string())),
        ],
        "derive_brain_focus must agree with the leaf derivation, repo-tagged"
    );
    assert!(brain_focus.next.is_empty());

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// (c) check_focus_drift: writer and checker agree in both directions.
// ---------------------------------------------------------------------------

#[test]
fn check_focus_drift_agrees_with_gated_writer_and_flags_ungated_stored_focus() {
    let dir = temp_dir("focus-drift");
    write_wiring_fixture(&dir, true, 10);
    let (loaded, config) = load_corpus(&dir);
    let graph = StateGraph::default();

    let (other_src, other_file) = loaded
        .iter()
        .find(|(s, _)| s.repo_slug == "other")
        .expect("other's state.json should be loaded");

    let gating = carryover_gating_from_config(&config, &loaded);
    let gated = derive_focus(other_src, other_file, &graph, &loaded, Some(&gating));
    let ungated = derive_focus(other_src, other_file, &graph, &loaded, None);

    // Sanity: the two derivations actually differ on this fixture — otherwise the
    // negative case below would prove nothing.
    assert_ne!(
        gated
            .blocked
            .iter()
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>(),
        ungated
            .blocked
            .iter()
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>(),
    );

    let to_focus_blocks = |ids: &[String], status: Option<&str>| -> Vec<mev::brain::state::Block> {
        ids.iter()
            .map(|id| mev::brain::state::Block {
                id: id.clone(),
                title: format!("block {id}"),
                status: status.map(|s| s.to_string()),
                note: None,
                repo: None,
                blocked_by: Vec::new(),
                priority: None,
                due: None,
                epics: Vec::new(),
            })
            .collect()
    };

    // Writer-agreeing case: stored focus == the gated derivation -> no drift.
    let mut file_gated_stored = other_file.clone();
    file_gated_stored.focus.next = to_focus_blocks(&gated.next, None);
    file_gated_stored.focus.blocked = to_focus_blocks(
        &gated
            .blocked
            .iter()
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>(),
        None,
    );
    let diags_in_sync = check_focus_drift(other_src, &file_gated_stored, &config, &graph, &loaded);
    assert!(
        diags_in_sync.is_empty(),
        "gated-writer output must not drift against the gated checker, got {diags_in_sync:?}"
    );

    // Checker-catches case: stored focus == the UNGATED derivation, enforcement on ->
    // drift.
    let mut file_ungated_stored = other_file.clone();
    file_ungated_stored.focus.next = to_focus_blocks(&ungated.next, None);
    file_ungated_stored.focus.blocked = to_focus_blocks(
        &ungated
            .blocked
            .iter()
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>(),
        None,
    );
    let diags_drifted =
        check_focus_drift(other_src, &file_ungated_stored, &config, &graph, &loaded);
    assert!(
        !diags_drifted.is_empty(),
        "ungated stored focus must be reported as drift while enforcement is on"
    );
    assert!(
        diags_drifted
            .iter()
            .any(|d| d.locator == "W_STATE_FOCUS_DRIFT"),
        "expected a W_STATE_FOCUS_DRIFT diagnostic, got {diags_drifted:?}"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// (d) cap 0 and an enforce:false entry each hold nothing in the emit plan.
// ---------------------------------------------------------------------------

#[test]
fn emit_plan_holds_nothing_with_cap_zero_or_enforce_false() {
    // cap = 0.
    let dir_cap0 = temp_dir("cap-zero");
    write_wiring_fixture(&dir_cap0, true, 0);
    let (loaded_cap0, config_cap0) = load_corpus(&dir_cap0);
    let plan_cap0 = plan_state_json(&loaded_cap0, &StateGraph::default(), &config_cap0);
    let other_cap0: StateFile =
        serde_json::from_str(find_action_content(&plan_cap0, "other")).unwrap();
    assert!(
        focus_ids(&other_cap0, "blocked").is_empty(),
        "max_gates_per_repo = 0 must hold nothing, got {:?}",
        focus_ids(&other_cap0, "blocked")
    );
    assert_eq!(
        focus_ids(&other_cap0, "next"),
        vec!["OT.1.A".to_string(), "OT.2.A".to_string()]
    );
    let _ = fs::remove_dir_all(&dir_cap0);

    // enforce: false on the one entry that would otherwise gate OT.1.A.
    let dir_opt_out = temp_dir("enforce-false");
    write_brain_toml(&dir_opt_out, &["mev", "other"], true, 10);
    write_json(
        &dir_opt_out,
        "repos/other/planning/state.json",
        &serde_json::json!({
            "repo": "other",
            "kind": "project",
            "updated": "2026-09-12",
            "focus": { "now": [], "next": [], "blocked": [] },
            "tracks": [{
                "title": "Phase 1",
                "blocks": [block("OT.1.A", None)]
            }],
            "carryover": []
        }),
    );
    write_json(
        &dir_opt_out,
        "repos/mev/planning/state.json",
        &serde_json::json!({
            "repo": "mev",
            "kind": "project",
            "updated": "2026-09-12",
            "focus": { "now": [], "next": [], "blocked": [] },
            "tracks": [{ "title": "Phase 1", "blocks": [] }],
            "carryover": [
                carryover_entry(
                    "gate-opt-out",
                    &[block_edge("other", "OT.1.A")],
                    Some(false),
                ),
            ]
        }),
    );
    let (loaded_opt_out, config_opt_out) = load_corpus(&dir_opt_out);
    let plan_opt_out = plan_state_json(&loaded_opt_out, &StateGraph::default(), &config_opt_out);
    let other_opt_out: StateFile =
        serde_json::from_str(find_action_content(&plan_opt_out, "other")).unwrap();
    assert!(
        focus_ids(&other_opt_out, "blocked").is_empty(),
        "an enforce:false entry must hold nothing, got {:?}",
        focus_ids(&other_opt_out, "blocked")
    );
    assert_eq!(
        focus_ids(&other_opt_out, "next"),
        vec!["OT.1.A".to_string()]
    );

    let _ = fs::remove_dir_all(&dir_opt_out);
}

// ---------------------------------------------------------------------------
// Task 3: the readiness query surfaces — block-graph export, frontier, lanes,
// availability, and blocks. Every "held" assertion below is the same runtime
// inversion as tasks 1/2: one fixture, `enforce_blocks` flipped, assertion flips.
// ---------------------------------------------------------------------------

fn other_ot1a_gate() -> &'static str {
    "carryover:mev:gate-open-lane-resident"
}

#[test]
fn frontier_brain_holds_the_lane_resident_head_when_enforced_and_releases_it_when_off() {
    let dir = temp_dir("frontier-brain-on-off");
    write_wiring_fixture(&dir, true, 10);

    let frontier_on = mev::frontier_brain(&dir).expect("frontier_brain should succeed");
    let entry_on = frontier_on
        .entries
        .iter()
        .find(|e| e.key == "other:OT.1.A")
        .expect("expected a frontier entry for other:OT.1.A");
    assert!(
        !entry_on.startable,
        "with enforcement on, other:OT.1.A must not be startable"
    );
    assert!(
        entry_on.unmet_gates.iter().any(|g| g == other_ot1a_gate()),
        "expected unmet_gates to name {}, got {:?}",
        other_ot1a_gate(),
        entry_on.unmet_gates
    );

    write_brain_toml(&dir, &["mev", "other"], false, 10);
    let frontier_off = mev::frontier_brain(&dir).expect("frontier_brain should succeed");
    let entry_off = frontier_off
        .entries
        .iter()
        .find(|e| e.key == "other:OT.1.A")
        .expect("expected a frontier entry for other:OT.1.A");
    assert!(
        entry_off.startable,
        "with enforcement off, other:OT.1.A must be startable"
    );
    assert!(
        entry_off.unmet_gates.is_empty(),
        "with enforcement off, unmet_gates must be empty, got {:?}",
        entry_off.unmet_gates
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn lanes_brain_does_not_report_the_gated_segment_available_when_enforced() {
    use mev::brain::availability::SegmentAvailability;

    let dir = temp_dir("lanes-brain-on-off");
    write_wiring_fixture(&dir, true, 10);

    let artifact_on = mev::lanes_brain(&dir).expect("lanes_brain should succeed");
    let segment_on = artifact_on
        .segments
        .iter()
        .find(|e| e.status.roadmap == "wiring-roadmap" && e.status.lane == "substrate");
    let startable_on =
        segment_on.is_some_and(|e| e.status.availability == SegmentAvailability::Startable);
    assert!(
        !startable_on,
        "with enforcement on, the OT.1.A segment must not report startable, got {:?}",
        segment_on
    );

    write_brain_toml(&dir, &["mev", "other"], false, 10);
    let artifact_off = mev::lanes_brain(&dir).expect("lanes_brain should succeed");
    let segment_off = artifact_off
        .segments
        .iter()
        .find(|e| e.status.roadmap == "wiring-roadmap" && e.status.lane == "substrate")
        .expect("expected a segment entry with enforcement off");
    assert_eq!(
        segment_off.status.availability,
        SegmentAvailability::Startable,
        "with enforcement off, the segment must report startable, got {:?}",
        segment_off
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn blocks_brain_reports_both_gated_targets_not_startable_when_enforced() {
    let dir = temp_dir("blocks-brain-on-off");
    write_wiring_fixture(&dir, true, 10);

    let query = mev::brain::query::BlockQuery::default();
    let report_on =
        mev::blocks_brain(&dir, &query, false, false, None).expect("blocks_brain should succeed");
    let ot1a_on = report_on
        .blocks
        .iter()
        .find(|r| r.key == "other:OT.1.A")
        .expect("expected a row for other:OT.1.A");
    let ot2a_on = report_on
        .blocks
        .iter()
        .find(|r| r.key == "other:OT.2.A")
        .expect("expected a row for other:OT.2.A");
    assert!(
        !ot1a_on.startable,
        "with enforcement on, other:OT.1.A must not be startable"
    );
    assert!(
        !ot2a_on.startable,
        "with enforcement on, other:OT.2.A (no lane) must not be startable"
    );

    write_brain_toml(&dir, &["mev", "other"], false, 10);
    let report_off =
        mev::blocks_brain(&dir, &query, false, false, None).expect("blocks_brain should succeed");
    let ot1a_off = report_off
        .blocks
        .iter()
        .find(|r| r.key == "other:OT.1.A")
        .expect("expected a row for other:OT.1.A");
    let ot2a_off = report_off
        .blocks
        .iter()
        .find(|r| r.key == "other:OT.2.A")
        .expect("expected a row for other:OT.2.A");
    assert!(
        ot1a_off.startable,
        "with enforcement off, other:OT.1.A must be startable"
    );
    assert!(
        ot2a_off.startable,
        "with enforcement off, other:OT.2.A must be startable"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn build_block_graph_export_does_not_mark_gated_blocks_ready_when_enforced() {
    use mev::brain::block_graph::{BlockGraphScope, build_block_graph_export};
    use mev::brain::state::{TierScope as BgTierScope, build_state_graph};

    let dir = temp_dir("block-graph-export-on-off");
    write_wiring_fixture(&dir, true, 10);
    let (loaded_on, config_on) = load_corpus(&dir);
    let graph_on = build_state_graph(&loaded_on);
    let scope = BlockGraphScope {
        tier: BgTierScope::All,
        epic: None,
        repo: None,
        include_closed: true,
        include_boundary: false,
        max_nodes: usize::MAX,
    };
    let export_on = build_block_graph_export(&dir, &config_on, &graph_on, &loaded_on, &scope);
    let node_ot1a_on = export_on
        .nodes
        .iter()
        .find(|n| n.key == "other:OT.1.A")
        .expect("expected a node for other:OT.1.A");
    let node_ot2a_on = export_on
        .nodes
        .iter()
        .find(|n| n.key == "other:OT.2.A")
        .expect("expected a node for other:OT.2.A");
    assert!(
        !node_ot1a_on.ready,
        "with enforcement on, other:OT.1.A must not be ready"
    );
    assert!(
        !node_ot2a_on.ready,
        "with enforcement on, other:OT.2.A (no lane) must not be ready"
    );

    write_brain_toml(&dir, &["mev", "other"], false, 10);
    let (loaded_off, config_off) = load_corpus(&dir);
    let graph_off = build_state_graph(&loaded_off);
    let export_off = build_block_graph_export(&dir, &config_off, &graph_off, &loaded_off, &scope);
    let node_ot1a_off = export_off
        .nodes
        .iter()
        .find(|n| n.key == "other:OT.1.A")
        .expect("expected a node for other:OT.1.A");
    let node_ot2a_off = export_off
        .nodes
        .iter()
        .find(|n| n.key == "other:OT.2.A")
        .expect("expected a node for other:OT.2.A");
    assert!(
        node_ot1a_off.ready,
        "with enforcement off, other:OT.1.A must be ready"
    );
    assert!(
        node_ot2a_off.ready,
        "with enforcement off, other:OT.2.A must be ready"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// off-equals-absent: [carryover] absent and enforce_blocks = false must produce
// identical serialized output on every wired surface.
// ---------------------------------------------------------------------------

#[test]
fn carryover_absent_and_enforce_blocks_false_produce_equal_output_on_every_surface() {
    use mev::brain::block_graph::{BlockGraphScope, build_block_graph_export};
    use mev::brain::state::{TierScope as BgTierScope, build_state_graph};

    let dir_absent = temp_dir("off-equals-absent-absent");
    write_wiring_fixture(&dir_absent, true, 10);
    // Rewrite brain.toml with NO [carryover] table at all.
    let toml_path = dir_absent.join("brain.toml");
    let toml_on_disk = fs::read_to_string(&toml_path).unwrap();
    let without_carryover_table = toml_on_disk
        .lines()
        .take_while(|l| *l != "[carryover]")
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&toml_path, without_carryover_table.as_bytes()).unwrap();

    let dir_off = temp_dir("off-equals-absent-off");
    write_wiring_fixture(&dir_off, false, 10);

    // frontier_brain
    let frontier_absent = mev::frontier_brain(&dir_absent).unwrap();
    let frontier_off = mev::frontier_brain(&dir_off).unwrap();
    assert_eq!(frontier_absent, frontier_off);

    // lanes_brain, excluding derived_at.
    let lanes_absent = mev::lanes_brain(&dir_absent).unwrap();
    let lanes_off = mev::lanes_brain(&dir_off).unwrap();
    assert_eq!(lanes_absent.degraded, lanes_off.degraded);
    // Diagnostic derives neither PartialEq nor Deserialize — compare via JSON, with
    // each `file` field reduced to its file name (the two fixtures live under
    // different temp-dir roots, so the absolute path itself always differs).
    let diag_shape = |issues: &[mev::Diagnostic]| -> serde_json::Value {
        let mut v = serde_json::to_value(issues).unwrap();
        if let Some(arr) = v.as_array_mut() {
            for item in arr {
                if let Some(file) = item.get("file").and_then(|f| f.as_str()) {
                    let name = Path::new(file)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string());
                    item["file"] = serde_json::json!(name);
                }
            }
        }
        v
    };
    assert_eq!(
        diag_shape(&lanes_absent.registration_issues),
        diag_shape(&lanes_off.registration_issues)
    );
    assert_eq!(lanes_absent.segments, lanes_off.segments);

    // blocks_brain
    let query = mev::brain::query::BlockQuery::default();
    let blocks_absent = mev::blocks_brain(&dir_absent, &query, false, false, None).unwrap();
    let blocks_off = mev::blocks_brain(&dir_off, &query, false, false, None).unwrap();
    assert_eq!(blocks_absent.blocks, blocks_off.blocks);

    // build_block_graph_export
    let (loaded_absent, config_absent) = load_corpus(&dir_absent);
    let (loaded_off_files, config_off_files) = load_corpus(&dir_off);
    let graph_absent = build_state_graph(&loaded_absent);
    let graph_off = build_state_graph(&loaded_off_files);
    let scope = BlockGraphScope {
        tier: BgTierScope::All,
        epic: None,
        repo: None,
        include_closed: true,
        include_boundary: false,
        max_nodes: usize::MAX,
    };
    let export_absent = build_block_graph_export(
        &dir_absent,
        &config_absent,
        &graph_absent,
        &loaded_absent,
        &scope,
    );
    let export_off = build_block_graph_export(
        &dir_off,
        &config_off_files,
        &graph_off,
        &loaded_off_files,
        &scope,
    );
    // BlockGraphNode/BlockGraphEdge derive neither PartialEq nor Serialize's
    // reciprocal Deserialize, so compare via serde_json::Value rather than a
    // hand-picked field projection that could silently miss a divergence.
    let nodes_absent = serde_json::to_value(&export_absent.nodes).unwrap();
    let nodes_off = serde_json::to_value(&export_off.nodes).unwrap();
    assert_eq!(nodes_absent, nodes_off);
    let edges_absent = serde_json::to_value(&export_absent.edges).unwrap();
    let edges_off = serde_json::to_value(&export_off.edges).unwrap();
    assert_eq!(edges_absent, edges_off);

    let _ = fs::remove_dir_all(&dir_absent);
    let _ = fs::remove_dir_all(&dir_off);
}

// ---------------------------------------------------------------------------
// One CLI test: the built binary's `frontier --json`, source build only (never
// the installed binary — that reproduction is HQ.7.C's).
// ---------------------------------------------------------------------------

#[test]
fn cli_frontier_json_reports_the_held_entry_on_the_enforcement_on_fixture() {
    let dir = temp_dir("cli-frontier-json");
    write_wiring_fixture(&dir, true, 10);

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_mev"))
        .args(["frontier", "--json"])
        .current_dir(&dir)
        .output()
        .expect("run mev frontier --json");
    assert!(
        output.status.success(),
        "mev frontier --json should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("expected valid JSON from mev frontier --json: {e}\n{stdout}"));
    let entries = value["entries"]
        .as_array()
        .expect("expected an entries array");
    let entry = entries
        .iter()
        .find(|e| e["key"] == "other:OT.1.A")
        .unwrap_or_else(|| panic!("expected an entry for other:OT.1.A in {entries:?}"));
    assert_eq!(entry["startable"], serde_json::json!(false));
    let unmet_gates = entry["unmet_gates"]
        .as_array()
        .expect("expected unmet_gates array");
    assert!(
        unmet_gates.iter().any(|g| g == other_ot1a_gate()),
        "expected unmet_gates to contain {}, got {:?}",
        other_ot1a_gate(),
        unmet_gates
    );

    let _ = fs::remove_dir_all(&dir);
}
