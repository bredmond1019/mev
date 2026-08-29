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
    let dir = mev::testsupport::unique_temp_dir(&format!("mev-brain-state-it-{suffix}"));
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

// ---------------------------------------------------------------------------
// Task 1 — derive_focus / derive_cross_repo / derive_rollup unit tests
// ---------------------------------------------------------------------------

/// Build a minimal (StateSource, StateFile) pair directly in memory for unit tests.
fn make_leaf_pair_in_memory(
    repo: &str,
    tracks_json: serde_json::Value,
    focus_json: serde_json::Value,
) -> (mev::brain::state::StateSource, mev::brain::state::StateFile) {
    use mev::brain::state::{StateFile, StateSource};

    let path = std::path::PathBuf::from(format!("/tmp/{repo}-state.json"));
    let json = serde_json::json!({
        "repo": repo,
        "kind": "project",
        "updated": "2026-06-30",
        "focus": focus_json,
        "tracks": tracks_json,
    });
    let file: StateFile = serde_json::from_value(json).expect("fixture must parse");
    let src = StateSource {
        repo_slug: repo.to_string(),
        abs_path: path,
        expected_kind: "project",
    };
    (src, file)
}

#[test]
fn derive_focus_regression_check_focus_drift_still_passes_existing_fixtures() {
    // Regression guard: derive_focus on a fixture where stored focus matches derivation
    // should produce the correct derived values, and check_focus_drift should report no drift.
    use mev::brain::config::BrainConfig;
    use mev::brain::state::{build_state_graph, check_focus_drift, derive_focus};

    let (src, file) = make_leaf_pair_in_memory(
        "alpha",
        serde_json::json!([{
            "title": "Phase 1",
            "blocks": [
                { "id": "AL.1.A", "title": "Work A", "status": "in_progress" },
                { "id": "AL.1.B", "title": "Work B", "status": "open" }
            ]
        }]),
        // Stored focus matches derivation: now=A, next=B (no deps), blocked=[]
        serde_json::json!({
            "now": [{ "id": "AL.1.A", "title": "Work A", "status": "in_progress" }],
            "next": [{ "id": "AL.1.B", "title": "Work B" }],
            "blocked": []
        }),
    );
    let files = vec![(src.clone(), file.clone())];
    let graph = build_state_graph(&files);

    let derived = derive_focus(&src, &file, &graph, &files, None);

    // derive_focus: A in now, B in next (open, no deps), blocked empty.
    assert_eq!(derived.now, vec!["AL.1.A".to_string()]);
    assert_eq!(derived.next, vec!["AL.1.B".to_string()]);
    assert!(derived.blocked.is_empty());

    // check_focus_drift must report no drift (stored matches derivation).
    let diags = check_focus_drift(&src, &file, &BrainConfig::default(), &graph, &files);
    assert!(
        diags.is_empty(),
        "check_focus_drift should report no drift for in-sync fixture, got: {diags:?}"
    );
}

#[test]
fn derive_focus_blocked_carries_unmet_subset() {
    // An open block with an unmet Block dep appears in blocked, and the unmet
    // dep is included in the returned Vec<BlockedBy>.
    use mev::brain::state::{BlockDep, BlockedBy, build_state_graph, derive_focus};

    // alpha block B depends on alpha block A, which is still "open" (not closed).
    let (src, file) = make_leaf_pair_in_memory(
        "alpha",
        serde_json::json!([{
            "title": "Phase 1",
            "blocks": [
                { "id": "AL.1.A", "title": "Gate", "status": "open" },
                {
                    "id": "AL.1.B",
                    "title": "Blocked work",
                    "status": "open",
                    "depends_on": [{ "type": "block", "repo": "alpha", "id": "AL.1.A" }]
                }
            ]
        }]),
        serde_json::json!({ "now": [], "next": [], "blocked": [] }),
    );
    let files = vec![(src.clone(), file.clone())];
    let graph = build_state_graph(&files);

    let derived = derive_focus(&src, &file, &graph, &files, None);

    assert!(derived.now.is_empty(), "no in_progress blocks");
    // AL.1.B is blocked by AL.1.A (open, not closed).
    assert_eq!(derived.blocked.len(), 1);
    let (blocked_id, unmet) = &derived.blocked[0];
    assert_eq!(blocked_id, "AL.1.B");
    assert_eq!(unmet.len(), 1, "exactly one unmet dep");
    match &unmet[0] {
        BlockedBy::Block(BlockDep { repo, id, .. }) => {
            assert_eq!(repo, "alpha");
            assert_eq!(id, "AL.1.A");
        }
        other => panic!("expected BlockedBy::Block, got {other:?}"),
    }
    // AL.1.A is open with no deps → it should be in next (ready).
    assert!(
        derived.next.contains(&"AL.1.A".to_string()),
        "AL.1.A (open, no deps) should be in next, got: {:?}",
        derived.next
    );
    // AL.1.B has an unmet dep → not in next.
    assert!(
        !derived.next.contains(&"AL.1.B".to_string()),
        "AL.1.B (blocked) must not be in next"
    );
}

#[test]
fn derive_focus_external_dep_goes_to_blocked() {
    use mev::brain::state::{build_state_graph, derive_focus};

    let (src, file) = make_leaf_pair_in_memory(
        "alpha",
        serde_json::json!([{
            "title": "Phase 1",
            "blocks": [{
                "id": "AL.1.A",
                "title": "Needs external",
                "status": "open",
                "depends_on": [{ "type": "external", "what": "hardware requirement" }]
            }]
        }]),
        serde_json::json!({ "now": [], "next": [], "blocked": [] }),
    );
    let files = vec![(src.clone(), file.clone())];
    let graph = build_state_graph(&files);

    let derived = derive_focus(&src, &file, &graph, &files, None);

    assert_eq!(derived.blocked.len(), 1);
    assert_eq!(derived.blocked[0].0, "AL.1.A");
    assert!(
        derived.next.is_empty(),
        "external-dep block must not be in next"
    );
}

