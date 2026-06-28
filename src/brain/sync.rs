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

use chrono::DateTime;

// ---------------------------------------------------------------------------
// RFC3339 parser
// ---------------------------------------------------------------------------

/// Parse `s` strictly as an RFC3339 datetime.
///
/// Returns `Ok(DateTime<FixedOffset>)` on success, or `Err(String)` describing why
/// the value was rejected.  A date-only string (e.g. `"2026-06-27"`) is **not** a
/// valid RFC3339 datetime and is rejected.
// Task 2 will call this from `check_sync`; allow dead-code until then.
#[allow(dead_code)]
pub(crate) fn parse_watermark(s: &str) -> Result<DateTime<chrono::FixedOffset>, String> {
    DateTime::parse_from_rfc3339(s).map_err(|e| format!("not valid RFC3339: {e} (value: {s:?})"))
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

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
}
