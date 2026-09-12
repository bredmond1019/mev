//! Integration tests for `mev::graduate_carryover` / `mev::graduate_carryover_as`
//! — the `mev create-block --from <file> --graduate-carryover <repo>:<slug>`
//! driver (`MV.ticket.create-block-graduates-a-carryover`, Task 2).
//!
//! Adapted from `tests/it/brain_block_create.rs`'s two-repo corpus helpers:
//! this file adds a `carryover[]` entry (with a `blocks[]` edge onto an
//! existing target that also carries a `planning/blocks/<id>.json` record)
//! to the same shape of fixture, so `graduate_carryover` can be driven
//! end-to-end over a real on-disk corpus.

use std::fs;
use std::path::{Path, PathBuf};

use mev::brain::block_create::{AcceptanceCriterion, BlockFiles, CreateBlockPayload};

fn temp_dir(tag: &str) -> PathBuf {
    let d = mev::testsupport::unique_temp_dir(&format!("mev-brain-block-graduate-it-{tag}"));
    fs::create_dir_all(&d).unwrap();
    d
}

fn write_file(root: &Path, rel: &str, content: &str) {
    let target = root.join(rel);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&target, content.as_bytes()).unwrap();
}

fn write_json(root: &Path, rel: &str, value: &serde_json::Value) {
    write_file(root, rel, &serde_json::to_string_pretty(value).unwrap());
}

fn read_json(root: &Path, rel: &str) -> serde_json::Value {
    let raw = fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parse {rel} as JSON: {e}"))
}

