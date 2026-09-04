//! Integration tests for `mev::create_block` — the `mev create-block --from
//! <file>` driver (MV.14.B, Task 4).
//!
//! `src/brain/block_create.rs` already carries unit-level coverage over
//! `validate_payload` (the pure payload checks) in its own `#[cfg(test)]`
//! module. What has no coverage below this file is the **assembly**: config
//! -> state discovery -> `plan_create_block` -> `apply_plan` -> the chained
//! scoped `emit_state` — i.e. `mev::create_block` run against a real on-disk
//! corpus with at least two registered repos, exactly the seam
//! `tests/it/blocks_driver.rs` and `tests/it/emit_state_scope.rs` cover for
//! `mev::blocks_brain` and `mev::emit_state`.
//!
//! `block.schema.json`'s 15 required fields, verified against
//! `base-template/.claude/workflows/block.schema.json` on 2026-09-02:
//! id, repo, kind, title, description, what, why, sdlc_workflow, model,
//! files, out_of_scope, acceptance_criteria, spec_dir, created, updated.
//! `additionalProperties: false`.

use std::fs;
use std::path::{Path, PathBuf};

use mev::brain::block_create::{AcceptanceCriterion, BlockFiles, CreateBlockPayload};

fn temp_dir(tag: &str) -> PathBuf {
    let d = mev::testsupport::unique_temp_dir(&format!("mev-brain-block-create-it-{tag}"));
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

fn exists(root: &Path, rel: &str) -> bool {
    root.join(rel).exists()
}

// ---------------------------------------------------------------------------
// Fixture: HQ root + two registered leaf repos ("alpha", "beta"), each
// carrying one seed block at wave 10 so wave-allocation rules have a
// non-zero baseline, plus a status.md + cache doc per repo so the chained
// `emit_state` on a successful `--write` has real derived surfaces and
// produces no spurious diagnostics.
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
            "updated": "2026-09-02",
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
                description: HQ operating board fixture for create-block coverage.\n\
                ---\n\n\
                # HQ Status\n\n\
                <!-- BEGIN generated:hq-board -->\n\
                <!-- END generated:hq-board -->\n";
    write_file(root, "planning/status.md", doc);
}

fn write_leaf_state(root: &Path, repo_slug: &str, block_id: &str) {
    // A real repo's `planning/blocks/` directory already exists (it holds
    // every other block's record); create it here too so `write_atomic` —
    // which writes its temp file into the destination's own directory and
    // does not create parents — has somewhere to land a brand-new record.
    fs::create_dir_all(root.join(format!("repos/{repo_slug}/planning/blocks"))).unwrap();
    write_json(
        root,
        &format!("repos/{repo_slug}/planning/state.json"),
        &serde_json::json!({
            "repo": repo_slug,
            "kind": "project",
            "updated": "2026-09-02",
            "focus": { "now": [], "next": [], "blocked": [] },
            "tracks": [{
                "title": "Phase 1",
                "blocks": [
                    {
                        "id": block_id,
                        "title": "Seed block",
                        "status": "open",
                        "wave": 10
                    }
                ]
            }]
        }),
    );
}

