//! Generic planner: given any `okf_core::BrainDocModel`, produce an
//! `EmitPlan` that writes or updates its target `.md` file.
//!
//! The `index.md` reconcile half of the materializer lands in a later task
//! in this spec (`MV.9.A` task 2); this file covers only the document write
//! itself.

use std::path::{Path, PathBuf};

use okf_core::{BodySection, BrainDocModel, render_document, serialize_nested_frontmatter};

use crate::Diagnostic;
use crate::brain::emit::{EmitAction, EmitError, EmitPlan, splice_generated};
use crate::doc::index_reconcile::plan_index_reconcile;

/// Compute the target file path for a document under `root`, derived from
/// its `IndexIntent` fields: `root/dirname(index_path)/link_target`.
///
/// Returns `None` when `index_path` has no parent directory component (e.g.
/// a bare filename like `"index.md"`) — the caller raises
/// `E_DOC_BAD_INDEX_PATH` in that case rather than writing to `root` itself.
fn target_path(root: &Path, index_path: &str, link_target: &str) -> Option<PathBuf> {
    let parent = Path::new(index_path).parent()?;
    if parent.as_os_str().is_empty() {
        return None;
    }
    Some(root.join(parent).join(link_target))
}

/// Find the byte offset just past a leading `---\n ... \n---\n` frontmatter
/// fence in `content` — i.e. the start of the body. Returns `None` when
/// `content` does not open with a frontmatter fence.
fn frontmatter_body_offset(content: &str) -> Option<usize> {
    if !content.starts_with("---\n") {
        return None;
    }
    let rest = &content[4..];
    let close = rest.find("\n---\n")?;
    Some(4 + close + 5)
}

/// Plan the `EmitAction` (if any) that writes or updates `model`'s target
/// document under `root`.
///
/// - **Create path** (target absent): content is `render_document(model)`;
///   one `EmitAction` is planned.
/// - **Update path** (target present): every `BodySection::Generated` in
///   `model.body()` is re-spliced via `splice_generated` over the existing
///   file's bytes — a missing sentinel pair pushes `W_DOC_MISSING_SENTINEL`
///   and leaves that section untouched rather than clobbering the file — and
///   the leading frontmatter fence is replaced with the model's current
///   serialized frontmatter. Every byte outside the sentinel pairs and the
///   frontmatter fence is preserved verbatim.
/// - **Idempotency:** when the computed content equals the file's existing
///   bytes, no `EmitAction` is planned and `W_DOC_UNCHANGED` is pushed
///   instead — a second run over an unchanged input is a zero-action no-op.
///
/// This function performs the one read needed to compute an update against
/// an existing target; it performs no writes of its own —
/// `crate::brain::emit::apply_plan` remains the single write point for the
/// returned plan.
pub fn plan_document(model: &impl BrainDocModel, root: &Path) -> EmitPlan {
    let intent = model.index_intent();

    let Some(path) = target_path(root, &intent.index_path, &intent.link_target) else {
        return EmitPlan {
            actions: vec![],
            diagnostics: vec![Diagnostic::error(
                root.join(&intent.index_path),
                "E_DOC_BAD_INDEX_PATH",
                format!(
                    "index_path '{}' has no parent directory component",
                    intent.index_path
                ),
            )],
        };
    };

    let mut diagnostics = Vec::new();

    let existing = std::fs::read_to_string(&path).ok();

    let new_content = match &existing {
        Some(existing) => {
            // Re-splice every generated section over the existing content.
            let mut spliced = existing.clone();
            for section in model.body().sections {
                if let BodySection::Generated { marker, content } = section {
                    match splice_generated(&spliced, &marker, &content) {
                        Ok(next) => spliced = next,
                        Err(EmitError::MissingSentinel { marker }) => {
                            diagnostics.push(Diagnostic::warning(
                                &path,
                                "W_DOC_MISSING_SENTINEL",
                                format!(
                                    "missing sentinel for marker '{marker}' in {} — section left untouched",
                                    path.display()
                                ),
                            ));
                        }
                    }
                }
            }

            // Reconcile the frontmatter block, preserving everything after it.
            let new_frontmatter = serialize_nested_frontmatter(&model.frontmatter());
            match frontmatter_body_offset(&spliced) {
                Some(body_start) => format!("{new_frontmatter}{}", &spliced[body_start..]),
                None => format!("{new_frontmatter}{spliced}"),
            }
        }
        None => render_document(model),
    };

    let mut actions = Vec::new();
    match &existing {
        Some(existing) if *existing == new_content => {
            diagnostics.push(Diagnostic::warning(
                &path,
                "W_DOC_UNCHANGED",
                format!("{} is already up to date", path.display()),
            ));
        }
        Some(_) => {
            actions.push(EmitAction {
                path: path.clone(),
                new_content,
                note: format!("update {} ({})", path.display(), model.doc_type()),
            });
        }
        None => {
            actions.push(EmitAction {
                path: path.clone(),
                new_content,
                note: format!("create {} ({})", path.display(), model.doc_type()),
            });
        }
    }

    let mut plan = EmitPlan {
        actions,
        diagnostics,
    };
    plan.extend(plan_index_reconcile(&intent, root));
    plan
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use okf_core::{LearningArtifact, Opportunity, Proposal};

    #[test]
    fn bad_index_path_raises_error_and_plans_nothing() {
        struct BadModel;
        impl BrainDocModel for BadModel {
            fn frontmatter(&self) -> Vec<(String, okf_core::FrontmatterValue)> {
                vec![]
            }
            fn body(&self) -> okf_core::BodySpec {
                okf_core::BodySpec::new(vec![])
            }
            fn slug(&self) -> String {
                "bad".to_string()
            }
            fn index_intent(&self) -> okf_core::IndexIntent {
                okf_core::IndexIntent::new("index.md", "bad.md", vec![])
            }
            fn doc_type(&self) -> &'static str {
                "bad"
            }
        }

        let tmp = tempfile::tempdir().unwrap();
        let plan = plan_document(&BadModel, tmp.path());
        assert!(plan.actions.is_empty());
        assert_eq!(plan.diagnostics.len(), 1);
        assert_eq!(plan.diagnostics[0].locator, "E_DOC_BAD_INDEX_PATH");
    }

    #[test]
    fn plans_successfully_for_all_three_okf_core_models() {
        let tmp = tempfile::tempdir().unwrap();

        let opp = Opportunity::from_company_brief(&serde_json::json!({
            "company_name": "Acme Corp",
            "summary": "A widget maker.",
        }));
        let plan = plan_document(&opp, tmp.path());
        assert_eq!(plan.actions.len(), 1);

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
        assert_eq!(plan.actions.len(), 1);

        let proposal =
            Proposal::from_automation_roadmap("Acme Corp", &serde_json::json!({"situation": "x"}));
        let plan = plan_document(&proposal, tmp.path());
        assert_eq!(plan.actions.len(), 1);
    }
}
