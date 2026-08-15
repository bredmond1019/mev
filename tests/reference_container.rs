//! Integration tests for `reference[]` validation in `check_schema`.
//!
//! `MV.ticket.reference-container-validation` — Task 1.
//!
//! Each test writes a `planning/state.json` fixture to a temp dir, loads it with
//! `load_state`, and runs `check_schema` directly — the same per-file ring
//! `validate_brain_state` composes from. Covers: an unknown `class` (error naming
//! all four valid values), each of the four class values accepted, a malformed
//! `scope` (zero or two of repo/tier/cross_repo set, `cross_repo: false` counting
//! as set), a malformed `created`/`reviewed` date, and a slug collision between
//! `reference[]` and `carryover[]` in the same file.

use std::fs;
use std::path::Path;

use mev::brain::state::{StateSource, check_schema, load_state};

/// Create a file at `root/rel` (creating parent dirs as needed) with `content`.
fn write_file(root: &Path, rel: &str, content: &str) {
    let full = root.join(rel);
    fs::create_dir_all(full.parent().unwrap()).unwrap();
    fs::write(&full, content.as_bytes()).unwrap();
}

/// Serialize `value` as pretty JSON and write it to `root/rel`.
fn write_json(root: &Path, rel: &str, value: &serde_json::Value) {
    write_file(root, rel, &serde_json::to_string_pretty(value).unwrap());
}