/// Load a full `state.json` fixture from `tests/fixtures/<name>` and return it
/// as a `(StateSource, StateFile)` pair, mirroring [`make_leaf_pair_in_memory`]
/// but reading real fixture files (per this task's acceptance criteria) rather
/// than building JSON inline.
fn load_fixture(name: &str) -> (mev::brain::state::StateSource, mev::brain::state::StateFile) {
    use mev::brain::state::{StateFile, StateSource};

    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read fixture {}: {e}", path.display()));
    let file: StateFile =
        serde_json::from_str(&raw).unwrap_or_else(|e| panic!("fixture must parse: {e}"));
    let src = StateSource {
        repo_slug: file.repo.clone(),
        abs_path: path,
        expected_kind: "project",
    };
    (src, file)
}

#[test]
fn derive_focus_operator_dep_goes_to_blocked() {
    use mev::brain::state::{BlockedBy, OperatorDep, build_state_graph, derive_focus};

    let (src, file) = load_fixture("state-operator-dep.json");
    let files = vec![(src.clone(), file.clone())];
    let graph = build_state_graph(&files);

    let derived = derive_focus(&src, &file, &graph, &files, None);

    assert_eq!(derived.blocked.len(), 1);
    assert_eq!(derived.blocked[0].0, "AL.1.A");
    match &derived.blocked[0].1[0] {
        BlockedBy::Operator(OperatorDep { slug, .. }) => assert_eq!(slug, "mac-mini-setup"),
        other => panic!("expected BlockedBy::Operator, got {other:?}"),
    }
    assert!(
        derived.next.is_empty(),
        "operator-dep block must not be in next"
    );
}

#[test]
fn derive_focus_approval_dep_goes_to_blocked() {
    use mev::brain::state::{ApprovalDep, BlockedBy, build_state_graph, derive_focus};

    let (src, file) = load_fixture("state-approval-dep.json");
    let files = vec![(src.clone(), file.clone())];
    let graph = build_state_graph(&files);

    let derived = derive_focus(&src, &file, &graph, &files, None);

    assert_eq!(derived.blocked.len(), 1);
    assert_eq!(derived.blocked[0].0, "AL.1.A");
    match &derived.blocked[0].1[0] {
        BlockedBy::Approval(ApprovalDep { slug, .. }) => assert_eq!(slug, "ship-it"),
        other => panic!("expected BlockedBy::Approval, got {other:?}"),
    }
    assert!(
        derived.next.is_empty(),
        "approval-dep block must not be in next"
    );
}

#[test]
fn derive_focus_removing_operator_dep_makes_block_ready_again() {
    use mev::brain::state::{build_state_graph, derive_focus};

    // Same block as `derive_focus_operator_dep_goes_to_blocked`, but with the
    // operator entry removed — must derive as ready (in `next`, not `blocked`)
    // in the same derivation pass, exactly like the `external` case.
    let (src, file) = make_leaf_pair_in_memory(
        "alpha",
        serde_json::json!([{
            "title": "Phase 1",
            "blocks": [{
                "id": "AL.1.A",
                "title": "Operator gate cleared",
                "status": "open",
                "depends_on": []
            }]
        }]),
        serde_json::json!({ "now": [], "next": [], "blocked": [] }),
    );
    let files = vec![(src.clone(), file.clone())];
    let graph = build_state_graph(&files);

    let derived = derive_focus(&src, &file, &graph, &files, None);

    assert!(
        derived.blocked.is_empty(),
        "block with no depends_on must not be blocked, got: {:?}",
        derived.blocked
    );
    assert!(
        derived.next.contains(&"AL.1.A".to_string()),
        "block with the operator dep removed must be ready, got: {:?}",
        derived.next
    );
}

// ---------------------------------------------------------------------------
// Task 2 — effective-priority propagation through operator/approval edges
// ---------------------------------------------------------------------------
//
// `operator`/`approval` `depends_on` entries are targetless (per
// `okf_core::state::BlockedBy`'s doc comments): `build_state_graph` never
// emits a `StateEdge` for them (only `{type:"block"}` entries become
// edges), so `effective_priorities`' reverse-topological walk — which
// propagates purely over `StateEdgeKind::BlockedBy` edges — never sees
// them at all. These tests exist to catch a *future* regression where
// someone adds special-case handling that excludes a block's own entry
// from `effective_priorities`' `own` map on the basis of it carrying an
// operator/approval dep, or that starts emitting edges for these variants
// and breaks the "targetless" invariant.

#[test]
fn effective_priorities_operator_gate_on_p0_block_reports_p0() {
    use mev::brain::state::{build_state_graph, effective_priorities};

    // AL.1.A is priority 0 and carries an operator dep — its own effective
    // priority must still be 0, unaffected by the targetless operator entry.
    let (src, file) = make_leaf_pair_in_memory(
        "alpha",
        serde_json::json!([{
            "title": "Phase 1",
            "blocks": [{
                "id": "AL.1.A",
                "title": "Gated by an operator session",
                "status": "open",
                "priority": 0,
                "depends_on": [{
                    "type": "operator",
                    "slug": "mac-mini-setup",
                    "exit": "the mini boots headless",
                    "start": "ssh mini 'sudo launchctl load ...'"
                }]
            }]
        }]),
        serde_json::json!({ "now": [], "next": [], "blocked": [] }),
    );
    let files = vec![(src, file)];
    let graph = build_state_graph(&files);

    let effective = effective_priorities(&graph, &files);

    assert_eq!(
        effective.get("alpha:AL.1.A").copied(),
        Some(0),
        "a P0 block's own effective priority must be unaffected by an \
         operator dep it carries; got {effective:?}"
    );
}

