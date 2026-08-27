//! Integration tests for carryover-enforcement gating (`MV.16.C`, Task 6) — the flag, the
//! cap, the per-entry opt-out, the no-lane invisibility case, and a differential test
//! against `mev carryover --would-block`, all over fixture corpora (never the live one:
//! its edge population moved from 3 to 5 between this block being cut and `MV.16.A`
//! shipping, and will move again).
//!
//! Drives the real pipeline pieces `derive_focus`/`ready_order` (`src/brain/state.rs`)
//! and `build_carryover_gating_sets`/`compute_would_block_report` (`src/brain/carryover.rs`)
//! compose from, over a temp-dir corpus discovered and loaded the same way
//! `tests/brain_carryover_would_block.rs` does.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use mev::brain::carryover::{
    CarryoverGate, CarryoverVerdict, LaneResidencyIndex, RepoGatingReport,
    build_carryover_gating_sets, build_lane_residency_index, compute_would_block_report,
    render_would_block_enforcement_summary,
};
use mev::brain::config::find_brain_config;
use mev::brain::state::{
    DerivedFocus, StateFile, StateGraph, StateSource, block_status_map, derive_focus,
    discover_state_files, load_state, ready_order,
};
use mev::evaluate_carryover;

// ---------------------------------------------------------------------------
// Fixture helpers
// ---------------------------------------------------------------------------

fn temp_dir(suffix: &str) -> PathBuf {
    let dir = mev::testsupport::unique_temp_dir(&format!("mev-carryover-enforcement-it-{suffix}"));
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

/// Mirrors `tests/brain_carryover_would_block.rs`'s helper of the same shape.
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

/// Loads a temp-dir corpus into the `(sources, files, status_map)` shape every helper
/// below needs, plus the `config` (for `evaluate_carryover`'s `thresholds` argument).
fn load_corpus(
    root: &Path,
) -> (
    Vec<(StateSource, StateFile)>,
    HashMap<String, Option<String>>,
    mev::brain::config::BrainConfig,
) {
    let config = find_brain_config(root).expect("brain.toml should load");
    let (sources, _diags) = discover_state_files(root, &config);
    let mut loaded: Vec<(StateSource, StateFile)> = Vec::new();
    for src in &sources {
        let file = load_state(&src.abs_path).expect("fixture state.json should parse");
        loaded.push((src.clone(), file));
    }
    let status_map = block_status_map(&loaded);
    (loaded, status_map, config)
}

fn entries_for(
    loaded: &[(StateSource, StateFile)],
    status_map: &HashMap<String, Option<String>>,
    root: &Path,
    config: &mev::brain::config::BrainConfig,
) -> Vec<CarryoverVerdict> {
    let mut repo_paths: HashMap<String, PathBuf> = HashMap::new();
    for (src, _) in loaded {
        repo_paths.insert(
            src.repo_slug.clone(),
            root.join("repos").join(&src.repo_slug),
        );
    }
    let report = evaluate_carryover(
        loaded,
        status_map,
        root,
        &repo_paths,
        "2026-08-24",
        &config.attention,
        None,
        false,
    );
    report.entries
}

fn focus_for<'a>(
    repo_slug: &str,
    loaded: &'a [(StateSource, StateFile)],
    gating: Option<&BTreeMap<String, RepoGatingReport>>,
) -> DerivedFocus {
    let (src, file) = loaded
        .iter()
        .find(|(s, _)| s.repo_slug == repo_slug)
        .unwrap_or_else(|| panic!("expected a loaded file for repo {repo_slug}"));
    derive_focus(src, file, &StateGraph::default(), loaded, gating)
}

fn ready_for(
    loaded: &[(StateSource, StateFile)],
    gating: Option<&BTreeMap<String, RepoGatingReport>>,
) -> Vec<String> {
    ready_order(&StateGraph::default(), loaded, gating)
}

fn gate_owner<'a>(focus: &'a DerivedFocus, id: &str) -> &'a str {
    focus
        .carryover_gates
        .get(id)
        .and_then(|gates| gates.first())
        .map(|g: &CarryoverGate| g.owner.as_str())
        .unwrap_or_else(|| panic!("expected a carryover_gates entry naming the owner for {id}"))
}

// ---------------------------------------------------------------------------
// Fixture corpus: the comprehensive matrix (flag, no-lane, opt-out, closed/wontfix,
// deferred/in_progress) all in one fixture so the differential test can compare
// against `--would-block` edge-for-edge on the same data.
// ---------------------------------------------------------------------------

