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
fn json_run_without_write_leaves_the_corpus_byte_identical() {
    // Task 5's write-path constraint: without --write, no file on disk is
    // modified -- asserted by a byte-comparison of the corpus before and after a
    // --json run, not merely "no write flag was passed".
    let tmp = build_planted_fixture();
    let state_path = tmp.path().join("planning/state.json");
    let before = fs::read(&state_path).unwrap();

    let output = run_mev(&["graph-findings", ".", "--json"], tmp.path());
    assert!(!output.status.success());

    let after = fs::read(&state_path).unwrap();
    assert_eq!(
        before, after,
        "state.json must be byte-identical without --write"
    );
}

#[test]
fn write_appends_a_carryover_entry_with_single_key_scope_and_a_kind_from_the_four() {
    let tmp = build_planted_fixture();
    let output = run_mev(&["graph-findings", ".", "--write"], tmp.path());
    assert!(
        !output.status.success(),
        "findings still exist after --write (it appends, it does not clear the finding); \
         stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let state_path = tmp.path().join("planning/state.json");
    let state: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&state_path).unwrap()).unwrap();
    let carryover = state["carryover"].as_array().unwrap();
    assert_eq!(
        carryover.len(),
        1,
        "expected exactly one appended entry: {carryover:?}"
    );

    let entry = &carryover[0];
    let scope = &entry["scope"];
    let non_null_keys = ["repo", "tier", "cross_repo"]
        .iter()
        .filter(|k| !scope[*k].is_null())
        .count();
    assert_eq!(
        non_null_keys, 1,
        "scope must have exactly one non-null key: {scope}"
    );
    assert_eq!(scope["repo"], "acme");

    assert!(
        matches!(
            entry["kind"].as_str(),
            Some("defect" | "deferred" | "drift" | "env")
        ),
        "kind must be one of the four live kinds: {}",
        entry["kind"]
    );
    assert_eq!(
        entry["finding_id"],
        mev::finding_id(mev::DetectorClass::UnregisteredLaneBlock, "acme:ACME.9.Z")
    );
}

#[test]
fn a_second_write_adds_nothing() {
    let tmp = build_planted_fixture();
    let first = run_mev(&["graph-findings", ".", "--write"], tmp.path());
    assert!(!first.status.success());

    let state_path = tmp.path().join("planning/state.json");
    let after_first = fs::read(&state_path).unwrap();

    let second = run_mev(&["graph-findings", ".", "--write"], tmp.path());
    assert!(!second.status.success());

    let after_second = fs::read(&state_path).unwrap();
    assert_eq!(
        after_first, after_second,
        "a second --write must not duplicate the already-written finding"
    );

    let state: serde_json::Value = serde_json::from_slice(&after_second).unwrap();
    assert_eq!(state["carryover"].as_array().unwrap().len(), 1);
}

#[test]
fn same_missing_path_planted_in_two_repos_shares_one_finding_id() {
    // The `render-spec.py` case the block record requires to correlate: two
    // repos independently reference a missing script; --write appends one row
    // to EACH repo's own carryover[], but both rows share one finding_id.
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();

    let toml = r#"[vocab]
layer = ["brain", "engine", "factory", "console", "surface", "infra", "business", "content", "meta"]
status = ["active", "draft", "deprecated", "superseded", "archived"]

[crawl]
skip_dirs = ["target", "node_modules", ".git"]

[[repos]]
slug = "repo-a"
tier = "_root"
repo_path = "repo-a"
status_file = "planning/status.md"
cache_doc = "docs/projects/repo-a.md"
heading = "Repo A"

[[repos]]
slug = "repo-b"
tier = "_root"
repo_path = "repo-b"
status_file = "planning/status.md"
cache_doc = "docs/projects/repo-b.md"
heading = "Repo B"
"#;
    write_file(root, "brain.toml", toml);

    for repo in ["repo-a", "repo-b"] {
        let state = serde_json::json!({
            "repo": repo,
            "kind": "project",
            "updated": "2026-08-22",
            "focus": { "now": [], "next": [], "blocked": [] },
            "tracks": []
        });
        write_json(root, &format!("{repo}/planning/state.json"), &state);
        write_file(
            root,
            &format!("{repo}/.claude/commands/example.md"),
            "Run `scripts/render_spec.py --check` before committing.\n",
        );
    }

    let output = run_mev(&["graph-findings", ".", "--write"], root);
    assert!(!output.status.success());

    let mut ids = Vec::new();
    for repo in ["repo-a", "repo-b"] {
        let state_path = root.join(repo).join("planning/state.json");
        let state: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&state_path).unwrap()).unwrap();
        let carryover = state["carryover"].as_array().unwrap();
        assert_eq!(
            carryover.len(),
            1,
            "repo {repo}: expected one appended entry"
        );
        ids.push(carryover[0]["finding_id"].as_str().unwrap().to_string());
    }
    assert_eq!(ids[0], ids[1], "the two repos must share one finding_id");
}