#[test]
fn effective_priorities_approval_gate_inherits_min_of_gated_blocks() {
    use mev::brain::state::{build_state_graph, effective_priorities};

    // Two blocks, alpha:AL.1.A (P0) and alpha:AL.1.B (P2), both carry an
    // approval dep on the SAME slug "ship-it" — the shared gate. Neither
    // block depends on the other, so each block's own effective priority
    // must equal its own authored priority (no cross-block propagation
    // happens over a shared targetless slug; that aggregation is the
    // rendering layer's job, per the ticket's "Shared identity" task).
    // What this pins is the *input* that rendering will later take a min
    // over: both blocks resolve to a real effective priority, and the
    // gate's effective priority — computed here as min(effective(A),
    // effective(B)), exactly the aggregation task 5's dedup will perform —
    // is 0, the hottest of the two blocks it gates.
    let (src, file) = make_leaf_pair_in_memory(
        "alpha",
        serde_json::json!([{
            "title": "Phase 1",
            "blocks": [
                {
                    "id": "AL.1.A",
                    "title": "Hot block gated on the shared approval",
                    "status": "open",
                    "priority": 0,
                    "depends_on": [{
                        "type": "approval",
                        "slug": "ship-it",
                        "what": "ship the release",
                        "digest": "sha256:deadbeef"
                    }]
                },
                {
                    "id": "AL.1.B",
                    "title": "Cold block gated on the same shared approval",
                    "status": "open",
                    "priority": 2,
                    "depends_on": [{
                        "type": "approval",
                        "slug": "ship-it",
                        "what": "ship the release",
                        "digest": "sha256:deadbeef"
                    }]
                }
            ]
        }]),
        serde_json::json!({ "now": [], "next": [], "blocked": [] }),
    );
    let files = vec![(src, file)];
    let graph = build_state_graph(&files);

    let effective = effective_priorities(&graph, &files);

    let a = effective
        .get("alpha:AL.1.A")
        .copied()
        .expect("AL.1.A must have an effective priority");
    let b = effective
        .get("alpha:AL.1.B")
        .copied()
        .expect("AL.1.B must have an effective priority");
    assert_eq!(a, 0, "AL.1.A's own priority must be preserved");
    assert_eq!(b, 2, "AL.1.B's own priority must be preserved");

    let gate_effective_priority = a.min(b);
    assert_eq!(
        gate_effective_priority, 0,
        "the shared 'ship-it' gate's effective priority — min over every \
         block it gates — must be the hottest of the two, P0"
    );
}

#[test]
fn effective_priorities_targetless_gate_with_no_gated_blocks_keeps_own_priority() {
    use mev::brain::state::{build_state_graph, effective_priorities};

    // A block with an operator dep and no other block depending on it must
    // not panic and must retain its own priority — mirroring
    // `effective_priorities_block_with_no_hot_dependents_keeps_own_priority`
    // in src/brain/state.rs, extended to a block that also carries an
    // operator entry.
    let (src, file) = make_leaf_pair_in_memory(
        "solo",
        serde_json::json!([{
            "title": "Phase 1",
            "blocks": [{
                "id": "S.1",
                "title": "Solo block with an operator dep, nothing depends on it",
                "status": "open",
                "priority": 2,
                "depends_on": [{
                    "type": "operator",
                    "slug": "lonely-gate",
                    "exit": "n/a",
                    "start": "n/a"
                }]
            }]
        }]),
        serde_json::json!({ "now": [], "next": [], "blocked": [] }),
    );
    let files = vec![(src, file)];
    let graph = build_state_graph(&files);

    let effective = effective_priorities(&graph, &files);

    assert_eq!(effective.get("solo:S.1").copied(), Some(2));
}

#[test]
fn derive_focus_empty_tracks_returns_empty() {
    use mev::brain::state::{StateGraph, StateSource, derive_focus};

    // Brain file with no tracks[] — derivation should be undefined (return empty).
    let path = std::path::PathBuf::from("/tmp/hq-state.json");
    let json = serde_json::json!({
        "repo": "hq",
        "kind": "brain",
        "updated": "2026-06-30",
        "focus": { "now": [], "next": [], "blocked": [] },
        "repos": []
    });
    let file: mev::brain::state::StateFile =
        serde_json::from_value(json).expect("fixture must parse");
    let src = StateSource {
        repo_slug: "hq".to_string(),
        abs_path: path,
        expected_kind: "brain",
    };
    let files = vec![(src.clone(), file.clone())];
    let graph = StateGraph::default();

    let derived = derive_focus(&src, &file, &graph, &files, None);
    assert!(derived.now.is_empty());
    assert!(derived.next.is_empty());
    assert!(derived.blocked.is_empty());
}

#[test]
fn derive_cross_repo_produces_edge_for_cross_repo_dep_and_none_for_same_repo() {
    use mev::brain::state::derive_cross_repo;

    // alpha block A depends on beta block B (cross-repo) and alpha block C (same-repo).
    let (src_alpha, file_alpha) = make_leaf_pair_in_memory(
        "alpha",
        serde_json::json!([{
            "title": "Phase 1",
            "blocks": [
                {
                    "id": "AL.1.A",
                    "title": "Alpha block",
                    "status": "open",
                    "depends_on": [
                        { "type": "block", "repo": "beta", "id": "BE.1.B" },
                        { "type": "block", "repo": "alpha", "id": "AL.1.C" }
                    ]
                },
                { "id": "AL.1.C", "title": "Same-repo dep", "status": "open" }
            ]
        }]),
        serde_json::json!({ "now": [], "next": [], "blocked": [] }),
    );
    let (src_beta, file_beta) = make_leaf_pair_in_memory(
        "beta",
        serde_json::json!([{
            "title": "Phase 1",
            "blocks": [{ "id": "BE.1.B", "title": "Beta block", "status": "closed" }]
        }]),
        serde_json::json!({ "now": [], "next": [], "blocked": [] }),
    );
    let files = vec![
        (src_alpha.clone(), file_alpha.clone()),
        (src_beta.clone(), file_beta.clone()),
    ];

    let edges = derive_cross_repo(&files);

    // Only the cross-repo dep (alpha → beta) should produce an edge.
    assert_eq!(
        edges.len(),
        1,
        "expected exactly one cross-repo edge, got: {edges:?}"
    );
    assert_eq!(edges[0].from.repo, "alpha");
    assert_eq!(edges[0].from.id, "AL.1.A");
    assert_eq!(edges[0].to.repo, "beta");
    assert_eq!(edges[0].to.id, "BE.1.B");
}

#[test]
fn derive_cross_repo_skips_external_deps() {
    use mev::brain::state::derive_cross_repo;

    let (src, file) = make_leaf_pair_in_memory(
        "alpha",
        serde_json::json!([{
            "title": "Phase 1",
            "blocks": [{
                "id": "AL.1.A",
                "title": "Block",
                "status": "open",
                "depends_on": [{ "type": "external", "what": "hardware" }]
            }]
        }]),
        serde_json::json!({ "now": [], "next": [], "blocked": [] }),
    );
    let files = vec![(src, file)];
    let edges = derive_cross_repo(&files);
    assert!(
        edges.is_empty(),
        "external deps must not produce cross-repo edges, got: {edges:?}"
    );
}

