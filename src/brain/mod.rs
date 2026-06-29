//! Brain consumer: crawl the company-brain repo's Markdown files for OKF validation.
//!
//! This module groups all brain-specific code. Phase 2, Block G implements the crawl
//! entry point; Block H adds OKF frontmatter parsing and [`BrainValidator`]; Block I
//! adds the `validate-brain` subcommand. Block J-crawl wires the registry-driven
//! [`crawl::crawl_corpus`] multi-root walk here, replacing the single-root walk.

pub mod config;
pub mod crawl;
pub mod graph;
pub mod okf;
pub mod scope;
pub mod state;
pub mod sync;

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
///
/// The crawl is driven by [`crawl::crawl_corpus`] — a registry-driven multi-root walk
/// (D4 forward-compat) that yields only corpus members (`planning/**`, `docs/**`, unit-root
/// `README.md`/`CLAUDE.md`) with each entry's owning-unit scope resolved once at walk time.
pub struct BrainValidator {
    /// Configuration sourced from `brain.toml` — supplies `skip_dirs` + `[[repos]]` scope
    /// units for the corpus crawl, and vocabulary lists for OKF validation.
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

    /// Walk `root` using the registry-driven corpus crawl, returning corpus members as
    /// [`MdFile`] items plus any walk-error diagnostics.
    ///
    /// Delegates to [`crawl::crawl_corpus`]: applies `skip_dirs` pruning, resolves each
    /// file's owning scope unit, and keeps only corpus members (`planning/**`, `docs/**`,
    /// unit-root `README.md`/`CLAUDE.md`). Ephemeral files (`handoff.md`, `_`-prefixed)
    /// are excluded. Each [`crawl::CorpusEntry`] is mapped to an [`MdFile`] for the
    /// trait's item type.
    fn crawl(&self, root: &Path) -> (Vec<MdFile>, Vec<Diagnostic>) {
        let (corpus, diags) = crawl::crawl_corpus(root, &self.config);
        let items = corpus
            .entries
            .into_iter()
            .map(|e| MdFile {
                path: e.path,
                rel: e.rel,
                stem: e.stem,
            })
            .collect();
        (items, diags)
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
        // Create planning/ with a file that should survive, and a skip_dir that should be pruned.
        std::fs::create_dir_all(dir.join("planning")).unwrap();
        let skip_dir = dir.join("my-skip-dir");
        std::fs::create_dir_all(&skip_dir).unwrap();
        std::fs::write(skip_dir.join("hidden.md"), b"hidden").unwrap();
        // A corpus member (planning/ file) that must be returned.
        std::fs::write(dir.join("planning/status.md"), b"visible").unwrap();

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
            "expected only planning/status.md; my-skip-dir should be pruned. Got: {items:?}"
        );
        assert_eq!(items[0].stem, "status");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