// ---------------------------------------------------------------------------
// Detector 3: dangling-related-edge (MV.ticket.dangling-related-edge-detector)
// ---------------------------------------------------------------------------
//
// A separate two-repo fixture family: `mev graph-findings` reuses the exact
// resolution `bastion validate-brain --graph` (`check_graph`) performs, so
// these fixtures build real OKF markdown under `docs/` (the corpus
// membership rule) with `doc_id`/`related:` frontmatter rather than planting
// lane/command files. Two `[[repos]]` entries ("acme", "widgets") are needed
// to exercise the repoint (a) and ambiguous (c) shapes, which both require a
// candidate owned by a scope OTHER than the referrer's own.

fn write_two_repo_brain_toml(root: &Path) {
    let toml = r#"[vocab]
layer = ["brain", "engine", "factory", "console", "surface", "infra", "business", "content", "meta"]
status = ["active", "draft", "deprecated", "superseded", "archived"]

[crawl]
skip_dirs = ["target", "node_modules", ".git"]

[[repos]]
slug = "acme"
tier = "_root"
repo_path = "acme"
status_file = "planning/status.md"
cache_doc = "docs/projects/acme.md"
heading = "Acme"

[[repos]]
slug = "widgets"
tier = "_root"
repo_path = "widgets"
status_file = "planning/status.md"
cache_doc = "docs/projects/widgets.md"
heading = "Widgets"
"#;
    write_file(root, "brain.toml", toml);
}

fn write_okf_doc(root: &Path, rel: &str, doc_id: &str, related: &[&str]) {
    let related_yaml = if related.is_empty() {
        String::new()
    } else {
        let items: String = related
            .iter()
            .map(|r| format!("  - \"{r}\"\n"))
            .collect::<Vec<_>>()
            .join("");
        format!("related:\n{items}")
    };
    let content = format!(
        "---\ntype: Reference\ntitle: {doc_id}\ndescription: fixture\ndoc_id: {doc_id}\n{related_yaml}---\n"
    );
    write_file(root, rel, &content);
}

#[test]
fn dangling_related_edge_unqualified_single_candidate_carries_the_repoint() {
    // Shape (a): acme's doc references bare `carryover-plan`, which resolves
    // in exactly one OTHER scope (widgets) -- the suggested `scope:doc_id`
    // repoint IS the fix.
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    write_two_repo_brain_toml(root);
    write_okf_doc(root, "acme/docs/a.md", "alpha", &["carryover-plan"]);
    write_okf_doc(root, "widgets/docs/owner.md", "carryover-plan", &[]);

    let output = run_mev(&["graph-findings", ".", "--json"], root);
    assert!(!output.status.success(), "expected a finding to fire");
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("--json output must parse");
    assert_eq!(value["dangling_related_edge"], 1, "report: {value}");

    let findings = value["findings"].as_array().unwrap();
    let row = findings
        .iter()
        .find(|f| f["detector"] == "dangling-related-edge")
        .expect("expected a dangling-related-edge row");
    assert_eq!(row["repo"], "acme");
    let message = row["message"].as_str().unwrap();
    assert!(
        message.contains("did you mean `widgets:carryover-plan`?"),
        "message must carry the repoint fix: {message}"
    );
}

#[test]
fn dangling_related_edge_absent_target_carries_no_repoint() {
    // Shape (b): the target does not exist anywhere in the corpus -- no
    // repoint to suggest.
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    write_two_repo_brain_toml(root);
    write_okf_doc(root, "acme/docs/a.md", "alpha", &["typo-nonexistent"]);

    let output = run_mev(&["graph-findings", ".", "--json"], root);
    assert!(!output.status.success(), "expected a finding to fire");
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("--json output must parse");
    assert_eq!(value["dangling_related_edge"], 1, "report: {value}");

    let findings = value["findings"].as_array().unwrap();
    let row = findings
        .iter()
        .find(|f| f["detector"] == "dangling-related-edge")
        .expect("expected a dangling-related-edge row");
    let message = row["message"].as_str().unwrap();
    assert!(
        !message.contains("did you mean") && !message.contains("matches multiple scopes"),
        "an absent target must carry no repoint: {message}"
    );
}

#[test]
fn dangling_related_edge_explicitly_qualified_unresolvable_target_is_shape_b_not_a() {
    // An explicitly qualified `scope:doc_id` that does not resolve must land
    // in shape (b) even though another scope happens to own the bare
    // doc_id -- graph.rs never second-guesses an authored prefix, and
    // neither does this detector.
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    write_two_repo_brain_toml(root);
    write_okf_doc(root, "acme/docs/a.md", "alpha", &["acme:nope"]);
    write_okf_doc(root, "widgets/docs/owner.md", "nope", &[]);

    let output = run_mev(&["graph-findings", ".", "--json"], root);
    assert!(!output.status.success(), "expected a finding to fire");
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("--json output must parse");
    assert_eq!(value["dangling_related_edge"], 1, "report: {value}");

    let findings = value["findings"].as_array().unwrap();
    let row = findings
        .iter()
        .find(|f| f["detector"] == "dangling-related-edge")
        .expect("expected a dangling-related-edge row");
    let message = row["message"].as_str().unwrap();
    assert!(
        !message.contains("did you mean") && !message.contains("matches multiple scopes"),
        "an explicitly qualified, unresolvable target must never gain a suggestion: {message}"
    );
}