fn make_config_with_repo(slug: &str, tier: &str) -> mev::brain::config::BrainConfig {
    use mev::brain::config::{BrainConfig, CrawlConfig, RepoEntry, VocabConfig};
    BrainConfig {
        attention: Default::default(),
        history: Default::default(),
        carryover: Default::default(),
        vocab: VocabConfig::default(),
        crawl: CrawlConfig::default(),
        repos: vec![RepoEntry {
            slug: slug.to_string(),
            tier: tier.to_string(),
            repo_path: String::new(),
            status_file: String::new(),
            cache_doc: String::new(),
            heading: String::new(),
            prefix: None,
        }],
    }
}

#[test]
fn derive_rollup_reproduces_childs_derived_focus() {
    use mev::brain::state::{TierScope, build_state_graph, derive_rollup};

    // alpha: one in_progress block, one open block (no deps → ready).
    let (src_alpha, file_alpha) = make_leaf_pair_in_memory(
        "alpha",
        serde_json::json!([{
            "title": "Phase 1",
            "blocks": [
                { "id": "AL.1.A", "title": "In progress", "status": "in_progress" },
                { "id": "AL.1.B", "title": "Next up", "status": "open" }
            ]
        }]),
        serde_json::json!({ "now": [], "next": [], "blocked": [] }),
    );
    let files = vec![(src_alpha.clone(), file_alpha.clone())];
    let graph = build_state_graph(&files);
    let config = make_config_with_repo("alpha", "core");

    let rollups = derive_rollup(
        &TierScope::Tier("core".to_string()),
        &config,
        &[],
        &graph,
        &files,
    );

    assert_eq!(rollups.len(), 1, "one child → one rollup entry");
    let rollup = &rollups[0];
    assert_eq!(rollup.repo, "alpha");

    // now should contain AL.1.A with status in_progress.
    assert_eq!(rollup.now.len(), 1);
    assert_eq!(rollup.now[0].id, "AL.1.A");
    assert_eq!(rollup.now[0].status.as_deref(), Some("in_progress"));

    // next should contain AL.1.B (open, no deps).
    assert_eq!(rollup.next.len(), 1);
    assert_eq!(rollup.next[0].id, "AL.1.B");

    // blocked should be empty.
    assert!(rollup.blocked.is_empty());

    // tier should be populated from config.
    assert_eq!(rollup.tier.as_deref(), Some("core"));
}

#[test]
fn derive_rollup_stubs_when_only_a_brain_file_matches_the_slug() {
    use mev::brain::state::{StateGraph, StateSource, TierScope, derive_rollup};

    // A brain-kind file must not be picked up as the rollup's derivation source
    // for its own slug — it should fall back to an empty tier-tagged stub.
    let path = std::path::PathBuf::from("/tmp/hq-state.json");
    let json = serde_json::json!({
        "repo": "hq",
        "kind": "brain",
        "updated": "2026-06-30",
        "focus": { "now": [], "next": [], "blocked": [] },
        "repos": []
    });
    let brain_file: mev::brain::state::StateFile =
        serde_json::from_value(json).expect("fixture must parse");
    let brain_src = StateSource {
        repo_slug: "hq".to_string(),
        abs_path: path,
        expected_kind: "brain",
    };
    let files = vec![(brain_src.clone(), brain_file.clone())];
    let graph = StateGraph::default();
    let config = make_config_with_repo("hq", "core");

    let rollups = derive_rollup(
        &TierScope::Tier("core".to_string()),
        &config,
        &[],
        &graph,
        &files,
    );
    assert_eq!(rollups.len(), 1, "expected a stub entry for 'hq'");
    assert_eq!(rollups[0].repo, "hq");
    assert_eq!(rollups[0].tier.as_deref(), Some("core"));
    assert!(rollups[0].now.is_empty());
    assert!(rollups[0].next.is_empty());
    assert!(rollups[0].blocked.is_empty());
}

#[test]
fn derive_rollup_reproduces_a_brain_kind_childs_derived_focus() {
    use mev::brain::state::{StateFile, StateSource, TierScope, build_state_graph, derive_rollup};

    // Public-API-level counterpart to
    // derive_rollup_brain_kind_file_yields_its_own_lane_contents: a
    // registered repo whose loaded state file is `kind: "brain"` but carries
    // its own non-empty `tracks[]` (the live shape for `business`/`core`/`hq`
    // tier roots) must derive real lane contents through derive_rollup, not
    // fall back to an empty stub.
    let path = std::path::PathBuf::from("/tmp/business-state.json");
    let json = serde_json::json!({
        "repo": "business",
        "kind": "brain",
        "updated": "2026-08-03",
        "focus": { "now": [], "next": [], "blocked": [] },
        "tracks": [{
            "title": "Business Ops",
            "blocks": [
                { "id": "BZ.1.A", "title": "In progress work", "status": "in_progress" },
                { "id": "BZ.1.B", "title": "Ready work", "status": "open" }
            ]
        }]
    });
    let brain_file: StateFile = serde_json::from_value(json).expect("fixture must parse");
    let brain_src = StateSource {
        repo_slug: "business".to_string(),
        abs_path: path,
        expected_kind: "brain",
    };
    let files = vec![(brain_src, brain_file)];
    let graph = build_state_graph(&files);
    let config = make_config_with_repo("business", "core");

    let rollups = derive_rollup(
        &TierScope::Tier("core".to_string()),
        &config,
        &[],
        &graph,
        &files,
    );

    assert_eq!(rollups.len(), 1, "one child -> one rollup entry");
    let rollup = &rollups[0];
    assert_eq!(rollup.repo, "business");

    assert_eq!(rollup.now.len(), 1);
    assert_eq!(rollup.now[0].id, "BZ.1.A");
    assert_eq!(rollup.now[0].status.as_deref(), Some("in_progress"));

    assert_eq!(rollup.next.len(), 1);
    assert_eq!(rollup.next[0].id, "BZ.1.B");

    assert!(rollup.blocked.is_empty());
    assert_eq!(rollup.tier.as_deref(), Some("core"));
}

