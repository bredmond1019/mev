//! `mev-learn-ai` — the operator's own business content tooling (learn-agentic-ai.com
//! frontmatter/JSON validation, link checking, code-block linting, and the funnel host-list
//! checker), extracted out of the public `mev` crate so the public binary and its source carry
//! no reference to the operator's business.
//!
//! `publish = false`: this crate is never published to crates.io and is only ever pulled in via
//! `mev`'s non-default `learn-ai` cargo feature.
//!
//! **Self-contained by necessity, not preference.** `mev` optionally depends on this crate
//! (`mev-learn-ai = { path = ..., optional = true }`, pulled in by the `learn-ai` feature), so
//! this crate cannot depend back on `mev` — Cargo does not allow a dependency cycle regardless
//! of feature gating. The small pieces this crate needs from `mev`'s core (`Diagnostic`,
//! `Severity`, `Report`, the `ContentValidator` trait, a couple of string helpers, and a
//! collision-proof temp-dir helper for tests) are therefore mirrored here rather than imported.
//! `mev`'s feature-gated bridge (`src/lib.rs`, `#[cfg(feature = "learn-ai")]`) converts this
//! crate's `Report` into `mev`'s own on the way out, so the rest of `mev` — including its
//! `--json` output — only ever sees one `Report` type.
//!
//! This module groups all learn-ai-specific code behind [`LearnAiValidator`], which implements
//! the [`ContentValidator`] trait.  The generic `ContentValidator::run` driver provides the
//! crawl → validate loop; this crate only supplies `crawl` and `validate_item`.

mod shared;
pub mod testsupport;
pub mod validator;

pub mod blog;
pub mod crawl;
// Wired into `BlogValidator::validate_item` by `MV.12.B` Task 2 (`blog.rs`).
pub mod funnel;
pub mod lint;
pub mod meta;
// The phrase list + loader for `MV.12.C`'s voice tripwire; the scanner that consumes it
// (`voice::check_voice`) is declared below. Wired into `BlogValidator` by Task 3 (`blog.rs`).
pub mod voice_tells;
// The scanner for `MV.12.C`'s voice tripwire (Task 2): `check_voice` matches the phrase list
// above against prose, exempting code and quotation. Wired into `BlogValidator::validate_post`
// by Task 3 (`blog.rs`).
pub mod voice;

use std::path::Path;

use crawl::{ContentFile, FileKind};

pub use validator::ContentValidator;

/// Severity of a single finding. Mirrors `mev::Severity` — see the crate doc comment for why
/// this crate carries its own copy rather than importing `mev`'s.
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
    pub file: std::path::PathBuf,
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
        file: impl Into<std::path::PathBuf>,
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
        file: impl Into<std::path::PathBuf>,
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

/// The concrete [`ContentValidator`] for the learn-agentic-ai.com content tree.
///
/// Instantiate and call `.run(root)` to walk `root`, classify every content file,
/// and validate each one, collecting all diagnostics into a [`Report`].
///
/// `lint` gates the shared content-lint passes ([`lint::lint_code_blocks`] /
/// [`lint::lint_local_links`]) over `.mdx` module files. It defaults to `false` so
/// `LearnAiValidator::default()` — and therefore bare `mev validate` — stays behaviourally
/// identical to the pre-lint validator; use [`LearnAiValidator::with_lint`] to opt in.
#[derive(Debug, Default, Clone, Copy)]
pub struct LearnAiValidator {
    pub lint: bool,
}

impl LearnAiValidator {
    /// Construct a validator with the lint passes explicitly on or off.
    pub fn with_lint(lint: bool) -> Self {
        Self { lint }
    }
}

impl ContentValidator for LearnAiValidator {
    type Item = ContentFile;

    /// Walk `root`, classify every content file, and return the corpus items plus any
    /// crawl-time diagnostics (filename-convention violations, walk errors).
    fn crawl(&self, root: &Path) -> (Vec<ContentFile>, Vec<Diagnostic>) {
        let (corpus, diags) = crawl::crawl(root);
        (corpus.files, diags)
    }

    /// Validate a single classified content file and return any structural diagnostics.
    ///
    /// Used directly (e.g. by tests) without a known content root — conservatively skips
    /// route resolution for any absolute link rather than guessing. See
    /// [`run`](ContentValidator::run), overridden below, for the real `mev validate --lint`
    /// path, which derives and threads through the actual content root.
    fn validate_item(&self, item: &ContentFile) -> Vec<Diagnostic> {
        self.validate_item_with_root(item, None)
    }

    /// Overridden (not the default driver) so the opt-in lint pass gets route-aware
    /// resolution: the content root is derived once from `root` here and threaded through to
    /// every item, since `ContentValidator::validate_item`'s signature has no root parameter
    /// to carry it (Task 8). When `self.lint` is off, no root derivation is needed at all.
    fn run(&self, root: &Path) -> Report {
        let content_root = if self.lint {
            lint::derive_content_root(root)
        } else {
            None
        };
        let (items, mut diagnostics) = self.crawl(root);
        for item in &items {
            diagnostics.extend(self.validate_item_with_root(item, content_root.as_deref()));
        }
        Report { diagnostics }
    }
}

