//! Covering test for the `sibling-rule-coverage` conformance check's first rule,
//! `dual-role-repo-resolution` — `MV.ticket.sibling-rule-coverage`, Task 3.
//!
//! `derive_rollup` and `derive_brain_focus` both resolve a repo's state file
//! through `resolve_repo_state_file`, and both must honour the dual-role rule: a
//! registered repo is either a leaf (`kind: "project"`) or a tier sub-brain root
//! (`kind: "brain"`) carrying its own authored `tracks[]`. `derive_brain_focus`
//! learned the rule first (`MV.ticket.brain-focus-dual-role-drift`);
//! `derive_rollup` kept hard-filtering `kind == "project"` and stayed silently
//! wrong for months, blinding every API consumer to `kind: "brain"` children —
//! the incident `MV.ticket.derive-rollup-dual-role-drift` fixed. This test is
//! the "asserted against BOTH" proof the sibling-rule-coverage check's
//! `covering_test` field keys off: it fails loudly (rather than silently) if
//! either resolver stops honouring the rule.

use std::fs;
use std::path::Path;

use mev::brain::config::{BrainConfig, CrawlConfig, RepoEntry, VocabConfig};
use mev::brain::state::{
    StateFile, StateGraph, StateSource, TierScope, check_status_consistency, derive_brain_focus,
    derive_focus, derive_rollup, ready_order,
};

/// Make a fresh uniquely-named temp dir for a test and return its path.
fn temp_dir(suffix: &str) -> std::path::PathBuf {
    let dir = mev::testsupport::unique_temp_dir(&format!("mev-sibling-rules-it-{suffix}"));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Write `json` to `dir/filename`, then build the `(StateSource, StateFile)` pair
/// `derive_rollup`/`derive_brain_focus` operate on — mirroring `state.rs`'s own
/// `make_pair` test helper, reimplemented here because integration tests cannot
/// reach `src/`'s `#[cfg(test)]`-gated helpers.
fn make_pair(
    dir: &Path,
    filename: &str,
    kind: &'static str,
    json: &str,
) -> (StateSource, StateFile) {
    let path = dir.join(filename);
    fs::write(&path, json).unwrap();
    let repo_slug = serde_json::from_str::<serde_json::Value>(json).unwrap()["repo"]
        .as_str()
        .unwrap()
        .to_string();
    let src = StateSource {
        repo_slug,
        abs_path: path,
        expected_kind: kind,
    };
    let file: StateFile = serde_json::from_str(json).expect("fixture must parse");
    (src, file)
}

/// A `kind: "brain"` sub-brain child with its own authored `tracks[]` — the
/// dual-role shape. Carries one `in_progress` block so the derivation surfaces
/// something observable when the rule is honoured.
fn dual_role_brain_pair(dir: &Path, repo: &str, block_id: &str) -> (StateSource, StateFile) {
    let json = format!(
        r#"{{
  "repo": "{repo}",
  "kind": "brain",
  "updated": "2026-08-04",
  "focus": {{ "now": [], "next": [], "blocked": [] }},
  "tracks": [{{
    "title": "Own Track",
    "blocks": [{{ "id": "{block_id}", "title": "Own now work", "status": "in_progress" }}]
  }}]
}}"#
    );
    make_pair(dir, &format!("{repo}-state.json"), "brain", &json)
}

/// A plain `kind: "project"` leaf sibling — present alongside the `kind: "brain"`
/// child so the fixture proves the rule holds for BOTH kinds, not just one.
fn leaf_pair(dir: &Path, repo: &str, block_id: &str) -> (StateSource, StateFile) {
    let json = format!(
        r#"{{
  "repo": "{repo}",
  "kind": "project",
  "updated": "2026-08-04",
  "focus": {{
    "now": [{{ "id": "{block_id}", "title": "Leaf work", "status": "in_progress" }}],
    "next": [],
    "blocked": []
  }},
  "tracks": [{{
    "title": "Phase 1",
    "blocks": [{{ "id": "{block_id}", "title": "Leaf work", "status": "in_progress" }}]
  }}]
}}"#
    );
    make_pair(dir, &format!("{repo}-state.json"), "project", &json)
}

