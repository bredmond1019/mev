//! Check `toolchain-freshness` — the running `mev` binary's build provenance vs the
//! source tree it was built from.
//!
//! This is the incident check: a `mev` binary built two days stale destroyed 29 authored
//! block notes in a single session because nothing compared it to its source before it
//! ran with `--write`. [`build.rs`](../../../build.rs) stamps three `cargo:rustc-env`
//! values into the binary at compile time (`MEV_BUILD_GIT_SHA`, `MEV_BUILD_DIRTY`,
//! `MEV_BUILD_SOURCE_DIR`); this check re-derives the live value now and compares.
//!
//! The verdict is a pure function of `(stamped_sha, live_sha, dirty, source_dir_exists)` —
//! [`verdict`] — so it is unit-tested directly without shelling out to git.

use std::path::Path;

use super::{CheckOutcome, CheckStatus, ConformanceCtx, FactSide};

const STAMPED_SHA: &str = env!("MEV_BUILD_GIT_SHA");
const STAMPED_DIRTY: &str = env!("MEV_BUILD_DIRTY");
const STAMPED_SOURCE_DIR: &str = env!("MEV_BUILD_SOURCE_DIR");

/// The pure verdict function: given the compiled-in stamp and the live state of the
/// source tree, decide pass / drift / not-evaluable. No I/O — callers gather the live
/// values (via git or otherwise) and pass them in, which is what makes this directly
/// unit-testable without shelling out to git in tests.
fn verdict(
    stamped_sha: &str,
    live_sha: Option<&str>,
    dirty: &str,
    source_dir_exists: bool,
) -> (CheckStatus, Vec<String>, Option<String>) {
    if stamped_sha == "unknown" || dirty == "unknown" || !source_dir_exists {
        return (
            CheckStatus::NotEvaluable,
            Vec::new(),
            Some(
                "build provenance unavailable: stamped SHA, dirty flag, or source dir missing"
                    .to_string(),
            ),
        );
    }

    let Some(live_sha) = live_sha else {
        return (
            CheckStatus::NotEvaluable,
            Vec::new(),
            Some(
                "could not determine the live HEAD of the source tree (git unavailable)"
                    .to_string(),
            ),
        );
    };

    if live_sha == "unknown" {
        return (
            CheckStatus::NotEvaluable,
            Vec::new(),
            Some(
                "could not determine the live HEAD of the source tree (git unavailable)"
                    .to_string(),
            ),
        );
    }

    if stamped_sha != live_sha {
        return (
            CheckStatus::Drift,
            vec![format!(
                "the running binary was built from {stamped_sha} but the source is now at \
                 {live_sha}; rebuild before any --write run"
            )],
            None,
        );
    }

    if dirty == "1" {
        return (
            CheckStatus::Drift,
            vec![
                "the binary was built from an uncommitted tree, so its provenance is \
                 unverifiable"
                    .to_string(),
            ],
            None,
        );
    }

    (CheckStatus::Pass, Vec::new(), None)
}

/// Build the `--build-stamp` JSON payload from raw stamp values, pure and testable without
/// touching the compiled-in consts.
///
/// This is a pinned cross-repo contract with bastion's `src/buildstamp.rs` (see
/// `MV.ticket.toolchain-freshness-covers-the-writer`) — the key set
/// (`git_sha`, `dirty`, `source_dir`) must never gain, lose, or rename a key. `dirty` is a
/// JSON boolean when the raw stamp is the literal `"0"`/`"1"`, and the JSON string
/// `"unknown"` for anything else — never guessed.
pub fn stamp_json_from(git_sha: &str, dirty: &str, source_dir: &str) -> serde_json::Value {
    let dirty_value = match dirty {
        "0" => serde_json::Value::Bool(false),
        "1" => serde_json::Value::Bool(true),
        _ => serde_json::Value::String("unknown".to_string()),
    };
    serde_json::json!({
        "git_sha": git_sha,
        "dirty": dirty_value,
        "source_dir": source_dir,
    })
}

/// The `--build-stamp` JSON payload for this compiled binary.
pub fn stamp_json() -> serde_json::Value {
    stamp_json_from(STAMPED_SHA, STAMPED_DIRTY, STAMPED_SOURCE_DIR)
}

/// Run `git rev-parse HEAD` in `source_dir` now, returning `None` if git or the command
/// is unavailable.
fn live_head(source_dir: &str) -> Option<String> {
    if !Path::new(source_dir).exists() {
        return None;
    }
    let output = crate::shared::git_command()
        .args(["rev-parse", "HEAD"])
        .current_dir(source_dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|s| s.trim().to_string())
}

