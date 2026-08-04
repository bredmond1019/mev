//! `mev conformance` — a registry of named drift checks over facts kept in two places.
//!
//! Each registered [`ConformanceCheck`] canonicalizes and digests both sides of a fact
//! duplicated somewhere in the corpus (a markdown surface and a `state.json` registry, a
//! doc watermark and a sub-repo's real status, a compiled-in build stamp and the live
//! source tree) and reports divergence with the concrete set difference in both
//! directions. Modelled on qm's `conformance.ts` — canonicalize, digest, compare, report
//! (no auto-repair; this is a gate, not a fixer).
//!
//! Individual checks live in sibling files (`backlog.rs`, `epics_index.rs`,
//! `project_cache.rs`, `toolchain.rs`) and register themselves into [`all_checks`]. This
//! file owns only the shared types, the FNV-1a digest, the `compare_sides` helper every
//! set-parity check reuses, and the [`run_checks`] driver.

use serde::Serialize;
use std::path::PathBuf;

use crate::brain::config::BrainConfig;
use crate::brain::state::{StateFile, StateSource};

/// Verdict for a single registered check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    /// Both sides canonicalized to the same digest.
    Pass,
    /// The two sides diverge — `findings` on the [`CheckOutcome`] names the difference.
    Drift,
    /// The check's inputs were absent (a file missing, a source dir gone); never a
    /// substitute for `Drift` — only a genuine two-sided comparison can drift.
    NotEvaluable,
}

/// One side of a duplicated fact, already canonicalized into a sorted item list.
#[derive(Debug, Clone, Serialize)]
pub struct FactSide {
    /// Human label for this side (e.g. `"backlog.md"`, `"state.json"`).
    pub label: String,
    /// Where this side was read from (a path or a description), for the report.
    pub source: String,
    /// FNV-1a digest of `items`, see [`digest`].
    pub digest: String,
    /// The canonical, sorted items this side contributed.
    pub items: Vec<String>,
}

/// The result of running one check.
#[derive(Debug, Clone, Serialize)]
pub struct CheckOutcome {
    pub status: CheckStatus,
    pub left: Option<FactSide>,
    pub right: Option<FactSide>,
    /// Human-readable divergence details (populated on `Drift`).
    pub findings: Vec<String>,
    /// Why the check could not run (populated on `NotEvaluable`).
    pub reason: Option<String>,
}

/// A named check paired with its outcome, as it appears in the report.
#[derive(Debug, Clone, Serialize)]
pub struct CheckResult {
    pub name: String,
    pub description: String,
    pub outcome: CheckOutcome,
}

/// The full result of a `mev conformance` run: every executed check plus tallies.
#[derive(Debug, Clone, Serialize)]
pub struct ConformanceReport {
    pub results: Vec<CheckResult>,
    pub drift_count: usize,
    pub pass_count: usize,
    pub not_evaluable_count: usize,
}

/// Everything a check needs, resolved once by the driver so individual checks stay pure
/// functions of `&ConformanceCtx`.
pub struct ConformanceCtx {
    /// The brain corpus root (the directory `brain.toml` was resolved from).
    pub root: PathBuf,
    /// Parsed `brain.toml`.
    pub config: BrainConfig,
    /// Every discovered + successfully loaded `planning/state.json`, source-tagged.
    pub files: Vec<(StateSource, StateFile)>,
}

/// A single registered check: a stable name, a one-line description, and a pure
/// `&ConformanceCtx -> CheckOutcome` function pointer.
#[derive(Clone, Copy)]
pub struct ConformanceCheck {
    pub name: &'static str,
    pub description: &'static str,
    pub run: fn(&ConformanceCtx) -> CheckOutcome,
}

/// The full registry of conformance checks. Adding a check is adding one entry here plus
/// its own file — nothing else changes.
pub fn all_checks() -> Vec<ConformanceCheck> {
    Vec::new()
}

/// FNV-1a 64-bit digest of `items`, joined with `\n` in the order given (the caller is
/// responsible for sorting beforehand — this function does not sort). Rendered as 16
/// lowercase hex characters.
///
/// This is an equality/display aid, not a security primitive — no new crate dependency is
/// introduced for it.
pub fn digest(items: &[String]) -> String {
    const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

    let joined = items.join("\n");
    let mut hash = FNV_OFFSET_BASIS;
    for byte in joined.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{hash:016x}")
}

/// The shared verdict body every set-parity check delegates to: equal digests pass;
/// otherwise `Drift` with findings listing `only in <left.label>: ...` and
/// `only in <right.label>: ...`, each direction sorted and deterministic.
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

    let mut findings = Vec::new();

    let mut only_left: Vec<&String> = left
        .items
        .iter()
        .filter(|i| !right.items.contains(i))
        .collect();
    only_left.sort();
    if !only_left.is_empty() {
        let items = only_left
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        findings.push(format!("only in {}: {}", left.label, items));
    }

    let mut only_right: Vec<&String> = right
        .items
        .iter()
        .filter(|i| !left.items.contains(i))
        .collect();
    only_right.sort();
    if !only_right.is_empty() {
        let items = only_right
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        findings.push(format!("only in {}: {}", right.label, items));
    }

    CheckOutcome {
        status: CheckStatus::Drift,
        left: Some(left),
        right: Some(right),
        findings,
        reason: None,
    }
}