fn temp_dir(suffix: &str) -> std::path::PathBuf {
    let dir = mev::testsupport::unique_temp_dir(&format!("mev-reference-container-it-{suffix}"));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Load `root/planning/state.json` and run `check_schema` over it.
fn check(root: &Path) -> Vec<mev::Diagnostic> {
    let path = root.join("planning/state.json");
    let file = load_state(&path).expect("state.json should load");
    let src = StateSource {
        repo_slug: "sample".to_string(),
        abs_path: path,
        expected_kind: "project",
    };
    check_schema(&src, &file)
}

#[test]
fn unknown_class_errors_naming_all_four_values() {
    let root = temp_dir("unknown-class");
    let state = serde_json::json!({
        "repo": "sample",
        "kind": "project",
        "updated": "2026-08-15",
        "reference": [
            {
                "slug": "bad-class",
                "scope": { "repo": "sample" },
                "class": "not_a_class",
                "text": "Some reference text.",
                "created": "2026-08-15"
            }
        ]
    });
    write_json(&root, "planning/state.json", &state);

    let diags = check(&root);
    let hit = diags
        .iter()
        .find(|d| d.locator == "E_STATE_SCHEMA_BAD_KIND" && d.message.contains("bad-class"))
        .unwrap_or_else(|| panic!("expected a bad-class diagnostic, got: {diags:?}"));

    for expected in ["trap", "invariant", "lesson", "deliberate"] {
        assert!(
            hit.message.contains(expected),
            "message should enumerate '{expected}': {}",
            hit.message
        );
    }
}

#[test]
fn each_class_value_is_accepted_with_no_diagnostic() {
    for class in ["trap", "invariant", "lesson", "deliberate"] {
        let root = temp_dir(&format!("accepted-{class}"));
        let state = serde_json::json!({
            "repo": "sample",
            "kind": "project",
            "updated": "2026-08-15",
            "reference": [
                {
                    "slug": format!("ref-{class}"),
                    "scope": { "repo": "sample" },
                    "class": class,
                    "text": "Some reference text.",
                    "created": "2026-08-15"
                }
            ]
        });
        write_json(&root, "planning/state.json", &state);

        // Filter out the unrelated `project` `tracks[]`-empty warning this
        // fixture also trips (`E_STATE_SCHEMA_MISSING_FIELD`) — irrelevant to
        // reference[] validation, which this test isolates.
        let diags: Vec<_> = check(&root)
            .into_iter()
            .filter(|d| d.locator != "E_STATE_SCHEMA_MISSING_FIELD")
            .collect();
        assert!(
            diags.is_empty(),
            "class '{class}' should be accepted with no diagnostics, got: {diags:?}"
        );
    }
}

#[test]
fn scope_with_zero_set_fields_errors() {
    let root = temp_dir("scope-zero");
    let state = serde_json::json!({
        "repo": "sample",
        "kind": "project",
        "updated": "2026-08-15",
        "reference": [
            {
                "slug": "no-scope",
                "scope": {},
                "class": "trap",
                "text": "Some reference text.",
                "created": "2026-08-15"
            }
        ]
    });
    write_json(&root, "planning/state.json", &state);

    let diags = check(&root);
    assert!(
        diags.iter().any(
            |d| d.locator == "E_STATE_SCHEMA_MALFORMED_SCOPE" && d.message.contains("no-scope")
        ),
        "expected E_STATE_SCHEMA_MALFORMED_SCOPE for empty scope, got: {diags:?}"
    );
}

#[test]
fn scope_with_two_set_fields_errors() {
    let root = temp_dir("scope-two");
    let state = serde_json::json!({
        "repo": "sample",
        "kind": "project",
        "updated": "2026-08-15",
        "reference": [
            {
                "slug": "double-scope",
                "scope": { "repo": "sample", "tier": "core" },
                "class": "trap",
                "text": "Some reference text.",
                "created": "2026-08-15"
            }
        ]
    });
    write_json(&root, "planning/state.json", &state);

    let diags = check(&root);
    assert!(
        diags
            .iter()
            .any(|d| d.locator == "E_STATE_SCHEMA_MALFORMED_SCOPE"
                && d.message.contains("double-scope")),
        "expected E_STATE_SCHEMA_MALFORMED_SCOPE for double-set scope, got: {diags:?}"
    );
}

#[test]
fn scope_cross_repo_false_counts_as_set() {
    let root = temp_dir("scope-cross-repo-false");
    let state = serde_json::json!({
        "repo": "sample",
        "kind": "project",
        "updated": "2026-08-15",
        "reference": [
            {
                "slug": "cross-repo-false",
                "scope": { "cross_repo": false },
                "class": "trap",
                "text": "Some reference text.",
                "created": "2026-08-15"
            }
        ]
    });
    write_json(&root, "planning/state.json", &state);

    let diags = check(&root);
    assert!(
        !diags
            .iter()
            .any(|d| d.locator == "E_STATE_SCHEMA_MALFORMED_SCOPE"),
        "cross_repo: false must count as set (exactly one scope field), got: {diags:?}"
    );
}

#[test]
fn malformed_created_date_errors() {
    let root = temp_dir("bad-created");
    let state = serde_json::json!({
        "repo": "sample",
        "kind": "project",
        "updated": "2026-08-15",
        "reference": [
            {
                "slug": "bad-created",
                "scope": { "repo": "sample" },
                "class": "trap",
                "text": "Some reference text.",
                "created": "not-a-date"
            }
        ]
    });
    write_json(&root, "planning/state.json", &state);

    let diags = check(&root);
    assert!(
        diags
            .iter()
            .any(|d| d.locator == "E_STATE_DATE_FORMAT" && d.message.contains("bad-created")),
        "expected E_STATE_DATE_FORMAT for malformed created date, got: {diags:?}"
    );
}

#[test]
fn malformed_reviewed_date_errors() {
    let root = temp_dir("bad-reviewed");
    let state = serde_json::json!({
        "repo": "sample",
        "kind": "project",
        "updated": "2026-08-15",
        "reference": [
            {
                "slug": "bad-reviewed",
                "scope": { "repo": "sample" },
                "class": "trap",
                "text": "Some reference text.",
                "created": "2026-08-15",
                "reviewed": "2026-13-99"
            }
        ]
    });
    write_json(&root, "planning/state.json", &state);

    let diags = check(&root);
    assert!(
        diags
            .iter()
            .any(|d| d.locator == "E_STATE_DATE_FORMAT" && d.message.contains("bad-reviewed")),
        "expected E_STATE_DATE_FORMAT for malformed reviewed date, got: {diags:?}"
    );
}

#[test]
fn slug_collision_between_reference_and_carryover_errors_naming_both_containers() {
    let root = temp_dir("collision");
    let state = serde_json::json!({
        "repo": "sample",
        "kind": "project",
        "updated": "2026-08-15",
        "carryover": [
            {
                "slug": "shared-slug",
                "scope": { "repo": "sample" },
                "kind": "deferred",
                "text": "A carryover entry.",
                "created": "2026-08-15"
            }
        ],
        "reference": [
            {
                "slug": "shared-slug",
                "scope": { "repo": "sample" },
                "class": "trap",
                "text": "A reference entry with the same slug.",
                "created": "2026-08-15"
            }
        ]
    });
    write_json(&root, "planning/state.json", &state);

    let diags = check(&root);
    let hit = diags
        .iter()
        .find(|d| d.locator == "E_STATE_REFERENCE_CARRYOVER_COLLISION")
        .unwrap_or_else(|| panic!("expected a collision diagnostic, got: {diags:?}"));
    assert!(hit.message.contains("shared-slug"));
    assert!(hit.message.contains("reference"));
    assert!(hit.message.contains("carryover"));
}
