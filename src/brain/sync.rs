//! Cross-repo sync watermark check for `mev validate-brain --sync`.
//!
//! Phase 3, Block M (HQ-Restructure Block N): per `brain.toml` `[[repos]]` entry,
//! compares the sub-repo's `planning/status.md` `timestamp` against the brain cache
//! doc's `synced_from`.  Strict RFC3339 parsing enforces the precision contract.
//!
//! Diagnostic locator codes:
//! - `E_SYNC_DRIFT`               — both watermarks parse but differ.
//! - `E_SYNC_WATERMARK_MISSING`   — `timestamp` or `synced_from` field is absent.
//! - `E_SYNC_WATERMARK_MALFORMED` — a watermark is present but not valid RFC3339.
//! - `E_SYNC_FILE_MISSING`        — `status_file` or `cache_doc` does not exist.

use std::path::Path;

use chrono::DateTime;
use serde::Deserialize;

use crate::Diagnostic;
use crate::brain::config::BrainConfig;
use crate::shared::extract_frontmatter;

// ---------------------------------------------------------------------------
// RFC3339 parser
// ---------------------------------------------------------------------------

/// Parse `s` strictly as an RFC3339 datetime.
///
/// Returns `Ok(DateTime<FixedOffset>)` on success, or `Err(String)` describing why
/// the value was rejected.  A date-only string (e.g. `"2026-06-27"`) is **not** a
/// valid RFC3339 datetime and is rejected.
pub(crate) fn parse_watermark(s: &str) -> Result<DateTime<chrono::FixedOffset>, String> {
    DateTime::parse_from_rfc3339(s).map_err(|e| format!("not valid RFC3339: {e} (value: {s:?})"))
}

// ---------------------------------------------------------------------------
// Watermark frontmatter struct
// ---------------------------------------------------------------------------

/// Minimal serde struct that captures only the watermark-relevant fields from a
/// Markdown frontmatter block.  Unknown fields are tolerated (no `deny_unknown_fields`).
///
/// Used for **both** the source `status_file` (reads `timestamp`) and the brain
/// cache `cache_doc` (reads `synced_from`).
#[derive(Debug, Deserialize)]
struct WatermarkFrontmatter {
    /// Source watermark: the `timestamp` scalar in a sub-repo's `status_file`.
    #[serde(default)]
    pub timestamp: Option<String>,
    /// Cache watermark: the `synced_from` scalar in the brain cache `cache_doc`.
    #[serde(default)]
    pub synced_from: Option<String>,
}

// ---------------------------------------------------------------------------
// File-level watermark reader
// ---------------------------------------------------------------------------

/// Read the file at `path`, extract its YAML frontmatter, and deserialize it as
/// [`WatermarkFrontmatter`].
///
/// Returns `Ok(WatermarkFrontmatter)` on success, or an `Err(String)` describing the
/// failure (file not readable, no frontmatter block, or parse error).  The caller is
/// responsible for translating the error into the appropriate [`Diagnostic`].
fn read_watermark(path: &Path) -> Result<WatermarkFrontmatter, String> {
    let contents = std::fs::read_to_string(path)
        .map_err(|e| format!("could not read {}: {e}", path.display()))?;

    let yaml = extract_frontmatter(&contents)
        .ok_or_else(|| format!("no frontmatter block found in {}", path.display()))?;

    serde_yaml::from_str::<WatermarkFrontmatter>(yaml)
        .map_err(|e| format!("could not parse frontmatter in {}: {e}", path.display()))
}

// ---------------------------------------------------------------------------
// Public sync check
// ---------------------------------------------------------------------------