/// Run the `toolchain-freshness` check.
pub fn run(_ctx: &ConformanceCtx) -> CheckOutcome {
    let source_dir_exists = Path::new(STAMPED_SOURCE_DIR).exists();
    let live_sha = if source_dir_exists {
        live_head(STAMPED_SOURCE_DIR)
    } else {
        None
    };

    let (status, findings, reason) = verdict(
        STAMPED_SHA,
        live_sha.as_deref(),
        STAMPED_DIRTY,
        source_dir_exists,
    );

    let left = FactSide {
        label: "compiled-in build stamp".to_string(),
        source: "MEV_BUILD_GIT_SHA / MEV_BUILD_DIRTY (env! at compile time)".to_string(),
        digest: super::digest(&[STAMPED_SHA.to_string()]),
        items: vec![
            format!("sha={STAMPED_SHA}"),
            format!("dirty={STAMPED_DIRTY}"),
        ],
    };
    let live_sha_display = live_sha
        .clone()
        .unwrap_or_else(|| "unavailable".to_string());
    let right = FactSide {
        label: "live source tree HEAD".to_string(),
        source: format!("git rev-parse HEAD in {STAMPED_SOURCE_DIR}"),
        digest: super::digest(std::slice::from_ref(&live_sha_display)),
        items: vec![format!("sha={live_sha_display}")],
    };

    CheckOutcome {
        status,
        left: Some(left),
        right: Some(right),
        findings,
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_evaluable_when_stamped_sha_unknown() {
        let (status, _findings, reason) = verdict("unknown", Some("abc123"), "0", true);
        assert_eq!(status, CheckStatus::NotEvaluable);
        assert!(reason.is_some());
    }

    #[test]
    fn not_evaluable_when_dirty_flag_unknown() {
        let (status, _findings, reason) = verdict("abc123", Some("abc123"), "unknown", true);
        assert_eq!(status, CheckStatus::NotEvaluable);
        assert!(reason.is_some());
    }

    #[test]
    fn not_evaluable_when_source_dir_missing() {
        let (status, _findings, reason) = verdict("abc123", Some("abc123"), "0", false);
        assert_eq!(status, CheckStatus::NotEvaluable);
        assert!(reason.is_some());
    }

    #[test]
    fn not_evaluable_when_live_sha_unavailable() {
        let (status, _findings, reason) = verdict("abc123", None, "0", true);
        assert_eq!(status, CheckStatus::NotEvaluable);
        assert!(reason.is_some());
    }

    #[test]
    fn not_evaluable_when_live_sha_literal_unknown() {
        let (status, _findings, reason) = verdict("abc123", Some("unknown"), "0", true);
        assert_eq!(status, CheckStatus::NotEvaluable);
        assert!(reason.is_some());
    }

    #[test]
    fn drift_when_sha_differs() {
        let (status, findings, reason) = verdict("abc123", Some("def456"), "0", true);
        assert_eq!(status, CheckStatus::Drift);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].contains("abc123"));
        assert!(findings[0].contains("def456"));
        assert!(findings[0].contains("rebuild"));
        assert!(reason.is_none());
    }

    #[test]
    fn drift_when_dirty_even_with_matching_sha() {
        let (status, findings, reason) = verdict("abc123", Some("abc123"), "1", true);
        assert_eq!(status, CheckStatus::Drift);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].contains("uncommitted"));
        assert!(reason.is_none());
    }

    #[test]
    fn dirty_drift_message_distinct_from_stale_sha_drift_message() {
        let (_status1, stale_findings, _) = verdict("abc123", Some("def456"), "0", true);
        let (_status2, dirty_findings, _) = verdict("abc123", Some("abc123"), "1", true);
        assert_ne!(stale_findings[0], dirty_findings[0]);
    }

    #[test]
    fn pass_when_sha_matches_and_clean() {
        let (status, findings, reason) = verdict("abc123", Some("abc123"), "0", true);
        assert_eq!(status, CheckStatus::Pass);
        assert!(findings.is_empty());
        assert!(reason.is_none());
    }

    #[test]
    fn stamp_json_from_reports_clean_boolean_dirty() {
        let v = stamp_json_from("abc123", "0", "/tmp/src");
        assert_eq!(v["git_sha"], serde_json::json!("abc123"));
        assert_eq!(v["dirty"], serde_json::json!(false));
        assert_eq!(v["source_dir"], serde_json::json!("/tmp/src"));
    }

    #[test]
    fn stamp_json_from_reports_dirty_boolean_true() {
        let v = stamp_json_from("abc123", "1", "/tmp/src");
        assert_eq!(v["dirty"], serde_json::json!(true));
    }

    #[test]
    fn stamp_json_from_reports_unknown_dirty_as_string_not_guessed() {
        let v = stamp_json_from("unknown", "unknown", "/tmp/src");
        assert_eq!(v["dirty"], serde_json::json!("unknown"));
    }

    #[test]
    fn stamp_json_from_has_exactly_three_keys() {
        let v = stamp_json_from("abc123", "0", "/tmp/src");
        let obj = v.as_object().expect("stamp json must be an object");
        assert_eq!(obj.len(), 3);
        assert!(obj.contains_key("git_sha"));
        assert!(obj.contains_key("dirty"));
        assert!(obj.contains_key("source_dir"));
    }

    #[test]
    fn stamp_json_uses_compiled_in_consts() {
        let v = stamp_json();
        assert_eq!(v["git_sha"], serde_json::json!(STAMPED_SHA));
        assert_eq!(v["source_dir"], serde_json::json!(STAMPED_SOURCE_DIR));
    }

    #[test]
    fn run_executes_without_panicking() {
        // Smoke test: the real `run` function reads the actual compiled-in stamp and
        // shells out to the real source dir. It must not panic regardless of the
        // environment this test runs in (git present or not, source dir intact or not).
        let ctx = ConformanceCtx {
            root: std::path::PathBuf::from("."),
            config: crate::brain::config::BrainConfig::default(),
            files: Vec::new(),
        };
        let outcome = run(&ctx);
        assert!(outcome.left.is_some());
        assert!(outcome.right.is_some());
    }
}
