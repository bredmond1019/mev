//! Integration tests for `src/doc/index_reconcile.rs` — the idempotent
//! `index.md` table-row upsert, plus an end-to-end `plan_document` check
//! confirming the doc write and the index row are planned together.

use std::path::Path;

use mev::doc::index_reconcile::plan_index_reconcile;
use mev::doc::plan_document;
use okf_core::{IndexIntent, Opportunity};

const SAMPLE: &str = "\
# Opportunities

## Files

| Opportunity | Kind | Stage |
|---|---|---|
| [Anthropic](anthropic.md) | `company` | `identified` |
";

fn write_index(dir: &Path, contents: &str) -> std::path::PathBuf {
    let path = dir.join("index.md");
    std::fs::write(&path, contents).unwrap();
    path
}

#[test]
fn insert_when_absent_appends_one_row() {
    let tmp = tempfile::tempdir().unwrap();
    write_index(tmp.path(), SAMPLE);

    let intent = IndexIntent::new(
        "index.md",
        "acme.md",
        vec![
            "Acme".to_string(),
            "`company`".to_string(),
            "`identified`".to_string(),
        ],
    );

    let plan = plan_index_reconcile(&intent, tmp.path());
    assert_eq!(plan.actions.len(), 1);
    let content = &plan.actions[0].new_content;
    assert!(content.contains("| [Anthropic](anthropic.md) | `company` | `identified` |"));
    assert!(content.contains("| [Acme](acme.md) | `company` | `identified` |"));
    // Exactly one new row was added: two data rows total.
    assert_eq!(content.matches("| [").count(), 2);
}

#[test]
fn update_in_place_leaves_row_count_and_siblings_untouched() {
    let tmp = tempfile::tempdir().unwrap();
    write_index(
        tmp.path(),
        "# Opportunities\n\n## Files\n\n| Opportunity | Kind | Stage |\n|---|---|---|\n\
         | [Anthropic](anthropic.md) | `company` | `identified` |\n\
         | [Beta](beta.md) | `company` | `researching` |\n",
    );

    let intent = IndexIntent::new(
        "index.md",
        "anthropic.md",
        vec![
            "Anthropic".to_string(),
            "`company`".to_string(),
            "`contacted`".to_string(),
        ],
    );

    let plan = plan_index_reconcile(&intent, tmp.path());
    assert_eq!(plan.actions.len(), 1);
    let content = &plan.actions[0].new_content;
    assert!(content.contains("| [Anthropic](anthropic.md) | `company` | `contacted` |"));
    // Sibling row byte-identical and still present exactly once.
    assert_eq!(
        content
            .matches("| [Beta](beta.md) | `company` | `researching` |")
            .count(),
        1
    );
    // Row count unchanged: two data rows total.
    assert_eq!(content.matches("| [").count(), 2);
}

#[test]
fn double_run_plans_zero_actions() {
    let tmp = tempfile::tempdir().unwrap();
    write_index(tmp.path(), SAMPLE);

    let intent = IndexIntent::new(
        "index.md",
        "anthropic.md",
        vec![
            "Anthropic".to_string(),
            "`company`".to_string(),
            "`identified`".to_string(),
        ],
    );

    let first = plan_index_reconcile(&intent, tmp.path());
    assert!(
        first.actions.is_empty(),
        "already matches, no change expected"
    );
    assert!(
        first
            .diagnostics
            .iter()
            .any(|d| d.locator == "W_DOC_UNCHANGED")
    );
}

#[test]
fn missing_index_yields_warning_and_no_action() {
    let tmp = tempfile::tempdir().unwrap();
    let intent = IndexIntent::new("index.md", "acme.md", vec!["Acme".to_string()]);

    let plan = plan_index_reconcile(&intent, tmp.path());
    assert!(plan.actions.is_empty());
    assert_eq!(plan.diagnostics.len(), 1);
    assert_eq!(plan.diagnostics[0].locator, "W_DOC_INDEX_MISSING");
}

#[test]
fn missing_table_yields_warning_and_no_action() {
    let tmp = tempfile::tempdir().unwrap();
    write_index(
        tmp.path(),
        "# Opportunities\n\nNo table here, just prose.\n",
    );
    let intent = IndexIntent::new("index.md", "acme.md", vec!["Acme".to_string()]);

    let plan = plan_index_reconcile(&intent, tmp.path());
    assert!(plan.actions.is_empty());
    assert_eq!(plan.diagnostics.len(), 1);
    assert_eq!(plan.diagnostics[0].locator, "W_DOC_INDEX_NO_TABLE");
}

#[test]
fn column_mismatch_yields_warning_and_no_action() {
    let tmp = tempfile::tempdir().unwrap();
    write_index(tmp.path(), SAMPLE);
    let intent = IndexIntent::new(
        "index.md",
        "acme.md",
        vec!["Acme".to_string(), "`company`".to_string()],
    );

    let plan = plan_index_reconcile(&intent, tmp.path());
    assert!(plan.actions.is_empty());
    assert_eq!(plan.diagnostics.len(), 1);
    assert_eq!(plan.diagnostics[0].locator, "W_DOC_INDEX_COLUMN_MISMATCH");
}

// ---------------------------------------------------------------------------
// End-to-end: one `plan_document` call on a first materialize plans exactly
// two actions — the doc write and the index row.
// ---------------------------------------------------------------------------

#[test]
fn plan_document_plans_both_doc_and_index_actions_on_first_materialize() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("business/docs/opportunities")).unwrap();
    write_index(
        &tmp.path().join("business/docs/opportunities"),
        "# Opportunities\n\n## Files\n\n| Opportunity | Kind | Stage |\n|---|---|---|\n",
    );

    let opportunity = Opportunity::from_company_brief(&serde_json::json!({
        "company_name": "Acme Corp",
        "summary": "A widget maker.",
    }));

    let plan = plan_document(&opportunity, tmp.path());
    assert_eq!(
        plan.actions.len(),
        2,
        "expected exactly two actions (doc + index), got {:?}",
        plan.actions
            .iter()
            .map(|a| a.path.clone())
            .collect::<Vec<_>>()
    );
}
