//! The `ContentValidator` trait — mirrors `mev::validator::ContentValidator`.
//!
//! `mev-learn-ai` cannot depend on `mev` (see the crate doc comment in `lib.rs` for why), so
//! this trait — small and stable — is mirrored here rather than imported.
//!
//! Each consumer (learn-ai, OKF, …) implements this trait against its own `Item` type.
//! The default `run` driver handles the crawl → validate loop; consumers only need to
//! supply `crawl` and `validate_item`.  Static dispatch throughout — no `dyn` needed.

use std::path::Path;

use crate::{Diagnostic, Report};

/// A pluggable content validator, generic over the per-item type the consumer crawls.
///
/// # How to implement
/// 1. Choose an `Item` type (e.g. `ContentFile` for learn-ai, a path-like struct for OKF).
/// 2. Implement `crawl` to walk `root`, return the items, and emit any crawl-time diagnostics.
/// 3. Implement `validate_item` to check a single item and return its diagnostics.
/// 4. Call `run` — the default driver stitches them together into a `Report`.
pub trait ContentValidator {
    /// The per-item unit this validator crawls and checks (e.g. a `ContentFile`).
    type Item;

    /// Walk `root`, returning the items to validate plus any crawl-time diagnostics.
    fn crawl(&self, root: &Path) -> (Vec<Self::Item>, Vec<Diagnostic>);

    /// Validate a single crawled item, returning any diagnostics found.
    fn validate_item(&self, item: &Self::Item) -> Vec<Diagnostic>;

    /// Default driver: crawl, then validate each item, collecting all diagnostics into a `Report`.
    ///
    /// Override only if you need a non-standard collect/merge strategy.
    fn run(&self, root: &Path) -> Report {
        let (items, mut diagnostics) = self.crawl(root);
        for item in &items {
            diagnostics.extend(self.validate_item(item));
        }
        Report { diagnostics }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Severity;
    use std::path::PathBuf;

    /// Minimal stub validator used only to exercise the default `run` driver.
    struct StubValidator;

    impl ContentValidator for StubValidator {
        type Item = ();

        fn crawl(&self, _root: &Path) -> (Vec<()>, Vec<Diagnostic>) {
            // Emit one crawl-time warning + two items.
            let crawl_diag = Diagnostic::warning(PathBuf::from("root"), "", "crawl-time warning");
            (vec![(), ()], vec![crawl_diag])
        }

        fn validate_item(&self, _item: &()) -> Vec<Diagnostic> {
            // Each item emits one error.
            vec![Diagnostic::error(
                PathBuf::from("item"),
                "field",
                "item error",
            )]
        }
    }

    #[test]
    fn run_collects_crawl_and_item_diagnostics() {
        let validator = StubValidator;
        let root = Path::new("/nonexistent");
        let report = validator.run(root);

        // 1 crawl diagnostic + 2 items × 1 error each = 3 total.
        assert_eq!(report.diagnostics.len(), 3);

        // First diagnostic is the crawl-time warning.
        assert_eq!(report.diagnostics[0].severity, Severity::Warning);
        assert_eq!(report.diagnostics[0].message, "crawl-time warning");

        // Remaining two are per-item errors.
        assert_eq!(report.diagnostics[1].severity, Severity::Error);
        assert_eq!(report.diagnostics[1].message, "item error");
        assert_eq!(report.diagnostics[2].severity, Severity::Error);
        assert_eq!(report.diagnostics[2].message, "item error");

        // Summary counts.
        assert_eq!(report.error_count(), 2);
        assert_eq!(report.warning_count(), 1);
        assert!(report.is_failure());
    }

    #[test]
    fn run_with_no_items_returns_only_crawl_diagnostics() {
        struct EmptyValidator;

        impl ContentValidator for EmptyValidator {
            type Item = ();

            fn crawl(&self, _root: &Path) -> (Vec<()>, Vec<Diagnostic>) {
                let d = Diagnostic::warning(PathBuf::from("x"), "", "lone crawl warning");
                (vec![], vec![d])
            }

            fn validate_item(&self, _item: &()) -> Vec<Diagnostic> {
                vec![]
            }
        }

        let report = EmptyValidator.run(Path::new("/"));
        assert_eq!(report.diagnostics.len(), 1);
        assert_eq!(report.warning_count(), 1);
        assert!(!report.is_failure());
    }

    #[test]
    fn run_with_no_diagnostics_returns_empty_report() {
        struct CleanValidator;

        impl ContentValidator for CleanValidator {
            type Item = ();

            fn crawl(&self, _root: &Path) -> (Vec<()>, Vec<Diagnostic>) {
                (vec![], vec![])
            }

            fn validate_item(&self, _item: &()) -> Vec<Diagnostic> {
                vec![]
            }
        }

        let report = CleanValidator.run(Path::new("/"));
        assert!(report.diagnostics.is_empty());
        assert!(!report.is_failure());
    }
}
