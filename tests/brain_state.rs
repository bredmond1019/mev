//! Integration tests for `validate_brain_state` — end-to-end `--state` over a fixture tree.
//!
//! Phase 3, Block P — Task 6.
//!
//! Builds a temp HQ-root fixture with two `[[repos]]` entries (alpha, beta), each with a
//! leaf `planning/state.json`, plus a brain `planning/state.json` that rolls them up via
//! `repos[]` and connects them with a `cross_repo` edge.
//!
//! Tests:
//!   1. Clean fixture → `validate_brain_state` returns 0 errors.
//!   2. One repo's `blocked_by` points at a nonexistent target id → exactly one
//!      `E_STATE_DANGLING_BLOCKED_BY`.
//!   3. A child advances past its brain `repos[]` headline → exactly one
//!      `W_STATE_ROLLUP_DRIFT`, and the report has 0 *errors* (exits 0).
//!   4. `--json` round-trip: a `State` diagnostic appears in the serialized envelope.

use std::fs;
use std::path::Path;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create a file at `root/rel` (creating parent dirs as needed) with `content`.
fn write_file(root: &Path, rel: &str, content: &str) {
    let full = root.join(rel);
    fs::create_dir_all(full.parent().unwrap()).unwrap();
    fs::write(&full, content.as_bytes()).unwrap();
}

/// Make a fresh uniquely-named temp dir for a test and return its path.
fn temp_dir(suffix: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("mev-brain-state-it-{suffix}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Write `brain.toml` with two leaf repos (alpha, beta) and a standard `[vocab]` block.
///
/// Neither repo has a `repo_path = "."` entry, so the HQ brain slug defaults to `"hq"`.
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
    write_file(root, rel, &serde_json::to_string_pretty(value).unwrap());
}

/// Write the clean HQ brain `planning/state.json`.
///
/// - `repos[]` caches alpha and beta with their canonical focus.
/// - `cross_repo[]` has one edge: alpha:AL.1.A → beta:BE.1.A.
fn write_hq_brain_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "hq",
        "kind": "brain",
        "updated": "2026-06-29",
        "focus": { "now": [], "next": [], "blocked": [] },
        "repos": [
            {
                "repo": "alpha",
                "now": [{ "block": "AL.1.A", "title": "Alpha block A", "status": "in_progress" }],
                "next": [],
                "blocked": []
            },
            {
                "repo": "beta",
                "now": [{ "block": "BE.1.A", "title": "Beta block A", "status": "in_progress" }],
                "next": [],
                "blocked": []
            }
        ],
        "cross_repo": [
            {
                "from": { "repo": "alpha", "block": "AL.1.A" },
                "to":   { "repo": "beta",  "block": "BE.1.A" },
                "note": "Alpha depends on beta integration."
            }
        ]
    });
    write_json(root, "planning/state.json", &state);
}

/// Write a clean alpha leaf `repos/alpha/planning/state.json`.
fn write_alpha_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "alpha",
        "kind": "project",
        "updated": "2026-06-29",
        "focus": {
            "now": [{ "block": "AL.1.A", "title": "Alpha block A", "status": "in_progress" }],
            "next": [],
            "blocked": []
        },
        "tracks": [
            {
                "title": "Phase 1",
                "blocks": [
                    { "id": "AL.1.A", "title": "Alpha block A", "status": "in_progress" },
                    { "id": "AL.1.B", "title": "Alpha block B", "status": "open" }
                ]
            }
        ]
    });
    write_json(root, "repos/alpha/planning/state.json", &state);
}

/// Write a clean beta leaf `repos/beta/planning/state.json`.
fn write_beta_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "beta",
        "kind": "project",
        "updated": "2026-06-29",
        "focus": {
            "now": [{ "block": "BE.1.A", "title": "Beta block A", "status": "in_progress" }],
            "next": [],
            "blocked": []
        },
        "tracks": [
            {
                "title": "Phase 1",
                "blocks": [
                    { "id": "BE.1.A", "title": "Beta block A", "status": "in_progress" }
                ]
            }
        ]
    });
    write_json(root, "repos/beta/planning/state.json", &state);
}

/// Build the complete clean fixture: brain.toml + HQ brain state + alpha leaf + beta leaf.
fn write_clean_fixture(root: &Path) {
    write_brain_toml(root);
    write_hq_brain_state(root);
    write_alpha_state(root);
    write_beta_state(root);
}