fn read_raw(root: &Path, rel: &str) -> String {
    fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

fn exists(root: &Path, rel: &str) -> bool {
    root.join(rel).exists()
}

fn errors_only(report: &mev::Report) -> Vec<&mev::Diagnostic> {
    report
        .diagnostics
        .iter()
        .filter(|d| d.severity == mev::Severity::Error)
        .collect()
}

// ---------------------------------------------------------------------------
// Fixture: HQ root + two registered leaf repos ("alpha", "beta"). "alpha"
// carries the carryover entry to graduate (slug "leftover-thing"), a
// `blocks[]` edge onto its own seed block `AL.1.A` (status `open`, so the
// edge is `Blocking`), and an existing `planning/blocks/AL.1.A.json` record
// with an empty `depends_on` so the empty-array insertion path is exercised.
// ---------------------------------------------------------------------------

fn write_brain_toml(root: &Path) {
    let toml = r#"[vocab]
layer = ["brain", "engine", "factory", "console", "surface", "infra", "business", "content", "meta"]
status = ["active", "draft", "deprecated", "superseded", "archived"]

[crawl]
skip_dirs = ["target", "node_modules", ".git"]

[[repos]]
slug = "brain"
tier = "_root"
repo_path = "."
status_file = "planning/status.md"
cache_doc = "README.md"
heading = "Company Brain"

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

fn write_hq_state(root: &Path) {
    write_json(
        root,
        "planning/state.json",
        &serde_json::json!({
            "repo": "brain",
            "kind": "brain",
            "updated": "2026-09-12",
            "focus": { "now": [], "next": [], "blocked": [] },
            "repos": [],
            "cross_repo": []
        }),
    );
}

fn write_hq_status_md(root: &Path) {
    let doc = "---\n\
                type: ProjectStatus\n\
                title: HQ status\n\
                description: HQ operating board fixture for graduate-carryover coverage.\n\
                ---\n\n\
                # HQ Status\n\n\
                <!-- BEGIN generated:hq-board -->\n\
                <!-- END generated:hq-board -->\n";
    write_file(root, "planning/status.md", doc);
}

fn write_leaf_status_md(root: &Path, repo_slug: &str) {
    let doc = format!(
        "---\n\
         type: ProjectStatus\n\
         title: {repo_slug} status\n\
         description: Status fixture for {repo_slug}.\n\
         timestamp: \"2026-09-12T12:00:00Z\"\n\
         ---\n\n\
         # Status\n"
    );
    write_file(root, &format!("repos/{repo_slug}/planning/status.md"), &doc);
}

fn write_project_cache_doc(root: &Path, repo_slug: &str) {
    let doc = format!(
        "---\n\
         type: ProjectStatus\n\
         title: {repo_slug} cache\n\
         description: Project cache fixture for {repo_slug}.\n\
         ---\n\n\
         # {repo_slug}\n\n\
         <!-- BEGIN generated:project-cache -->\n\
         <!-- END generated:project-cache -->\n"
    );
    write_file(root, &format!("docs/projects/{repo_slug}.md"), &doc);
}

/// `alpha`'s `state.json`: one seed block `AL.1.A` (status configurable, so
/// tests can exercise both a `Blocking` and a non-blocking target), plus the
/// `carryover[]` entry to graduate — a `blocks[]` edge onto `AL.1.A`.
fn write_alpha_state(root: &Path, al_1_a_status: &str) {
    fs::create_dir_all(root.join("repos/alpha/planning/blocks")).unwrap();
    write_json(
        root,
        "repos/alpha/planning/state.json",
        &serde_json::json!({
            "repo": "alpha",
            "kind": "project",
            "updated": "2026-09-12",
            "focus": { "now": [], "next": [], "blocked": [] },
            "tracks": [{
                "title": "Phase 1",
                "blocks": [
                    {
                        "id": "AL.1.A",
                        "title": "Seed block",
                        "status": al_1_a_status,
                        "wave": 10
                    }
                ]
            }],
            "carryover": [
                {
                    "slug": "leftover-thing",
                    "scope": { "repo": "alpha", "tier": null, "cross_repo": null },
                    "kind": "deferred",
                    "text": "A carryover entry used only by this fixture.",
                    "created": "2026-09-12",
                    "blocks": [
                        { "type": "block", "repo": "alpha", "id": "AL.1.A" }
                    ]
                }
            ]
        }),
    );
}

/// `AL.1.A`'s own `planning/blocks/AL.1.A.json` record, with an empty
/// `depends_on` array — exercises the "expand an empty array" insertion
/// path.
fn write_al_1_a_record(root: &Path) {
    write_json(
        root,
        "repos/alpha/planning/blocks/AL.1.A.json",
        &serde_json::json!({
            "id": "AL.1.A",
            "repo": "alpha",
            "kind": "block",
            "phase": 1,
            "title": "Seed block",
            "description": "Seed block used as a graduation target.",
            "what": "Exists so a carryover can gate it.",
            "why": "Test fixture.",
            "sdlc_workflow": "task",
            "model": "sonnet",
            "files": { "modified": [], "new": [] },
            "out_of_scope": ["Everything else."],
            "acceptance_criteria": ["It works."],
            "depends_on": [],
            "forward_looking": false,
            "spec_dir": "planning/AL.1.A/",
            "created": "2026-09-02",
            "updated": "2026-09-02"
        }),
    );
}

fn write_beta_state(root: &Path) {
    fs::create_dir_all(root.join("repos/beta/planning/blocks")).unwrap();
    write_json(
        root,
        "repos/beta/planning/state.json",
        &serde_json::json!({
            "repo": "beta",
            "kind": "project",
            "updated": "2026-09-12",
            "focus": { "now": [], "next": [], "blocked": [] },
            "tracks": [{
                "title": "Phase 1",
                "blocks": [
                    { "id": "BE.1.A", "title": "Seed block", "status": "open", "wave": 10 }
                ]
            }]
        }),
    );
}

fn write_corpus(root: &Path) {
    write_brain_toml(root);
    write_hq_state(root);
    write_hq_status_md(root);

    write_alpha_state(root, "open");
    write_al_1_a_record(root);
    write_leaf_status_md(root, "alpha");
    write_project_cache_doc(root, "alpha");

    write_beta_state(root);
    write_leaf_status_md(root, "beta");
    write_project_cache_doc(root, "beta");
}

fn graduating_payload(id: &str, repo: &str) -> CreateBlockPayload {
    CreateBlockPayload {
        id: id.to_string(),
        repo: repo.to_string(),
        kind: "ticket".to_string(),
        title: "Graduated ticket".to_string(),
        description: "A ticket filed to graduate a carryover in an integration test.".to_string(),
        what: "Does the thing the test needs done.".to_string(),
        why: "Because the test needs a legal payload to file.".to_string(),
        sdlc_workflow: "task".to_string(),
        model: "sonnet".to_string(),
        phase: None,
        initiative: None,
        workflow_rationale: None,
        origin: None,
        files: BlockFiles::default(),
        interfaces: Vec::new(),
        out_of_scope: vec!["Everything else.".to_string()],
        acceptance_criteria: vec![AcceptanceCriterion::Simple("It works.".to_string())],
        testing_strategy: Some("Covered by an integration test.".to_string()),
        validation_commands: Vec::new(),
        depends_on: Vec::new(),
        carryover_context: Vec::new(),
        related: Vec::new(),
        notes: None,
        forward_looking: false,
        epics: vec!["test-epic".to_string()],
    }
}

// ---------------------------------------------------------------------------
// (1) Dry-run leaves every file in the corpus byte-identical.
// ---------------------------------------------------------------------------

#[test]
fn dry_run_is_byte_identical_everywhere() {
    let dir = temp_dir("dry-run");
    write_corpus(&dir);

    let before_alpha_state = read_raw(&dir, "repos/alpha/planning/state.json");
    let before_beta_state = read_raw(&dir, "repos/beta/planning/state.json");
    let before_al_record = read_raw(&dir, "repos/alpha/planning/blocks/AL.1.A.json");

    let payload = graduating_payload("BE.9.A", "beta");
    let report = mev::graduate_carryover(&dir, &payload, "alpha:leftover-thing", false, None)
        .expect("dry-run should not error");
    assert!(
        errors_only(&report).is_empty(),
        "dry-run of a legal graduation should have no errors; got {:#?}",
        errors_only(&report)
    );

    assert_eq!(
        before_alpha_state,
        read_raw(&dir, "repos/alpha/planning/state.json"),
        "dry-run must not touch alpha's state.json"
    );
    assert_eq!(
        before_beta_state,
        read_raw(&dir, "repos/beta/planning/state.json"),
        "dry-run must not touch beta's state.json"
    );
    assert_eq!(
        before_al_record,
        read_raw(&dir, "repos/alpha/planning/blocks/AL.1.A.json"),
        "dry-run must not touch the held target's record"
    );
    assert!(
        !exists(&dir, "repos/beta/planning/blocks/BE.9.A.json"),
        "dry-run must not write the new block record"
    );
    assert!(
        !exists(&dir, "repos/alpha/planning/carryover-archive.jsonl"),
        "dry-run must not create the archive file"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// (2) `--write` creates the block, moves the edge (state.json + record),
//     removes the carryover, and archives it with reason `promoted`.
// ---------------------------------------------------------------------------

#[test]
fn write_creates_block_moves_edge_and_archives_carryover() {
    let dir = temp_dir("write-full");
    write_corpus(&dir);
    let before_al_record = read_raw(&dir, "repos/alpha/planning/blocks/AL.1.A.json");

    let payload = graduating_payload("BE.9.B", "beta");
    let report = mev::graduate_carryover(&dir, &payload, "alpha:leftover-thing", true, None)
        .expect("write should not error");
    assert!(
        errors_only(&report).is_empty(),
        "write of a legal graduation should have no errors; got {:#?}",
        errors_only(&report)
    );

    // The new block was created in beta, with origin naming the carryover.
    let beta_state = read_json(&dir, "repos/beta/planning/state.json");
    let created = beta_state["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|t| t["blocks"].as_array().unwrap())
        .find(|b| b["id"] == "BE.9.B")
        .expect("created block must be registered in beta's state.json");
    assert_eq!(
        created["origin"],
        serde_json::json!({"type": "carryover", "slug": "leftover-thing"})
    );

    let record = read_json(&dir, "repos/beta/planning/blocks/BE.9.B.json");
    assert_eq!(
        record["origin"],
        serde_json::json!({"type": "carryover", "slug": "leftover-thing"})
    );

    // The held target (alpha:AL.1.A) gained the edge in state.json.
    let alpha_state = read_json(&dir, "repos/alpha/planning/state.json");
    let al_1_a = alpha_state["tracks"][0]["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["id"] == "AL.1.A")
        .unwrap();
    let depends_on = al_1_a["depends_on"].as_array().unwrap();
    assert_eq!(depends_on.len(), 1);
    assert_eq!(depends_on[0]["type"], "block");
    assert_eq!(depends_on[0]["repo"], "beta");
    assert_eq!(depends_on[0]["id"], "BE.9.B");

    // The carryover entry is gone.
    assert!(
        alpha_state["carryover"].as_array().unwrap().is_empty(),
        "graduated carryover must be removed from carryover[]; got {:#?}",
        alpha_state["carryover"]
    );

    // Exactly one archive row, reason promoted, evidence naming the block.
    let archive_raw = read_raw(&dir, "repos/alpha/planning/carryover-archive.jsonl");
    let lines: Vec<&str> = archive_raw
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect();
    assert_eq!(
        lines.len(),
        1,
        "expected exactly one archive row: {lines:#?}"
    );
    let row: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(row["slug"], "leftover-thing");
    assert_eq!(row["reason"], "promoted");
    assert_eq!(row["reconstructed"], false);
    assert!(
        row["evidence"].as_str().unwrap().contains("beta:BE.9.B"),
        "evidence must name the graduated-to block; got {:?}",
        row["evidence"]
    );

    // The held target's record gained the same edge; everything else about
    // the record (including key order) is unchanged — the insertion is a
    // surgical text edit, never a serde_json::Value round trip.
    let after_al_record = read_raw(&dir, "repos/alpha/planning/blocks/AL.1.A.json");
    assert_ne!(before_al_record, after_al_record);
    let al_record_value: serde_json::Value = serde_json::from_str(&after_al_record).unwrap();
    let record_depends_on = al_record_value["depends_on"].as_array().unwrap();
    assert_eq!(record_depends_on.len(), 1);
    assert_eq!(record_depends_on[0]["type"], "block");
    assert_eq!(record_depends_on[0]["repo"], "beta");
    assert_eq!(record_depends_on[0]["id"], "BE.9.B");
    assert_eq!(
        record_depends_on[0]["why"],
        "graduated from carryover alpha:leftover-thing"
    );
    // Every field other than depends_on is untouched, byte-for-byte: the
    // insertion is a surgical text edit, so the prefix up to the array's
    // opening `[` is copied verbatim, and every OTHER top-level field's own
    // "key": value text (found by key, not by position, since the inserted
    // element shifts every later byte offset) survives unchanged.
    let prefix_marker = "\"depends_on\": [";
    let prefix_end = before_al_record.find(prefix_marker).unwrap() + prefix_marker.len();
    assert_eq!(
        &before_al_record[..prefix_end],
        &after_al_record[..prefix_end],
        "everything up to and including 'depends_on: [' must be untouched"
    );
    for key in [
        "\"description\": \"Seed block used as a graduation target.\"",
        "\"phase\": 1",
        "\"title\": \"Seed block\"",
        "\"updated\": \"2026-09-02\"",
        "\"what\": \"Exists so a carryover can gate it.\"",
        "\"why\": \"Test fixture.\"",
    ] {
        assert!(
            after_al_record.contains(key),
            "record must keep '{key}' verbatim; got:\n{after_al_record}"
        );
    }

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// (3) An unknown carryover key writes nothing, even with `--write`.
// ---------------------------------------------------------------------------

#[test]
fn unknown_carryover_key_writes_nothing_even_with_write_true() {
    let dir = temp_dir("unknown-carryover");
    write_corpus(&dir);

    let before_alpha_state = read_raw(&dir, "repos/alpha/planning/state.json");
    let before_beta_state = read_raw(&dir, "repos/beta/planning/state.json");

    let payload = graduating_payload("BE.9.C", "beta");
    let report = mev::graduate_carryover(&dir, &payload, "alpha:no-such-slug", true, None)
        .expect("call should not error");

    let errs = errors_only(&report);
    assert!(
        errs.iter()
            .any(|d| d.locator == mev::brain::block_create::E_BLOCK_CREATE_UNKNOWN_CARRYOVER),
        "expected E_BLOCK_CREATE_UNKNOWN_CARRYOVER, got {errs:#?}"
    );

    assert_eq!(
        before_alpha_state,
        read_raw(&dir, "repos/alpha/planning/state.json")
    );
    assert_eq!(
        before_beta_state,
        read_raw(&dir, "repos/beta/planning/state.json")
    );
    assert!(!exists(&dir, "repos/beta/planning/blocks/BE.9.C.json"));
    assert!(!exists(
        &dir,
        "repos/alpha/planning/carryover-archive.jsonl"
    ));

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// (4) A mismatched origin writes nothing, even with `--write`.
// ---------------------------------------------------------------------------

#[test]
fn origin_mismatch_writes_nothing_even_with_write_true() {
    let dir = temp_dir("origin-mismatch");
    write_corpus(&dir);

    let before_alpha_state = read_raw(&dir, "repos/alpha/planning/state.json");
    let before_beta_state = read_raw(&dir, "repos/beta/planning/state.json");

    let mut payload = graduating_payload("BE.9.D", "beta");
    payload.origin = Some(serde_json::json!({"type": "carryover", "slug": "some-other-slug"}));
    let report = mev::graduate_carryover(&dir, &payload, "alpha:leftover-thing", true, None)
        .expect("call should not error");

    let errs = errors_only(&report);
    assert!(
        errs.iter()
            .any(|d| d.locator == mev::brain::block_create::E_BLOCK_CREATE_ORIGIN_MISMATCH),
        "expected E_BLOCK_CREATE_ORIGIN_MISMATCH, got {errs:#?}"
    );

    assert_eq!(
        before_alpha_state,
        read_raw(&dir, "repos/alpha/planning/state.json")
    );
    assert_eq!(
        before_beta_state,
        read_raw(&dir, "repos/beta/planning/state.json")
    );
    assert!(!exists(&dir, "repos/beta/planning/blocks/BE.9.D.json"));
    assert!(!exists(
        &dir,
        "repos/alpha/planning/carryover-archive.jsonl"
    ));

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// (5) A non-blocking target (closed) is skipped, not edged, and reported.
// ---------------------------------------------------------------------------

#[test]
fn closed_target_is_skipped_not_edged() {
    let dir = temp_dir("closed-target");
    write_brain_toml(&dir);
    write_hq_state(&dir);
    write_hq_status_md(&dir);
    write_alpha_state(&dir, "closed");
    write_al_1_a_record(&dir);
    write_leaf_status_md(&dir, "alpha");
    write_project_cache_doc(&dir, "alpha");
    write_beta_state(&dir);
    write_leaf_status_md(&dir, "beta");
    write_project_cache_doc(&dir, "beta");

    let payload = graduating_payload("BE.9.E", "beta");
    let report = mev::graduate_carryover(&dir, &payload, "alpha:leftover-thing", true, None)
        .expect("write should not error");
    assert!(
        errors_only(&report).is_empty(),
        "got {:#?}",
        errors_only(&report)
    );

    // The carryover is still removed and archived...
    let alpha_state = read_json(&dir, "repos/alpha/planning/state.json");
    assert!(alpha_state["carryover"].as_array().unwrap().is_empty());
    assert!(exists(&dir, "repos/alpha/planning/carryover-archive.jsonl"));

    // ...but the closed target gained no edge, in either state.json or its
    // own record.
    let al_1_a = alpha_state["tracks"][0]["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["id"] == "AL.1.A")
        .unwrap();
    assert!(
        al_1_a.get("depends_on").is_none() || al_1_a["depends_on"].as_array().unwrap().is_empty(),
        "a closed target must not gain the graduation edge; got {:#?}",
        al_1_a
    );
    let record = read_json(&dir, "repos/alpha/planning/blocks/AL.1.A.json");
    assert!(
        record["depends_on"].as_array().unwrap().is_empty(),
        "a closed target's own record must not gain the edge either; got {:#?}",
        record["depends_on"]
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// (6) The created record and registration match plain `create_block` output
//     for the same payload, apart from `origin`.
// ---------------------------------------------------------------------------

#[test]
fn created_record_matches_plain_create_block_apart_from_origin() {
    let plain_dir = temp_dir("plain-create");
    write_corpus(&plain_dir);
    let graduate_dir = temp_dir("graduate-create");
    write_corpus(&graduate_dir);

    let payload = graduating_payload("BE.9.F", "beta");

    mev::create_block(&plain_dir, &payload, true, None)
        .expect("plain create_block should not error");
    mev::graduate_carryover(&graduate_dir, &payload, "alpha:leftover-thing", true, None)
        .expect("graduate_carryover should not error");

    let mut plain_record = read_json(&plain_dir, "repos/beta/planning/blocks/BE.9.F.json");
    let mut graduated_record = read_json(&graduate_dir, "repos/beta/planning/blocks/BE.9.F.json");
    plain_record.as_object_mut().unwrap().remove("origin");
    graduated_record.as_object_mut().unwrap().remove("origin");
    assert_eq!(
        plain_record, graduated_record,
        "a graduated block's own record must match a plain create_block record apart from origin"
    );

    let plain_state = read_json(&plain_dir, "repos/beta/planning/state.json");
    let graduated_state = read_json(&graduate_dir, "repos/beta/planning/state.json");
    let plain_block = plain_state["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|t| t["blocks"].as_array().unwrap())
        .find(|b| b["id"] == "BE.9.F")
        .unwrap();
    let graduated_block = graduated_state["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|t| t["blocks"].as_array().unwrap())
        .find(|b| b["id"] == "BE.9.F")
        .unwrap();
    let mut plain_block = plain_block.clone();
    let mut graduated_block = graduated_block.clone();
    plain_block.as_object_mut().unwrap().remove("origin");
    graduated_block.as_object_mut().unwrap().remove("origin");
    assert_eq!(
        plain_block, graduated_block,
        "a graduated block's state.json registration must match a plain create_block \
         registration apart from origin"
    );

    let _ = fs::remove_dir_all(&plain_dir);
    let _ = fs::remove_dir_all(&graduate_dir);
}

// ---------------------------------------------------------------------------
// (7) Graduating into the carryover's own repo merges every mutation into
//     one, internally-consistent state.json.
// ---------------------------------------------------------------------------

#[test]
fn graduating_within_one_repo_applies_every_mutation() {
    let dir = temp_dir("same-repo");
    write_corpus(&dir);

    let payload = graduating_payload("AL.9.G", "alpha");
    let report = mev::graduate_carryover(&dir, &payload, "alpha:leftover-thing", true, None)
        .expect("write should not error");
    assert!(
        errors_only(&report).is_empty(),
        "got {:#?}",
        errors_only(&report)
    );

    let alpha_state = read_json(&dir, "repos/alpha/planning/state.json");
    let ids: Vec<&str> = alpha_state["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|t| t["blocks"].as_array().unwrap())
        .map(|b| b["id"].as_str().unwrap())
        .collect();
    assert!(
        ids.contains(&"AL.9.G"),
        "new block must be registered: {ids:?}"
    );
    assert!(alpha_state["carryover"].as_array().unwrap().is_empty());

    let al_1_a = alpha_state["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|t| t["blocks"].as_array().unwrap())
        .find(|b| b["id"] == "AL.1.A")
        .unwrap();
    let depends_on = al_1_a["depends_on"].as_array().unwrap();
    assert_eq!(depends_on.len(), 1);
    assert_eq!(depends_on[0]["repo"], "alpha");
    assert_eq!(depends_on[0]["id"], "AL.9.G");

    assert!(exists(&dir, "repos/alpha/planning/blocks/AL.9.G.json"));
    assert!(exists(&dir, "repos/alpha/planning/carryover-archive.jsonl"));

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// (8) CLI: `mev create-block --from <payload> --graduate-carryover <repo:slug>
//     <root>` without --write — dry-run over the real binary, matching the
//     pattern in tests/it/force_operator_gate.rs (env!("CARGO_BIN_EXE_mev")).
// ---------------------------------------------------------------------------

#[test]
fn cli_dry_run_prints_plan_and_leaves_fixture_byte_identical() {
    let dir = temp_dir("cli-dry-run");
    write_corpus(&dir);

    let before_alpha_state = read_raw(&dir, "repos/alpha/planning/state.json");
    let before_beta_state = read_raw(&dir, "repos/beta/planning/state.json");
    let before_al_record = read_raw(&dir, "repos/alpha/planning/blocks/AL.1.A.json");

    let payload = graduating_payload("BE.9.A", "beta");
    let payload_path = dir.join("payload.json");
    fs::write(
        &payload_path,
        serde_json::to_string_pretty(&payload).unwrap(),
    )
    .unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_mev"))
        .arg("create-block")
        .arg("--from")
        .arg(&payload_path)
        .arg("--graduate-carryover")
        .arg("alpha:leftover-thing")
        .arg(&dir)
        .current_dir(&dir)
        .output()
        .expect("failed to spawn mev binary");

    assert!(
        output.status.success(),
        "dry-run CLI invocation should exit 0; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("BE.9.A") || stdout.contains("beta"),
        "plan output should name the created block; got: {stdout}"
    );
    assert!(
        stdout.contains("AL.1.A") || stdout.contains("alpha"),
        "plan output should name the added edge's target; got: {stdout}"
    );

    assert_eq!(
        before_alpha_state,
        read_raw(&dir, "repos/alpha/planning/state.json"),
        "CLI dry-run must not touch alpha's state.json"
    );
    assert_eq!(
        before_beta_state,
        read_raw(&dir, "repos/beta/planning/state.json"),
        "CLI dry-run must not touch beta's state.json"
    );
    assert_eq!(
        before_al_record,
        read_raw(&dir, "repos/alpha/planning/blocks/AL.1.A.json"),
        "CLI dry-run must not touch the held target's record"
    );
    assert!(
        !exists(&dir, "repos/beta/planning/blocks/BE.9.A.json"),
        "CLI dry-run must not write the new block record"
    );
    assert!(
        !exists(&dir, "repos/alpha/planning/carryover-archive.jsonl"),
        "CLI dry-run must not create the archive file"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// (9) CLI: `create-block --help` lists --graduate-carryover (source-built
//     stand-in for the installed-binary acceptance criterion — installing the
//     binary fleet-wide is HQ.7.C's job, not this task's).
// ---------------------------------------------------------------------------

#[test]
fn cli_help_lists_graduate_carryover_flag() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_mev"))
        .arg("create-block")
        .arg("--help")
        .output()
        .expect("failed to spawn mev binary");

    assert!(output.status.success(), "--help should exit 0");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("--graduate-carryover"),
        "create-block --help must list --graduate-carryover; got: {stdout}"
    );
}
