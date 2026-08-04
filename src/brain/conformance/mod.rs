//! `mev conformance` — a registry of named drift checks.
//!
//! Modelled on qm's `cli/src/commands/conformance.ts:60-77`: each check canonicalizes
//! both sides of a fact that is stored in two places, digests each side, and reports
//! divergence with the concrete set difference. This module (`mod.rs`) supplies the
//! shared types, the in-house FNV-1a digest, the shared `compare_sides` verdict body
//! every set-parity check uses, and the registry driver (`all_checks`/`run_checks`).
//!
//! Seed checks (tasks 2-5) each live in their own file and register one entry in
//! [`all_checks`]:
//! - `backlog.rs` — `backlog-parity`
//! - `epics_index.rs` — `epics-index-parity`
//! - `project_cache.rs` — `project-cache-watermark` (adapter over [`crate::brain::sync`])
//! - `toolchain.rs` — `toolchain-freshness`
//!
//! Adding a fifth check is adding one file plus one entry in [`all_checks`] — nothing
//! else here changes.

use std::path::PathBuf;

use serde::Serialize;

use crate::brain::config::BrainConfig;
use crate::brain::state::{StateFile, StateSource};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// The verdict a single check reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    /// Both sides canonicalize and digest identically.
    Pass,
    /// The two sides diverge — the check found a real, reportable difference.
    Drift,
    /// The check's inputs were absent (a file missing, a source not loaded), so no
    /// genuine two-sided comparison could run. Never used for a real divergence.
    NotEvaluable,
}

/// One side of a duplicated fact: what it was read from, its canonical sorted item
/// list, and that list's digest.
#[derive(Debug, Clone, Serialize)]
pub struct FactSide {
    /// Human label for this side (e.g. `"planning/backlog.md"`).
    pub label: String,
    /// Where this side's data was actually read from (a path or a source description).
    pub source: String,
    /// The FNV-1a digest of `items`, computed by [`digest`].
    pub digest: String,
    /// The canonical, sorted item list this side reduced to.
    pub items: Vec<String>,
}

/// The result of running a single check.
#[derive(Debug, Clone, Serialize)]
pub struct CheckOutcome {
    pub status: CheckStatus,
    pub left: Option<FactSide>,
    pub right: Option<FactSide>,
    /// Human-readable findings — populated on [`CheckStatus::Drift`].
    #[serde(default)]
    pub findings: Vec<String>,
    /// Why the check could not run — populated on [`CheckStatus::NotEvaluable`].
    pub reason: Option<String>,
}

/// One named check paired with the outcome it reached.
#[derive(Debug, Clone, Serialize)]
pub struct CheckResult {
    pub name: String,
    pub description: String,
    pub outcome: CheckOutcome,
}

/// The full report from a `mev conformance` run: every check's result plus tallies.
#[derive(Debug, Clone, Serialize)]
pub struct ConformanceReport {
    pub results: Vec<CheckResult>,
    pub drift_count: usize,
    pub pass_count: usize,
    pub not_evaluable_count: usize,
}

/// Everything a check needs, resolved once by the driver before any check runs.
pub struct ConformanceCtx {
    /// The brain root (the directory `brain.toml` was resolved from).
    pub root: PathBuf,
    /// The resolved `brain.toml` configuration.
    pub config: BrainConfig,
    /// Every successfully-discovered-and-loaded `planning/state.json`, paired with the
    /// [`StateSource`] that located it.
    pub files: Vec<(StateSource, StateFile)>,
}

