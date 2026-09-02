//! Check `project-cache-watermark` — each `docs/projects/<project>.md` `synced_from` vs
//! the sub-repo's real `planning/status.md` `timestamp`.
//!
//! This is an **adapter**, not a reimplementation: the actual comparison already lives
//! in [`crate::brain::sync::check_sync`] (`mev validate-brain --sync`), which parses
//! RFC3339 watermarks and compares them as instants. This check calls that function
//! and translates its `Vec<Diagnostic>` into a [`super::CheckOutcome`] so the drift
//! shows up in the same report as the other three checks — it does not re-parse
//! frontmatter or re-implement RFC3339 handling.

use super::{CheckOutcome, CheckStatus, ConformanceCtx, FactSide};
use crate::brain::sync::{check_sync, read_watermark};

/// Render one repo's watermark as `"<repo>=<watermark-or-missing>"` for display/digest
/// purposes only — the verdict itself comes from `check_sync`'s diagnostics, not from
/// comparing these rendered strings.
fn render_status_watermark(
    root: &std::path::Path,
    repo: &crate::brain::config::RepoEntry,
) -> String {
    let path = root.join(&repo.status_file);
    let watermark = read_watermark(&path)
        .ok()
        .and_then(|fm| fm.timestamp)
        .unwrap_or_else(|| "missing".to_string());
    format!("{}={watermark}", repo.slug)
}

/// Render one repo's cache-doc watermark as `"<repo>=<watermark-or-missing>"`.
fn render_cache_watermark(
    root: &std::path::Path,
    repo: &crate::brain::config::RepoEntry,
) -> String {
    let path = root.join(&repo.cache_doc);
    let watermark = read_watermark(&path)
        .ok()
        .and_then(|fm| fm.synced_from)
        .unwrap_or_else(|| "missing".to_string());
    format!("{}={watermark}", repo.slug)
}

/// Run the `project-cache-watermark` check.
pub fn run(ctx: &ConformanceCtx) -> CheckOutcome {
    let diagnostics = check_sync(&ctx.root, &ctx.config);

    let mut left_items: Vec<String> = ctx
        .config
        .repos
        .iter()
        .map(|r| render_status_watermark(&ctx.root, r))
        .collect();
    left_items.sort();

    let mut right_items: Vec<String> = ctx
        .config
        .repos
        .iter()
        .map(|r| render_cache_watermark(&ctx.root, r))
        .collect();
    right_items.sort();

    let left = FactSide {
        label: "sub-repo status_file timestamps".to_string(),
        source: "each [[repos]].status_file".to_string(),
        digest: super::digest(&left_items),
        items: left_items,
    };
    let right = FactSide {
        label: "brain cache_doc synced_from".to_string(),
        source: "each [[repos]].cache_doc".to_string(),
        digest: super::digest(&right_items),
        items: right_items,
    };

    if diagnostics.is_empty() {
        return CheckOutcome {
            status: CheckStatus::Pass,
            left: Some(left),
            right: Some(right),
            findings: Vec::new(),
            reason: None,
        };
    }

    let findings = diagnostics
        .iter()
        .map(|d| format!("{}: {}", d.locator, d.message))
        .collect();

    CheckOutcome {
        status: CheckStatus::Drift,
        left: Some(left),
        right: Some(right),
        findings,
        reason: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::config::{BrainConfig, RepoEntry};

    fn write_watermark_doc(path: &std::path::Path, field: &str, value: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, format!("---\n{field}: \"{value}\"\n---\n\n# doc\n")).unwrap();
    }

    fn repo_entry(slug: &str) -> RepoEntry {
        RepoEntry {
            public: false,
            slug: slug.to_string(),
            tier: "primary".to_string(),
            repo_path: slug.to_string(),
            status_file: format!("{slug}/planning/status.md"),
            cache_doc: format!("docs/projects/{slug}.md"),
            heading: slug.to_string(),
            prefix: None,
        }
    }

    fn ctx_with(root: &std::path::Path, repos: Vec<RepoEntry>) -> ConformanceCtx {
        ConformanceCtx {
            root: root.to_path_buf(),
            config: BrainConfig {
                repos,
                ..Default::default()
            },
            files: Vec::new(),
        }
    }

    #[test]
    fn matching_watermarks_pass() {
        let root = crate::testsupport::unique_temp_dir("mev-conformance-cache-match");
        let repo = repo_entry("demo");
        write_watermark_doc(
            &root.join(&repo.status_file),
            "timestamp",
            "2026-08-01T00:00:00Z",
        );
        write_watermark_doc(
            &root.join(&repo.cache_doc),
            "synced_from",
            "2026-08-01T00:00:00Z",
        );
        let ctx = ctx_with(&root, vec![repo]);

        let outcome = run(&ctx);
        assert_eq!(outcome.status, CheckStatus::Pass);
        assert!(outcome.findings.is_empty());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn mismatched_watermarks_drift_with_e_sync_drift_code() {
        let root = crate::testsupport::unique_temp_dir("mev-conformance-cache-drift");
        let repo = repo_entry("demo");
        write_watermark_doc(
            &root.join(&repo.status_file),
            "timestamp",
            "2026-08-01T00:00:00Z",
        );
        write_watermark_doc(
            &root.join(&repo.cache_doc),
            "synced_from",
            "2026-07-01T00:00:00Z",
        );
        let ctx = ctx_with(&root, vec![repo]);

        let outcome = run(&ctx);
        assert_eq!(outcome.status, CheckStatus::Drift);
        assert!(
            outcome.findings.iter().any(|f| f.contains("E_SYNC_DRIFT")),
            "expected an E_SYNC_DRIFT finding, got {:?}",
            outcome.findings
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_cache_doc_drifts_with_e_sync_file_missing_code() {
        let root = crate::testsupport::unique_temp_dir("mev-conformance-cache-missing");
        let repo = repo_entry("demo");
        write_watermark_doc(
            &root.join(&repo.status_file),
            "timestamp",
            "2026-08-01T00:00:00Z",
        );
        // cache_doc intentionally not written.
        let ctx = ctx_with(&root, vec![repo]);

        let outcome = run(&ctx);
        assert_eq!(outcome.status, CheckStatus::Drift);
        assert!(
            outcome
                .findings
                .iter()
                .any(|f| f.contains("E_SYNC_FILE_MISSING")),
            "expected an E_SYNC_FILE_MISSING finding, got {:?}",
            outcome.findings
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}