// ---------------------------------------------------------------------------
// Test 1 — clean fixture produces zero errors
// ---------------------------------------------------------------------------

#[test]
fn clean_fixture_produces_no_errors() {
    let dir = temp_dir("clean");
    write_clean_fixture(&dir);

    let report = mev::validate_brain_state(&dir).expect("validate_brain_state should not error");

    // Filter to State-specific diagnostics to avoid coupling to OKF or other validators.
    let state_errors: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.locator.starts_with("E_STATE_"))
        .collect();

    assert!(
        state_errors.is_empty(),
        "clean fixture should produce no State errors, got: {state_errors:#?}"
    );

    // Also assert zero total errors (no .md files → OKF pass is silent too).
    assert_eq!(
        report.error_count(),
        0,
        "clean fixture should produce zero total errors, got: {:#?}",
        report.diagnostics
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Test 2 — dangling blocked_by → exactly one E_STATE_DANGLING_BLOCKED_BY
// ---------------------------------------------------------------------------

#[test]
fn dangling_blocked_by_produces_exactly_one_error() {
    let dir = temp_dir("dangling-bb");
    write_clean_fixture(&dir);

    // Overwrite beta's state.json so its focus.blocked references a block that
    // does not exist in alpha's tracks[].
    let beta_state = serde_json::json!({
        "repo": "beta",
        "kind": "project",
        "updated": "2026-06-29",
        "focus": {
            "now": [],
            "next": [],
            "blocked": [
                {
                    "block": "BE.1.A",
                    "title": "Beta waiting on alpha ghost",
                    "blocked_by": [
                        { "type": "block", "repo": "alpha", "id": "AL.1.GHOST" }
                    ]
                }
            ]
        },
        "tracks": [
            {
                "title": "Phase 1",
                "blocks": [
                    { "id": "BE.1.A", "title": "Beta block A", "status": "open" }
                ]
            }
        ]
    });
    write_json(&dir, "repos/beta/planning/state.json", &beta_state);

    let report = mev::validate_brain_state(&dir).expect("validate_brain_state should not error");

    let dangling: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.locator == "E_STATE_DANGLING_BLOCKED_BY")
        .collect();

    assert_eq!(
        dangling.len(),
        1,
        "expected exactly one E_STATE_DANGLING_BLOCKED_BY, got: {:#?}",
        report.diagnostics
    );

    // The message must identify the missing target id.
    assert!(
        dangling[0].message.contains("AL.1.GHOST"),
        "E_STATE_DANGLING_BLOCKED_BY message should name the missing id, got: {:?}",
        dangling[0].message
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Test 3 — rollup drift → exactly one W_STATE_ROLLUP_DRIFT, zero errors
// ---------------------------------------------------------------------------

#[test]
fn rollup_drift_produces_warning_not_error() {
    let dir = temp_dir("drift");
    write_clean_fixture(&dir);

    // Advance alpha's focus to AL.1.B — the brain's repos[] still caches AL.1.A.
    let alpha_advanced = serde_json::json!({
        "repo": "alpha",
        "kind": "project",
        "updated": "2026-06-29",
        "focus": {
            "now": [{ "block": "AL.1.B", "title": "Alpha block B", "status": "in_progress" }],
            "next": [],
            "blocked": []
        },
        "tracks": [
            {
                "title": "Phase 1",
                "blocks": [
                    { "id": "AL.1.A", "title": "Alpha block A", "status": "closed" },
                    { "id": "AL.1.B", "title": "Alpha block B", "status": "in_progress" }
                ]
            }
        ]
    });
    write_json(&dir, "repos/alpha/planning/state.json", &alpha_advanced);

    // Also update the HQ brain's cross_repo edge to point at AL.1.B (still valid) so no
    // E_STATE_DANGLING_CROSS_REPO fires and we can isolate the drift warning.
    let hq_updated = serde_json::json!({
        "repo": "hq",
        "kind": "brain",
        "updated": "2026-06-29",
        "focus": { "now": [], "next": [], "blocked": [] },
        "repos": [
            {
                "repo": "alpha",
                // Still caches the OLD block id AL.1.A — this is intentional drift.
                "now": [{ "block": "AL.1.A", "title": "Alpha block A", "status": "in_progress" }],
                "next": [],
                "blocked": []
            },
            {
                "repo": "beta",
                "now": [{ "block": "BE.1.A", "title": "Beta block A", "status": "in_progress" }],
                "next": [],
                "blocked": []
            }
        ],
        "cross_repo": [
            {
                "from": { "repo": "alpha", "block": "AL.1.A" },
                "to":   { "repo": "beta",  "block": "BE.1.A" },
                "note": "Alpha depends on beta integration."
            }
        ]
    });
    write_json(&dir, "planning/state.json", &hq_updated);

    let report = mev::validate_brain_state(&dir).expect("validate_brain_state should not error");

    // Exactly one W_STATE_ROLLUP_DRIFT warning.
    let drift: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.locator == "W_STATE_ROLLUP_DRIFT")
        .collect();

    assert_eq!(
        drift.len(),
        1,
        "expected exactly one W_STATE_ROLLUP_DRIFT, got: {:#?}",
        report.diagnostics
    );

    // The drift diagnostic must be Warning severity, not Error (scoping decision 4).
    assert_eq!(
        drift[0].severity,
        mev::Severity::Warning,
        "W_STATE_ROLLUP_DRIFT must be Warning severity, not Error"
    );

    // Zero State *errors* — drift alone must not fail the gate (exits 0).
    let state_errors: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.locator.starts_with("E_STATE_"))
        .collect();

    assert!(
        state_errors.is_empty(),
        "rollup drift alone must not produce any State errors; got: {state_errors:#?}"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Test 4 — --json round-trip: a State diagnostic appears in the serialized envelope
// ---------------------------------------------------------------------------

#[test]
fn json_round_trip_includes_state_diagnostic() {
    let dir = temp_dir("json-roundtrip");
    write_clean_fixture(&dir);

    // Introduce a dangling blocked_by on beta so there is a State error to serialize.
    let beta_state = serde_json::json!({
        "repo": "beta",
        "kind": "project",
        "updated": "2026-06-29",
        "focus": {
            "now": [],
            "next": [],
            "blocked": [
                {
                    "block": "BE.1.A",
                    "title": "Beta blocked",
                    "blocked_by": [
                        { "type": "block", "repo": "alpha", "id": "AL.1.GHOST" }
                    ]
                }
            ]
        },
        "tracks": [
            {
                "title": "Phase 1",
                "blocks": [
                    { "id": "BE.1.A", "title": "Beta block A", "status": "open" }
                ]
            }
        ]
    });
    write_json(&dir, "repos/beta/planning/state.json", &beta_state);

    let report = mev::validate_brain_state(&dir).expect("validate_brain_state should not error");

    let envelope = mev::JsonReport::new("brain", &dir, &report);
    let json_str = envelope.to_json().expect("to_json should not fail");

    let value: serde_json::Value =
        serde_json::from_str(&json_str).expect("envelope must be valid JSON");

    // The errors count must be at least 1.
    assert!(
        value["errors"].as_u64().unwrap_or(0) >= 1,
        "JSON envelope must report at least one error for the dangling blocked_by, got: {}",
        value["errors"]
    );

    // The diagnostics array must contain an E_STATE_DANGLING_BLOCKED_BY entry.
    let diags = value["diagnostics"]
        .as_array()
        .expect("diagnostics must be a JSON array");

    let state_diags: Vec<_> = diags
        .iter()
        .filter(|d| {
            d["locator"]
                .as_str()
                .map(|l| l.starts_with("E_STATE_") || l.starts_with("W_STATE_"))
                .unwrap_or(false)
        })
        .collect();

    assert!(
        !state_diags.is_empty(),
        "serialized diagnostics must include at least one State diagnostic, got array: {diags:#?}"
    );

    let dangling_diags: Vec<_> = diags
        .iter()
        .filter(|d| d["locator"].as_str() == Some("E_STATE_DANGLING_BLOCKED_BY"))
        .collect();

    assert!(
        !dangling_diags.is_empty(),
        "serialized diagnostics must include E_STATE_DANGLING_BLOCKED_BY, got array: {diags:#?}"
    );

    // Every diagnostic must have severity serialized as lowercase "error" or "warning".
    for diag in diags {
        let sev = diag["severity"].as_str().unwrap_or("");
        assert!(
            sev == "error" || sev == "warning",
            "severity must be 'error' or 'warning', got: {sev:?} in {diag}"
        );
    }

    let _ = fs::remove_dir_all(&dir);
}
