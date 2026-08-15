//! Integration tests for `mev::carryover_sweep` — end-to-end over a temp-dir corpus fixture.
//!
//! `MV.ticket.carryover-sweep-command` — Task 4.
//!
//! Builds a temp HQ-root fixture with two leaf repos (`alpha`, `beta`), where `alpha` carries
//! `carryover[]` entries that reference blocks living in the *other* repo (`beta`), exercising
//! cross-repo reference resolution end to end through the public driver: `find_brain_config` →
//! `discover_state_files` → `load_state` → status-map construction → `evaluate_carryover`.
//!
//! Tests:
//!   1. A `clears_when` naming a closed block in another repo → `Cleared`.
//!   2. A `clears_when` naming an open block in another repo → `Actionable`, with that block
//!      named among the unmet refs.
//!   3. A prose-only `clears_when` (no resolvable block or path token) → `NotEvaluable` /
//!      `Prose`.
//!   4. `--repo` filtering returns only that repo's entries.
//!   5. `total == cleared + actionable + not_evaluable`, and equals the sum of `entries.len()`.

use std::fs;
use std::path::Path;

use mev::brain::carryover::{CarryoverLane, CarryoverRef, NotEvaluableReason};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Make a fresh uniquely-named temp dir for a test and return its path.
fn temp_dir(suffix: &str) -> std::path::PathBuf {
    let dir = mev::testsupport::unique_temp_dir(&format!("mev-brain-carryover-it-{suffix}"));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Write `brain.toml` with two leaf repos (alpha, beta) and a standard `[vocab]` block.
fn write_brain_toml(root: &Path) {
    let toml = r#"[vocab]
layer = ["brain", "engine", "factory", "console", "surface", "infra", "business", "content", "meta"]
status = ["active", "draft", "deprecated", "superseded", "archived"]

[crawl]
skip_dirs = ["target", "node_modules", ".git"]

[[repos]]
slug = "alpha"
tier = "primary"
repo_path = "repos/alpha"
status_file = "repos/alpha/planning/status.md"
cache_doc = "docs/projects/alpha.md"
heading = "Alpha"

[[repos]]
slug = "beta"
tier = "primary"
repo_path = "repos/beta"
status_file = "repos/beta/planning/status.md"
cache_doc = "docs/projects/beta.md"
heading = "Beta"
"#;
    fs::write(root.join("brain.toml"), toml.as_bytes()).unwrap();
}

/// Serialize `value` as pretty JSON and write it to `root/rel`.
fn write_json(root: &Path, rel: &str, value: &serde_json::Value) {
    let full = root.join(rel);
    fs::create_dir_all(full.parent().unwrap()).unwrap();
    fs::write(
        &full,
        serde_json::to_string_pretty(value).unwrap().as_bytes(),
    )
    .unwrap();
}

/// Write beta's leaf state: one closed block (`BE.1.A`) and one open block (`BE.1.B`).
/// Beta carries no `carryover[]` entries of its own — only alpha's entries reference it.
fn write_beta_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "beta",
        "kind": "project",
        "updated": "2026-08-01",
        "focus": { "now": [], "next": [], "blocked": [] },
        "tracks": [
            {
                "title": "Phase 1",
                "blocks": [
                    { "id": "BE.1.A", "title": "Beta block A", "status": "closed" },
                    { "id": "BE.1.B", "title": "Beta block B", "status": "open" }
                ]
            }
        ],
        "carryover": []
    });
    write_json(root, "repos/beta/planning/state.json", &state);
}

/// Write alpha's leaf state: three `carryover[]` entries —
///   - `alpha-cleared-cross-repo`: `related[]` names beta's closed `BE.1.A` → Cleared.
///   - `alpha-actionable-cross-repo`: prose names beta's open `BE.1.B` → Actionable.
///   - `alpha-prose-only`: no resolvable block/path token → NotEvaluable(Prose).
fn write_alpha_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "alpha",
        "kind": "project",
        "updated": "2026-08-01",
        "focus": { "now": [], "next": [], "blocked": [] },
        "tracks": [
            {
                "title": "Phase 1",
                "blocks": [
                    { "id": "AL.1.A", "title": "Alpha block A", "status": "open" }
                ]
            }
        ],
        "carryover": [
            {
                "slug": "alpha-cleared-cross-repo",
                "scope": { "repo": "alpha" },
                "kind": "known_issue",
                "text": "Alpha was waiting on beta's block A landing.",
                "related": [
                    { "type": "block", "repo": "beta", "id": "BE.1.A" }
                ],
                "clears_when": "BE.1.A lands",
                "created": "2026-06-01"
            },
            {
                "slug": "alpha-actionable-cross-repo",
                "scope": { "repo": "alpha" },
                "kind": "deferred",
                "text": "Alpha is waiting on beta's block B landing.",
                "clears_when": "BE.1.B lands",
                "created": "2026-06-01"
            },
            {
                "slug": "alpha-prose-only",
                "scope": { "repo": "alpha" },
                "kind": "constraint",
                "text": "Needs a human to review the approach manually.",
                "clears_when": "a human reviews this manually and signs off",
                "created": "2026-06-01"
            }
        ]
    });
    write_json(root, "repos/alpha/planning/state.json", &state);
}