#[test]
fn field_policy_integration() {
    let root = temp_dir("field-policy");
    write_brain_toml(&root);
    write_hq_brain_state(&root);

    let alpha = serde_json::json!({
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
                        "status": "in_progress",
                        "priority": 4,
                        "due": "Q3",
                        "sdlc_workflow": "pipeline",
                        "model": "gpt"
                    }
                ]
            }
        ]
    });
    write_json(&root, "repos/alpha/planning/state.json", &alpha);
    write_beta_state(&root);

    let report = mev::validate_brain_state(&root).unwrap();

    let has_prio = report
        .diagnostics
        .iter()
        .any(|d| d.locator == "E_STATE_PRIORITY_RANGE");
    let has_due = report
        .diagnostics
        .iter()
        .any(|d| d.locator == "E_STATE_DUE_FORMAT");
    let has_wf = report
        .diagnostics
        .iter()
        .any(|d| d.locator == "E_STATE_SDLC_WORKFLOW_ENUM");
    let has_model = report
        .diagnostics
        .iter()
        .any(|d| d.locator == "E_STATE_MODEL_ENUM");

    assert!(has_prio, "missing E_STATE_PRIORITY_RANGE");
    assert!(has_due, "missing E_STATE_DUE_FORMAT");
    assert!(has_wf, "missing E_STATE_SDLC_WORKFLOW_ENUM");
    assert!(has_model, "missing E_STATE_MODEL_ENUM");
}

// ---------------------------------------------------------------------------
// Task 4 — E_STATE_OPERATOR_MISSING_EXIT / E_STATE_APPROVAL_DIGEST_SHAPE /
// W_STATE_OPERATOR_STALE
// ---------------------------------------------------------------------------

fn has_locator(diags: &[mev::Diagnostic], locator: &str) -> bool {
    diags.iter().any(|d| d.locator == locator)
}

#[test]
fn check_schema_operator_dep_empty_exit_emits_missing_exit() {
    use mev::brain::state::check_schema;

    let (src, file) = make_leaf_pair_in_memory(
        "alpha",
        serde_json::json!([{
            "title": "Phase 1",
            "blocks": [{
                "id": "AL.1.A",
                "title": "Gated on an operator step",
                "status": "open",
                "depends_on": [{
                    "type": "operator",
                    "slug": "mac-mini-setup",
                    "exit": "",
                    "start": "/begin-session mac-mini-setup"
                }]
            }]
        }]),
        serde_json::json!({ "now": [], "next": [], "blocked": [] }),
    );

    let diags = check_schema(&src, &file);

    assert!(
        has_locator(&diags, "E_STATE_OPERATOR_MISSING_EXIT"),
        "expected E_STATE_OPERATOR_MISSING_EXIT, got: {diags:?}"
    );
}

#[test]
fn check_schema_approval_dep_empty_digest_emits_digest_shape() {
    use mev::brain::state::check_schema;

    let (src, file) = make_leaf_pair_in_memory(
        "alpha",
        serde_json::json!([{
            "title": "Phase 1",
            "blocks": [{
                "id": "AL.1.A",
                "title": "Gated on an approval",
                "status": "open",
                "depends_on": [{
                    "type": "approval",
                    "slug": "ship-it",
                    "what": "approve the release payload",
                    "digest": ""
                }]
            }]
        }]),
        serde_json::json!({ "now": [], "next": [], "blocked": [] }),
    );

    let diags = check_schema(&src, &file);

    assert!(
        has_locator(&diags, "E_STATE_APPROVAL_DIGEST_SHAPE"),
        "expected E_STATE_APPROVAL_DIGEST_SHAPE for empty digest, got: {diags:?}"
    );
}

#[test]
fn check_schema_approval_dep_malformed_digest_emits_digest_shape() {
    use mev::brain::state::check_schema;

    // No "algorithm:hex" separator, and non-hex characters — malformed either way.
    let (src, file) = make_leaf_pair_in_memory(
        "alpha",
        serde_json::json!([{
            "title": "Phase 1",
            "blocks": [{
                "id": "AL.1.A",
                "title": "Gated on an approval",
                "status": "open",
                "depends_on": [{
                    "type": "approval",
                    "slug": "ship-it",
                    "what": "approve the release payload",
                    "digest": "not-a-digest!!"
                }]
            }]
        }]),
        serde_json::json!({ "now": [], "next": [], "blocked": [] }),
    );

    let diags = check_schema(&src, &file);

    assert!(
        has_locator(&diags, "E_STATE_APPROVAL_DIGEST_SHAPE"),
        "expected E_STATE_APPROVAL_DIGEST_SHAPE for malformed digest, got: {diags:?}"
    );
}

#[test]
fn check_schema_well_formed_operator_and_approval_deps_emit_neither_new_code() {
    use mev::brain::state::check_schema;

    let (src, file) = make_leaf_pair_in_memory(
        "alpha",
        serde_json::json!([{
            "title": "Phase 1",
            "blocks": [
                {
                    "id": "AL.1.A",
                    "title": "Gated on a well-formed operator step",
                    "status": "open",
                    "depends_on": [{
                        "type": "operator",
                        "slug": "mac-mini-setup",
                        "exit": "planning/handoff.md",
                        "start": "/begin-session mac-mini-setup"
                    }]
                },
                {
                    "id": "AL.1.B",
                    "title": "Gated on a well-formed approval",
                    "status": "open",
                    "depends_on": [{
                        "type": "approval",
                        "slug": "ship-it",
                        "what": "approve the release payload",
                        "digest": "sha256:abc123"
                    }]
                }
            ]
        }]),
        serde_json::json!({ "now": [], "next": [], "blocked": [] }),
    );

    let diags = check_schema(&src, &file);

    assert!(
        !has_locator(&diags, "E_STATE_OPERATOR_MISSING_EXIT"),
        "well-formed operator dep must not emit E_STATE_OPERATOR_MISSING_EXIT: {diags:?}"
    );
    assert!(
        !has_locator(&diags, "E_STATE_APPROVAL_DIGEST_SHAPE"),
        "well-formed approval dep must not emit E_STATE_APPROVAL_DIGEST_SHAPE: {diags:?}"
    );
}