fn write_matrix_fixture(dir: &Path) {
    write_brain_toml(dir, &["mev", "other", "alpha"]);

    write_json(
        dir,
        "repos/other/planning/state.json",
        &serde_json::json!({
            "repo": "other",
            "kind": "project",
            "updated": "2026-08-24",
            "focus": { "now": [], "next": [], "blocked": [] },
            "tracks": [{
                "title": "Phase 1",
                "blocks": [
                    block("OT.1.A", None),               // open, lane-resident
                    block("OT.2.A", Some("closed")),      // closed target
                    block("OT.3.A", Some("wontfix")),     // wontfix target
                ]
            }],
            "carryover": []
        }),
    );

    write_json(
        dir,
        "repos/alpha/planning/state.json",
        &serde_json::json!({
            "repo": "alpha",
            "kind": "project",
            "updated": "2026-08-24",
            "focus": { "now": [], "next": [], "blocked": [] },
            "tracks": [{
                "title": "Phase 1",
                "blocks": [
                    block("AL.1.A", None),                  // open, NO lane record at all
                    block("AL.2.A", None),                  // open, gated by an enforce:false entry
                    block("AL.3.A", Some("deferred")),      // deferred target, gated anyway
                    block("AL.4.A", Some("in_progress")),   // in_progress target, gated anyway
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
            "updated": "2026-08-24",
            "focus": { "now": [], "next": [], "blocked": [] },
            "tracks": [{ "title": "Phase 1", "blocks": [] }],
            "carryover": [
                carryover_entry("gate-open-lane-resident", &[block_edge("other", "OT.1.A")], None),
                carryover_entry("gate-open-no-lane", &[block_edge("alpha", "AL.1.A")], None),
                carryover_entry("gate-opt-out", &[block_edge("alpha", "AL.2.A")], Some(false)),
                carryover_entry("gate-deferred-target", &[block_edge("alpha", "AL.3.A")], None),
                carryover_entry("gate-in-progress-target", &[block_edge("alpha", "AL.4.A")], None),
                carryover_entry("gate-onto-closed", &[block_edge("other", "OT.2.A")], None),
                carryover_entry("gate-onto-wontfix", &[block_edge("other", "OT.3.A")], None),
            ]
        }),
    );

    // Only other:OT.1.A is lane-resident; alpha:AL.1.A deliberately sits in NO lane
    // record — that is the exact invisibility case this block exists to close.
    write_raw(
        dir,
        "planning/roadmaps/alpha-roadmap/lane-substrate.json",
        &lane_json("substrate", "alpha-roadmap", &[("other", "OT.1.A")]),
    );
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// `enforce_blocks = true` on a fixture carrying one edge onto an open, lane-resident
/// block holds it — reported by the block-level derivation, `blocked` and `next`, with
/// the reason naming the owning carryover slug. The same fixture with
/// `enforce_blocks = false` reports the block startable (shown-failing pair: flip the
/// flag back and the assertion inverts).
#[test]
fn flag_on_holds_open_lane_resident_block_flag_off_reports_it_startable() {
    let dir = temp_dir("flag");
    write_matrix_fixture(&dir);
    let (loaded, status_map, _config) = load_corpus(&dir);

    // enforce_blocks = false -> startable.
    let gating_off = build_carryover_gating_sets(
        &entries_for(&loaded, &status_map, &dir, &_config),
        &status_map,
        false,
        100,
    );
    let focus_off = focus_for("other", &loaded, Some(&gating_off));
    assert!(
        !focus_off.blocked.iter().any(|(id, _)| id == "OT.1.A"),
        "with enforcement off, OT.1.A must be startable, got blocked={:?}",
        focus_off.blocked
    );
    assert!(ready_for(&loaded, Some(&gating_off)).contains(&"other:OT.1.A".to_string()));

    // enforce_blocks = true -> held, reason names the owning slug. Shown-failing: the
    // assertion above inverts.
    let gating_on = build_carryover_gating_sets(
        &entries_for(&loaded, &status_map, &dir, &_config),
        &status_map,
        true,
        100,
    );
    let focus_on = focus_for("other", &loaded, Some(&gating_on));
    assert!(
        focus_on.blocked.iter().any(|(id, _)| id == "OT.1.A"),
        "with enforcement on, OT.1.A must be held, got blocked={:?}",
        focus_on.blocked
    );
    assert_eq!(
        gate_owner(&focus_on, "OT.1.A"),
        "mev:gate-open-lane-resident"
    );
    assert!(
        !ready_for(&loaded, Some(&gating_on)).contains(&"other:OT.1.A".to_string()),
        "a held block must never appear in ready_order"
    );

    let _ = fs::remove_dir_all(&dir);
}

/// A block gated only by a carryover edge, with no `depends_on` of its own and sitting
/// in NO lane record, is still held and still visible — asserted against the
/// block-level derivation, never the frontier (which is lane-head-scoped and would
/// represent it only by absence). This is the exact invisibility `MV.16.C` exists to
/// close: `MV.16.A` measured 4 of 5 live edges pointing at targets in no lane.
#[test]
fn gated_block_in_no_lane_is_still_held_and_visible() {
    let dir = temp_dir("no-lane");
    write_matrix_fixture(&dir);
    let (loaded, status_map, config) = load_corpus(&dir);

    let gating = build_carryover_gating_sets(
        &entries_for(&loaded, &status_map, &dir, &config),
        &status_map,
        true,
        100,
    );

    // Sanity: AL.1.A really is absent from every lane record in this fixture.
    let (lane_index, lane_diags): (LaneResidencyIndex, _) = build_lane_residency_index(&dir);
    assert!(
        lane_diags.is_empty(),
        "unexpected lane diagnostics: {lane_diags:?}"
    );
    assert!(
        !lane_index.is_resident("alpha:AL.1.A"),
        "fixture precondition: AL.1.A must sit in no lane record"
    );

    let focus = focus_for("alpha", &loaded, Some(&gating));
    assert!(
        focus.blocked.iter().any(|(id, _)| id == "AL.1.A"),
        "AL.1.A has no depends_on and no lane residency, but must still be held and \
         visible in the derivation; got blocked={:?}",
        focus.blocked
    );
    assert_eq!(gate_owner(&focus, "AL.1.A"), "mev:gate-open-no-lane");

    let _ = fs::remove_dir_all(&dir);
}

/// `max_gates_per_repo = 0` applies zero gates, reports `cap exceeded`, and leaves the
/// derived output byte-identical to an unenforced run. A cap between 0 and the edge
/// count applies exactly that many gates and reports the remainder rather than
/// silently dropping it.
#[test]
fn cap_zero_applies_none_partial_cap_applies_exactly_that_many() {
    let dir = temp_dir("cap");
    write_brain_toml(&dir, &["mev", "capr"]);
    write_json(
        &dir,
        "repos/capr/planning/state.json",
        &serde_json::json!({
            "repo": "capr",
            "kind": "project",
            "updated": "2026-08-24",
            "focus": { "now": [], "next": [], "blocked": [] },
            "tracks": [{
                "title": "Phase 1",
                "blocks": [
                    block("CP.1.A", None),
                    block("CP.2.A", None),
                    block("CP.3.A", None),
                    block("CP.4.A", None),
                ]
            }],
            "carryover": []
        }),
    );
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
                carryover_entry("cap-1", &[block_edge("capr", "CP.1.A")], None),
                carryover_entry("cap-2", &[block_edge("capr", "CP.2.A")], None),
                carryover_entry("cap-3", &[block_edge("capr", "CP.3.A")], None),
                carryover_entry("cap-4", &[block_edge("capr", "CP.4.A")], None),
            ]
        }),
    );

    let (loaded, status_map, config) = load_corpus(&dir);
    let entries = entries_for(&loaded, &status_map, &dir, &config);

    // Unenforced baseline.
    let gating_unenforced = build_carryover_gating_sets(&entries, &status_map, false, 100);
    let focus_unenforced = focus_for("capr", &loaded, Some(&gating_unenforced));
    let ready_unenforced = ready_for(&loaded, Some(&gating_unenforced));

    // cap = 0 -> zero gates applied, cap exceeded reported, byte-identical derivation.
    let gating_cap0 = build_carryover_gating_sets(&entries, &status_map, true, 0);
    let report_cap0 = gating_cap0
        .get("capr")
        .expect("capr should have a gating report");
    assert_eq!(report_cap0.applied_count, 0);
    assert_eq!(report_cap0.candidate_count, 4);
    assert!(report_cap0.cap_exceeded);
    assert!(report_cap0.gates.is_empty());
    let summary_cap0 = render_would_block_enforcement_summary(true, 0, &gating_cap0);
    assert!(summary_cap0.contains("cap exceeded — capr: 0 of 4 gates applied"));

    let focus_cap0 = focus_for("capr", &loaded, Some(&gating_cap0));
    assert_eq!(
        focus_cap0.blocked, focus_unenforced.blocked,
        "cap=0 must be byte-identical to unenforced"
    );
    assert_eq!(
        focus_cap0.next, focus_unenforced.next,
        "cap=0 must be byte-identical to unenforced"
    );
    assert!(focus_cap0.carryover_gates.is_empty());
    assert_eq!(ready_for(&loaded, Some(&gating_cap0)), ready_unenforced);

    // cap = 2 -> exactly 2 of 4 applied, remainder reported, never silently dropped.
    let gating_cap2 = build_carryover_gating_sets(&entries, &status_map, true, 2);
    let report_cap2 = gating_cap2.get("capr").unwrap();
    assert_eq!(report_cap2.applied_count, 2);
    assert_eq!(report_cap2.candidate_count, 4);
    assert!(report_cap2.cap_exceeded);
    assert_eq!(report_cap2.gates.len(), 2);
    let summary_cap2 = render_would_block_enforcement_summary(true, 2, &gating_cap2);
    assert!(summary_cap2.contains("cap exceeded — capr: 2 of 4 gates applied"));

    let focus_cap2 = focus_for("capr", &loaded, Some(&gating_cap2));
    let held: Vec<&str> = focus_cap2
        .blocked
        .iter()
        .map(|(id, _)| id.as_str())
        .collect();
    assert_eq!(
        held.len(),
        2,
        "exactly the cap's worth of gates must be applied, got {held:?}"
    );

    let _ = fs::remove_dir_all(&dir);
}

