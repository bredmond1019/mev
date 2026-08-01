//! Integration tests for `src/doc/opportunity.rs` — the Opportunity command
//! family (`ingest` / `set-stage` / `add-action` / `merge-contacts`) as
//! idempotent library planners (`MV.9.A` task 3).

use std::path::Path;

use mev::brain::emit::apply_plan;
use mev::doc::opportunity::{
    OpportunityKind, plan_add_action, plan_ingest, plan_merge_contacts, plan_set_stage,
};
use okf_core::{Opportunity, parse_nested_frontmatter};

fn fixture_brief() -> serde_json::Value {
    let raw =
        std::fs::read_to_string("tests/fixtures/company_brief.json").expect("fixture must exist");
    serde_json::from_str(&raw).expect("fixture must be valid JSON")
}

/// Every test writes into `<root>/business/docs/opportunities/` — ensure the
/// parent directory exists before any `apply_plan(&plan, true)` call. No
/// `index.md` is created here (the index-reconcile half is covered by
/// `tests/doc_index_reconcile.rs`), so every plan in this file carries
/// exactly one doc-write action, plus a `W_DOC_INDEX_MISSING` diagnostic.
fn opportunities_dir(root: &Path) -> std::path::PathBuf {
    let dir = root.join("business/docs/opportunities");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Mark `root` as a brain root — `find_brain_root` only checks for the
/// file's existence, so an empty `brain.toml` is sufficient.
fn write_brain_root(root: &Path) {
    std::fs::write(root.join("brain.toml"), "").unwrap();
}

/// The default seven-stage vocabulary, formatted exactly as the real
/// `business/docs/pipeline.md` production line — used by tests that don't
/// care about a non-default vocabulary but do need `resolve_stage_vocabulary`
/// to succeed.
const DEFAULT_STAGES_LINE: &str = "`identified` → `researching` → `contacted` → `conversation` → `proposal-sent` → `closed-won` → `closed-lost`";

/// Write `business/docs/pipeline.md` under `root` with a `## Stages` section
/// carrying `stages_line` as the vocabulary line.
fn write_pipeline_md(root: &Path, stages_line: &str) {
    let dir = root.join("business/docs");
    std::fs::create_dir_all(&dir).unwrap();
    let content = format!("# Pipeline\n\n## Stages\n\n{stages_line}\n");
    std::fs::write(dir.join("pipeline.md"), content).unwrap();
}

/// Full happy-path fixture: a brain root with the default seven-stage
/// `pipeline.md` in place.
fn default_pipeline_fixture(root: &Path) {
    write_brain_root(root);
    write_pipeline_md(root, DEFAULT_STAGES_LINE);
}

fn write_and_read(root: &Path, plan: mev::brain::emit::EmitPlan) -> String {
    let diags = apply_plan(&plan, true);
    assert!(
        diags.iter().all(|d| d.severity != mev::Severity::Error),
        "apply_plan produced errors: {diags:?}"
    );
    let path = root
        .join("business/docs/opportunities")
        .join("anthropic.md");
    std::fs::read_to_string(path).expect("file must exist after write")
}

// ── ingest ───────────────────────────────────────────────────────────────

#[test]
fn ingest_from_real_fixture_matches_contract() {
    let tmp = tempfile::tempdir().unwrap();
    opportunities_dir(tmp.path());
    let brief = fixture_brief();

    let plan = plan_ingest(&brief, Some(OpportunityKind::Company), tmp.path());
    assert_eq!(plan.actions.len(), 1, "expected the doc-write action");

    let content = write_and_read(tmp.path(), plan);

    // Required frontmatter fields present.
    assert!(content.contains("type: Opportunity"));
    assert!(content.contains("title: Anthropic"));
    assert!(content.contains("kind: company"));
    assert!(content.contains("stage: identified"));

    // Raw brief JSON is the first fenced `json` block in the body.
    let heading_pos = content
        .find("## Research Brief")
        .expect("must contain research brief heading");
    let fence_pos = content.find("```json").expect("must contain json fence");
    assert!(heading_pos < fence_pos);
    assert!(content.contains("\"company_name\": \"Anthropic\""));
}

#[test]
fn ingest_round_trips_through_frontmatter() {
    let tmp = tempfile::tempdir().unwrap();
    opportunities_dir(tmp.path());
    let brief = fixture_brief();

    let opp = Opportunity::from_company_brief(&brief);
    let plan = plan_ingest(&brief, Some(OpportunityKind::Company), tmp.path());
    let content = write_and_read(tmp.path(), plan);

    let fields = parse_nested_frontmatter(&content).expect("must parse frontmatter");
    let recovered = Opportunity::from_frontmatter(&fields).expect("must reconstruct");

    let mut expected = opp;
    expected.doc_id = recovered.doc_id.clone();
    assert_eq!(recovered.title, expected.title);
    assert_eq!(recovered.description, expected.description);
    assert_eq!(recovered.kind, expected.kind);
    assert_eq!(recovered.stage, expected.stage);
    assert_eq!(recovered.layer, expected.layer);
}

#[test]
fn ingest_kind_job_posting_lands_in_frontmatter() {
    let tmp = tempfile::tempdir().unwrap();
    opportunities_dir(tmp.path());
    let brief = fixture_brief();

    let plan = plan_ingest(&brief, Some(OpportunityKind::JobPosting), tmp.path());
    let content = write_and_read(tmp.path(), plan);
    assert!(content.contains("kind: job-posting"));
}

#[test]
fn ingest_auto_detects_company_shape() {
    let tmp = tempfile::tempdir().unwrap();
    opportunities_dir(tmp.path());
    let brief = fixture_brief();

    let plan = plan_ingest(&brief, None, tmp.path());
    assert_eq!(plan.actions.len(), 1);
    let content = write_and_read(tmp.path(), plan);
    assert!(content.contains("kind: company"));
}

#[test]
fn ingest_auto_detects_prospecting_sweep_shape() {
    let tmp = tempfile::tempdir().unwrap();
    opportunities_dir(tmp.path());
    let sweep = serde_json::json!({
        "vertical": "legal-tech",
        "prospects": [],
        "common_pain_points": ["Manual contract review"],
        "sources": [],
    });

    let plan = plan_ingest(&sweep, None, tmp.path());
    let diags = apply_plan(&plan, true);
    assert!(diags.iter().all(|d| d.severity != mev::Severity::Error));
    let path = tmp
        .path()
        .join("business/docs/opportunities")
        .join("legal-tech-prospecting-sweep.md");
    let content = std::fs::read_to_string(path).unwrap();
    assert!(content.contains("kind: prospecting-sweep"));
}

#[test]
fn ingest_unknown_shape_errors_and_plans_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let bogus = serde_json::json!({"foo": "bar"});

    let plan = plan_ingest(&bogus, None, tmp.path());
    assert!(plan.actions.is_empty());
    assert_eq!(plan.diagnostics.len(), 1);
    assert_eq!(plan.diagnostics[0].locator, "E_DOC_UNKNOWN_INPUT_SHAPE");
}