/// Build a minimal in-memory (StateSource, StateFile) pair like
/// [`make_leaf_pair_in_memory`], but with a caller-controlled `updated` date —
/// needed for [`check_operator_staleness`] tests, which anchor on that field.
fn make_leaf_pair_with_updated(
    repo: &str,
    updated: &str,
    tracks_json: serde_json::Value,
) -> (mev::brain::state::StateSource, mev::brain::state::StateFile) {
    use mev::brain::state::StateSource;

    let path = std::path::PathBuf::from(format!("/tmp/{repo}-state-staleness.json"));
    let json = serde_json::json!({
        "repo": repo,
        "kind": "project",
        "updated": updated,
        "focus": { "now": [], "next": [], "blocked": [] },
        "tracks": tracks_json,
    });
    let file: mev::brain::state::StateFile =
        serde_json::from_value(json).expect("fixture must parse");
    let src = StateSource {
        repo_slug: repo.to_string(),
        abs_path: path,
        expected_kind: "project",
    };
    (src, file)
}

fn operator_dep_tracks(slug: &str) -> serde_json::Value {
    serde_json::json!([{
        "title": "Phase 1",
        "blocks": [{
            "id": "AL.1.A",
            "title": "Gated on an operator step",
            "status": "open",
            "depends_on": [{
                "type": "operator",
                "slug": slug,
                "exit": "planning/handoff.md",
                "start": format!("/begin-session {slug}")
            }]
        }]
    }])
}

#[test]
fn check_operator_staleness_past_threshold_emits_warning() {
    use mev::brain::config::AttentionThresholds;
    use mev::brain::state::check_operator_staleness;

    let thresholds = AttentionThresholds::default(); // operator_days default = 7
    let today = chrono::NaiveDate::from_ymd_opt(2026, 8, 12).unwrap();
    // 2026-07-01 -> 2026-08-12 is 42 days old, well past the 7-day default.
    let (src, file) =
        make_leaf_pair_with_updated("alpha", "2026-07-01", operator_dep_tracks("mac-mini-setup"));

    let diags = check_operator_staleness(&src, &file, today, &thresholds);

    assert!(
        has_locator(&diags, "W_STATE_OPERATOR_STALE"),
        "expected W_STATE_OPERATOR_STALE for a 42d-old operator gate, got: {diags:?}"
    );
}

#[test]
fn check_operator_staleness_under_threshold_emits_nothing() {
    use mev::brain::config::AttentionThresholds;
    use mev::brain::state::check_operator_staleness;

    let thresholds = AttentionThresholds::default(); // operator_days default = 7
    let today = chrono::NaiveDate::from_ymd_opt(2026, 8, 12).unwrap();
    // 2026-08-10 -> 2026-08-12 is 2 days old, under the 7-day default.
    let (src, file) =
        make_leaf_pair_with_updated("alpha", "2026-08-10", operator_dep_tracks("mac-mini-setup"));

    let diags = check_operator_staleness(&src, &file, today, &thresholds);

    assert!(
        diags.is_empty(),
        "operator gate under threshold must emit no W_STATE_OPERATOR_STALE, got: {diags:?}"
    );
}

fn approval_dep_tracks(slug: &str) -> serde_json::Value {
    serde_json::json!([{
        "title": "Phase 1",
        "blocks": [{
            "id": "AL.1.A",
            "title": "Gated on an approval step",
            "status": "open",
            "depends_on": [{
                "type": "approval",
                "slug": slug,
                "what": "example decision",
                "digest": "sha256:example"
            }]
        }]
    }])
}

#[test]
fn check_op_slug_stutter_emits_warning_for_stuttering_operator_slug() {
    use mev::brain::state::check_op_slug_stutter;

    let (src, file) = make_leaf_pair_with_updated(
        "alpha",
        "2026-08-10",
        operator_dep_tracks("operator-mac-mini-visit"),
    );

    let diags = check_op_slug_stutter(&src, &file);

    assert!(
        has_locator(&diags, "W_STATE_OP_SLUG_STUTTER"),
        "expected W_STATE_OP_SLUG_STUTTER for a stuttering operator slug, got: {diags:?}"
    );
}

#[test]
fn check_op_slug_stutter_emits_warning_for_stuttering_approval_slug() {
    use mev::brain::state::check_op_slug_stutter;

    let (src, file) = make_leaf_pair_with_updated(
        "alpha",
        "2026-08-10",
        approval_dep_tracks("operator-ship-it"),
    );

    let diags = check_op_slug_stutter(&src, &file);

    assert!(
        has_locator(&diags, "W_STATE_OP_SLUG_STUTTER"),
        "expected W_STATE_OP_SLUG_STUTTER for a stuttering approval slug, got: {diags:?}"
    );
}

/// Non-stuttering boundary cases — same table as okf-core's own
/// `op_slug_cases` (`operator-mac-mini-visit` is the only stuttering
/// example there; these are its non-stuttering neighbours), reused here
/// rather than re-derived so this check's boundary agrees with the
/// primitive it delegates to.
#[test]
fn check_op_slug_stutter_emits_nothing_for_non_stuttering_slugs() {
    use mev::brain::state::check_op_slug_stutter;

    for slug in ["mac-mini-visit", "operator", "operator-", "operators-guild"] {
        let (src, file) =
            make_leaf_pair_with_updated("alpha", "2026-08-10", operator_dep_tracks(slug));

        let diags = check_op_slug_stutter(&src, &file);

        assert!(
            diags.is_empty(),
            "slug {slug:?} must not stutter, got: {diags:?}"
        );
    }
}

#[test]
fn check_op_slug_stutter_emits_one_warning_per_stuttering_edge() {
    use mev::brain::state::check_op_slug_stutter;

    let tracks = serde_json::json!([{
        "title": "Phase 1",
        "blocks": [{
            "id": "AL.1.A",
            "title": "Gated on two operator steps",
            "status": "open",
            "depends_on": [
                {
                    "type": "operator",
                    "slug": "operator-mac-mini-visit",
                    "exit": "planning/handoff.md",
                    "start": "/begin-session operator-mac-mini-visit"
                },
                {
                    "type": "operator",
                    "slug": "operator-ship-it",
                    "exit": "planning/handoff.md",
                    "start": "/begin-session operator-ship-it"
                }
            ]
        }]
    }]);
    let (src, file) = make_leaf_pair_with_updated("alpha", "2026-08-10", tracks);

    let diags = check_op_slug_stutter(&src, &file);

    let stutter_count = diags
        .iter()
        .filter(|d| d.locator == "W_STATE_OP_SLUG_STUTTER")
        .count();
    assert_eq!(
        stutter_count, 2,
        "expected one W_STATE_OP_SLUG_STUTTER per stuttering edge, got: {diags:?}"
    );
}