/// One registered check: a stable name, a human description, and the pure function
/// that evaluates it against a [`ConformanceCtx`].
#[derive(Clone, Copy)]
pub struct ConformanceCheck {
    pub name: &'static str,
    pub description: &'static str,
    pub run: fn(&ConformanceCtx) -> CheckOutcome,
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// The full set of registered checks. Adding a check is adding one entry here (plus
/// its own file) — nothing else in the driver changes.
pub fn all_checks() -> Vec<ConformanceCheck> {
    // Seed checks register themselves here in tasks 2-5. Empty for now (task 1).
    Vec::new()
}

/// Run every registered check, or — when `only` is `Some` — exactly the one whose
/// name matches. Returns an error naming the valid checks when `only` names no
/// registered check.
pub fn run_checks(ctx: &ConformanceCtx, only: Option<&str>) -> anyhow::Result<ConformanceReport> {
    let checks = all_checks();

    let selected: Vec<ConformanceCheck> = match only {
        None => checks.clone(),
        Some(name) => {
            let found = checks.iter().find(|c| c.name == name).copied();
            match found {
                Some(c) => vec![c],
                None => {
                    let mut valid: Vec<&str> = checks.iter().map(|c| c.name).collect();
                    valid.sort_unstable();
                    return Err(anyhow::anyhow!(
                        "unknown conformance check: {name} (valid checks: {})",
                        if valid.is_empty() {
                            "none registered".to_string()
                        } else {
                            valid.join(", ")
                        }
                    ));
                }
            }
        }
    };

    let mut results = Vec::with_capacity(selected.len());
    let mut drift_count = 0usize;
    let mut pass_count = 0usize;
    let mut not_evaluable_count = 0usize;

    for check in &selected {
        let outcome = (check.run)(ctx);
        match outcome.status {
            CheckStatus::Pass => pass_count += 1,
            CheckStatus::Drift => drift_count += 1,
            CheckStatus::NotEvaluable => not_evaluable_count += 1,
        }
        results.push(CheckResult {
            name: check.name.to_string(),
            description: check.description.to_string(),
            outcome,
        });
    }

    Ok(ConformanceReport {
        results,
        drift_count,
        pass_count,
        not_evaluable_count,
    })
}

// ---------------------------------------------------------------------------
// Digest
// ---------------------------------------------------------------------------

const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

/// FNV-1a 64-bit digest of `items`, newline-joined in the order given (callers must
/// sort first for a deterministic, order-insensitive result), rendered as 16
/// lowercase hex characters.
///
/// This is an equality/display aid, not a security primitive — no new crate
/// dependency is introduced for it.
pub fn digest(items: &[String]) -> String {
    let joined = items.join("\n");
    let mut hash = FNV_OFFSET_BASIS;
    for byte in joined.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{hash:016x}")
}

// ---------------------------------------------------------------------------
// Shared verdict body for set-parity checks
// ---------------------------------------------------------------------------

/// The shared body every set-parity check uses: equal digests → [`CheckStatus::Pass`];
/// otherwise [`CheckStatus::Drift`] with findings listing `only in <left.label>: ...`
/// and `only in <right.label>: ...`, sorted and deterministic in both directions.
pub fn compare_sides(left: FactSide, right: FactSide) -> CheckOutcome {
    if left.digest == right.digest {
        return CheckOutcome {
            status: CheckStatus::Pass,
            left: Some(left),
            right: Some(right),
            findings: Vec::new(),
            reason: None,
        };
    }

    let left_set: std::collections::BTreeSet<&String> = left.items.iter().collect();
    let right_set: std::collections::BTreeSet<&String> = right.items.iter().collect();

    let mut findings = Vec::new();

    let mut only_left: Vec<&&String> = left_set.difference(&right_set).collect();
    only_left.sort();
    for item in only_left {
        findings.push(format!("only in {}: {item}", left.label));
    }

    let mut only_right: Vec<&&String> = right_set.difference(&left_set).collect();
    only_right.sort();
    for item in only_right {
        findings.push(format!("only in {}: {item}", right.label));
    }

    // Digests differed but every item matched (e.g. duplicate entries on one side) —
    // still a real divergence; say so explicitly rather than emitting no findings.
    if findings.is_empty() {
        findings.push(format!(
            "digests differ ({} vs {}) though item sets match — check for duplicates",
            left.digest, right.digest
        ));
    }

    CheckOutcome {
        status: CheckStatus::Drift,
        left: Some(left),
        right: Some(right),
        findings,
        reason: None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn side(label: &str, items: &[&str]) -> FactSide {
        let items: Vec<String> = items.iter().map(|s| s.to_string()).collect();
        FactSide {
            label: label.to_string(),
            source: format!("{label}-source"),
            digest: digest(&items),
            items,
        }
    }

    #[test]
    fn digest_is_stable_across_calls() {
        let items = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(digest(&items), digest(&items));
    }

    #[test]
    fn digest_is_order_sensitive_through_the_callers_sort() {
        // digest() itself does not sort — it trusts the caller's canonicalization.
        // Different input order over the same logical set yields a different digest,
        // which is exactly why every check must sort before calling digest().
        let a = vec!["a".to_string(), "b".to_string()];
        let b = vec!["b".to_string(), "a".to_string()];
        assert_ne!(digest(&a), digest(&b));
    }

    #[test]
    fn digest_differs_for_different_item_sets() {
        let a = vec!["a".to_string(), "b".to_string()];
        let b = vec!["a".to_string(), "c".to_string()];
        assert_ne!(digest(&a), digest(&b));
    }

    #[test]
    fn compare_sides_passes_on_identical_sets() {
        let left = side("left", &["x", "y"]);
        let right = side("right", &["x", "y"]);
        let outcome = compare_sides(left, right);
        assert_eq!(outcome.status, CheckStatus::Pass);
        assert!(outcome.findings.is_empty());
    }

    #[test]
    fn compare_sides_reports_both_drift_directions_deterministically() {
        let left = side("markdown", &["shared", "only-left-b", "only-left-a"]);
        let right = side("json", &["shared", "only-right"]);
        let outcome = compare_sides(left, right);
        assert_eq!(outcome.status, CheckStatus::Drift);

        // Deterministic ordering: within each direction, findings are alphabetical.
        assert_eq!(
            outcome.findings,
            vec![
                "only in markdown: only-left-a".to_string(),
                "only in markdown: only-left-b".to_string(),
                "only in json: only-right".to_string(),
            ]
        );
    }

    #[test]
    fn compare_sides_findings_are_reproducible_across_runs() {
        let left = side("markdown", &["a-only", "z-only"]);
        let right = side("json", &["b-only"]);
        let first = compare_sides(left.clone(), right.clone());
        let second = compare_sides(left, right);
        assert_eq!(first.findings, second.findings);
    }

    #[test]
    fn all_checks_have_unique_non_empty_names() {
        let checks = all_checks();
        let mut names: Vec<&str> = checks.iter().map(|c| c.name).collect();
        let before_dedup_len = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(
            names.len(),
            before_dedup_len,
            "duplicate check name registered"
        );
        for name in &names {
            assert!(!name.is_empty(), "check name must not be empty");
        }
    }

    #[test]
    fn run_checks_with_unknown_name_errors_and_lists_valid_names() {
        let ctx = ConformanceCtx {
            root: PathBuf::from("/nonexistent"),
            config: BrainConfig::default(),
            files: Vec::new(),
        };
        let result = run_checks(&ctx, Some("definitely-not-a-real-check"));
        assert!(result.is_err());
        let message = result.unwrap_err().to_string();
        assert!(message.contains("definitely-not-a-real-check"));
    }

    #[test]
    fn run_checks_with_no_filter_runs_every_registered_check() {
        let ctx = ConformanceCtx {
            root: PathBuf::from("/nonexistent"),
            config: BrainConfig::default(),
            files: Vec::new(),
        };
        let report = run_checks(&ctx, None).expect("run_checks should succeed");
        assert_eq!(report.results.len(), all_checks().len());
        assert_eq!(
            report.pass_count + report.drift_count + report.not_evaluable_count,
            report.results.len()
        );
    }
}
