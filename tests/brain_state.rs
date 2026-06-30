//! Integration tests for `validate_brain_state` — end-to-end `--state` over a fixture tree.
//!
//! Phase 3, Block P2 — Task 6.
//!
//! Builds a temp HQ-root fixture with two `[[repos]]` entries (alpha, beta), each with a
//! leaf `planning/state.json`, plus a brain `planning/state.json` that rolls them up via
//! `repos[]` and connects them with a `cross_repo` edge.
//!
//! Tests (v2 fixtures: `id` in focus + cross_repo endpoints, `depends_on` on track blocks):
//!   1. Clean v2 fixture → `validate_brain_state` returns 0 errors.
//!   2. One repo's `depends_on` points at a nonexistent target id → exactly one
//!      `E_STATE_DANGLING_BLOCKED_BY`.
//!   3. A child advances past its brain `repos[]` headline → exactly one
//!      `W_STATE_ROLLUP_DRIFT`, and the report has 0 *errors* (exits 0).
//!   4. `--json` round-trip: a `State` diagnostic appears in the serialized envelope.
//!   5. Cyclic `depends_on` → `E_STATE_CYCLE` (exit 1).
//!   6. Authored `status: "blocked"` → `E_STATE_AUTHORED_BLOCKED` (exit 1).
//!   7. Closed block depends on non-closed block → `E_STATE_STATUS_INCONSISTENT` (exit 1).
//!   8. Backlog `depends_on` dangling → `E_STATE_DANGLING_BLOCKED_BY` (exit 1).
//!   9. Promoted backlog node with missing block pointer → `E_STATE_DANGLING_PROMOTION` (exit 1).
//!  10. Focus that disagrees with derivation → `W_STATE_FOCUS_DRIFT` (warning, exit 0).

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

/// Write the clean HQ brain `planning/state.json` (v2: `id` in focus + cross_repo endpoints).
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
                "now": [{ "id": "AL.1.A", "title": "Alpha block A", "status": "in_progress" }],
                "next": [],
                "blocked": []
            },
            {
                "repo": "beta",
                "now": [{ "id": "BE.1.A", "title": "Beta block A", "status": "in_progress" }],
                "next": [],
                "blocked": []
            }
        ],
        "cross_repo": [
            {
                "from": { "repo": "alpha", "id": "AL.1.A" },
                "to":   { "repo": "beta",  "id": "BE.1.A" },
                "note": "Alpha depends on beta integration."
            }
        ]
    });
    write_json(root, "planning/state.json", &state);
}