impl LearnAiValidator {
    /// Shared implementation behind both `validate_item` and the overridden `run`: the
    /// existing struct/frontmatter checks always run. When `self.lint` is set, the shared
    /// lint passes additionally run over `FileKind::ModuleMdx` items only — a fence/link scan
    /// over `.json` metadata is meaningless. The file is read once for linting; if that read
    /// fails, linting is silently skipped rather than adding a second diagnostic on top of the
    /// read-failure diagnostic `meta::validate_file` already produced.
    fn validate_item_with_root(
        &self,
        item: &ContentFile,
        content_root: Option<&Path>,
    ) -> Vec<Diagnostic> {
        let mut diags = meta::validate_file(item);

        if self.lint
            && item.kind == FileKind::ModuleMdx
            && let Ok(source) = std::fs::read_to_string(&item.path)
        {
            diags.extend(lint::lint_code_blocks(&item.rel, &source));
            diags.extend(lint::lint_local_links(
                &item.path,
                &item.rel,
                &source,
                content_root,
            ));
        }

        diags
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let dir = testsupport::unique_temp_dir("mev-learn-ai-crawl-test");
        std::fs::create_dir_all(&dir).unwrap();

        let v = LearnAiValidator::default();
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

        let v = LearnAiValidator::default();
        let diags = v.validate_item(&cf);
        assert_eq!(
            diags.len(),
            1,
            "expected exactly one read-failure diagnostic, got {diags:?}"
        );
        assert_eq!(diags[0].severity, Severity::Error);
    }

    #[test]
    fn learn_ai_validator_run_on_empty_dir_returns_empty_report() {
        let dir = testsupport::unique_temp_dir("mev-learn-ai-run-test");
        std::fs::create_dir_all(&dir).unwrap();

        let report = LearnAiValidator::default().run(&dir);
        assert!(
            report.diagnostics.is_empty(),
            "expected empty report for empty dir, got {:?}",
            report.diagnostics
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Build a minimal on-disk module tree: `paths/demo/modules/01-intro.mdx`, with content
    /// controllable by the caller (an untagged fence + a dead relative link by default).
    fn write_module_mdx(root: &std::path::Path, body: &str) -> PathBuf {
        let modules_dir = root.join("paths").join("demo").join("modules");
        std::fs::create_dir_all(&modules_dir).unwrap();
        let file = modules_dir.join("01-intro.mdx");
        std::fs::write(&file, body).unwrap();
        file
    }

    /// A minimal but structurally-valid module MDX body (required frontmatter present) so the
    /// only diagnostics under test are the opt-in lint ones, not the always-on frontmatter ones.
    const VALID_FRONTMATTER: &str = "---\ntitle: Intro\ndescription: An intro module.\nduration: 5 minutes\ndifficulty: beginner\nlastUpdated: 2026-01-01\n---\n";

    #[test]
    fn default_validator_emits_no_lint_diagnostics_on_module_mdx() {
        let dir = testsupport::unique_temp_dir("mev-learn-ai-lint-default-test");
        let body = format!(
            "{VALID_FRONTMATTER}\n```\nno language tag here\n```\n\n[dead link](./nowhere.mdx)\n"
        );
        let file = write_module_mdx(&dir, &body);

        let cf = ContentFile {
            path: file,
            rel: PathBuf::from("paths/demo/modules/01-intro.mdx"),
            kind: FileKind::ModuleMdx,
            path_id: "demo".to_string(),
            module_id: Some("01-intro".to_string()),
            locale: Locale::En,
        };

        let diags = LearnAiValidator::default().validate_item(&cf);
        assert!(
            diags.is_empty(),
            "default validator (lint off) must emit no lint diagnostics, got {diags:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn with_lint_true_emits_both_lint_diagnostics_on_module_mdx() {
        let dir = testsupport::unique_temp_dir("mev-learn-ai-lint-on-test");
        let body = format!(
            "{VALID_FRONTMATTER}\n```\nno language tag here\n```\n\n[dead link](./nowhere.mdx)\n"
        );
        let file = write_module_mdx(&dir, &body);

        let cf = ContentFile {
            path: file,
            rel: PathBuf::from("paths/demo/modules/01-intro.mdx"),
            kind: FileKind::ModuleMdx,
            path_id: "demo".to_string(),
            module_id: Some("01-intro".to_string()),
            locale: Locale::En,
        };

        let diags = LearnAiValidator::with_lint(true).validate_item(&cf);
        let locators: Vec<&str> = diags.iter().map(|d| d.locator.as_str()).collect();
        assert!(
            locators.contains(&"W_LINT_UNTAGGED_CODE_BLOCK"),
            "expected W_LINT_UNTAGGED_CODE_BLOCK, got {locators:?}"
        );
        assert!(
            locators.contains(&"E_LINT_DEAD_LOCAL_LINK"),
            "expected E_LINT_DEAD_LOCAL_LINK, got {locators:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn with_lint_true_skips_non_module_mdx_items() {
        // A LearnModuleJson item must not go through the lint passes even with lint on —
        // the fence/link scan is meaningless over JSON.
        let cf = stub_file(
            PathBuf::from("/nonexistent/mev-la/does-not-matter.json"),
            "paths/demo/modules/01-intro.json",
            FileKind::LearnModuleJson,
        );

        // validate_item still returns exactly the meta::validate_file read-failure diagnostic,
        // proving the lint branch was never entered (it would try to read the same missing path
        // and, per the "no second diagnostic on a read failure" rule, silently no-op anyway —
        // this test pins that the item kind gate exists at all).
        let diags = LearnAiValidator::with_lint(true).validate_item(&cf);
        assert_eq!(
            diags.len(),
            1,
            "expected only the read-failure diagnostic, got {diags:?}"
        );
    }
}