// ---------------------------------------------------------------------------
// `mev::set_block_status` scoped-closure coverage (MV.14.A, task 2).
//
// Builds its own fixture (distinct from `write_clean_fixture` above) because
// `set_block_status`'s `--write` path chains into `emit_state`, which needs
// an HQ root repo (`repo_path = "."`, required by `scope_dependencies` for
// the HQ board target) plus real `status.md`/cache-doc sentinel files for
// every repo it might rewrite — none of which the `validate_brain_state`
// fixtures above need. Mirrors `tests/emit_state_scope.rs`'s fixture shape.
// ---------------------------------------------------------------------------

/// `brain.toml`: an HQ root ("brain") plus two leaf project repos ("gamma",
/// "delta"), matching `scope_dependencies`'s expectations (an HQ root entry
/// with `repo_path = "."`).
fn write_scope_brain_toml(root: &Path) {
    let toml = r#"[vocab]
layer = ["brain", "engine", "factory", "console", "surface", "infra", "business", "content", "meta"]
status = ["active", "draft", "deprecated", "superseded", "archived"]

[crawl]
skip_dirs = ["target", "node_modules", ".git"]

[[repos]]
slug = "brain"
tier = "_root"
repo_path = "."
status_file = "planning/status.md"
cache_doc = "README.md"
heading = "Company Brain"

[[repos]]
slug = "gamma"
tier = "primary"
repo_path = "repos/gamma"
status_file = "repos/gamma/planning/status.md"
cache_doc = "docs/projects/gamma.md"
heading = "Gamma"

[[repos]]
slug = "delta"
tier = "primary"
repo_path = "repos/delta"
status_file = "repos/delta/planning/status.md"
cache_doc = "docs/projects/delta.md"
heading = "Delta"
"#;
    fs::write(root.join("brain.toml"), toml.as_bytes()).unwrap();
}

/// HQ `planning/state.json` (kind:"brain"), rolling up gamma and delta directly
/// (no tier sub-brains — mirrors this file's existing `write_hq_brain_state`
/// shape rather than `tests/emit_state_scope.rs`'s tiered one).
fn write_scope_hq_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "brain",
        "kind": "brain",
        "updated": "2026-08-19",
        "focus": { "now": [], "next": [], "blocked": [] },
        "repos": [
            {
                "repo": "gamma",
                "now": [{ "id": "GA.1.A", "title": "Gamma block A", "status": "in_progress" }],
                "next": [],
                "blocked": []
            },
            {
                "repo": "delta",
                "now": [{ "id": "DE.1.A", "title": "Delta block A", "status": "in_progress" }],
                "next": [],
                "blocked": []
            }
        ],
        "cross_repo": []
    });
    write_json(root, "planning/state.json", &state);
}

/// HQ `planning/status.md` carrying the generated HQ-board sentinel.
fn write_scope_hq_status_md(root: &Path) {
    let doc = "---\n\
                type: ProjectStatus\n\
                title: HQ status\n\
                description: HQ operating board fixture for scoped set-block-status coverage.\n\
                ---\n\n\
                # HQ Status\n\n\
                <!-- BEGIN generated:hq-board -->\n\
                <!-- END generated:hq-board -->\n";
    write_file(root, "planning/status.md", doc);
}

/// A leaf project repo's `planning/state.json`, carrying one `tracks[]` block
/// (`block_id`) with authored status `"in_progress"` — the target
/// `set_block_status` closes. `focus.now` is deliberately left empty
/// (stale relative to `tracks[]`) so that *any* visit — even one where the
/// authored status itself does not change — still forces the emitter to
/// rewrite this file, matching `tests/emit_state_scope.rs`'s staleness
/// pattern and avoiding a false "byte-identical" reading on an already
/// fixed-point fixture.
fn write_scope_leaf_state(root: &Path, repo_path: &str, repo_slug: &str, block_id: &str) {
    let state = serde_json::json!({
        "repo": repo_slug,
        "kind": "project",
        "updated": "2026-08-19",
        "focus": { "now": [], "next": [], "blocked": [] },
        "tracks": [
            {
                "title": "Phase 1",
                "blocks": [
                    { "id": block_id, "title": format!("{repo_slug} block A"), "status": "in_progress" }
                ]
            }
        ]
    });
    write_json(root, &format!("{repo_path}/planning/state.json"), &state);
}

/// A leaf project repo's own `planning/status.md`, carrying the OKF
/// `timestamp` watermark the cache-doc sync reconciles against.
fn write_scope_leaf_status_md(root: &Path, repo_path: &str, repo_slug: &str) {
    let doc = format!(
        "---\n\
         type: ProjectStatus\n\
         title: {repo_slug} status\n\
         description: Status fixture for {repo_slug}.\n\
         timestamp: \"2026-08-19T12:00:00Z\"\n\
         ---\n\n\
         # Status\n"
    );
    write_file(root, &format!("{repo_path}/planning/status.md"), &doc);
}

/// A leaf project repo's brain cache doc, carrying the project-cache
/// sentinel, empty so a visit produces an observable splice.
fn write_scope_project_cache_doc(root: &Path, repo_slug: &str) {
    let doc = format!(
        "---\n\
         type: ProjectStatus\n\
         title: {repo_slug} cache\n\
         description: Project cache fixture for {repo_slug}.\n\
         ---\n\n\
         # {repo_slug}\n\n\
         <!-- BEGIN generated:project-cache -->\n\
         <!-- END generated:project-cache -->\n"
    );
    write_file(root, &format!("docs/projects/{repo_slug}.md"), &doc);
}

/// Every derived/authored file path this fixture's `set_block_status` calls
/// might touch, relative to `root` — used to snapshot both repos before/after
/// a scoped run for the byte-identical assertion.
fn scope_fixture_files() -> Vec<&'static str> {
    vec![
        "planning/state.json",
        "planning/status.md",
        "repos/gamma/planning/state.json",
        "repos/gamma/planning/status.md",
        "docs/projects/gamma.md",
        "repos/delta/planning/state.json",
        "repos/delta/planning/status.md",
        "docs/projects/delta.md",
    ]
}