#[test]
fn ingest_second_run_is_zero_action_noop() {
    let tmp = tempfile::tempdir().unwrap();
    opportunities_dir(tmp.path());
    let brief = fixture_brief();

    let plan1 = plan_ingest(&brief, Some(OpportunityKind::Company), tmp.path());
    let before = write_and_read(tmp.path(), plan1);

    let plan2 = plan_ingest(&brief, Some(OpportunityKind::Company), tmp.path());
    assert!(
        plan2.actions.is_empty(),
        "second identical ingest must plan zero actions"
    );

    let after = std::fs::read_to_string(
        tmp.path()
            .join("business/docs/opportunities")
            .join("anthropic.md"),
    )
    .unwrap();
    assert_eq!(before, after);
}

// ── set-stage ────────────────────────────────────────────────────────────

#[test]
fn set_stage_updates_stage_and_is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    opportunities_dir(tmp.path());
    default_pipeline_fixture(tmp.path());
    let brief = fixture_brief();
    let plan = plan_ingest(&brief, Some(OpportunityKind::Company), tmp.path());
    write_and_read(tmp.path(), plan);

    let plan = plan_set_stage("anthropic", "contacted", tmp.path());
    assert_eq!(plan.actions.len(), 1, "expected exactly one doc update");
    let content = write_and_read(tmp.path(), plan);
    assert!(content.contains("stage: contacted"));

    // Re-running with the same stage is a zero-action no-op.
    let plan2 = plan_set_stage("anthropic", "contacted", tmp.path());
    assert!(plan2.actions.is_empty());
}