/// Write a clean alpha leaf `repos/alpha/planning/state.json` (v2: `id` in focus).
fn write_alpha_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "alpha",
        "kind": "project",
        "updated": "2026-06-29",
        "focus": {
            "now": [{ "id": "AL.1.A", "title": "Alpha block A", "status": "in_progress" }],
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

/// Write a clean beta leaf `repos/beta/planning/state.json` (v2: `id` in focus).
fn write_beta_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "beta",
        "kind": "project",
        "updated": "2026-06-29",
        "focus": {
            "now": [{ "id": "BE.1.A", "title": "Beta block A", "status": "in_progress" }],
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
// Test 1 — clean v2 fixture produces zero errors
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

    // Overwrite beta's state.json so its tracks[].blocks[].depends_on references a block
    // that does not exist in alpha's tracks[] (v2: edges come from depends_on, not focus).
    let beta_state = serde_json::json!({
        "repo": "beta",
        "kind": "project",
        "updated": "2026-06-29",
        "focus": {
            "now": [],
            "next": [],
            "blocked": [{ "id": "BE.1.A", "title": "Beta waiting on alpha ghost" }]
        },
        "tracks": [
            {
                "title": "Phase 1",
                "blocks": [
                    {
                        "id": "BE.1.A",
                        "title": "Beta block A",
                        "status": "open",
                        "depends_on": [
                            { "type": "block", "repo": "alpha", "id": "AL.1.GHOST" }
                        ]
                    }
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
            "now": [{ "id": "AL.1.B", "title": "Alpha block B", "status": "in_progress" }],
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
                "now": [{ "id": "AL.1.A", "title": "Alpha block A", "status": "in_progress" }],
                "next": [],
                "blocked": []
            },
            {
                "repo": "beta",
                "now": [{ "id": "BE.1.A", "title": "Beta block A", "status": "in_progress" }],
                "next": [],
                "blocked": []
            }
        ],
        "cross_repo": [
            {
                "from": { "repo": "alpha", "id": "AL.1.A" },
                "to":   { "repo": "beta",  "id": "BE.1.A" },
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

    // Introduce a dangling depends_on on beta so there is a State error to serialize.
    // v2: edges come from tracks[].blocks[].depends_on[], not focus.blocked.blocked_by[].
    let beta_state = serde_json::json!({
        "repo": "beta",
        "kind": "project",
        "updated": "2026-06-29",
        "focus": {
            "now": [],
            "next": [],
            "blocked": [{ "id": "BE.1.A", "title": "Beta blocked" }]
        },
        "tracks": [
            {
                "title": "Phase 1",
                "blocks": [
                    {
                        "id": "BE.1.A",
                        "title": "Beta block A",
                        "status": "open",
                        "depends_on": [
                            { "type": "block", "repo": "alpha", "id": "AL.1.GHOST" }
                        ]
                    }
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

// ---------------------------------------------------------------------------
// Test 5 — cyclic depends_on → E_STATE_CYCLE (exit 1)
// ---------------------------------------------------------------------------

#[test]
fn cyclic_depends_on_produces_e_state_cycle() {
    let dir = temp_dir("cycle");
    write_clean_fixture(&dir);

    // Create a cycle: alpha:AL.1.A depends_on beta:BE.1.A, which depends_on alpha:AL.1.A.
    let alpha_cyclic = serde_json::json!({
        "repo": "alpha",
        "kind": "project",
        "updated": "2026-06-29",
        "focus": {
            "now": [],
            "next": [],
            "blocked": [{ "id": "AL.1.A", "title": "Alpha blocked on beta" }]
        },
        "tracks": [
            {
                "title": "Phase 1",
                "blocks": [
                    {
                        "id": "AL.1.A",
                        "title": "Alpha block A",
                        "status": "open",
                        "depends_on": [
                            { "type": "block", "repo": "beta", "id": "BE.1.A" }
                        ]
                    }
                ]
            }
        ]
    });
    let beta_cyclic = serde_json::json!({
        "repo": "beta",
        "kind": "project",
        "updated": "2026-06-29",
        "focus": {
            "now": [],
            "next": [],
            "blocked": [{ "id": "BE.1.A", "title": "Beta blocked on alpha" }]
        },
        "tracks": [
            {
                "title": "Phase 1",
                "blocks": [
                    {
                        "id": "BE.1.A",
                        "title": "Beta block A",
                        "status": "open",
                        "depends_on": [
                            { "type": "block", "repo": "alpha", "id": "AL.1.A" }
                        ]
                    }
                ]
            }
        ]
    });
    write_json(&dir, "repos/alpha/planning/state.json", &alpha_cyclic);
    write_json(&dir, "repos/beta/planning/state.json", &beta_cyclic);

    let report = mev::validate_brain_state(&dir).expect("validate_brain_state should not error");

    let cycle_diags: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.locator == "E_STATE_CYCLE")
        .collect();

    assert!(
        !cycle_diags.is_empty(),
        "cyclic depends_on must produce at least one E_STATE_CYCLE, got: {:#?}",
        report.diagnostics
    );

    // The cycle diagnostic must be Error severity (exit 1).
    assert_eq!(
        cycle_diags[0].severity,
        mev::Severity::Error,
        "E_STATE_CYCLE must be Error severity"
    );

    // The message must name the cycle path.
    let msg = &cycle_diags[0].message;
    assert!(
        msg.contains("AL.1.A") || msg.contains("BE.1.A"),
        "E_STATE_CYCLE message should identify cycle participants, got: {msg:?}"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Test 6 — authored status:"blocked" → E_STATE_AUTHORED_BLOCKED (exit 1)
// ---------------------------------------------------------------------------

#[test]
fn authored_blocked_status_produces_e_state_authored_blocked() {
    let dir = temp_dir("authored-blocked");
    write_clean_fixture(&dir);

    // Alpha has a track block with status:"blocked" — which is derived, not authored.
    let alpha_bad = serde_json::json!({
        "repo": "alpha",
        "kind": "project",
        "updated": "2026-06-29",
        "focus": {
            "now": [],
            "next": [],
            "blocked": [{ "id": "AL.1.A", "title": "Alpha A blocked" }]
        },
        "tracks": [
            {
                "title": "Phase 1",
                "blocks": [
                    {
                        "id": "AL.1.A",
                        "title": "Alpha block A",
                        "status": "blocked"
                    }
                ]
            }
        ]
    });
    write_json(&dir, "repos/alpha/planning/state.json", &alpha_bad);

    let report = mev::validate_brain_state(&dir).expect("validate_brain_state should not error");

    let authored_blocked: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.locator == "E_STATE_AUTHORED_BLOCKED")
        .collect();

    assert!(
        !authored_blocked.is_empty(),
        "authored status:'blocked' must produce E_STATE_AUTHORED_BLOCKED, got: {:#?}",
        report.diagnostics
    );

    assert_eq!(
        authored_blocked[0].severity,
        mev::Severity::Error,
        "E_STATE_AUTHORED_BLOCKED must be Error severity"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Test 7 — closed block depends on non-closed block → E_STATE_STATUS_INCONSISTENT
// ---------------------------------------------------------------------------

#[test]
fn closed_depends_on_non_closed_produces_e_state_status_inconsistent() {
    let dir = temp_dir("status-inconsistent");
    write_clean_fixture(&dir);

    // Alpha: AL.1.B is closed but depends on AL.1.A which is still open.
    let alpha_inconsistent = serde_json::json!({
        "repo": "alpha",
        "kind": "project",
        "updated": "2026-06-29",
        "focus": {
            "now": [{ "id": "AL.1.A", "title": "Alpha block A", "status": "in_progress" }],
            "next": [],
            "blocked": []
        },
        "tracks": [
            {
                "title": "Phase 1",
                "blocks": [
                    {
                        "id": "AL.1.A",
                        "title": "Alpha block A",
                        "status": "in_progress"
                    },
                    {
                        "id": "AL.1.B",
                        "title": "Alpha block B",
                        "status": "closed",
                        "depends_on": [
                            { "type": "block", "repo": "alpha", "id": "AL.1.A" }
                        ]
                    }
                ]
            }
        ]
    });
    write_json(&dir, "repos/alpha/planning/state.json", &alpha_inconsistent);

    let report = mev::validate_brain_state(&dir).expect("validate_brain_state should not error");

    let inconsistent: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.locator == "E_STATE_STATUS_INCONSISTENT")
        .collect();

    assert_eq!(
        inconsistent.len(),
        1,
        "expected exactly one E_STATE_STATUS_INCONSISTENT, got: {:#?}",
        report.diagnostics
    );

    assert_eq!(
        inconsistent[0].severity,
        mev::Severity::Error,
        "E_STATE_STATUS_INCONSISTENT must be Error severity"
    );

    // The message must identify both blocks.
    let msg = &inconsistent[0].message;
    assert!(
        msg.contains("AL.1.B") && msg.contains("AL.1.A"),
        "E_STATE_STATUS_INCONSISTENT message should name both blocks, got: {msg:?}"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Test 8 — dangling backlog depends_on → E_STATE_DANGLING_BLOCKED_BY
// ---------------------------------------------------------------------------

#[test]
fn dangling_backlog_dep_produces_error() {
    let dir = temp_dir("backlog-dangling");
    write_clean_fixture(&dir);

    // HQ brain has a backlog node whose depends_on references a non-existent block.
    let hq_with_backlog = serde_json::json!({
        "repo": "hq",
        "kind": "brain",
        "updated": "2026-06-29",
        "focus": { "now": [], "next": [], "blocked": [] },
        "repos": [
            {
                "repo": "alpha",
                "now": [{ "id": "AL.1.A", "title": "Alpha block A", "status": "in_progress" }],
                "next": [],
                "blocked": []
            },
            {
                "repo": "beta",
                "now": [{ "id": "BE.1.A", "title": "Beta block A", "status": "in_progress" }],
                "next": [],
                "blocked": []
            }
        ],
        "cross_repo": [],
        "backlog": [
            {
                "slug": "future-feature",
                "title": "Future Feature",
                "repo": "alpha",
                "type": "feature",
                "status": "idea",
                "depends_on": [
                    { "type": "block", "repo": "alpha", "id": "AL.99.GHOST" }
                ]
            }
        ]
    });
    write_json(&dir, "planning/state.json", &hq_with_backlog);

    let report = mev::validate_brain_state(&dir).expect("validate_brain_state should not error");

    let dangling: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.locator == "E_STATE_DANGLING_BLOCKED_BY")
        .collect();

    assert!(
        !dangling.is_empty(),
        "dangling backlog depends_on must produce E_STATE_DANGLING_BLOCKED_BY, got: {:#?}",
        report.diagnostics
    );

    assert!(
        dangling[0].message.contains("AL.99.GHOST"),
        "E_STATE_DANGLING_BLOCKED_BY message should name the missing id, got: {:?}",
        dangling[0].message
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Test 9 — promoted backlog node with missing block pointer → E_STATE_DANGLING_PROMOTION
// ---------------------------------------------------------------------------

#[test]
fn orphan_promoted_backlog_produces_e_state_dangling_promotion() {
    let dir = temp_dir("backlog-orphan-promo");
    write_clean_fixture(&dir);

    // HQ brain has a promoted backlog node pointing at a block that doesn't exist.
    let hq_orphan = serde_json::json!({
        "repo": "hq",
        "kind": "brain",
        "updated": "2026-06-29",
        "focus": { "now": [], "next": [], "blocked": [] },
        "repos": [
            {
                "repo": "alpha",
                "now": [{ "id": "AL.1.A", "title": "Alpha block A", "status": "in_progress" }],
                "next": [],
                "blocked": []
            },
            {
                "repo": "beta",
                "now": [{ "id": "BE.1.A", "title": "Beta block A", "status": "in_progress" }],
                "next": [],
                "blocked": []
            }
        ],
        "cross_repo": [],
        "backlog": [
            {
                "slug": "promoted-ghost",
                "title": "Promoted Ghost Feature",
                "repo": "alpha",
                "type": "feature",
                "status": "promoted",
                "block": "AL.99.NONEXISTENT"
            }
        ]
    });
    write_json(&dir, "planning/state.json", &hq_orphan);

    let report = mev::validate_brain_state(&dir).expect("validate_brain_state should not error");

    let promotion_diags: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.locator == "E_STATE_DANGLING_PROMOTION")
        .collect();

    assert!(
        !promotion_diags.is_empty(),
        "promoted backlog node with missing block must produce E_STATE_DANGLING_PROMOTION, got: {:#?}",
        report.diagnostics
    );

    assert_eq!(
        promotion_diags[0].severity,
        mev::Severity::Error,
        "E_STATE_DANGLING_PROMOTION must be Error severity"
    );

    assert!(
        promotion_diags[0].message.contains("AL.99.NONEXISTENT")
            || promotion_diags[0].message.contains("promoted-ghost"),
        "E_STATE_DANGLING_PROMOTION message should identify the node or missing block, got: {:?}",
        promotion_diags[0].message
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Test 10 — focus drift → W_STATE_FOCUS_DRIFT (warning, exit 0)
// ---------------------------------------------------------------------------

#[test]
fn focus_drift_produces_warning_not_error() {
    let dir = temp_dir("focus-drift");
    write_clean_fixture(&dir);

    // Alpha's stored focus says AL.1.A is in_progress (now),
    // but tracks[] has AL.1.A as closed and AL.1.B as in_progress.
    // So the stored focus disagrees with what would be derived from tracks[].
    let alpha_drifted = serde_json::json!({
        "repo": "alpha",
        "kind": "project",
        "updated": "2026-06-29",
        "focus": {
            // Stored: AL.1.A in now — but tracks[] says AL.1.A is closed.
            "now": [{ "id": "AL.1.A", "title": "Alpha block A", "status": "in_progress" }],
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
    write_json(&dir, "repos/alpha/planning/state.json", &alpha_drifted);

    let report = mev::validate_brain_state(&dir).expect("validate_brain_state should not error");

    let focus_drift: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.locator == "W_STATE_FOCUS_DRIFT")
        .collect();

    assert!(
        !focus_drift.is_empty(),
        "focus mismatch with tracks[] must produce W_STATE_FOCUS_DRIFT, got: {:#?}",
        report.diagnostics
    );

    // Must be Warning severity (exit 0).
    assert_eq!(
        focus_drift[0].severity,
        mev::Severity::Warning,
        "W_STATE_FOCUS_DRIFT must be Warning severity, not Error"
    );

    // Zero State *errors* — focus drift alone must not fail the gate.
    let state_errors: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.locator.starts_with("E_STATE_"))
        .collect();

    assert!(
        state_errors.is_empty(),
        "focus drift alone must not produce any State errors; got: {state_errors:#?}"
    );

    let _ = fs::remove_dir_all(&dir);
}