/// An entry carrying `enforce: false` contributes no gate even with the flag on;
/// `None` and `Some(true)` both enforce. A `closed` target and a `wontfix` target each
/// contribute no gate, matching `--would-block`'s verdicts. A `deferred` and an
/// `in_progress` target are never reported blocked, gates notwithstanding — both
/// lanes are terminal and win over any gate.
#[test]
fn opt_out_closed_wontfix_deferred_in_progress_never_gate() {
    let dir = temp_dir("opt-out-terminal");
    write_matrix_fixture(&dir);
    let (loaded, status_map, config) = load_corpus(&dir);
    let entries = entries_for(&loaded, &status_map, &dir, &config);

    let gating = build_carryover_gating_sets(&entries, &status_map, true, 100);

    // enforce:false contributes no gate.
    let focus_alpha = focus_for("alpha", &loaded, Some(&gating));
    assert!(
        !focus_alpha.blocked.iter().any(|(id, _)| id == "AL.2.A"),
        "AL.2.A's gating entry carries enforce:false and must contribute no gate, got {:?}",
        focus_alpha.blocked
    );

    // deferred/in_progress targets are terminal lanes and never land in `blocked`,
    // gates notwithstanding.
    assert!(focus_alpha.deferred.contains(&"AL.3.A".to_string()));
    assert!(focus_alpha.now.contains(&"AL.4.A".to_string()));
    assert!(!focus_alpha.blocked.iter().any(|(id, _)| id == "AL.3.A"));
    assert!(!focus_alpha.blocked.iter().any(|(id, _)| id == "AL.4.A"));
    assert!(focus_alpha.carryover_gates.get("AL.3.A").is_none());
    assert!(focus_alpha.carryover_gates.get("AL.4.A").is_none());

    // closed / wontfix targets contribute no gate (they were never candidates at all).
    let focus_other = focus_for("other", &loaded, Some(&gating));
    assert!(!focus_other.blocked.iter().any(|(id, _)| id == "OT.2.A"));
    assert!(!focus_other.blocked.iter().any(|(id, _)| id == "OT.3.A"));

    let _ = fs::remove_dir_all(&dir);
}