#[test]
fn set_stage_rejects_unknown_stage() {
    let tmp = tempfile::tempdir().unwrap();
    default_pipeline_fixture(tmp.path());
    let plan = plan_set_stage("anthropic", "bogus-stage", tmp.path());
    assert!(plan.actions.is_empty());
    assert_eq!(plan.diagnostics.len(), 1);
    assert_eq!(plan.diagnostics[0].locator, "E_DOC_BAD_STAGE");
}

#[test]
fn set_stage_missing_file_errors() {
    let tmp = tempfile::tempdir().unwrap();
    default_pipeline_fixture(tmp.path());
    let plan = plan_set_stage("ghost", "contacted", tmp.path());
    assert!(plan.actions.is_empty());
    assert_eq!(plan.diagnostics.len(), 1);
    assert_eq!(plan.diagnostics[0].locator, "E_DOC_NOT_FOUND");
}

// ── stage vocabulary read from pipeline.md (D58) ────────────────────────

/// The load-bearing test: a fixture whose `pipeline.md` declares a
/// non-default vocabulary (the seven defaults plus `on-hold`) must accept an
/// opportunity authored with `stage: on-hold` and reject a stage that
/// fixture does not declare. A fixture using the default seven would pass
/// even against the old hardcoded `VALID_STAGES` const, so this is the only
/// test that actually proves the vocabulary is read rather than compiled in.
#[test]
fn set_stage_non_default_vocabulary_changes_what_validates() {
    let tmp = tempfile::tempdir().unwrap();
    opportunities_dir(tmp.path());
    write_brain_root(tmp.path());
    write_pipeline_md(
        tmp.path(),
        "`identified` → `researching` → `contacted` → `conversation` → `proposal-sent` → \
         `on-hold` → `closed-won` → `closed-lost`",
    );
    let brief = fixture_brief();
    let plan = plan_ingest(&brief, Some(OpportunityKind::Company), tmp.path());
    write_and_read(tmp.path(), plan);

    // Accept: `on-hold` is declared only in this fixture's pipeline.md, not
    // in the hardcoded default seven.
    let plan = plan_set_stage("anthropic", "on-hold", tmp.path());
    assert_eq!(
        plan.actions.len(),
        1,
        "expected 'on-hold' to validate against this fixture's vocabulary"
    );
    let content = write_and_read(tmp.path(), plan);
    assert!(content.contains("stage: on-hold"));

    // Reject: a stage this fixture never declared.
    let plan = plan_set_stage("anthropic", "nurturing", tmp.path());
    assert!(plan.actions.is_empty());
    assert_eq!(plan.diagnostics.len(), 1);
    assert_eq!(plan.diagnostics[0].locator, "E_DOC_BAD_STAGE");
}

/// A fixture with no `business/docs/pipeline.md` at all (but a valid brain
/// root) produces exactly one file-level diagnostic naming the file — never
/// a panic, and never a per-opportunity storm.
#[test]
fn set_stage_missing_pipeline_md_yields_one_file_level_diagnostic() {
    let tmp = tempfile::tempdir().unwrap();
    write_brain_root(tmp.path());
    opportunities_dir(tmp.path());
    let brief = fixture_brief();
    let plan = plan_ingest(&brief, Some(OpportunityKind::Company), tmp.path());
    write_and_read(tmp.path(), plan);

    let plan = plan_set_stage("anthropic", "contacted", tmp.path());
    assert!(plan.actions.is_empty());
    assert_eq!(
        plan.diagnostics.len(),
        1,
        "missing pipeline.md must yield exactly one file-level diagnostic, not one per opportunity"
    );
    assert_eq!(plan.diagnostics[0].locator, "E_DOC_PIPELINE_MD_MISSING");
    assert!(plan.diagnostics[0].message.contains("pipeline.md"));
}