fn write_leaf_status_md(root: &Path, repo_slug: &str) {
    let doc = format!(
        "---\n\
         type: ProjectStatus\n\
         title: {repo_slug} status\n\
         description: Status fixture for {repo_slug}.\n\
         timestamp: \"2026-09-02T12:00:00Z\"\n\
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

fn write_corpus(root: &Path) {
    write_brain_toml(root);
    write_hq_state(root);
    write_hq_status_md(root);

    write_leaf_state(root, "alpha", "AL.1.A");
    write_leaf_status_md(root, "alpha");
    write_project_cache_doc(root, "alpha");

    write_leaf_state(root, "beta", "BE.1.A");
    write_leaf_status_md(root, "beta");
    write_project_cache_doc(root, "beta");
}

// ---------------------------------------------------------------------------
// Payload builders — every test mutates a field off of one of these rather
// than repeating the whole literal.
// ---------------------------------------------------------------------------

fn legal_block_payload(id: &str, phase: i64) -> CreateBlockPayload {
    CreateBlockPayload {
        id: id.to_string(),
        repo: "alpha".to_string(),
        kind: "block".to_string(),
        title: "A created block".to_string(),
        description: "A block filed by an integration test.".to_string(),
        what: "Does the thing the test needs done.".to_string(),
        why: "Because the test needs a legal payload to file.".to_string(),
        sdlc_workflow: "task".to_string(),
        model: "sonnet".to_string(),
        phase: Some(phase),
        initiative: None,
        workflow_rationale: None,
        files: BlockFiles::default(),
        interfaces: Vec::new(),
        out_of_scope: vec!["Everything else.".to_string()],
        acceptance_criteria: vec![AcceptanceCriterion::Simple("It works.".to_string())],
        testing_strategy: None,
        validation_commands: Vec::new(),
        depends_on: Vec::new(),
        carryover_context: Vec::new(),
        related: Vec::new(),
        notes: None,
        forward_looking: false,
        epics: vec!["test-epic".to_string()],
        origin: None,
    }
}

fn legal_ticket_payload(id: &str) -> CreateBlockPayload {
    let mut p = legal_block_payload(id, 0);
    p.kind = "ticket".to_string();
    p.phase = None;
    p.testing_strategy = Some("Covered by an integration test.".to_string());
    p
}

fn errors_only(report: &mev::Report) -> Vec<&mev::Diagnostic> {
    report
        .diagnostics
        .iter()
        .filter(|d| d.severity == mev::Severity::Error)
        .collect()
}

// ---------------------------------------------------------------------------
// (1) Dry-run writes nothing at all.
// ---------------------------------------------------------------------------

#[test]
fn dry_run_writes_nothing() {
    let dir = temp_dir("dry-run");
    write_corpus(&dir);

    let payload = legal_block_payload("AL.9.A", 9);
    let report = mev::create_block(&dir, &payload, false, None).expect("dry-run should not error");
    assert!(
        errors_only(&report).is_empty(),
        "dry-run of a legal payload should have no errors; got {:#?}",
        errors_only(&report)
    );

    assert!(
        !exists(&dir, "repos/alpha/planning/blocks/AL.9.A.json"),
        "dry-run must not write the block record"
    );
    let state = read_json(&dir, "repos/alpha/planning/state.json");
    let ids: Vec<&str> = state["tracks"][0]["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        vec!["AL.1.A"],
        "dry-run must not register the new block in state.json"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// (2) `--write` writes a block record that validates against every one of
//     the 15 required fields, carries no extra keys, and derives `spec_dir`.
// ---------------------------------------------------------------------------

#[test]
fn write_creates_a_schema_valid_record_with_all_required_fields_and_no_extra_keys() {
    let dir = temp_dir("schema-valid");
    write_corpus(&dir);

    let payload = legal_block_payload("AL.9.B", 9);
    let report = mev::create_block(&dir, &payload, true, None).expect("write should not error");
    assert!(
        errors_only(&report).is_empty(),
        "write of a legal payload should have no errors; got {:#?}",
        errors_only(&report)
    );

    let record = read_json(&dir, "repos/alpha/planning/blocks/AL.9.B.json");
    let map = record.as_object().expect("record is a JSON object");

    let required = [
        "id",
        "repo",
        "kind",
        "title",
        "description",
        "what",
        "why",
        "sdlc_workflow",
        "model",
        "files",
        "out_of_scope",
        "acceptance_criteria",
        "spec_dir",
        "created",
        "updated",
    ];
    for field in required {
        assert!(
            map.contains_key(field),
            "record is missing required field '{field}'; record: {record:#?}"
        );
    }

    // additionalProperties: false — every key present must be a schema
    // property. `phase` is legal here since kind="block".
    let allowed: std::collections::HashSet<&str> = [
        "id",
        "repo",
        "kind",
        "phase",
        "initiative",
        "title",
        "description",
        "what",
        "why",
        "sdlc_workflow",
        "model",
        "workflow_rationale",
        "files",
        "interfaces",
        "out_of_scope",
        "acceptance_criteria",
        "testing_strategy",
        "validation_commands",
        "depends_on",
        "carryover_context",
        "origin",
        "forward_looking",
        "spec_dir",
        "related",
        "notes",
        "created",
        "updated",
        "closed",
        "commit",
    ]
    .into_iter()
    .collect();
    for key in map.keys() {
        assert!(
            allowed.contains(key.as_str()),
            "record carries key '{key}', which is not in block.schema.json's property set — \
             additionalProperties is false"
        );
    }

    assert_eq!(
        record["spec_dir"], "planning/AL.9.B/",
        "spec_dir must be derived as planning/<BlockID>/, never taken from the payload"
    );
    assert_eq!(record["id"], "AL.9.B");
    assert_eq!(record["repo"], "alpha");

    // `epics` has no property in block.schema.json — it must never appear on
    // the block record itself.
    assert!(
        !map.contains_key("epics"),
        "block record must never carry 'epics' — it is state.json-registration-only"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// (3) The state.json registration carries the authored `epics`; a payload
//     with no epic is refused and nothing is written.
// ---------------------------------------------------------------------------

#[test]
fn state_registration_carries_explicit_epics() {
    let dir = temp_dir("epics-present");
    write_corpus(&dir);

    let payload = legal_block_payload("AL.9.C", 9);
    mev::create_block(&dir, &payload, true, None).expect("write should not error");

    let state = read_json(&dir, "repos/alpha/planning/state.json");
    let created = state["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|t| t["blocks"].as_array().unwrap())
        .find(|b| b["id"] == "AL.9.C")
        .expect("created block must be registered in state.json");
    assert_eq!(
        created["epics"],
        serde_json::json!(["test-epic"]),
        "state.json registration must carry the payload's authored epics"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn payload_with_no_epics_is_refused_and_nothing_is_written() {
    let dir = temp_dir("epics-missing");
    write_corpus(&dir);

    let mut payload = legal_block_payload("AL.9.D", 9);
    payload.epics = Vec::new();
    let report = mev::create_block(&dir, &payload, true, None).expect("call should not error");

    let errs = errors_only(&report);
    assert!(
        errs.iter()
            .any(|d| d.locator == mev::brain::block_create::E_BLOCK_CREATE_MISSING_EPICS),
        "expected E_BLOCK_CREATE_MISSING_EPICS, got {errs:#?}"
    );
    assert!(
        !exists(&dir, "repos/alpha/planning/blocks/AL.9.D.json"),
        "a refused create must write no block record"
    );
    let state = read_json(&dir, "repos/alpha/planning/state.json");
    assert!(
        !state["tracks"][0]["blocks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|b| b["id"] == "AL.9.D"),
        "a refused create must not register anything in state.json"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// (4) Enum rejection — `model: "opus"`, an out-of-vocabulary `sdlc_workflow`,
//     and an out-of-vocabulary `kind` are all refused with nothing written.
// ---------------------------------------------------------------------------

#[test]
fn model_opus_is_refused_and_nothing_is_written() {
    let dir = temp_dir("model-opus");
    write_corpus(&dir);

    let mut payload = legal_block_payload("AL.9.E", 9);
    payload.model = "opus".to_string();
    let report = mev::create_block(&dir, &payload, true, None).expect("call should not error");

    let errs = errors_only(&report);
    assert!(
        errs.iter()
            .any(|d| d.locator == mev::brain::block_create::E_BLOCK_CREATE_MODEL_ENUM),
        "expected E_BLOCK_CREATE_MODEL_ENUM for model 'opus', got {errs:#?}"
    );
    assert!(!exists(&dir, "repos/alpha/planning/blocks/AL.9.E.json"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn out_of_vocabulary_sdlc_workflow_is_refused_and_nothing_is_written() {
    let dir = temp_dir("sdlc-workflow-enum");
    write_corpus(&dir);

    let mut payload = legal_block_payload("AL.9.F", 9);
    payload.sdlc_workflow = "orchestrate".to_string();
    let report = mev::create_block(&dir, &payload, true, None).expect("call should not error");

    let errs = errors_only(&report);
    assert!(
        errs.iter()
            .any(|d| d.locator == mev::brain::block_create::E_BLOCK_CREATE_SDLC_WORKFLOW_ENUM),
        "expected E_BLOCK_CREATE_SDLC_WORKFLOW_ENUM, got {errs:#?}"
    );
    assert!(!exists(&dir, "repos/alpha/planning/blocks/AL.9.F.json"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn out_of_vocabulary_kind_is_refused_and_nothing_is_written() {
    let dir = temp_dir("kind-enum");
    write_corpus(&dir);

    let mut payload = legal_block_payload("AL.9.G", 9);
    payload.kind = "epic".to_string();
    let report = mev::create_block(&dir, &payload, true, None).expect("call should not error");

    let errs = errors_only(&report);
    assert!(
        errs.iter()
            .any(|d| d.locator == mev::brain::block_create::E_BLOCK_CREATE_KIND_ENUM),
        "expected E_BLOCK_CREATE_KIND_ENUM, got {errs:#?}"
    );
    assert!(!exists(&dir, "repos/alpha/planning/blocks/AL.9.G.json"));

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// (5) A dangling `depends_on` target is refused, naming the unresolved
//     `(repo, id)`, with nothing written.
// ---------------------------------------------------------------------------

#[test]
fn dangling_dependency_is_refused_and_names_the_unresolved_target() {
    let dir = temp_dir("dangling-dep");
    write_corpus(&dir);

    let mut payload = legal_block_payload("AL.9.H", 9);
    payload.depends_on = vec![okf_core::BlockedBy::Block(okf_core::BlockDep {
        repo: "alpha".to_string(),
        id: "AL.99.ZZZ".to_string(),
        what: None,
    })];
    let report = mev::create_block(&dir, &payload, true, None).expect("call should not error");

    let errs = errors_only(&report);
    let hit = errs
        .iter()
        .find(|d| d.locator == mev::brain::block_create::E_BLOCK_CREATE_DANGLING_DEPENDENCY)
        .unwrap_or_else(|| panic!("expected E_BLOCK_CREATE_DANGLING_DEPENDENCY, got {errs:#?}"));
    assert!(
        hit.message.contains("alpha") && hit.message.contains("AL.99.ZZZ"),
        "diagnostic must name the unresolved (repo, id); got: {}",
        hit.message
    );
    assert!(!exists(&dir, "repos/alpha/planning/blocks/AL.9.H.json"));

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// (6) Dependency-before-dependent ordering, asserted by the test.
// ---------------------------------------------------------------------------

#[test]
fn creating_the_dependency_first_then_the_dependent_succeeds_in_that_order() {
    let dir = temp_dir("order-dep-first");
    write_corpus(&dir);

    let dependency = legal_block_payload("AL.9.I", 9);
    let dep_report =
        mev::create_block(&dir, &dependency, true, None).expect("dependency write should run");
    assert!(
        errors_only(&dep_report).is_empty(),
        "the dependency's own create must succeed first; got {:#?}",
        errors_only(&dep_report)
    );
    assert!(exists(&dir, "repos/alpha/planning/blocks/AL.9.I.json"));

    let mut dependent = legal_block_payload("AL.9.J", 9);
    dependent.depends_on = vec![okf_core::BlockedBy::Block(okf_core::BlockDep {
        repo: "alpha".to_string(),
        id: "AL.9.I".to_string(),
        what: Some("needs the dependency filed first".to_string()),
    })];
    let dependent_report = mev::create_block(&dir, &dependent, true, None)
        .expect("dependent write should run now that the dependency exists");
    assert!(
        errors_only(&dependent_report).is_empty(),
        "the dependent must now succeed since its dependency already exists; got {:#?}",
        errors_only(&dependent_report)
    );
    assert!(exists(&dir, "repos/alpha/planning/blocks/AL.9.J.json"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn creating_the_dependent_before_the_dependency_is_refused() {
    let dir = temp_dir("order-dependent-first");
    write_corpus(&dir);

    let mut dependent = legal_block_payload("AL.9.K", 9);
    dependent.depends_on = vec![okf_core::BlockedBy::Block(okf_core::BlockDep {
        repo: "alpha".to_string(),
        id: "AL.9.NOT-YET-FILED".to_string(),
        what: None,
    })];
    let report = mev::create_block(&dir, &dependent, true, None)
        .expect("call should not error at the driver level");
    assert!(
        errors_only(&report)
            .iter()
            .any(|d| d.locator == mev::brain::block_create::E_BLOCK_CREATE_DANGLING_DEPENDENCY),
        "filing the dependent before its dependency exists must be refused"
    );
    assert!(!exists(&dir, "repos/alpha/planning/blocks/AL.9.K.json"));

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// (7) Wave allocation — `10 * phase` for a block; the next multiple of ten
//     past the repo's highest existing wave (never `max + 1`) for a ticket.
// ---------------------------------------------------------------------------

#[test]
fn block_wave_is_ten_times_phase() {
    let dir = temp_dir("wave-block");
    write_corpus(&dir);

    let payload = legal_block_payload("AL.7.A", 7);
    mev::create_block(&dir, &payload, true, None).expect("write should not error");

    let state = read_json(&dir, "repos/alpha/planning/state.json");
    let created = state["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|t| t["blocks"].as_array().unwrap())
        .find(|b| b["id"] == "AL.7.A")
        .expect("created block must be registered");
    assert_eq!(created["wave"], 70, "wave must be 10 * phase (10 * 7)");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn ticket_wave_is_next_multiple_of_ten_past_the_repos_highest_wave_not_max_plus_one() {
    let dir = temp_dir("wave-ticket");
    write_corpus(&dir);
    // Seed block AL.1.A already has wave 10 (write_leaf_state). Highest wave
    // in the repo is 10, so a ticket must land at wave 20 — NOT 11.

    let payload = legal_ticket_payload("AL.ticket.some-chore");
    mev::create_block(&dir, &payload, true, None).expect("write should not error");

    let state = read_json(&dir, "repos/alpha/planning/state.json");
    let created = state["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|t| t["blocks"].as_array().unwrap())
        .find(|b| b["id"] == "AL.ticket.some-chore")
        .expect("created ticket must be registered");
    assert_eq!(
        created["wave"], 20,
        "a ticket's wave must be the next multiple of ten past the repo's highest \
         existing wave (10), i.e. 20 — never max + 1 (11)"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// (8) `depends_on` parity between the block record and the state.json
//     registration: same (type, repo, id) triple, same gloss text, in both.
// ---------------------------------------------------------------------------

#[test]
fn depends_on_is_parity_matched_between_record_and_state_json() {
    let dir = temp_dir("depends-on-parity");
    write_corpus(&dir);

    let dependency = legal_block_payload("AL.9.L", 9);
    mev::create_block(&dir, &dependency, true, None).expect("dependency write should run");

    let mut dependent = legal_block_payload("AL.9.M", 9);
    dependent.depends_on = vec![okf_core::BlockedBy::Block(okf_core::BlockDep {
        repo: "alpha".to_string(),
        id: "AL.9.L".to_string(),
        what: Some("gloss text".to_string()),
    })];
    let report =
        mev::create_block(&dir, &dependent, true, None).expect("dependent write should run");
    assert!(
        errors_only(&report).is_empty(),
        "{:#?}",
        errors_only(&report)
    );

    let record = read_json(&dir, "repos/alpha/planning/blocks/AL.9.M.json");
    let record_edge = &record["depends_on"][0];
    assert_eq!(record_edge["type"], "block");
    assert_eq!(record_edge["repo"], "alpha");
    assert_eq!(record_edge["id"], "AL.9.L");
    assert_eq!(
        record_edge["why"], "gloss text",
        "the record glosses a block-type edge with 'why'"
    );

    let state = read_json(&dir, "repos/alpha/planning/state.json");
    let created = state["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|t| t["blocks"].as_array().unwrap())
        .find(|b| b["id"] == "AL.9.M")
        .expect("dependent must be registered");
    let state_edge = &created["depends_on"][0];
    assert_eq!(state_edge["type"], "block");
    assert_eq!(state_edge["repo"], "alpha");
    assert_eq!(state_edge["id"], "AL.9.L");
    assert_eq!(
        state_edge["what"], "gloss text",
        "state.json glosses the same edge with 'what' — same (type, repo, id) \
         triple and gloss text as the record, just under a different key name"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// (9) An existing block id is a no-op refusal, never an overwrite — the
//     pre-existing record and state.json are byte-unchanged.
// ---------------------------------------------------------------------------

#[test]
fn existing_id_is_a_no_op_refusal_not_an_overwrite() {
    let dir = temp_dir("existing-id");
    write_corpus(&dir);

    let payload = legal_block_payload("AL.9.N", 9);
    mev::create_block(&dir, &payload, true, None).expect("first create should succeed");

    let record_before = fs::read(dir.join("repos/alpha/planning/blocks/AL.9.N.json")).unwrap();
    let state_before = fs::read(dir.join("repos/alpha/planning/state.json")).unwrap();

    // Attempt to re-create the SAME id with different content.
    let mut again = legal_block_payload("AL.9.N", 9);
    again.title = "A completely different title".to_string();
    let report = mev::create_block(&dir, &again, true, None).expect("call should not error");

    let errs = errors_only(&report);
    assert!(
        errs.iter()
            .any(|d| d.locator == mev::brain::block_create::E_BLOCK_CREATE_EXISTS),
        "expected E_BLOCK_CREATE_EXISTS, got {errs:#?}"
    );

    let record_after = fs::read(dir.join("repos/alpha/planning/blocks/AL.9.N.json")).unwrap();
    let state_after = fs::read(dir.join("repos/alpha/planning/state.json")).unwrap();
    assert_eq!(
        record_before, record_after,
        "the pre-existing block record must be byte-unchanged, not overwritten"
    );
    assert_eq!(
        state_before, state_after,
        "the pre-existing state.json must be byte-unchanged, not overwritten"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Cross-repo creation still resolves correctly with two independent repos —
// creating in "beta" must never touch "alpha"'s files.
// ---------------------------------------------------------------------------

#[test]
fn create_in_one_repo_never_touches_the_other_repos_files() {
    let dir = temp_dir("cross-repo-isolation");
    write_corpus(&dir);

    let alpha_before = fs::read(dir.join("repos/alpha/planning/state.json")).unwrap();

    // Scope the chained emit-state to beta only (MV.14.A's `--scope`) — an
    // unscoped write regenerates every repo's derived state.json, which
    // would make alpha's file change for reasons unrelated to this test.
    let config = mev::brain::config::load_brain_config(&dir.join("brain.toml")).unwrap();
    let scope = config
        .scope_dependencies("beta")
        .expect("beta is registered");

    let payload = legal_block_payload("BE.9.A", 9);
    let mut beta_payload = payload;
    beta_payload.repo = "beta".to_string();
    let report = mev::create_block(&dir, &beta_payload, true, Some(&scope))
        .expect("beta write should not error");
    assert!(
        errors_only(&report).is_empty(),
        "{:#?}",
        errors_only(&report)
    );

    assert!(exists(&dir, "repos/beta/planning/blocks/BE.9.A.json"));
    assert!(
        !exists(&dir, "repos/alpha/planning/blocks/BE.9.A.json"),
        "the block must be filed under its own repo (beta), never alpha"
    );
    let alpha_after = fs::read(dir.join("repos/alpha/planning/state.json")).unwrap();
    assert_eq!(
        alpha_before, alpha_after,
        "creating a block in beta must not touch alpha's state.json"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// demote-block / promote-block — `mev::brain::block_create::{demote_block,
// promote_block}`, the full driver assembly (config -> discover -> plan ->
// apply -> chained emit), same seam this file already covers for
// `mev::create_block`.
// ---------------------------------------------------------------------------

fn write_block_record(root: &Path, repo_slug: &str, id: &str) {
    write_json(
        root,
        &format!("repos/{repo_slug}/planning/blocks/{id}.json"),
        &serde_json::json!({
            "id": id,
            "repo": repo_slug,
            "kind": "block",
            "title": "Seed block",
            "description": "A seed block used by demote-block coverage.",
            "what": "Nothing, it is a fixture.",
            "why": "To give demote-block something real to park.",
            "sdlc_workflow": "task",
            "model": "sonnet",
            "phase": 1,
            "files": {},
            "out_of_scope": ["Everything else."],
            "acceptance_criteria": ["It exists."],
            "spec_dir": format!("planning/{id}/"),
            "created": "2026-09-02",
            "updated": "2026-09-02",
        }),
    );
}

#[test]
fn demote_block_dry_run_writes_nothing() {
    let dir = temp_dir("demote-dry-run");
    write_corpus(&dir);
    write_block_record(&dir, "alpha", "AL.1.A");

    let report = mev::brain::block_create::demote_block(&dir, "alpha:AL.1.A", false, None)
        .expect("dry-run should not error");
    assert!(
        errors_only(&report).is_empty(),
        "{:#?}",
        errors_only(&report)
    );

    let state = read_json(&dir, "repos/alpha/planning/state.json");
    let ids: Vec<&str> = state["tracks"][0]["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["AL.1.A"], "dry-run must not remove the block");
    assert!(
        state["backlog"].as_array().is_none_or(|a| a.is_empty()),
        "dry-run must not append a backlog entry"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn demote_block_write_parks_the_block_and_leaves_the_record_untouched() {
    let dir = temp_dir("demote-write");
    write_corpus(&dir);
    write_block_record(&dir, "alpha", "AL.1.A");
    let record_before = fs::read(dir.join("repos/alpha/planning/blocks/AL.1.A.json")).unwrap();

    let report = mev::brain::block_create::demote_block(&dir, "alpha:AL.1.A", true, None)
        .expect("write should not error");
    assert!(
        errors_only(&report).is_empty(),
        "{:#?}",
        errors_only(&report)
    );

    let state = read_json(&dir, "repos/alpha/planning/state.json");
    let ids: Vec<&str> = state["tracks"][0]["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["id"].as_str().unwrap())
        .collect();
    assert!(
        !ids.contains(&"AL.1.A"),
        "AL.1.A must be removed from tracks[]: {ids:?}"
    );
    let backlog = state["backlog"].as_array().expect("backlog[] present");
    let entry = backlog
        .iter()
        .find(|b| b["slug"] == "AL.1.A")
        .expect("a parked backlog entry for AL.1.A");
    assert_eq!(entry["status"], "parked");
    assert_eq!(entry["record"], "planning/blocks/AL.1.A.json");

    // AC2: the record is byte-identical — demote-block never touches it.
    let record_after = fs::read(dir.join("repos/alpha/planning/blocks/AL.1.A.json")).unwrap();
    assert_eq!(
        record_before, record_after,
        "planning/blocks/AL.1.A.json must be byte-identical before/after a --write demote"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn demote_block_missing_record_is_refused_and_writes_nothing() {
    let dir = temp_dir("demote-missing-record");
    write_corpus(&dir);
    // No write_block_record() call — AL.1.A has no record on disk.

    let report = mev::brain::block_create::demote_block(&dir, "alpha:AL.1.A", true, None)
        .expect("driver call itself should not error");
    assert_eq!(
        errors_only(&report)
            .iter()
            .map(|d| d.locator.as_str())
            .collect::<Vec<_>>(),
        vec!["E_DEMOTE_BLOCK_RECORD_MISSING"]
    );

    let state = read_json(&dir, "repos/alpha/planning/state.json");
    let ids: Vec<&str> = state["tracks"][0]["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        vec!["AL.1.A"],
        "a refused demote must not remove the block"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn demote_then_promote_round_trips_via_the_full_driver() {
    let dir = temp_dir("demote-promote-roundtrip");
    write_corpus(&dir);
    write_block_record(&dir, "alpha", "AL.1.A");

    // Canonicalize the hand-written fixture through one StateFile round trip
    // first, matching what `action_for` itself always writes (explicit
    // `null`s for absent-but-not-skipped `TrackBlock` fields) — otherwise
    // this comparison would fail on a difference in how the *fixture* was
    // authored, not on anything demote/promote actually did.
    let raw_before = fs::read_to_string(dir.join("repos/alpha/planning/state.json")).unwrap();
    let canonical: mev::brain::state::StateFile =
        serde_json::from_str(&raw_before).expect("fixture state.json parses");
    let mut canonical_content = serde_json::to_string_pretty(&canonical).unwrap();
    canonical_content.push('\n');
    fs::write(
        dir.join("repos/alpha/planning/state.json"),
        &canonical_content,
    )
    .unwrap();

    let original_state = read_json(&dir, "repos/alpha/planning/state.json");
    let original_block = original_state["tracks"][0]["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["id"] == "AL.1.A")
        .cloned()
        .expect("original AL.1.A row");

    let demote_report = mev::brain::block_create::demote_block(&dir, "alpha:AL.1.A", true, None)
        .expect("demote write should not error");
    assert!(
        errors_only(&demote_report).is_empty(),
        "{:#?}",
        errors_only(&demote_report)
    );

    let promote_report = mev::brain::block_create::promote_block(&dir, "alpha:AL.1.A", true, None)
        .expect("promote write should not error");
    assert!(
        errors_only(&promote_report).is_empty(),
        "{:#?}",
        errors_only(&promote_report)
    );

    let restored_state = read_json(&dir, "repos/alpha/planning/state.json");
    let restored_block = restored_state["tracks"][0]["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["id"] == "AL.1.A")
        .cloned()
        .expect("restored AL.1.A row");
    assert_eq!(
        restored_block, original_block,
        "demote-then-promote must restore the tracks[] row with no field lost"
    );

    let backlog_entry = restored_state["backlog"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["slug"] == "AL.1.A")
        .cloned()
        .expect("backlog entry retained, not deleted, after promotion");
    assert_eq!(backlog_entry["status"], "promoted");
    assert_eq!(backlog_entry["block"], "AL.1.A");
    assert!(
        backlog_entry.get("record").is_none(),
        "a promoted entry must not still carry the parked record pointer: {backlog_entry:#?}"
    );

    // The record file was never touched through either half of the round trip.
    assert!(exists(&dir, "repos/alpha/planning/blocks/AL.1.A.json"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn promote_block_not_parked_is_refused() {
    let dir = temp_dir("promote-not-parked");
    write_corpus(&dir);

    let report = mev::brain::block_create::promote_block(&dir, "alpha:AL.1.A", true, None)
        .expect("driver call itself should not error");
    assert_eq!(
        errors_only(&report)
            .iter()
            .map(|d| d.locator.as_str())
            .collect::<Vec<_>>(),
        vec!["E_BLOCK_NOT_FOUND"],
        "AL.1.A was never demoted, so there is no backlog slug to promote"
    );

    let _ = fs::remove_dir_all(&dir);
}
