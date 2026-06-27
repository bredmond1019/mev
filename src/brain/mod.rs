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
/// Construct with [`BrainValidator::new`] supplying a [`BrainConfig`] loaded from
/// `brain.toml`, then call `.run(root)` to walk `root`, collect every `.md` file, and
/// validate each one's OKF frontmatter, collecting all diagnostics into a [`crate::Report`].
pub struct BrainValidator {
    /// Configuration sourced from `brain.toml` — supplies `skip_dirs` for the crawl and
    /// vocabulary lists for OKF validation (wired in Task 3).
    pub(crate) config: BrainConfig,
}

impl BrainValidator {
    /// Create a new [`BrainValidator`] using the supplied `brain.toml` config.
    pub fn new(config: BrainConfig) -> Self {
        Self { config }
    }
}

impl ContentValidator for BrainValidator {
    type Item = MdFile;

    /// Walk `root`, collect every `.md` file (applying `skip_dirs` from config), and return
    /// the items plus any crawl-time diagnostics.
    fn crawl(&self, root: &Path) -> (Vec<MdFile>, Vec<Diagnostic>) {
        crawl::crawl_brain(root, &self.config.crawl.skip_dirs)
    }

    /// Validate a single brain `.md` file against the OKF frontmatter schema.
    fn validate_item(&self, item: &MdFile) -> Vec<Diagnostic> {
        okf::validate_md_file(item, &self.config)
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

        let v = BrainValidator::new(BrainConfig::default());
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

        let report = BrainValidator::new(BrainConfig::default()).run(&dir);
        assert!(
            report.diagnostics.is_empty(),
            "expected empty report for empty dir, got {:?}",
            report.diagnostics
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn brain_validator_skip_dirs_from_config_are_used() {
        let dir = std::env::temp_dir().join("mev-brain-validator-skip-dirs-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // Create a file in a directory that should be pruned by config skip_dirs.
        let skip_dir = dir.join("my-skip-dir");
        std::fs::create_dir_all(&skip_dir).unwrap();
        std::fs::write(skip_dir.join("hidden.md"), b"hidden").unwrap();
        // And a file at root level that should be found.
        std::fs::write(dir.join("visible.md"), b"visible").unwrap();

        use crate::brain::config::{BrainConfig, CrawlConfig};
        let config = BrainConfig {
            crawl: CrawlConfig {
                skip_dirs: vec!["my-skip-dir".to_string()],
            },
            ..BrainConfig::default()
        };
        let v = BrainValidator::new(config);
        let (items, diags) = v.crawl(&dir);

        assert!(diags.is_empty(), "expected no diagnostics, got {diags:?}");
        assert_eq!(
            items.len(),
            1,
            "expected only visible.md; my-skip-dir should be pruned. Got: {items:?}"
        );
        assert_eq!(items[0].stem, "visible");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
