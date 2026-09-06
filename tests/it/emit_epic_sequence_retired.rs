//! `MV.chore.retire-the-epic-sequence-planner` Task 2 — behavioural pin.
//!
//! An epic is now a standing area, not an initiative (2026-09-06 consolidation), so
//! it has no sequence to render. The `plan_epic_sequences` planner and its
//! `<!-- BEGIN generated:epic-sequence -->` sentinel handling were removed from
//! `mev` in Task 1. This test proves the removal at the behavioural level, through
//! the same top-level entry point `src/lib.rs` drives (`mev::emit_state`), not by
//! calling a planner in isolation — the point being that **no planner in the
//! registered set** claims a doc carrying the old sentinel pair any more.
//!
//! A dry-run emit reports one `W_EMIT_DRY_RUN` diagnostic per planned write,
//! carrying the target path (`apply_plan` in `src/brain/emit.rs`). That is the
//! observable this test hooks: if any planner still claimed the epic's `plan`
//! doc, a `W_EMIT_DRY_RUN` diagnostic would name that doc's path.
//!
//! A positive control accompanies the three absence assertions: the same fixture
//! corpus must still produce at least one planned action for a surface the emit
//! path *does* own (here, the leaf repo's stale `focus.now` regeneration), so an
//! empty result cannot pass because the fixture itself is inert.

use std::fs;
use std::path::Path;

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let d = mev::testsupport::unique_temp_dir(&format!("mev-emit-epic-sequence-retired-{tag}"));
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

/// Minimal `brain.toml` registering one leaf repo, `alpha`.
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

/// A fixture epic `plan` doc carrying the old `epic-sequence` sentinel pair,
/// written literally rather than via `markers::EPIC_SEQUENCE` — that constant
/// no longer exists after Task 1's removal, and the point of this fixture is to
/// simulate a leftover doc from before the retirement, not to import the
/// removed marker.
fn epic_plan_doc_with_sentinel() -> String {
    "---\n\
     type: ProjectStatus\n\
     title: T\n\
     description: D\n\
     ---\n\n\
     # Status\n\n\
     Before.\n\n\
     <!-- BEGIN generated:epic-sequence -->\n\
     <!-- END generated:epic-sequence -->\n\n\
     After.\n"
        .to_string()
}

/// Build the fixture corpus:
/// - `planning/state.json` (HQ, kind `brain`) carrying one `epics[]` entry
///   whose `plan` points at a doc with the leftover sentinel pair.
/// - `repos/alpha/planning/state.json` (kind `project`) with a stale
///   `focus.now` entry — the emit path's own positive-control surface: this is
///   exactly the fixture shape `dry_run_leaves_files_unchanged` in
///   `tests/it/brain_emit.rs` already proves produces a `W_EMIT_DRY_RUN`.
fn write_fixture(root: &Path, plan_rel: &str) {
    write_brain_toml(root);
    write_file(root, plan_rel, &epic_plan_doc_with_sentinel());

    let hq_state = serde_json::json!({
        "repo": "hq",
        "kind": "brain",
        "updated": "2026-07-24",
        "focus": { "now": [], "next": [], "blocked": [] },
        "epics": [
            { "slug": "bastion-os", "title": "Bastion OS", "status": "active", "plan": plan_rel }
        ],
        "repos": [
            { "repo": "alpha", "now": [], "next": [], "blocked": [] }
        ],
        "cross_repo": []
    });
    write_json(root, "planning/state.json", &hq_state);

    // Stale: focus.now claims AL.1.A is in_progress; the track below has it
    // `open` — this mismatch is what makes `plan_state_json` queue a rewrite of
    // this very file, giving the positive control something real to find.
    let alpha_state = serde_json::json!({
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
                    { "id": "AL.1.A", "title": "Alpha block A", "status": "open" }
                ]
            }
        ]
    });
    write_json(root, "repos/alpha/planning/state.json", &alpha_state);
}

#[test]
fn no_planner_claims_a_leftover_epic_sequence_sentinel() {
    let dir = temp_dir("basic");
    let plan_rel = "core/planning/bastion-os.md";
    write_fixture(&dir, plan_rel);

    let plan_doc_path = dir.join(plan_rel);
    let alpha_state_path = dir.join("repos/alpha/planning/state.json");
    let before = fs::read(&plan_doc_path).unwrap();

    let report = mev::emit_state(&dir, false, None).expect("emit_state should not error");

    // Absence assertion 1: no planned action (surfaced as a W_EMIT_DRY_RUN
    // diagnostic naming the target path) targets the epic plan doc.
    let dry_run_targets_plan_doc = report.diagnostics.iter().any(|d| {
        d.locator == "W_EMIT_DRY_RUN"
            && d.file
                .canonicalize()
                .map(|p| p == plan_doc_path.canonicalize().unwrap())
                .unwrap_or(false)
    });
    assert!(
        !dry_run_targets_plan_doc,
        "no planner should claim the epic plan doc any more; diagnostics: {:#?}",
        report.diagnostics
    );

    // Absence assertion 2: nothing in the whole diagnostics set mentions
    // epic-sequence — not the retired planner, not its retired sentinel.
    let mentions_epic_sequence = report
        .diagnostics
        .iter()
        .any(|d| d.locator.contains("epic-sequence") || d.message.contains("epic-sequence"));
    assert!(
        !mentions_epic_sequence,
        "no diagnostic should mention epic-sequence; diagnostics: {:#?}",
        report.diagnostics
    );

    // Absence assertion 3: the sentinel pair is still byte-identical on disk —
    // nothing rewrote it. (Dry-run never writes regardless; this pins the
    // *content*, not merely the write mode, as the thing under test.)
    let after = fs::read(&plan_doc_path).unwrap();
    assert_eq!(
        before, after,
        "the leftover sentinel doc must be left byte-identical"
    );
    let after_text = String::from_utf8(after).unwrap();
    assert!(after_text.contains("<!-- BEGIN generated:epic-sequence -->"));
    assert!(after_text.contains("<!-- END generated:epic-sequence -->"));

    // Positive control: the SAME fixture corpus must still produce at least one
    // planned action for a surface the emit path does own — alpha's stale
    // `focus.now`. If this finds nothing, the fixture is inert and the three
    // absence assertions above would have passed vacuously.
    let dry_run_targets_alpha_state = report.diagnostics.iter().any(|d| {
        d.locator == "W_EMIT_DRY_RUN"
            && d.file
                .canonicalize()
                .map(|p| p == alpha_state_path.canonicalize().unwrap())
                .unwrap_or(false)
    });
    assert!(
        dry_run_targets_alpha_state,
        "positive control failed: expected a planned rewrite of alpha's stale \
         state.json (focus.now regeneration) — got diagnostics: {:#?}",
        report.diagnostics
    );

    let _ = fs::remove_dir_all(&dir);
}