/// A `pipeline.md` with no parseable `## Stages` section produces the
/// unparseable diagnostic, naming the file.
#[test]
fn set_stage_unparseable_pipeline_md_section_errors() {
    let tmp = tempfile::tempdir().unwrap();
    write_brain_root(tmp.path());
    opportunities_dir(tmp.path());
    let dir = tmp.path().join("business/docs");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("pipeline.md"),
        "# Pipeline\n\nNo Stages section here.\n",
    )
    .unwrap();

    let plan = plan_set_stage("anthropic", "contacted", tmp.path());
    assert!(plan.actions.is_empty());
    assert_eq!(plan.diagnostics.len(), 1);
    assert_eq!(
        plan.diagnostics[0].locator,
        "E_DOC_PIPELINE_STAGES_UNPARSEABLE"
    );
    assert!(plan.diagnostics[0].message.contains("pipeline.md"));
}

// ── add-action ───────────────────────────────────────────────────────────

#[test]
fn add_action_appends_and_is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    opportunities_dir(tmp.path());
    let brief = fixture_brief();
    let plan = plan_ingest(&brief, Some(OpportunityKind::Company), tmp.path());
    write_and_read(tmp.path(), plan);

    let plan = plan_add_action(
        "anthropic",
        "2026-07-27",
        "outreach",
        "Sent first email",
        tmp.path(),
    );
    assert_eq!(plan.actions.len(), 1);
    let content = write_and_read(tmp.path(), plan);
    assert!(content.contains("Sent first email"));

    // Re-adding the identical triple plans zero actions.
    let plan2 = plan_add_action(
        "anthropic",
        "2026-07-27",
        "outreach",
        "Sent first email",
        tmp.path(),
    );
    assert!(plan2.actions.is_empty());
}

// ── merge-contacts ───────────────────────────────────────────────────────

#[test]
fn merge_contacts_adds_new_and_enriches_existing_without_duplicating() {
    let tmp = tempfile::tempdir().unwrap();
    opportunities_dir(tmp.path());
    let brief = fixture_brief();
    let plan = plan_ingest(&brief, Some(OpportunityKind::Company), tmp.path());
    write_and_read(tmp.path(), plan);

    let first = serde_json::json!([{
        "name": "Alice",
        "role": "CTO",
        "emails": ["alice@example.com"],
    }]);
    let plan = plan_merge_contacts("anthropic", &first, tmp.path());
    assert_eq!(plan.actions.len(), 1);
    let content = write_and_read(tmp.path(), plan);
    assert!(content.contains("Alice"));

    // Enrich with a new email and a note — role must not be clobbered.
    let second = serde_json::json!([{
        "name": "Alice",
        "emails": ["a@corp.com"],
        "note": "Met at conference",
    }]);
    let plan = plan_merge_contacts("anthropic", &second, tmp.path());
    assert_eq!(plan.actions.len(), 1);
    let content = write_and_read(tmp.path(), plan);
    assert!(content.contains("alice@example.com"));
    assert!(content.contains("a@corp.com"));
    assert!(content.contains("CTO"));
    assert!(content.contains("Met at conference"));
    // Exactly one contact row for Alice — no duplication.
    assert_eq!(content.matches("name: Alice").count(), 1);

    // Re-merging the already-merged contact plans zero actions.
    let plan3 = plan_merge_contacts("anthropic", &second, tmp.path());
    assert!(plan3.actions.is_empty());
}

#[test]
fn merge_contacts_missing_file_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let contacts = serde_json::json!([{"name": "Alice"}]);
    let plan = plan_merge_contacts("ghost", &contacts, tmp.path());
    assert!(plan.actions.is_empty());
    assert_eq!(plan.diagnostics.len(), 1);
    assert_eq!(plan.diagnostics[0].locator, "E_DOC_NOT_FOUND");
}
