//! Learn-AI consumer: crawl, classify, and validate the learn-agentic-ai.com content tree.
//!
//! This module groups all learn-ai-specific code behind [`LearnAiValidator`], which implements
//! the [`crate::validator::ContentValidator`] trait.  The generic `ContentValidator::run` driver
//! provides the crawl → validate loop; this module only supplies `crawl` and `validate_item`.

pub mod crawl;
pub mod meta;

use std::path::Path;

use crate::Diagnostic;
use crate::validator::ContentValidator;
use crawl::ContentFile;

/// The concrete [`ContentValidator`] for the learn-agentic-ai.com content tree.
///
/// Instantiate and call `.run(root)` to walk `root`, classify every content file,
/// and validate each one, collecting all diagnostics into a [`crate::Report`].
pub struct LearnAiValidator;

impl ContentValidator for LearnAiValidator {
    type Item = ContentFile;

    /// Walk `root`, classify every content file, and return the corpus items plus any
    /// crawl-time diagnostics (filename-convention violations, walk errors).
    fn crawl(&self, root: &Path) -> (Vec<ContentFile>, Vec<Diagnostic>) {
        let (corpus, diags) = crawl::crawl(root);
        (corpus.files, diags)
    }

    /// Validate a single classified content file and return any structural diagnostics.
    fn validate_item(&self, item: &ContentFile) -> Vec<Diagnostic> {
        meta::validate_file(item)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validator::ContentValidator;
    use crawl::{FileKind, Locale};
    use std::path::PathBuf;

    /// Build a minimal ContentFile for testing without touching the filesystem.
    fn stub_file(path: PathBuf, rel: &str, kind: FileKind) -> ContentFile {
        ContentFile {
            path,
            rel: PathBuf::from(rel),
            kind,
            path_id: "demo".to_string(),
            module_id: None,
            locale: Locale::En,
        }
    }

    #[test]
    fn learn_ai_validator_crawl_returns_items_and_diags() {
        // Point at a temp dir with no content — the crawl should return an empty corpus
        // and no diagnostics (the root itself is not a "paths/" subtree).
        let dir = crate::testsupport::unique_temp_dir("mev-learn-ai-crawl-test");
        std::fs::create_dir_all(&dir).unwrap();

        let v = LearnAiValidator;
        let (items, diags) = v.crawl(&dir);
        assert!(
            items.is_empty(),
            "expected empty corpus for empty dir, got {items:?}"
        );
        assert!(
            diags.is_empty(),
            "expected no diagnostics for empty dir, got {diags:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn learn_ai_validator_validate_item_surfaces_missing_file_as_error() {
        // A ContentFile pointing at a nonexistent path must produce an error diagnostic
        // from validate_item (via meta::validate_file's read_content failure path).
        let cf = stub_file(
            PathBuf::from("/nonexistent/mev-la/missing.json"),
            "paths/demo/metadata.json",
            FileKind::PathMetadataJson,
        );

        let v = LearnAiValidator;
        let diags = v.validate_item(&cf);
        assert_eq!(
            diags.len(),
            1,
            "expected exactly one read-failure diagnostic, got {diags:?}"
        );
        assert_eq!(diags[0].severity, crate::Severity::Error);
    }

    #[test]
    fn learn_ai_validator_run_on_empty_dir_returns_empty_report() {
        let dir = crate::testsupport::unique_temp_dir("mev-learn-ai-run-test");
        std::fs::create_dir_all(&dir).unwrap();

        let report = LearnAiValidator.run(&dir);
        assert!(
            report.diagnostics.is_empty(),
            "expected empty report for empty dir, got {:?}",
            report.diagnostics
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