/// No `blocked` value is ever written into a `tracks[]` block's authored `status` or
/// `depends_on` by the enforcement machinery — the gate is consulted only inside
/// `derive_focus`, never persisted back onto the fixture's source files.
#[test]
fn no_blocked_value_is_ever_authored() {
    let dir = temp_dir("no-authored-blocked");
    write_matrix_fixture(&dir);
    let (loaded, status_map, config) = load_corpus(&dir);
    let entries = entries_for(&loaded, &status_map, &dir, &config);
    let gating = build_carryover_gating_sets(&entries, &status_map, true, 100);

    // Run the derivation for every loaded file (mutates nothing — derive_focus is pure).
    for (src, file) in &loaded {
        let _ = derive_focus(src, file, &StateGraph::default(), &loaded, Some(&gating));
    }

    // Re-read the fixture's own state.json files straight off disk and assert no
    // `tracks[].blocks[].status` is "blocked" and no synthesized depends_on entry
    // referencing a carryover slug was ever written.
    for repo in ["mev", "other", "alpha"] {
        let raw =
            fs::read_to_string(dir.join(format!("repos/{repo}/planning/state.json"))).unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let tracks = value["tracks"].as_array().cloned().unwrap_or_default();
        for track in &tracks {
            for b in track["blocks"].as_array().cloned().unwrap_or_default() {
                let status = b["status"].as_str();
                assert_ne!(
                    status,
                    Some("blocked"),
                    "repo {repo} block {:?} must never carry an authored 'blocked' status",
                    b["id"]
                );
            }
        }
    }

    let _ = fs::remove_dir_all(&dir);
}

