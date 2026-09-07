//! Integration tests for `MV.20.B` Task 1: `mev::block_has_unmet_operator_gate`,
//! `mev::E_QUIESCE_LEASE_HELD`, `mev::E_BLOCK_OPERATOR_GATED`, and
//! `mev::quiesce_refusal` are all reachable from OUTSIDE the crate. A `src/` unit
//! test could reach a private item too, so this file (an integration test in a
//! separate crate) is the only thing that actually proves the relocation shipped a
//! public surface, not merely a moved-but-still-private one.

use std::fs;
use std::path::{Path, PathBuf};

fn temp_dir(tag: &str) -> PathBuf {
    let d = mev::testsupport::unique_temp_dir(&format!("mev-lib-guard-relocation-{tag}"));
    fs::create_dir_all(&d).unwrap();
    d
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

[[repos]]
slug = "alpha"
tier = "primary"
repo_path = "repos/alpha"
status_file = "repos/alpha/planning/status.md"
cache_doc = "docs/projects/alpha.md"
heading = "Alpha"
"#;
    fs::write(root.join("brain.toml"), toml.as_bytes()).unwrap();
}

fn write_hq_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "hq",
        "kind": "brain",
        "updated": "2026-09-07",
        "focus": { "now": [], "next": [], "blocked": [] },
        "repos": [
            { "repo": "alpha", "now": [], "next": [], "blocked": [] }
        ],
        "cross_repo": [],
        "epics": []
    });
    write_json(root, "planning/state.json", &state);
}

/// One plain block, one operator-gated block.
fn write_alpha_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "alpha",
        "kind": "project",
        "updated": "2026-09-07",
        "focus": { "now": [], "next": [], "blocked": [] },
        "tracks": [
            {
                "title": "Phase 1",
                "blocks": [
                    { "id": "AL.1.A", "title": "Plain block", "status": "open", "wave": 1 },
                    {
                        "id": "AL.1.B",
                        "title": "Gated block",
                        "status": "open",
                        "wave": 1,
                        "depends_on": [
                            {
                                "type": "operator",
                                "slug": "op-slug",
                                "exit": "planning/handoff.md",
                                "start": "/begin-session x"
                            }
                        ]
                    }
                ]
            }
        ]
    });
    write_json(root, "repos/alpha/planning/state.json", &state);
}

// ---------------------------------------------------------------------------
// mev::block_has_unmet_operator_gate — public, reachable, correct on all four
// documented outcomes.
// ---------------------------------------------------------------------------

#[test]
fn block_has_unmet_operator_gate_is_public_and_true_for_a_gated_block() {
    let root = temp_dir("gated");
    write_brain_toml(&root);
    write_hq_state(&root);
    write_alpha_state(&root);

    assert_eq!(
        mev::block_has_unmet_operator_gate(&root, "alpha:AL.1.B"),
        Some(true)
    );
}

#[test]
fn block_has_unmet_operator_gate_is_false_for_a_plain_block() {
    let root = temp_dir("plain");
    write_brain_toml(&root);
    write_hq_state(&root);
    write_alpha_state(&root);

    assert_eq!(
        mev::block_has_unmet_operator_gate(&root, "alpha:AL.1.A"),
        Some(false)
    );
}

#[test]
fn block_has_unmet_operator_gate_is_none_for_an_unknown_block() {
    let root = temp_dir("unknown");
    write_brain_toml(&root);
    write_hq_state(&root);
    write_alpha_state(&root);

    assert_eq!(
        mev::block_has_unmet_operator_gate(&root, "alpha:NOPE.9.Z"),
        None
    );
}

#[test]
fn block_has_unmet_operator_gate_is_none_for_a_malformed_key() {
    let root = temp_dir("badkey");
    write_brain_toml(&root);
    write_hq_state(&root);
    write_alpha_state(&root);

    // No ':' separator — `split_once` fails, so this must be `None`, never a panic.
    assert_eq!(mev::block_has_unmet_operator_gate(&root, "alpha"), None);
}

// ---------------------------------------------------------------------------
// mev::E_QUIESCE_LEASE_HELD / mev::E_BLOCK_OPERATOR_GATED — public constants,
// reachable and carrying their documented values.
// ---------------------------------------------------------------------------

#[test]
fn diagnostic_code_constants_are_public_and_stable() {
    assert_eq!(mev::E_QUIESCE_LEASE_HELD, "E_QUIESCE_LEASE_HELD");
    assert_eq!(mev::E_BLOCK_OPERATOR_GATED, "E_BLOCK_OPERATOR_GATED");
}

// ---------------------------------------------------------------------------
// mev::quiesce_refusal — public, reachable, and it delegates to the SAME decision
// `mev::brain::lease::check_quiesce` makes (no behaviour change from the relocation).
// ---------------------------------------------------------------------------

fn write_exclusive_lease(lock_dir: &Path, lane: &str, agent: &str, repo: &str, scope: &str) {
    let leases_dir = lock_dir.join("leases");
    fs::create_dir_all(&leases_dir).unwrap();
    let lease = serde_json::json!({
        "lane": lane,
        "agent": agent,
        "repo": repo,
        "kind": "exclusive",
        "scope": scope,
        "acquired_at": chrono::Local::now().to_rfc3339(),
    });
    write_json(&leases_dir, &format!("{lane}.json"), &lease);
}

#[test]
fn quiesce_refusal_is_public_and_refuses_under_a_foreign_repo_scoped_lease() {
    let root = temp_dir("quiesce-held");
    write_brain_toml(&root);
    let dir = root.join("repos/alpha");
    fs::create_dir_all(&dir).unwrap();
    let lock_dir = root.join(".fleet-locks");
    write_exclusive_lease(&lock_dir, "other-lane", "other-agent", "alpha", "repo");

    let held = mev::quiesce_refusal(&root, &dir, Some("me"), Some(&lock_dir));
    let held = held.expect("a foreign exclusive repo-scope lease over 'alpha' must refuse");
    assert_eq!(held.lane, "other-lane");
    assert_eq!(held.agent, "other-agent");
    assert_eq!(held.repo, "alpha");
    assert_eq!(held.scope, "repo");
}

#[test]
fn quiesce_refusal_self_exemption_still_applies_through_the_new_entry_point() {
    let root = temp_dir("quiesce-self");
    write_brain_toml(&root);
    let dir = root.join("repos/alpha");
    fs::create_dir_all(&dir).unwrap();
    let lock_dir = root.join(".fleet-locks");
    write_exclusive_lease(&lock_dir, "my-lane", "me", "alpha", "repo");

    // The holder's own agent is never refused by its own lease.
    assert_eq!(
        mev::quiesce_refusal(&root, &dir, Some("me"), Some(&lock_dir)),
        None
    );
}

#[test]
fn quiesce_refusal_is_clear_with_no_lease_store() {
    let root = temp_dir("quiesce-clear");
    write_brain_toml(&root);
    let dir = root.join("repos/alpha");
    fs::create_dir_all(&dir).unwrap();
    let lock_dir = root.join(".fleet-locks-missing");

    assert_eq!(
        mev::quiesce_refusal(&root, &dir, Some("me"), Some(&lock_dir)),
        None
    );
}
