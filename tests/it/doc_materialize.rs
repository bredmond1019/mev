//! Integration tests for `src/doc/materialize.rs` — the generic
//! `plan_document` planner over `okf_core::BrainDocModel`.

use std::path::Path;

use mev::brain::emit::apply_plan;
use mev::doc::plan_document;
use okf_core::{
    BodySection, BodySpec, BrainDocModel, FrontmatterValue, IndexIntent, LearningArtifact,
    Opportunity, Proposal, derive_slug,
};

// ---------------------------------------------------------------------------
// A local test model mixing a Verbatim section with a Generated section —
// none of the three real okf-core models mix both, and this spec's
// acceptance criteria require proving a hand-edited verbatim region survives
// an update while the generated region is re-spliced.
// ---------------------------------------------------------------------------

struct Note {
    title: String,
    generated: String,
}

impl BrainDocModel for Note {
    fn frontmatter(&self) -> Vec<(String, FrontmatterValue)> {
        vec![
            (
                "type".to_string(),
                FrontmatterValue::Scalar("Note".to_string()),
            ),
            (
                "title".to_string(),
                FrontmatterValue::Scalar(self.title.clone()),
            ),
        ]
    }

    fn body(&self) -> BodySpec {
        BodySpec::new(vec![
            BodySection::Verbatim(format!("# {}\n\nHand-written intro.", self.title)),
            BodySection::Generated {
                marker: "summary".to_string(),
                content: self.generated.clone(),
            },
        ])
    }

    fn slug(&self) -> String {
        derive_slug(&self.title)
    }

    fn index_intent(&self) -> IndexIntent {
        IndexIntent::new(
            "notes/index.md",
            format!("{}.md", self.slug()),
            vec![self.title.clone()],
        )
    }

    fn doc_type(&self) -> &'static str {
        "note"
    }
}

fn target_path(root: &Path, model: &impl BrainDocModel) -> std::path::PathBuf {
    let intent = model.index_intent();
    root.join(Path::new(&intent.index_path).parent().unwrap())
        .join(&intent.link_target)
}

// ---------------------------------------------------------------------------
// Create path
// ---------------------------------------------------------------------------

#[test]
fn creates_file_from_absent_target() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("notes")).unwrap();
    let model = Note {
        title: "My Note".to_string(),
        generated: "v1".to_string(),
    };

    let plan = plan_document(&model, tmp.path());
    assert_eq!(plan.actions.len(), 1, "expected one create action");

    let diags = apply_plan(&plan, true);
    assert!(
        diags.iter().all(|d| d.severity != mev::Severity::Error),
        "unexpected error diagnostics: {diags:?}"
    );

    let path = target_path(tmp.path(), &model);
    assert!(path.exists());
    let content = std::fs::read_to_string(&path).unwrap();
    assert!(content.starts_with("---\n"));
    assert!(content.contains("Hand-written intro."));
    assert!(
        content.contains("<!-- BEGIN generated:summary -->\nv1\n<!-- END generated:summary -->")
    );
}

// ---------------------------------------------------------------------------
// Idempotency — second run plans zero actions and is byte-identical
// ---------------------------------------------------------------------------

#[test]
fn second_run_plans_zero_actions_and_is_byte_identical() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("notes")).unwrap();
    let model = Note {
        title: "My Note".to_string(),
        generated: "v1".to_string(),
    };

    let plan = plan_document(&model, tmp.path());
    apply_plan(&plan, true);

    let path = target_path(tmp.path(), &model);
    let before = std::fs::read_to_string(&path).unwrap();

    let plan2 = plan_document(&model, tmp.path());
    assert!(
        plan2.actions.is_empty(),
        "expected zero actions on unchanged second run"
    );
    assert!(
        plan2
            .diagnostics
            .iter()
            .any(|d| d.locator == "W_DOC_UNCHANGED"),
        "expected W_DOC_UNCHANGED diagnostic, got {:?}",
        plan2.diagnostics
    );

    apply_plan(&plan2, true);
    let after = std::fs::read_to_string(&path).unwrap();
    assert_eq!(before, after, "second run must not change file bytes");
}

// ---------------------------------------------------------------------------
// Sentinel-preserving update: hand-edited verbatim region survives; the
// generated region is re-spliced.
// ---------------------------------------------------------------------------

#[test]
fn hand_edited_verbatim_region_survives_update_while_generated_is_respliced() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("notes")).unwrap();
    let model_v1 = Note {
        title: "My Note".to_string(),
        generated: "v1".to_string(),
    };

    let plan = plan_document(&model_v1, tmp.path());
    apply_plan(&plan, true);

    let path = target_path(tmp.path(), &model_v1);

    // Hand-edit the verbatim region.
    let mut content = std::fs::read_to_string(&path).unwrap();
    content = content.replace(
        "Hand-written intro.",
        "Hand-written intro — EDITED BY HUMAN.",
    );
    std::fs::write(&path, &content).unwrap();

    // Re-plan with new generated content; verbatim edit must survive.
    let model_v2 = Note {
        title: "My Note".to_string(),
        generated: "v2".to_string(),
    };
    let plan2 = plan_document(&model_v2, tmp.path());
    assert_eq!(plan2.actions.len(), 1);
    apply_plan(&plan2, true);

    let after = std::fs::read_to_string(&path).unwrap();
    assert!(
        after.contains("Hand-written intro — EDITED BY HUMAN."),
        "hand-edited verbatim region must survive: {after}"
    );
    assert!(
        after.contains("<!-- BEGIN generated:summary -->\nv2\n<!-- END generated:summary -->"),
        "generated region must be re-spliced to v2: {after}"
    );
    assert!(
        !after.contains("v1"),
        "stale generated content must be gone"
    );
}