/// The self/root brain pair `derive_brain_focus` needs its own `self_src`/`self_file`
/// arguments — empty `tracks[]` so it contributes nothing beyond folding children,
/// keeping the assertions focused on the dual-role resolution being tested.
fn empty_self_pair(dir: &Path, repo: &str) -> (StateSource, StateFile) {
    let json = format!(
        r#"{{
  "repo": "{repo}",
  "kind": "brain",
  "updated": "2026-08-04",
  "focus": {{ "now": [], "next": [], "blocked": [] }},
  "tracks": []
}}"#
    );
    make_pair(dir, &format!("{repo}-state.json"), "brain", &json)
}

/// Config with one `kind: "brain"` sub-brain repo and one `kind: "project"` leaf
/// repo, both in the same tier — the mixed corpus the dual-role rule must
/// resolve identically for both `derive_rollup` and `derive_brain_focus`.
fn mixed_kind_config() -> BrainConfig {
    BrainConfig {
        attention: Default::default(),
        history: Default::default(),
        vocab: VocabConfig::default(),
        crawl: CrawlConfig::default(),
        repos: vec![
            RepoEntry {
                slug: "subbrain".to_string(),
                tier: "core".to_string(),
                repo_path: "core/subbrain".to_string(),
                status_file: String::new(),
                cache_doc: String::new(),
                heading: String::new(),
                prefix: None,
            },
            RepoEntry {
                slug: "leafalpha".to_string(),
                tier: "core".to_string(),
                repo_path: "core/leafalpha".to_string(),
                status_file: String::new(),
                cache_doc: String::new(),
                heading: String::new(),
                prefix: None,
            },
        ],
    }
}

/// One fixture corpus — a `kind: "brain"` state file with non-empty authored
/// `tracks[]` alongside a `kind: "project"` leaf — run through BOTH
/// `derive_rollup` and `derive_brain_focus`, asserting each surfaces the brain
/// file's own block. This is the "asserted against BOTH" proof
/// `sibling-rule-coverage`'s `dual-role-repo-resolution` rule keys its
/// `test-not-covering` finding off: both `derive_rollup` and `derive_brain_focus`
/// must appear, literally, in this test's body.
///
/// Regression this protects: `MV.ticket.derive-rollup-dual-role-drift` —
/// `derive_brain_focus` learned the dual-role rule and `derive_rollup` did not,
/// so `derive_rollup` kept hard-filtering `kind == "project"` and stayed
/// silently blind to every `kind: "brain"` child for months.
#[test]
fn dual_role_rule_holds_for_both_resolvers() {
    let dir = temp_dir("dual-role");
    let config = mixed_kind_config();
    let scope = TierScope::All;
    let graph = StateGraph::default();

    let subbrain_pair = dual_role_brain_pair(dir.as_path(), "subbrain", "SUBBRAIN.1.A");
    let leaf_pair_entry = leaf_pair(dir.as_path(), "leafalpha", "LEAFALPHA.1.A");
    let files = vec![subbrain_pair, leaf_pair_entry];

    let (self_src, self_file) = empty_self_pair(dir.as_path(), "hq");

    // --- derive_brain_focus: the sub-brain child's own block must fold into
    // `focus.now`, tagged with its slug, exactly like the project leaf's does.
    let focus = derive_brain_focus(&self_src, &self_file, &scope, &config, &graph, &files);
    let now_pairs: Vec<(Option<&str>, &str)> = focus
        .now
        .iter()
        .map(|b| (b.repo.as_deref(), b.id.as_str()))
        .collect();
    assert!(
        now_pairs.contains(&(Some("subbrain"), "SUBBRAIN.1.A")),
        "derive_brain_focus must fold the kind:\"brain\" child's own block into focus.now \
         (dual-role rule), got: {now_pairs:?}"
    );
    assert!(
        now_pairs.contains(&(Some("leafalpha"), "LEAFALPHA.1.A")),
        "derive_brain_focus must still surface the kind:\"project\" leaf's block, got: {now_pairs:?}"
    );

    // --- derive_rollup: the sub-brain child must be resolved (not filtered to
    // an empty stub) and its own block must surface in the rollup entry's `now`.
    let rollup = derive_rollup(&scope, &config, &[], &graph, &files);
    let subbrain_entry = rollup
        .iter()
        .find(|r| r.repo == "subbrain")
        .expect("derive_rollup must produce an entry for the kind:\"brain\" child");
    assert!(
        subbrain_entry.now.iter().any(|b| b.id == "SUBBRAIN.1.A"),
        "derive_rollup must resolve the kind:\"brain\" child via the dual-role rule instead of \
         emitting an empty stub, got: {:?}",
        subbrain_entry.now
    );
    let leaf_entry = rollup
        .iter()
        .find(|r| r.repo == "leafalpha")
        .expect("derive_rollup must still produce an entry for the kind:\"project\" leaf");
    assert!(
        leaf_entry.now.iter().any(|b| b.id == "LEAFALPHA.1.A"),
        "derive_rollup must still resolve the kind:\"project\" leaf, got: {:?}",
        leaf_entry.now
    );
}