#[test]
fn dangling_related_edge_multiple_scopes_names_every_candidate() {
    // Shape (c): the bare target matches candidates in BOTH other scopes.
    // Three repos: acme (referrer), widgets and gadgets (both owning
    // "shared-target").
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    let toml = r#"[vocab]
layer = ["brain", "engine", "factory", "console", "surface", "infra", "business", "content", "meta"]
status = ["active", "draft", "deprecated", "superseded", "archived"]

[crawl]
skip_dirs = ["target", "node_modules", ".git"]

[[repos]]
slug = "acme"
tier = "_root"
repo_path = "acme"
status_file = "planning/status.md"
cache_doc = "docs/projects/acme.md"
heading = "Acme"

[[repos]]
slug = "widgets"
tier = "_root"
repo_path = "widgets"
status_file = "planning/status.md"
cache_doc = "docs/projects/widgets.md"
heading = "Widgets"

[[repos]]
slug = "gadgets"
tier = "_root"
repo_path = "gadgets"
status_file = "planning/status.md"
cache_doc = "docs/projects/gadgets.md"
heading = "Gadgets"
"#;
    write_file(root, "brain.toml", toml);
    write_okf_doc(root, "acme/docs/a.md", "alpha", &["shared-target"]);
    write_okf_doc(root, "widgets/docs/owner.md", "shared-target", &[]);
    write_okf_doc(root, "gadgets/docs/owner.md", "shared-target", &[]);

    let output = run_mev(&["graph-findings", ".", "--json"], root);
    assert!(!output.status.success(), "expected a finding to fire");
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("--json output must parse");
    assert_eq!(value["dangling_related_edge"], 1, "report: {value}");

    let findings = value["findings"].as_array().unwrap();
    let row = findings
        .iter()
        .find(|f| f["detector"] == "dangling-related-edge")
        .expect("expected a dangling-related-edge row");
    let message = row["message"].as_str().unwrap();
    assert!(
        message.contains("matches multiple scopes")
            && message.contains("widgets:shared-target")
            && message.contains("gadgets:shared-target"),
        "message must name every candidate: {message}"
    );
    assert!(
        !message.contains("did you mean"),
        "an ambiguous match must not phrase a single 'did you mean': {message}"
    );
}

#[test]
fn dangling_related_edge_clean_corpus_produces_zero_findings() {
    // The positive control: every `related:` edge resolves, so an empty
    // result is distinguishable from a detector that never ran.
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    write_two_repo_brain_toml(root);
    write_okf_doc(root, "acme/docs/a.md", "alpha", &["widgets:target"]);
    write_okf_doc(root, "widgets/docs/owner.md", "target", &[]);

    let output = run_mev(&["graph-findings", ".", "--json"], root);
    assert!(
        output.status.success(),
        "expected exit 0 on a clean corpus; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("--json output must parse");
    assert_eq!(value["total"], 0, "report: {value}");
    assert_eq!(value["dangling_related_edge"], 0, "report: {value}");
}

#[test]
fn dangling_related_edge_finding_id_is_stable_across_two_runs_from_different_working_dirs() {
    // finding_id must be content-derived: two independent `mev graph-findings`
    // invocations over byte-identical fixture corpora built in two different
    // temp directories (standing in for "two different working directories")
    // must agree on the same edge's finding_id.
    let tmp_a = tempfile::tempdir().expect("tempdir");
    let tmp_b = tempfile::tempdir().expect("tempdir");
    for root in [tmp_a.path(), tmp_b.path()] {
        write_two_repo_brain_toml(root);
        write_okf_doc(root, "acme/docs/a.md", "alpha", &["typo-nonexistent"]);
    }

    let out_a = run_mev(&["graph-findings", ".", "--json"], tmp_a.path());
    let out_b = run_mev(&["graph-findings", ".", "--json"], tmp_b.path());
    assert!(!out_a.status.success());
    assert!(!out_b.status.success());

    let val_a: serde_json::Value = serde_json::from_slice(&out_a.stdout).unwrap();
    let val_b: serde_json::Value = serde_json::from_slice(&out_b.stdout).unwrap();

    let id_a = val_a["findings"][0]["finding_id"].as_str().unwrap();
    let id_b = val_b["findings"][0]["finding_id"].as_str().unwrap();
    assert_eq!(
        id_a, id_b,
        "the same edge must yield the same finding_id across two independent runs"
    );

    // And matches the library function directly, over the normalized subject
    // this detector uses (`{from}->{to_ref}`).
    assert_eq!(
        id_a,
        mev::finding_id(
            mev::DetectorClass::DanglingRelatedEdge,
            "acme:alpha->typo-nonexistent"
        )
    );
}
