//! Integration tests for `reference[]` validation in `check_schema`
//! (Task 1) plus the triage-surface exclusion pins (Task 3).
//!
//! `MV.ticket.reference-container-validation`.
//!
//! Each test writes a `planning/state.json` fixture to a temp dir, loads it with
//! `load_state`, and runs `check_schema` directly — the same per-file ring
//! `validate_brain_state` composes from. Covers: an unknown `class` (error naming
//! all four valid values), each of the four class values accepted, a malformed
//! `scope` (zero or two of repo/tier/cross_repo set, `cross_repo: false` counting
//! as set), a malformed `created`/`reviewed` date, and a slug collision between
//! `reference[]` and `carryover[]` in the same file.
//!
//! The `triage_surface_exclusion` module (Task 3) pins that a `reference[]`
//! entry — however old — never produces a staleness diagnostic, never lands
//! in an Attention-board lane, and never reaches `mev attention-queue`'s
//! payload output. Nothing in the codebase reads `.reference` from any of
//! those three call sites today (`rg '\.reference\b' src/` outside
//! `check_schema` returns zero hits), so these are regression pins against a
//! future change quietly starting to read it — not new behaviour.

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

// ===========================================================================
// Task 3 — reference[] never reaches a triage surface
// ===========================================================================
mod triage_surface_exclusion {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use mev::brain::config::BrainConfig;
    use mev::brain::emit::plan_attention_board;
    use mev::brain::state::{
        CarryoverScope, Reference, StateFile, StateSource, build_state_graph,
        check_carryover_staleness,
    };

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            mev::testsupport::unique_temp_dir(&format!("mev-reference-container-exclusion-{tag}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn day(s: &str) -> chrono::NaiveDate {
        chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    /// A `reference[]` entry created a year (and change) before `today` — old
    /// enough that any of `carryover[]`'s per-kind staleness thresholds
    /// (max 45d) would flag it many times over, if `reference[]` were ever
    /// run through that machinery.
    fn year_old_reference(slug: &str, repo: &str) -> Reference {
        Reference {
            slug: slug.to_string(),
            scope: CarryoverScope {
                repo: Some(repo.to_string()),
                tier: None,
                cross_repo: None,
            },
            class: "invariant".to_string(),
            text: format!("Permanently-true reference text for {slug}."),
            created: "2020-01-01".to_string(),
            related: vec![],
            reviewed: None,
        }
    }

    /// #### AC: "A year-old reference[] fixture entry yields zero staleness diagnostics"
    #[test]
    fn year_old_reference_entry_never_stales() {
        let dir = temp_dir("staleness");
        let path = dir.join("planning/state.json");
        let src = StateSource {
            repo_slug: "hq".to_string(),
            abs_path: path,
            expected_kind: "brain",
        };
        let file = StateFile {
            repo: "hq".to_string(),
            kind: "brain".to_string(),
            updated: "2026-08-15".to_string(),
            reference: vec![year_old_reference("perm-invariant", "hq")],
            ..Default::default()
        };

        let thresholds = mev::brain::config::AttentionThresholds::default();
        let diags = check_carryover_staleness(&src, &file, day("2026-08-15"), &thresholds);
        assert!(
            diags.is_empty(),
            "a reference[] entry has no clock and must never nag, however old: {diags:?}"
        );
    }

    /// #### AC: "The same entry appears in zero Attention-board lanes ... in zero
    /// attention_payload outputs" (board half)
    #[test]
    fn year_old_reference_entry_never_reaches_an_attention_board_lane() {
        let dir = temp_dir("board");
        let planning_dir = dir.join("planning");
        fs::create_dir_all(&planning_dir).unwrap();
        fs::write(
            planning_dir.join("status.md"),
            "# status\n\nbefore\n\n<!-- BEGIN generated:attention -->\n<!-- END generated:attention -->\n\nafter\n",
        )
        .unwrap();

        let reference_slug = "perm-invariant-board";
        let file = StateFile {
            repo: "hq".to_string(),
            kind: "brain".to_string(),
            updated: "2026-08-15".to_string(),
            reference: vec![year_old_reference(reference_slug, "hq")],
            ..Default::default()
        };
        let files = vec![(
            StateSource {
                repo_slug: "hq".to_string(),
                abs_path: planning_dir.join("state.json"),
                expected_kind: "brain",
            },
            file,
        )];
        let config = BrainConfig::default();
        let graph = build_state_graph(&files);
        let plan = plan_attention_board(&files, &graph, &config, day("2026-08-15"));

        assert!(
            !plan.actions.is_empty(),
            "expected at least one board write action; actions were empty"
        );
        for action in &plan.actions {
            assert!(
                !action.new_content.contains(reference_slug),
                "reference[] slug '{reference_slug}' leaked into a board write at {}: {}",
                action.path.display(),
                action.new_content
            );
        }
    }

    fn write_file(root: &Path, rel: &str, content: &str) {
        let target = root.join(rel);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&target, content.as_bytes()).unwrap();
    }

    fn write_json(root: &Path, rel: &str, value: &serde_json::Value) {
        write_file(root, rel, &serde_json::to_string_pretty(value).unwrap());
    }

    fn write_brain_toml(root: &Path) {
        let toml = r#"[vocab]
layer = ["brain", "engine", "factory", "console", "surface", "infra", "business", "content", "meta"]
status = ["active", "draft", "deprecated", "superseded", "archived"]

[crawl]
skip_dirs = ["target", "node_modules", ".git"]
"#;
        fs::write(root.join("brain.toml"), toml.as_bytes()).unwrap();
    }

    fn run_mev(args: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_mev"))
            .args(args)
            .output()
            .expect("failed to spawn mev binary")
    }

    /// #### AC: "The same entry appears in zero ... attention_payload outputs"
    /// (queue half) — `mev attention-queue` is the CLI surface
    /// `attention_payload` actually renders through end to end.
    #[test]
    fn year_old_reference_entry_never_reaches_attention_queue() {
        let dir = temp_dir("queue");
        write_brain_toml(&dir);

        let reference_slug = "perm-invariant-queue";
        let state = serde_json::json!({
            "repo": "hq",
            "kind": "brain",
            "updated": "2026-08-15",
            "focus": { "now": [], "next": [], "blocked": [] },
            "repos": [],
            "cross_repo": [],
            "reference": [
                {
                    "slug": reference_slug,
                    "scope": { "repo": "hq" },
                    "class": "invariant",
                    "text": "Permanently-true reference text.",
                    "created": "2020-01-01"
                }
            ]
        });
        write_json(&dir, "planning/state.json", &state);

        let out = run_mev(&["attention-queue", dir.to_str().unwrap()]);
        assert!(
            out.status.success(),
            "attention-queue should exit 0; stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );

        let stdout = String::from_utf8_lossy(&out.stdout);
        let payloads: Vec<serde_json::Value> = serde_json::from_str(&stdout)
            .unwrap_or_else(|e| panic!("stdout is not a JSON array: {e}\nstdout: {stdout}"));
        assert!(
            payloads.is_empty(),
            "a corpus with only a reference[] entry (no carryover/backlog/distilled) \
             must produce zero attention-queue payloads, got: {payloads:#?}"
        );
        assert!(
            !stdout.contains(reference_slug),
            "reference[] slug '{reference_slug}' must never appear in attention-queue output: {stdout}"
        );
    }
}