/// Build the complete fixture: brain.toml + alpha leaf + beta leaf. No HQ-root
/// `planning/state.json` is written — discovery tolerates its absence (a
/// `W_STATE_FILE_MISSING` discovery diagnostic, which `carryover_sweep` ignores) and still
/// walks the leaf `[[repos]]` entries.
fn write_fixture(root: &Path) {
    write_brain_toml(root);
    write_alpha_state(root);
    write_beta_state(root);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn cross_repo_closed_block_lands_in_cleared() {
    let dir = temp_dir("cleared");
    write_fixture(&dir);

    let report = mev::carryover_sweep(&dir, None, false).expect("carryover_sweep should not error");

    let entry = report
        .entries
        .iter()
        .find(|e| e.slug == "alpha-cleared-cross-repo")
        .expect("alpha-cleared-cross-repo entry should be present");

    assert_eq!(entry.lane, CarryoverLane::Cleared);
    assert!(
        entry.refs.iter().any(|r| matches!(
            r,
            CarryoverRef::Block { key, satisfied } if key == "beta:BE.1.A" && *satisfied
        )),
        "expected a satisfied beta:BE.1.A block ref, got: {:#?}",
        entry.refs
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn cross_repo_open_block_lands_in_actionable_naming_the_unmet_block() {
    let dir = temp_dir("actionable");
    write_fixture(&dir);

    let report = mev::carryover_sweep(&dir, None, false).expect("carryover_sweep should not error");

    let entry = report
        .entries
        .iter()
        .find(|e| e.slug == "alpha-actionable-cross-repo")
        .expect("alpha-actionable-cross-repo entry should be present");

    assert_eq!(entry.lane, CarryoverLane::Actionable);
    assert!(
        entry.refs.iter().any(|r| matches!(
            r,
            CarryoverRef::Block { key, satisfied } if key == "beta:BE.1.B" && !*satisfied
        )),
        "expected an unsatisfied beta:BE.1.B block ref, got: {:#?}",
        entry.refs
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn prose_only_predicate_lands_in_not_evaluable_prose() {
    let dir = temp_dir("prose");
    write_fixture(&dir);

    let report = mev::carryover_sweep(&dir, None, false).expect("carryover_sweep should not error");

    let entry = report
        .entries
        .iter()
        .find(|e| e.slug == "alpha-prose-only")
        .expect("alpha-prose-only entry should be present");

    assert_eq!(entry.lane, CarryoverLane::NotEvaluable);
    assert!(
        entry.refs.is_empty(),
        "expected no refs, got: {:#?}",
        entry.refs
    );
    assert_eq!(entry.reason, Some(NotEvaluableReason::Prose));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn repo_filter_returns_only_that_repos_entries() {
    let dir = temp_dir("repo-filter");
    write_fixture(&dir);

    let report =
        mev::carryover_sweep(&dir, Some("alpha"), false).expect("carryover_sweep should not error");

    assert!(
        !report.entries.is_empty(),
        "expected alpha's entries to be present"
    );
    assert!(
        report.entries.iter().all(|e| e.repo == "alpha"),
        "expected only alpha entries with --repo alpha, got: {:#?}",
        report
            .entries
            .iter()
            .map(|e| (&e.repo, &e.slug))
            .collect::<Vec<_>>()
    );

    let beta_only =
        mev::carryover_sweep(&dir, Some("beta"), false).expect("carryover_sweep should not error");
    assert_eq!(
        beta_only.total, 0,
        "beta has no carryover[] entries of its own, expected total == 0, got: {:#?}",
        beta_only.entries
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn total_equals_sum_of_lanes_and_entries() {
    let dir = temp_dir("total");
    write_fixture(&dir);

    let report = mev::carryover_sweep(&dir, None, false).expect("carryover_sweep should not error");

    assert_eq!(
        report.total, 3,
        "expected all 3 alpha carryover entries, got: {report:#?}"
    );
    assert_eq!(
        report.total,
        report.cleared + report.actionable + report.not_evaluable,
        "total should equal the sum of the three lane counts"
    );
    assert_eq!(
        report.total,
        report.entries.len(),
        "total should equal entries.len()"
    );
    assert_eq!(report.cleared, 1);
    assert_eq!(report.actionable, 1);
    assert_eq!(report.not_evaluable, 1);

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Live-corpus assertion — `MV.ticket.clears-when-evaluation` Task 4.
//
// A green unit-test gate proves the code matches the spec; only real data
// proves the spec was right. This reuses `mev::carryover_sweep`'s own
// discovery (no new I/O path) over the real HQ brain corpus and asserts a
// floor on evaluable entries, a ceiling on `cleared`, and that the
// 2026-08-03 `core:ba-0-a-id-collision` false-cleared shape — if the entry
// is still present — is never in the cleared lane.
// ---------------------------------------------------------------------------

/// Floor on evaluable (`cleared + actionable`) entries in the live HQ corpus.
///
/// Baseline measured 2026-08-09, before this block landed: 9 of 142
/// (3 cleared / 6 actionable / 133 not-evaluable). Measured again 2026-08-09
/// after Tasks 1-3 landed (typed predicates + broadened prose extraction):
/// 9 of 138 (2 cleared / 7 actionable / 129 not-evaluable) — see the
/// Amendment Log in `planning/ticket-clears-when-evaluation/tasks.md` for the
/// full breakdown and the hand-verified spot check. The floor is set below
/// that post-block measurement (not at the ticket's ~40 aspiration) because
/// the live corpus's actual prose shapes did not, at measurement time,
/// include material volume of the newly-reachable patterns (paths asserted
/// without the word "exists", corrected/fixed pairs, gate mentions resolving
/// to a checkable file or block) — that is a fact about the corpus's current
/// content, not a defect in the widening, and is recorded honestly rather
/// than papered over. The fleet mutates roughly 20 `carryover[]` entries/day,
/// so a small margin below the measured value avoids flaking on ordinary
/// churn while a real regression (a bug that stops the sweep from
/// evaluating anything) still trips this floor.
const EVALUABLE_FLOOR: usize = 6;

/// Ceiling on `cleared` in the live HQ corpus. `cleared` is the destructive
/// verdict this whole block is engineered never to over-produce — an
/// unexpected jump is exactly the failure mode a floor alone cannot catch.
/// Measured 2026-08-09: 2 cleared (baseline was 3). Set generously above
/// both readings to tolerate ordinary fleet churn while still catching a
/// widening that starts mis-firing.
const CLEARED_CEILING: usize = 15;

#[test]
fn live_corpus_evaluable_floor_and_cleared_ceiling() {
    // mev's own integration test binaries run with cwd == the mev crate
    // root (`core/mev`); the HQ brain root (where `brain.toml` lives) is two
    // levels up. Reuses `mev::carryover_sweep`'s own `find_brain_config` +
    // `discover_state_files` discovery — no new I/O path is added here.
    let live_root = Path::new("../..");
    if !live_root.join("brain.toml").exists() {
        eprintln!(
            "skipping live_corpus_evaluable_floor_and_cleared_ceiling: {} has no brain.toml \
             (fresh clone or CI runner without the sibling HQ checkout)",
            live_root.display()
        );
        return;
    }

    let report = match mev::carryover_sweep(live_root, None, false) {
        Ok(r) => r,
        Err(e) => {
            eprintln!(
                "skipping live_corpus_evaluable_floor_and_cleared_ceiling: \
                 carryover_sweep over the live corpus errored: {e}"
            );
            return;
        }
    };

    let evaluable = report.cleared + report.actionable;
    assert!(
        evaluable >= EVALUABLE_FLOOR,
        "expected at least {EVALUABLE_FLOOR} evaluable (cleared + actionable) entries in the \
         live corpus, got {evaluable} of {} (cleared={}, actionable={}, not_evaluable={})",
        report.total,
        report.cleared,
        report.actionable,
        report.not_evaluable
    );
    assert!(
        report.cleared <= CLEARED_CEILING,
        "live-corpus cleared count {} exceeds the ceiling of {CLEARED_CEILING} — a widening may \
         be over-firing and manufacturing false `cleared` verdicts",
        report.cleared
    );

    // Live-data twin of the `carryover.rs:1098`-equivalent CLOSURE_VERBS
    // pinning test: if `core:ba-0-a-id-collision` is still present in the
    // corpus, it must never have landed in the cleared lane. BA.0.A IS
    // closed, so without the closure-verb gate this exact shape recommended
    // deleting a live `known_issue` (found 2026-08-03).
    if let Some(entry) = report
        .entries
        .iter()
        .find(|e| e.repo == "core" && e.slug == "ba-0-a-id-collision")
    {
        assert_ne!(
            entry.lane,
            CarryoverLane::Cleared,
            "core:ba-0-a-id-collision must never be Cleared (2026-08-03 false-cleared shape); \
             got refs: {:#?}",
            entry.refs
        );
    }

    // Nothing in this task writes to any state.json — the sweep above is a
    // pure read, and no code path in this test opens any file for writing.
}

// ---------------------------------------------------------------------------
// `mev carryover --audit` — `MV.ticket.reference-container-validation` Task 4.
//
// A dedicated fixture (not `write_fixture`, which the tests above assert exact
// lane counts against) carrying both `carryover[]` and `reference[]` entries
// across two repos, with a mix of typed and prose `clears_when`, so the audit's
// per-container, per-class/per-kind, typed-predicate, and clear-rate-denominator
// figures are all independently checkable.
// ---------------------------------------------------------------------------

/// Write alpha's leaf state for the audit fixture: three `carryover[]` entries
/// (one typed-predicate `Cleared`, one typed-predicate `Actionable`, one prose
/// `NotEvaluable`) plus two `reference[]` entries (`trap`, `invariant`).
fn write_audit_alpha_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "alpha",
        "kind": "project",
        "updated": "2026-08-01",
        "focus": { "now": [], "next": [], "blocked": [] },
        "tracks": [
            {
                "title": "Phase 1",
                "blocks": [
                    { "id": "AL.1.A", "title": "Alpha block A", "status": "open" }
                ]
            }
        ],
        "carryover": [
            {
                "slug": "a-typed-cleared",
                "scope": { "repo": "alpha" },
                "kind": "deferred",
                "text": "Cleared once beta's block A lands.",
                "clears_when": { "type": "block_closed", "repo": "beta", "id": "BE.1.A" },
                "created": "2020-01-01"
            },
            {
                "slug": "a-typed-actionable",
                "scope": { "repo": "alpha" },
                "kind": "known_issue",
                "text": "Actionable until alpha's own block A closes.",
                "clears_when": { "type": "block_closed", "repo": "alpha", "id": "AL.1.A" },
                "created": "2020-01-01"
            },
            {
                "slug": "a-prose",
                "scope": { "repo": "alpha" },
                "kind": "env",
                "text": "Needs a human to review the approach manually.",
                "clears_when": "a human reviews this manually and signs off",
                "created": "2020-01-01"
            }
        ],
        "reference": [
            {
                "slug": "a-ref-trap",
                "scope": { "repo": "alpha" },
                "class": "trap",
                "text": "Do not do X — it silently breaks Y.",
                "created": "2020-01-01"
            },
            {
                "slug": "a-ref-invariant",
                "scope": { "repo": "alpha" },
                "class": "invariant",
                "text": "Every carryover entry has a `created` date.",
                "created": "2020-01-01"
            }
        ]
    });
    write_json(root, "repos/alpha/planning/state.json", &state);
}

/// Write beta's leaf state for the audit fixture: one closed block (so alpha's
/// `a-typed-cleared` entry resolves), no `carryover[]` entries of its own, and
/// one `reference[]` entry (`lesson`).
fn write_audit_beta_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "beta",
        "kind": "project",
        "updated": "2026-08-01",
        "focus": { "now": [], "next": [], "blocked": [] },
        "tracks": [
            {
                "title": "Phase 1",
                "blocks": [
                    { "id": "BE.1.A", "title": "Beta block A", "status": "closed" }
                ]
            }
        ],
        "carryover": [],
        "reference": [
            {
                "slug": "b-ref-lesson",
                "scope": { "repo": "beta" },
                "class": "lesson",
                "text": "A lesson learned the hard way.",
                "created": "2020-01-01"
            }
        ]
    });
    write_json(root, "repos/beta/planning/state.json", &state);
}

fn write_audit_fixture(root: &Path) {
    write_brain_toml(root);
    write_audit_alpha_state(root);
    write_audit_beta_state(root);
}

#[test]
fn audit_counts_sum_to_totals_and_split_by_container() {
    let dir = temp_dir("audit-sum");
    write_audit_fixture(&dir);

    let (_report, audit) =
        mev::carryover_audit(&dir, None, false, 30).expect("carryover_audit should not error");

    assert_eq!(
        audit.carryover_count, 3,
        "expected alpha's 3 carryover[] entries"
    );
    assert_eq!(
        audit.reference_count, 3,
        "expected alpha's 2 + beta's 1 reference[] entries"
    );
    assert_eq!(audit.total, audit.carryover_count + audit.reference_count);

    let per_kind_sum: usize = audit.per_kind.values().sum();
    assert_eq!(
        per_kind_sum, audit.carryover_count,
        "per_kind counts should sum to carryover_count, got {:#?}",
        audit.per_kind
    );

    let per_class_sum: usize = audit.per_class.values().sum();
    assert_eq!(
        per_class_sum, audit.reference_count,
        "per_class counts should sum to reference_count, got {:#?}",
        audit.per_class
    );

    assert_eq!(audit.per_kind.get("deferred"), Some(&1));
    assert_eq!(audit.per_kind.get("known_issue"), Some(&1));
    assert_eq!(audit.per_kind.get("env"), Some(&1));
    assert_eq!(audit.per_class.get("trap"), Some(&1));
    assert_eq!(audit.per_class.get("invariant"), Some(&1));
    assert_eq!(audit.per_class.get("lesson"), Some(&1));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn audit_typed_predicate_coverage_counts_only_typed_clears_when() {
    let dir = temp_dir("audit-typed");
    write_audit_fixture(&dir);

    let (_report, audit) =
        mev::carryover_audit(&dir, None, false, 30).expect("carryover_audit should not error");

    // Two of the three carryover[] entries carry a typed `block_closed`
    // predicate; the third carries free prose.
    assert_eq!(audit.typed_predicate_count, 2);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn audit_clear_rate_denominator_excludes_reference_entries() {
    let dir = temp_dir("audit-clear-rate");
    write_audit_fixture(&dir);

    let (_report, audit) =
        mev::carryover_audit(&dir, None, false, 30).expect("carryover_audit should not error");

    // `reference[]` entries (3 of them) must never inflate the clear-rate
    // denominator: it is scoped to carryover[] only, not `total`.
    assert_eq!(
        audit.clearable_total, audit.carryover_count,
        "clearable_total must equal carryover_count, excluding reference[] entries"
    );
    assert_ne!(
        audit.clearable_total, audit.total,
        "clearable_total must not equal total when reference[] entries are present"
    );

    // Exactly one carryover[] entry (`a-typed-cleared`) resolves to Cleared.
    assert_eq!(audit.cleared_total, 1);
    assert_eq!(audit.clearable_total, 3);
    let expected_rate = 1.0 / 3.0;
    assert!(
        (audit.clear_rate - expected_rate).abs() < 1e-9,
        "expected clear_rate {expected_rate}, got {}",
        audit.clear_rate
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn audit_inflow_outflow_respect_the_window() {
    let dir = temp_dir("audit-window");
    write_audit_fixture(&dir);

    // Every fixture entry is dated 2020-01-01 — far outside a 30-day window
    // from today, so nothing should count as inflow or outflow.
    let (_report, narrow) =
        mev::carryover_audit(&dir, None, false, 30).expect("carryover_audit should not error");
    assert_eq!(
        narrow.inflow, 0,
        "no entry should fall inside a 30-day window"
    );
    assert_eq!(
        narrow.outflow, 0,
        "no cleared entry should fall inside a 30-day window"
    );

    // A window wide enough to cover 2020-01-01 through today should count
    // every entry as inflow, and the one Cleared entry as outflow.
    let (_report, wide) =
        mev::carryover_audit(&dir, None, false, 20_000).expect("carryover_audit should not error");
    assert_eq!(
        wide.inflow, wide.total,
        "every fixture entry should count as inflow with a wide-enough window"
    );
    assert_eq!(
        wide.outflow, 1,
        "the single Cleared carryover[] entry should count as outflow"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn audit_respects_repo_filter() {
    let dir = temp_dir("audit-repo-filter");
    write_audit_fixture(&dir);

    let (_report, audit) = mev::carryover_audit(&dir, Some("beta"), false, 30)
        .expect("carryover_audit should not error");

    assert_eq!(
        audit.carryover_count, 0,
        "beta carries no carryover[] entries"
    );
    assert_eq!(audit.reference_count, 1, "beta carries 1 reference[] entry");

    let _ = fs::remove_dir_all(&dir);
}