// ---------------------------------------------------------------------------
// Missing sentinel yields W_DOC_MISSING_SENTINEL and does not clobber
// ---------------------------------------------------------------------------

#[test]
fn missing_sentinel_warns_and_does_not_clobber() {
    let tmp = tempfile::tempdir().unwrap();
    let model = Note {
        title: "My Note".to_string(),
        generated: "v1".to_string(),
    };

    let path = target_path(tmp.path(), &model);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    // A file with frontmatter and prose, but no sentinel pair at all.
    let original = "---\ntype: Note\ntitle: My Note\n---\n# My Note\n\nNo sentinels here.\n";
    std::fs::write(&path, original).unwrap();

    let plan = plan_document(&model, tmp.path());
    assert!(
        plan.diagnostics
            .iter()
            .any(|d| d.locator == "W_DOC_MISSING_SENTINEL"),
        "expected W_DOC_MISSING_SENTINEL, got {:?}",
        plan.diagnostics
    );

    // The body (outside the frontmatter) must be untouched — only the
    // frontmatter block is reconciled.
    if let Some(action) = plan.actions.first() {
        assert!(action.new_content.contains("No sentinels here."));
    }

    apply_plan(&plan, true);
    let after = std::fs::read_to_string(&path).unwrap();
    assert!(
        after.contains("No sentinels here."),
        "must not clobber body"
    );
}

// ---------------------------------------------------------------------------
// Bad index_path yields E_DOC_BAD_INDEX_PATH
// ---------------------------------------------------------------------------

#[test]
fn bad_index_path_with_no_parent_dir_raises_error() {
    struct Bad;
    impl BrainDocModel for Bad {
        fn frontmatter(&self) -> Vec<(String, FrontmatterValue)> {
            vec![]
        }
        fn body(&self) -> BodySpec {
            BodySpec::new(vec![])
        }
        fn slug(&self) -> String {
            "bad".to_string()
        }
        fn index_intent(&self) -> IndexIntent {
            IndexIntent::new("index.md", "bad.md", vec![])
        }
        fn doc_type(&self) -> &'static str {
            "bad"
        }
    }

    let tmp = tempfile::tempdir().unwrap();
    let plan = plan_document(&Bad, tmp.path());
    assert!(plan.actions.is_empty());
    assert_eq!(plan.diagnostics.len(), 1);
    assert_eq!(plan.diagnostics[0].locator, "E_DOC_BAD_INDEX_PATH");
    assert_eq!(plan.diagnostics[0].severity, mev::Severity::Error);
}

// ---------------------------------------------------------------------------
// Genericity — the SAME plan_document call plans successfully for all three
// okf-core models (Opportunity, LearningArtifact, Proposal).
// ---------------------------------------------------------------------------

#[test]
fn plan_document_is_generic_over_all_three_okf_core_models() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("business/docs/opportunities")).unwrap();
    std::fs::create_dir_all(tmp.path().join("docs/content/learning-corpus")).unwrap();
    std::fs::create_dir_all(tmp.path().join("business/docs/proposals")).unwrap();

    let opportunity = Opportunity::from_company_brief(&serde_json::json!({
        "company_name": "Acme Corp",
        "summary": "A widget maker.",
    }));
    let plan = plan_document(&opportunity, tmp.path());
    assert_eq!(plan.actions.len(), 1, "opportunity should plan a create");
    apply_plan(&plan, true);

    let artifact = LearningArtifact::from_payload(&serde_json::json!({
        "artifact_id": "abc-123",
        "channel_type": "podcast",
        "source_ref": "https://example.com/ep1",
        "summary": "A podcast episode.",
        "digest_markdown": "# Digest\n",
        "entities": ["thing"],
        "language": "en",
    }));
    let plan = plan_document(&artifact, tmp.path());
    assert_eq!(
        plan.actions.len(),
        1,
        "learning artifact should plan a create"
    );
    apply_plan(&plan, true);

    let proposal =
        Proposal::from_automation_roadmap("Acme Corp", &serde_json::json!({"situation": "x"}));
    let plan = plan_document(&proposal, tmp.path());
    assert_eq!(plan.actions.len(), 1, "proposal should plan a create");
    apply_plan(&plan, true);

    // A second run over each is a zero-action no-op.
    assert!(plan_document(&opportunity, tmp.path()).actions.is_empty());
    assert!(plan_document(&artifact, tmp.path()).actions.is_empty());
    assert!(plan_document(&proposal, tmp.path()).actions.is_empty());
}