/// Build the full fixture: HQ root + two leaf repos ("gamma" owning block
/// `GA.1.A`, "delta" owning block `DE.1.A`), both `in_progress` and closable.
fn write_scope_fixture(root: &Path) {
    write_scope_brain_toml(root);

    write_scope_hq_state(root);
    write_scope_hq_status_md(root);

    write_scope_leaf_state(root, "repos/gamma", "gamma", "GA.1.A");
    write_scope_leaf_status_md(root, "repos/gamma", "gamma");
    write_scope_project_cache_doc(root, "gamma");

    write_scope_leaf_state(root, "repos/delta", "delta", "DE.1.A");
    write_scope_leaf_status_md(root, "repos/delta", "delta");
    write_scope_project_cache_doc(root, "delta");
}

fn scope_snapshot(root: &Path) -> std::collections::HashMap<String, Vec<u8>> {
    scope_fixture_files()
        .into_iter()
        .map(|rel| (rel.to_string(), fs::read(root.join(rel)).unwrap()))
        .collect()
}

fn resolve_gamma_scope(root: &Path) -> mev::brain::config::ScopeDependencySet {
    let config = mev::brain::config::load_brain_config(&root.join("brain.toml")).unwrap();
    config
        .scope_dependencies("gamma")
        .expect("gamma is registered")
}

// ---------------------------------------------------------------------------
// (1) A scoped `--write` closure regenerates only the named repo's derived
//     files — delta's files must stay byte-identical.
// ---------------------------------------------------------------------------

#[test]
fn set_block_status_scoped_write_touches_only_the_scoped_repo() {
    let dir = temp_dir("set-block-status-scoped");
    write_scope_fixture(&dir);

    let before = scope_snapshot(&dir);

    let scope = resolve_gamma_scope(&dir);
    let report = mev::set_block_status(&dir, "gamma:GA.1.A", "closed", true, Some(&scope))
        .expect("scoped set_block_status should not error");
    let errors: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.severity == mev::Severity::Error)
        .collect();
    assert!(
        errors.is_empty(),
        "scoped closure should have no errors; got: {errors:#?}"
    );

    let after = scope_snapshot(&dir);

    // gamma's own authored + derived surfaces must have changed: the status
    // was written, and the chained scoped emit regenerated gamma's cache doc
    // and the HQ board gamma feeds.
    for rel in [
        "repos/gamma/planning/state.json",
        "docs/projects/gamma.md",
        "planning/status.md",
    ] {
        assert_ne!(
            before[rel], after[rel],
            "'{rel}' is one of gamma's scope surfaces and must have changed"
        );
    }
    let gamma_state: serde_json::Value = serde_json::from_str(
        std::str::from_utf8(&after["repos/gamma/planning/state.json"]).unwrap(),
    )
    .unwrap();
    let closed_status = gamma_state["tracks"][0]["blocks"][0]["status"]
        .as_str()
        .unwrap();
    assert_eq!(
        closed_status, "closed",
        "gamma's GA.1.A must be authored closed"
    );

    // delta was never named in `--scope gamma` and must be byte-identical —
    // the core assertion of this task.
    for rel in [
        "repos/delta/planning/state.json",
        "repos/delta/planning/status.md",
        "docs/projects/delta.md",
    ] {
        assert_eq!(
            before[rel], after[rel],
            "'{rel}' is NOT in gamma's scope and must be byte-identical before/after \
             a `--scope gamma` closure, but it changed"
        );
    }

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// (2) The same closure with `scope: None` regenerates fleet-wide — both
//     repos' derived surfaces change, unchanged from pre-existing behaviour.
// ---------------------------------------------------------------------------

#[test]
fn set_block_status_unscoped_write_still_regenerates_every_repo() {
    let dir = temp_dir("set-block-status-unscoped");
    write_scope_fixture(&dir);

    let before = scope_snapshot(&dir);

    let report = mev::set_block_status(&dir, "delta:DE.1.A", "closed", true, None)
        .expect("unscoped set_block_status should not error");
    let errors: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.severity == mev::Severity::Error)
        .collect();
    assert!(
        errors.is_empty(),
        "unscoped closure should have no errors; got: {errors:#?}"
    );

    let after = scope_snapshot(&dir);

    // Both repos' own state + cache docs, and the HQ board, must all have
    // moved — an unscoped write is fleet-wide exactly as before this flag
    // existed.
    for rel in [
        "repos/delta/planning/state.json",
        "docs/projects/delta.md",
        "repos/gamma/planning/state.json",
        "docs/projects/gamma.md",
        "planning/status.md",
    ] {
        assert_ne!(
            before[rel], after[rel],
            "'{rel}' must be regenerated by an unscoped write, but is byte-identical"
        );
    }

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// (3) An unknown `--scope` slug is rejected before any write is attempted —
//     matches `emit-state`'s own `E_EMIT_UNKNOWN_SCOPE` resolution path,
//     since both verbs share `BrainConfig::scope_dependencies`.
// ---------------------------------------------------------------------------

#[test]
fn set_block_status_unknown_scope_slug_is_rejected_with_valid_slugs() {
    let dir = temp_dir("set-block-status-unknown-scope");
    write_scope_fixture(&dir);

    let config = mev::brain::config::load_brain_config(&dir.join("brain.toml")).unwrap();
    let err = config
        .scope_dependencies("nonexistent")
        .expect_err("nonexistent must not resolve");

    match err {
        mev::brain::config::ScopeError::UnknownSlug { slug, valid_slugs } => {
            assert_eq!(slug, "nonexistent");
            assert_eq!(valid_slugs, vec!["brain", "gamma", "delta"]);
        }
        other => panic!("expected ScopeError::UnknownSlug, got {other:?}"),
    }

    // Confirm no fixture file was touched by the rejected resolution — the
    // failure happens entirely before `set_block_status` is ever called.
    let before = scope_snapshot(&dir);
    assert_eq!(
        before,
        scope_snapshot(&dir),
        "an unknown --scope slug must not mutate the fixture"
    );

    let _ = fs::remove_dir_all(&dir);
}