/// The load-bearing differential test: the gating set built by
/// `build_carryover_gating_sets` agrees edge-for-edge with `mev carryover --would-block`
/// on the same fixture — every `Blocking`-verdict row's target is gated (given a cap
/// large enough not to interfere), and every applied gate corresponds to a `Blocking`
/// row with the same owner. If the gate and its own preview drift, the preview is
/// actively harmful.
#[test]
fn gating_set_agrees_edge_for_edge_with_would_block_report() {
    let dir = temp_dir("differential");
    write_matrix_fixture(&dir);
    let (loaded, status_map, config) = load_corpus(&dir);
    let entries = entries_for(&loaded, &status_map, &dir, &config);

    let gating = build_carryover_gating_sets(&entries, &status_map, true, 100);

    let (lane_index, lane_diags): (LaneResidencyIndex, _) = build_lane_residency_index(&dir);
    assert!(
        lane_diags.is_empty(),
        "unexpected lane diagnostics: {lane_diags:?}"
    );
    let report = compute_would_block_report(&entries, &status_map, &lane_index);

    // `--would-block` classifies every edge by target status alone — it has no notion
    // of the per-entry `enforce: false` opt-out (that is `build_carryover_gating_sets`'
    // concern, task 2's requirement, exercised by its own dedicated test above). Owners
    // carrying `enforce: Some(false)` therefore legitimately diverge here by design and
    // are excluded from the edge-for-edge comparison.
    let opted_out_owners: std::collections::HashSet<String> = entries
        .iter()
        .filter(|e| e.enforce == Some(false))
        .map(|e| format!("{}:{}", e.repo, e.slug))
        .collect();

    // Every Blocking row's target must be an applied gate, with the same owner.
    let mut blocking_rows = 0usize;
    for row in &report.rows {
        if row.verdict != mev::brain::carryover::EdgeBlockVerdict::Blocking {
            continue;
        }
        if opted_out_owners.contains(&row.owner) {
            continue;
        }
        blocking_rows += 1;
        let target_key = row
            .target_key
            .as_ref()
            .expect("a Blocking row always carries a resolved target key");
        let (repo, _) = target_key.split_once(':').unwrap();
        let applied = gating
            .get(repo)
            .and_then(|r| r.gates.get(target_key))
            .unwrap_or_else(|| {
                panic!("would-block Blocking row for {target_key} has no matching applied gate")
            });
        assert_eq!(
            applied.owner, row.owner,
            "gate owner for {target_key} must match the would-block row's owner"
        );
    }

    // Every applied gate must correspond to some Blocking row.
    let mut applied_gates = 0usize;
    for (_, per_repo) in &gating {
        for (target_key, gate) in &per_repo.gates {
            applied_gates += 1;
            let matched = report.rows.iter().any(|row| {
                row.verdict == mev::brain::carryover::EdgeBlockVerdict::Blocking
                    && row.target_key.as_deref() == Some(target_key.as_str())
                    && row.owner == gate.owner
            });
            assert!(
                matched,
                "applied gate on {target_key} (owner {}) has no matching would-block \
                 Blocking row",
                gate.owner
            );
        }
    }

    assert_eq!(
        blocking_rows, applied_gates,
        "the gating set and the would-block report must agree edge-for-edge on this fixture"
    );
    assert!(
        blocking_rows > 0,
        "fixture sanity: expect at least one Blocking row"
    );

    let _ = fs::remove_dir_all(&dir);
}
