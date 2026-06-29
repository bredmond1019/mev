//! mev (`mev`) — parses, validates, and (later) compiles the
//! MDX/Markdown content for learn-agentic-ai.com.
//!
//! Phase 0 lays the testable skeleton: a CLI surface and the `Diagnostic` type that every
//! future check emits. Phase 1, Block B adds content-tree crawl + classification (see
//! `planning/master-plan.md`).

pub mod brain;
mod learn_ai;
mod shared;
pub mod theme;
mod validator;
pub use brain::BrainValidator;
pub use brain::crawl::{MdFile, crawl_brain};
pub use brain::okf::{OkfFrontmatter, validate_md_file};
pub use learn_ai::LearnAiValidator;
pub use learn_ai::crawl::{ContentFile, Corpus, FileKind, Locale, crawl};
pub use learn_ai::meta::validate_file;
pub use validator::ContentValidator;

use std::path::PathBuf;

/// Severity of a single finding. Drives the process exit code: any [`Severity::Error`]
/// makes a run fail (exit 1); warnings are reported but do not fail the run (exit 0).
/// This mirrors the error/warning split of the site's existing `scripts/validate-content.ts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

/// A single validation finding. Every check produces `Diagnostic`s; only the reporter prints.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Diagnostic {
    pub severity: Severity,
    /// File the finding concerns, relative to the content root where possible.
    pub file: PathBuf,
    /// In-file locator (e.g. `metadata.title`, `sections[2].id`) or empty for whole-file findings.
    pub locator: String,
    pub message: String,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Severity::Error => f.write_str("error"),
            Severity::Warning => f.write_str("warning"),
        }
    }
}

impl Diagnostic {
    pub fn error(
        file: impl Into<PathBuf>,
        locator: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            severity: Severity::Error,
            file: file.into(),
            locator: locator.into(),
            message: message.into(),
        }
    }

    pub fn warning(
        file: impl Into<PathBuf>,
        locator: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            severity: Severity::Warning,
            file: file.into(),
            locator: locator.into(),
            message: message.into(),
        }
    }
}

/// Outcome of a validation run: the findings plus whether they constitute a failure.
#[derive(Debug, Default)]
pub struct Report {
    pub diagnostics: Vec<Diagnostic>,
}

impl Report {
    pub fn error_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .count()
    }

    pub fn warning_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .count()
    }

    /// A run fails when any error-severity diagnostic is present.
    pub fn is_failure(&self) -> bool {
        self.error_count() > 0
    }
}

/// Validate the content tree rooted at `root`.
///
/// Block B: crawl + classify + filename conventions.
/// Block C: struct and frontmatter validation — each file in the [`Corpus`] is dispatched to
/// [`meta::validate_file`], which checks required fields, enum values, and format constraints.
/// All diagnostics (filename + struct/frontmatter) are collected into the returned [`Report`].
///
/// Delegates to [`LearnAiValidator`] via the [`ContentValidator`] trait's default `run` driver.
pub fn validate(root: &std::path::Path) -> anyhow::Result<Report> {
    Ok(LearnAiValidator.run(root))
}

/// Validate the company-brain repo rooted at `root` for OKF frontmatter compliance.
///
/// Phase 2, Block I + Block J-crawl: mirrors [`validate`] for the brain consumer — delegates
/// to [`BrainValidator`] which applies the registry-driven corpus crawl (`crawl_corpus`:
/// `skip_dirs` pruning, corpus membership, scope resolution) and Block H's OKF checks.
/// Root instruction files (`README.md`/`CLAUDE.md`) without frontmatter are exempt from the
/// "missing frontmatter" error — they are included in the corpus as leaves.
///
/// Resolves `brain.toml` by walking up from `root` via [`brain::config::find_brain_config`].
/// If no `brain.toml` is found, a fatal `Error`-severity diagnostic with locator
/// `E_CONFIG_NOT_FOUND` is returned in the report rather than panicking — the caller
/// should treat this as a configuration error (exit 1).
pub fn validate_brain(root: &std::path::Path) -> anyhow::Result<Report> {
    use brain::config::find_brain_config;

    let config = match find_brain_config(root) {
        Ok(cfg) => cfg,
        Err(e) => {
            let mut report = Report::default();
            report.diagnostics.push(Diagnostic::error(
                root,
                "E_CONFIG_NOT_FOUND",
                format!("brain.toml not found or unreadable: {e}"),
            ));
            return Ok(report);
        }
    };

    Ok(BrainValidator::new(config).run(root))
}

/// Validate the company-brain repo for OKF frontmatter compliance **plus** cross-repo
/// sync watermark integrity.
///
/// Phase 3, Block M (HQ-Restructure Block N): runs the full schema pass (identical to
/// [`validate_brain`]) and then appends [`brain::sync::check_sync`] diagnostics into
/// the same [`Report`].  A `Sync` error (any `E_SYNC_*` locator) is `Error`-severity
/// and causes `report.is_failure()` to return `true`, producing exit code 1.
///
/// Resolves `brain.toml` the same way as [`validate_brain`] — see that function's
/// doc for the `E_CONFIG_NOT_FOUND` fallback behaviour.
pub fn validate_brain_sync(root: &std::path::Path) -> anyhow::Result<Report> {
    use brain::config::find_brain_config;
    use brain::sync::check_sync;

    let config = match find_brain_config(root) {
        Ok(cfg) => cfg,
        Err(e) => {
            let mut report = Report::default();
            report.diagnostics.push(Diagnostic::error(
                root,
                "E_CONFIG_NOT_FOUND",
                format!("brain.toml not found or unreadable: {e}"),
            ));
            return Ok(report);
        }
    };

    // Schema pass (OKF frontmatter)
    let mut report = BrainValidator::new(config.clone()).run(root);

    // Sync watermark pass
    let sync_diags = check_sync(root, &config);
    report.diagnostics.extend(sync_diags);

    Ok(report)
}

/// Machine-readable envelope emitted by the `--json` flag for any `mev` subcommand.
///
/// Consumed by the Brain RAG indexer as a pre-`--rebuild` gate.
#[derive(Debug, serde::Serialize)]
pub struct JsonReport {
    /// Which validator produced this report (`"brain"` or `"learn-ai"`).
    pub validator: String,
    /// Display path of the root that was validated.
    pub root: String,
    /// Number of error-severity diagnostics.
    pub errors: usize,
    /// Number of warning-severity diagnostics.
    pub warnings: usize,
    /// All diagnostics emitted during the run.
    pub diagnostics: Vec<Diagnostic>,
}

impl JsonReport {
    /// Build a [`JsonReport`] from the component pieces.
    pub fn new(validator: &str, root: &std::path::Path, report: &Report) -> Self {
        Self {
            validator: validator.to_owned(),
            root: root.display().to_string(),
            errors: report.error_count(),
            warnings: report.warning_count(),
            diagnostics: report.diagnostics.clone(),
        }
    }

    /// Serialize to a pretty-printed JSON string.
    pub fn to_json(&self) -> anyhow::Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }
}