/// Run every registered check against `ctx`, or — when `only` is `Some` — exactly the
/// named check. Returns an error naming the valid check names when `only` does not match
/// any registered check.
pub fn run_checks(ctx: &ConformanceCtx, only: Option<&str>) -> anyhow::Result<ConformanceReport> {
    let checks = all_checks();

    let selected: Vec<ConformanceCheck> = match only {
        None => checks.clone(),
        Some(name) => {
            let found = checks.iter().find(|c| c.name == name).copied();
            match found {
                Some(c) => vec![c],
                None => {
                    let mut names: Vec<&str> = checks.iter().map(|c| c.name).collect();
                    names.sort_unstable();
                    return Err(anyhow::anyhow!(
                        "unknown conformance check: {name} (valid checks: {})",
                        names.join(", ")
                    ));
                }
            }
        }
    };

    let mut results = Vec::with_capacity(selected.len());
    let mut drift_count = 0;
    let mut pass_count = 0;
    let mut not_evaluable_count = 0;

    for check in selected {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn side(label: &str, items: &[&str]) -> FactSide {
        let mut items: Vec<String> = items.iter().map(|s| s.to_string()).collect();
        items.sort();
        let d = digest(&items);
        FactSide {
            label: label.to_string(),
            source: format!("{label}-source"),
            digest: d,
            items,
        }
    }

    #[test]
    fn digest_is_stable() {
        let items = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(digest(&items), digest(&items));
    }

    #[test]
    fn digest_is_order_sensitive() {
        let a = vec!["a".to_string(), "b".to_string()];
        let b = vec!["b".to_string(), "a".to_string()];
        assert_ne!(
            digest(&a),
            digest(&b),
            "digest only means order via the caller's sort"
        );
    }

    #[test]
    fn digest_differs_on_content() {
        let a = vec!["a".to_string()];
        let b = vec!["b".to_string()];
        assert_ne!(digest(&a), digest(&b));
    }

    #[test]
    fn compare_sides_pass_when_equal() {
        let left = side("left", &["x", "y"]);
        let right = side("right", &["x", "y"]);
        let outcome = compare_sides(left, right);
        assert_eq!(outcome.status, CheckStatus::Pass);
        assert!(outcome.findings.is_empty());
    }

    #[test]
    fn compare_sides_reports_both_directions() {
        let left = side("left", &["only-left", "shared"]);
        let right = side("right", &["only-right", "shared"]);
        let outcome = compare_sides(left, right);
        assert_eq!(outcome.status, CheckStatus::Drift);
        assert_eq!(outcome.findings.len(), 2);
        assert!(
            outcome
                .findings
                .iter()
                .any(|f| f.starts_with("only in left") && f.contains("only-left"))
        );
        assert!(
            outcome
                .findings
                .iter()
                .any(|f| f.starts_with("only in right") && f.contains("only-right"))
        );
    }

    #[test]
    fn compare_sides_is_deterministic() {
        let left = side("left", &["z", "a", "m"]);
        let right = side("right", &[]);
        let outcome1 = compare_sides(left.clone(), right.clone());
        let outcome2 = compare_sides(left, right);
        assert_eq!(outcome1.findings, outcome2.findings);
        // Sorted regardless of input order.
        assert!(outcome1.findings[0].contains("a, m, z"));
    }

    fn ctx() -> ConformanceCtx {
        ConformanceCtx {
            root: PathBuf::from("."),
            config: BrainConfig::default(),
            files: Vec::new(),
        }
    }

    #[test]
    fn run_checks_unknown_name_errors_with_valid_list() {
        let c = ctx();
        let err = run_checks(&c, Some("does-not-exist")).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("does-not-exist"));
        assert!(msg.contains("unknown conformance check"));
    }

    #[test]
    fn run_checks_empty_registry_is_ok() {
        let c = ctx();
        let report = run_checks(&c, None).unwrap();
        assert_eq!(report.results.len(), 0);
        assert_eq!(report.drift_count, 0);
        assert_eq!(report.pass_count, 0);
        assert_eq!(report.not_evaluable_count, 0);
    }

    #[test]
    fn all_checks_have_unique_non_empty_names() {
        let checks = all_checks();
        let mut names: Vec<&str> = checks.iter().map(|c| c.name).collect();
        for n in &names {
            assert!(!n.is_empty(), "check name must not be empty");
        }
        let before = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), before, "check names must be unique");
    }
}