/// One fixture, run through `check_status_consistency`, `ready_order`, and
/// `derive_focus`, asserting all three resolve the same block to the same
/// authored status — the covering test for the `block-status-map-construction`
/// sibling rule. All three functions now build their `"{repo}:{id}" -> status`
/// lookup exclusively through `block_status_map` (`src/brain/state.rs`); this
/// test is the "asserted against BOTH [all three]" proof the check's
/// `test-not-covering` finding keys off — `check_status_consistency`,
/// `ready_order`, and `derive_focus` must all appear, literally, in this
/// test's body.
#[test]
fn all_status_consumers_agree_on_one_fixture() {
    let dir = temp_dir("status-map");
    let graph = StateGraph::default();

    // gamma: GA.1.A closed (no deps); GA.1.B open, depends_on GA.1.A (block dep,
    // satisfied -> ready); GA.1.C in_progress.
    let json = r#"{
  "repo": "gamma",
  "kind": "project",
  "updated": "2026-08-04",
  "focus": { "now": [], "next": [], "blocked": [] },
  "tracks": [{
    "title": "Phase 1",
    "blocks": [
      { "id": "GA.1.A", "title": "Done", "status": "closed" },
      {
        "id": "GA.1.B",
        "title": "Ready",
        "status": "open",
        "depends_on": [{ "type": "block", "repo": "gamma", "id": "GA.1.A" }]
      },
      { "id": "GA.1.C", "title": "Active", "status": "in_progress" }
    ]
  }]
}"#;
    let pair = make_pair(dir.as_path(), "gamma-state.json", "project", json);
    let files = vec![pair];
    let (src, file) = &files[0];

    // --- check_status_consistency: GA.1.A is closed and has no deps, so no
    // block in this fixture is a closed-depending-on-non-closed pair. All three
    // functions must agree GA.1.A's authored status is "closed".
    let diags = check_status_consistency(&files);
    assert!(
        diags.is_empty(),
        "no closed block depends on a non-closed block in this fixture, got: {diags:?}"
    );

    // --- ready_order: GA.1.B is open with its sole block-dep (GA.1.A) closed,
    // so it must be ready — proving ready_order also resolved GA.1.A as closed.
    let ready = ready_order(&graph, &files);
    assert!(
        ready.contains(&"gamma:GA.1.B".to_string()),
        "GA.1.B's dependency GA.1.A is closed, so GA.1.B must be ready, got: {ready:?}"
    );
    assert!(
        !ready.contains(&"gamma:GA.1.A".to_string()),
        "GA.1.A is closed, not open, so it must not appear in ready_order, got: {ready:?}"
    );

    // --- derive_focus: GA.1.C (in_progress) lands in `now`, GA.1.B (open, dep
    // closed) lands in `next` — the same GA.1.A-is-closed resolution once more,
    // this time through derive_focus's own status_map lookup.
    let focus = derive_focus(src, file, &graph, &files);
    assert!(
        focus.now.contains(&"GA.1.C".to_string()),
        "GA.1.C is in_progress, expected in derive_focus.now, got: {:?}",
        focus.now
    );
    assert!(
        focus.next.contains(&"GA.1.B".to_string()),
        "GA.1.B is ready (dep GA.1.A closed), expected in derive_focus.next, got: {:?}",
        focus.next
    );
    assert!(
        focus.blocked.is_empty(),
        "no block in this fixture has an unmet dependency, got: {:?}",
        focus.blocked
    );
}