/// Run the sync watermark check for every `[[repos]]` entry in `config`.
///
/// For each entry, resolves `root.join(repo.status_file)` (the source watermark) and
/// `root.join(repo.cache_doc)` (the cache watermark), then compares them.  Emits one
/// [`Diagnostic`] per mismatch per the locator-code table in the module doc.
///
/// Returns a (possibly empty) `Vec<Diagnostic>`.  An empty vec means all repos are
/// in-sync.
pub fn check_sync(root: &Path, config: &BrainConfig) -> Vec<Diagnostic> {
    let mut diags: Vec<Diagnostic> = Vec::new();

    for repo in &config.repos {
        let source_path = root.join(&repo.status_file);
        let cache_path = root.join(&repo.cache_doc);

        // --- check source file exists ---
        if !source_path.exists() {
            diags.push(Diagnostic::error(
                source_path.clone(),
                "E_SYNC_FILE_MISSING",
                format!(
                    "repo '{}': status_file '{}' does not exist",
                    repo.slug, repo.status_file
                ),
            ));
            continue;
        }

        // --- check cache file exists ---
        if !cache_path.exists() {
            diags.push(Diagnostic::error(
                cache_path.clone(),
                "E_SYNC_FILE_MISSING",
                format!(
                    "repo '{}': cache_doc '{}' does not exist",
                    repo.slug, repo.cache_doc
                ),
            ));
            continue;
        }

        // --- read source watermark ---
        let source_fm = match read_watermark(&source_path) {
            Ok(fm) => fm,
            Err(e) => {
                diags.push(Diagnostic::error(
                    source_path.clone(),
                    "E_SYNC_WATERMARK_MALFORMED",
                    format!("repo '{}': could not read status_file: {e}", repo.slug),
                ));
                continue;
            }
        };

        // --- read cache watermark ---
        let cache_fm = match read_watermark(&cache_path) {
            Ok(fm) => fm,
            Err(e) => {
                diags.push(Diagnostic::error(
                    cache_path.clone(),
                    "E_SYNC_WATERMARK_MALFORMED",
                    format!("repo '{}': could not read cache_doc: {e}", repo.slug),
                ));
                continue;
            }
        };

        // --- check source `timestamp` present ---
        let timestamp_str = match &source_fm.timestamp {
            Some(s) => s.clone(),
            None => {
                diags.push(Diagnostic::error(
                    source_path.clone(),
                    "E_SYNC_WATERMARK_MISSING",
                    format!(
                        "repo '{}': status_file '{}' has no 'timestamp' field",
                        repo.slug, repo.status_file
                    ),
                ));
                continue;
            }
        };

        // --- check cache `synced_from` present ---
        let synced_from_str = match &cache_fm.synced_from {
            Some(s) => s.clone(),
            None => {
                diags.push(Diagnostic::error(
                    cache_path.clone(),
                    "E_SYNC_WATERMARK_MISSING",
                    format!(
                        "repo '{}': cache_doc '{}' has no 'synced_from' field",
                        repo.slug, repo.cache_doc
                    ),
                ));
                continue;
            }
        };

        // --- validate RFC3339 format for source timestamp ---
        let source_dt = match parse_watermark(&timestamp_str) {
            Ok(dt) => dt,
            Err(e) => {
                diags.push(Diagnostic::error(
                    source_path.clone(),
                    "E_SYNC_WATERMARK_MALFORMED",
                    format!(
                        "repo '{}': status_file 'timestamp' is not valid RFC3339: {e}",
                        repo.slug
                    ),
                ));
                continue;
            }
        };

        // --- validate RFC3339 format for cache synced_from ---
        let cache_dt = match parse_watermark(&synced_from_str) {
            Ok(dt) => dt,
            Err(e) => {
                diags.push(Diagnostic::error(
                    cache_path.clone(),
                    "E_SYNC_WATERMARK_MALFORMED",
                    format!(
                        "repo '{}': cache_doc 'synced_from' is not valid RFC3339: {e}",
                        repo.slug
                    ),
                ));
                continue;
            }
        };

        // --- compare watermarks ---
        if source_dt != cache_dt {
            diags.push(Diagnostic::error(
                cache_path.clone(),
                "E_SYNC_DRIFT",
                format!(
                    "repo '{}': timestamp '{timestamp_str}' != synced_from '{synced_from_str}'",
                    repo.slug
                ),
            ));
        }
        // equal → no diagnostic
    }

    diags
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::config::{BrainConfig, CrawlConfig, RepoEntry, VocabConfig};

    // -----------------------------------------------------------------------
    // parse_watermark tests (from Task 1)
    // -----------------------------------------------------------------------

    #[test]
    fn full_rfc3339_parses_ok() {
        assert!(
            parse_watermark("2026-06-27T12:00:00+00:00").is_ok(),
            "a full RFC3339 datetime should parse"
        );
    }

    #[test]
    fn full_rfc3339_with_z_suffix_parses_ok() {
        assert!(
            parse_watermark("2026-06-27T00:00:00Z").is_ok(),
            "RFC3339 with Z suffix should parse"
        );
    }

    #[test]
    fn date_only_is_rejected() {
        let result = parse_watermark("2026-06-27");
        assert!(
            result.is_err(),
            "date-only value should be rejected as non-RFC3339"
        );
    }

    #[test]
    fn garbage_value_is_rejected() {
        let result = parse_watermark("not-a-date");
        assert!(result.is_err(), "garbage value should be rejected");
    }

    #[test]
    fn datetime_without_offset_is_rejected() {
        // ISO 8601 without timezone offset is not RFC3339.
        let result = parse_watermark("2026-06-27T12:00:00");
        assert!(
            result.is_err(),
            "datetime without timezone offset should be rejected as non-RFC3339"
        );
    }

    // -----------------------------------------------------------------------
    // check_sync fixture helpers
    // -----------------------------------------------------------------------

    /// Build a minimal `BrainConfig` with one `[[repos]]` entry whose paths point
    /// at `status_file_rel` and `cache_doc_rel` (relative to the temp root).
    fn make_config(status_file_rel: &str, cache_doc_rel: &str) -> BrainConfig {
        BrainConfig {
            vocab: VocabConfig::default(),
            crawl: CrawlConfig::default(),
            repos: vec![RepoEntry {
                slug: "test-repo".to_string(),
                tier: "primary".to_string(),
                repo_path: ".".to_string(),
                status_file: status_file_rel.to_string(),
                cache_doc: cache_doc_rel.to_string(),
                heading: "Test Repo".to_string(),
            }],
        }
    }

    fn status_md_with_timestamp(ts: &str) -> String {
        format!(
            "---\ntype: ProjectStatus\ntitle: Status\ndescription: Test\ntimestamp: \"{ts}\"\n---\n\n# Status\n"
        )
    }

    fn cache_md_with_synced_from(sf: &str) -> String {
        format!(
            "---\ntype: ProjectContext\ntitle: Cache\ndescription: Test cache\nsynced_from: \"{sf}\"\n---\n\n# Cache\n"
        )
    }

    // -----------------------------------------------------------------------
    // check_sync unit tests
    // -----------------------------------------------------------------------

    #[test]
    fn in_sync_repo_produces_no_diagnostics() {
        let dir = std::env::temp_dir().join("mev-sync-test-in-sync");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let ts = "2026-06-27T00:00:00Z";
        std::fs::write(dir.join("status.md"), status_md_with_timestamp(ts)).unwrap();
        std::fs::write(dir.join("cache.md"), cache_md_with_synced_from(ts)).unwrap();

        let config = make_config("status.md", "cache.md");
        let diags = check_sync(&dir, &config);

        assert!(
            diags.is_empty(),
            "in-sync repo should produce no diagnostics, got: {diags:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn drifted_repo_produces_e_sync_drift() {
        let dir = std::env::temp_dir().join("mev-sync-test-drift");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        std::fs::write(
            dir.join("status.md"),
            status_md_with_timestamp("2026-06-28T00:00:00Z"),
        )
        .unwrap();
        std::fs::write(
            dir.join("cache.md"),
            cache_md_with_synced_from("2026-06-27T00:00:00Z"),
        )
        .unwrap();

        let config = make_config("status.md", "cache.md");
        let diags = check_sync(&dir, &config);

        assert_eq!(
            diags.len(),
            1,
            "drifted repo should produce exactly one diagnostic, got: {diags:?}"
        );
        assert_eq!(
            diags[0].locator, "E_SYNC_DRIFT",
            "expected E_SYNC_DRIFT, got: {:?}",
            diags[0].locator
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_timestamp_produces_e_sync_watermark_missing() {
        let dir = std::env::temp_dir().join("mev-sync-test-missing-ts");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // status.md has no timestamp field
        std::fs::write(
            dir.join("status.md"),
            "---\ntype: ProjectStatus\ntitle: Status\ndescription: Test\n---\n\n# Status\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("cache.md"),
            cache_md_with_synced_from("2026-06-27T00:00:00Z"),
        )
        .unwrap();

        let config = make_config("status.md", "cache.md");
        let diags = check_sync(&dir, &config);

        assert_eq!(
            diags.len(),
            1,
            "missing timestamp should produce exactly one diagnostic, got: {diags:?}"
        );
        assert_eq!(
            diags[0].locator, "E_SYNC_WATERMARK_MISSING",
            "expected E_SYNC_WATERMARK_MISSING, got: {:?}",
            diags[0].locator
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn date_only_watermark_produces_e_sync_watermark_malformed() {
        let dir = std::env::temp_dir().join("mev-sync-test-malformed");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // timestamp is date-only (not RFC3339)
        std::fs::write(
            dir.join("status.md"),
            status_md_with_timestamp("2026-06-27"),
        )
        .unwrap();
        std::fs::write(
            dir.join("cache.md"),
            cache_md_with_synced_from("2026-06-27T00:00:00Z"),
        )
        .unwrap();

        let config = make_config("status.md", "cache.md");
        let diags = check_sync(&dir, &config);

        assert_eq!(
            diags.len(),
            1,
            "malformed watermark should produce exactly one diagnostic, got: {diags:?}"
        );
        assert_eq!(
            diags[0].locator, "E_SYNC_WATERMARK_MALFORMED",
            "expected E_SYNC_WATERMARK_MALFORMED, got: {:?}",
            diags[0].locator
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_status_file_produces_e_sync_file_missing() {
        let dir = std::env::temp_dir().join("mev-sync-test-file-missing");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // Only create the cache doc; no status.md
        std::fs::write(
            dir.join("cache.md"),
            cache_md_with_synced_from("2026-06-27T00:00:00Z"),
        )
        .unwrap();

        let config = make_config("status.md", "cache.md");
        let diags = check_sync(&dir, &config);

        assert_eq!(
            diags.len(),
            1,
            "missing file should produce exactly one diagnostic, got: {diags:?}"
        );
        assert_eq!(
            diags[0].locator, "E_SYNC_FILE_MISSING",
            "expected E_SYNC_FILE_MISSING, got: {:?}",
            diags[0].locator
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_cache_doc_produces_e_sync_file_missing() {
        let dir = std::env::temp_dir().join("mev-sync-test-cache-missing");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // Only create the status file; no cache.md
        std::fs::write(
            dir.join("status.md"),
            status_md_with_timestamp("2026-06-27T00:00:00Z"),
        )
        .unwrap();

        let config = make_config("status.md", "cache.md");
        let diags = check_sync(&dir, &config);

        assert_eq!(
            diags.len(),
            1,
            "missing cache doc should produce exactly one diagnostic, got: {diags:?}"
        );
        assert_eq!(
            diags[0].locator, "E_SYNC_FILE_MISSING",
            "expected E_SYNC_FILE_MISSING, got: {:?}",
            diags[0].locator
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn re_aligning_cache_clears_drift_error() {
        let dir = std::env::temp_dir().join("mev-sync-test-realign");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let ts = "2026-06-28T00:00:00Z";
        // First: out of sync
        std::fs::write(dir.join("status.md"), status_md_with_timestamp(ts)).unwrap();
        std::fs::write(
            dir.join("cache.md"),
            cache_md_with_synced_from("2026-06-27T00:00:00Z"),
        )
        .unwrap();

        let config = make_config("status.md", "cache.md");
        let diags_before = check_sync(&dir, &config);
        assert_eq!(
            diags_before.len(),
            1,
            "should have one drift error before re-align"
        );

        // Now align cache to match the timestamp
        std::fs::write(dir.join("cache.md"), cache_md_with_synced_from(ts)).unwrap();
        let diags_after = check_sync(&dir, &config);
        assert!(
            diags_after.is_empty(),
            "re-aligning cache should clear the drift error, got: {diags_after:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_synced_from_produces_e_sync_watermark_missing() {
        let dir = std::env::temp_dir().join("mev-sync-test-missing-sf");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        std::fs::write(
            dir.join("status.md"),
            status_md_with_timestamp("2026-06-27T00:00:00Z"),
        )
        .unwrap();
        // cache.md has no synced_from
        std::fs::write(
            dir.join("cache.md"),
            "---\ntype: ProjectContext\ntitle: Cache\ndescription: Test cache\n---\n\n# Cache\n",
        )
        .unwrap();

        let config = make_config("status.md", "cache.md");
        let diags = check_sync(&dir, &config);

        assert_eq!(
            diags.len(),
            1,
            "missing synced_from should produce exactly one diagnostic, got: {diags:?}"
        );
        assert_eq!(
            diags[0].locator, "E_SYNC_WATERMARK_MISSING",
            "expected E_SYNC_WATERMARK_MISSING, got: {:?}",
            diags[0].locator
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
