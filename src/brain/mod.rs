//! Brain consumer: crawl the company-brain repo's Markdown files for OKF validation.
//!
//! This module groups all brain-specific code. Phase 2, Block G implements the crawl
//! entry point; Block H adds OKF frontmatter parsing and [`BrainValidator`]; Block I
//! adds the `validate-brain` subcommand.

pub mod config;
pub mod crawl;
pub mod okf;

pub use config::BrainConfig;

use std::path::Path;

use crate::Diagnostic;
use crate::validator::ContentValidator;
use crawl::MdFile;

/// The concrete [`ContentValidator`] for the company-brain OKF frontmatter checks.
///
/// Instantiate and call `.run(root)` to walk `root`, collect every `.md` file, and
/// validate each one's OKF frontmatter, collecting all diagnostics into a [`crate::Report`].
pub struct BrainValidator;

impl ContentValidator for BrainValidator {
    type Item = MdFile;

    /// Walk `root`, collect every `.md` file (applying the brain skip-list), and return
    /// the items plus any crawl-time diagnostics.
    fn crawl(&self, root: &Path) -> (Vec<MdFile>, Vec<Diagnostic>) {
        crawl::crawl_brain(root)
    }

    /// Validate a single brain `.md` file against the OKF frontmatter schema.
    fn validate_item(&self, item: &MdFile) -> Vec<Diagnostic> {
        okf::validate_md_file(item)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validator::ContentValidator;

    #[test]
    fn brain_validator_crawl_empty_dir_returns_no_items() {
        let dir = std::env::temp_dir().join("mev-brain-validator-crawl-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let v = BrainValidator;
        let (items, diags) = v.crawl(&dir);
        assert!(
            items.is_empty(),
            "expected empty list for empty dir, got {items:?}"
        );
        assert!(
            diags.is_empty(),
            "expected no diagnostics for empty dir, got {diags:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn brain_validator_run_on_empty_dir_returns_empty_report() {
        let dir = std::env::temp_dir().join("mev-brain-validator-run-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let report = BrainValidator.run(&dir);
        assert!(
            report.diagnostics.is_empty(),
            "expected empty report for empty dir, got {:?}",
            report.diagnostics
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
