//! Integration tests for `W_EMIT_SCOPE_SHARED_SURFACE` (block
//! `MV.ticket.emit-state-write-must-bound-which-files-it-writes`, task 1).
//!
//! `--scope <repo>`'s write set is already bounded by `filter_plan_by_scope`
//! (see `tests/it/emit_state_scope.rs`); this file covers the *reporting* layer
//! on top of that bound: a scoped write that lands on a SHARED surface (the
//! tier rollup or the HQ board — surfaces every other repo's scope also
//! feeds) must say so, because `git status` on that file no longer attributes
//! to this scope alone.
//!
//! Fixture shape mirrors `emit_state_scope.rs`'s (HQ root + one tier
//! sub-brain + one leaf repo under it, each stale so a visit produces a real
//! write), trimmed to what this file's four assertions need.

use std::fs;
use std::path::{Path, PathBuf};

fn temp_dir(tag: &str) -> PathBuf {
    let d = mev::testsupport::unique_temp_dir(&format!("mev-emit-scope-shared-surface-{tag}"));
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

/// `brain.toml`: HQ root ("brain"), one tier-container self-entry ("core"),
/// and one leaf repo ("mev") under it — the minimal shape that gives `mev`'s
/// scope both a tier rollup and an HQ board as SHARED targets, plus its own
/// state.json/status.md/cache doc as OWN targets.
fn write_brain_toml(root: &Path) {
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
slug = "core"
tier = "_root"
repo_path = "core"
status_file = "core/planning/status.md"
cache_doc = "docs/projects/core.md"
heading = "Core Tier"

[[repos]]
slug = "mev"
tier = "core"
repo_path = "core/mev"
status_file = "core/mev/planning/status.md"
cache_doc = "docs/projects/mev.md"
heading = "mev"
"#;
    fs::write(root.join("brain.toml"), toml.as_bytes()).unwrap();
}

fn write_hq_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "brain",
        "kind": "brain",
        "updated": "2026-09-06",
        "focus": { "now": [], "next": [], "blocked": [] },
        "repos": [],
        "cross_repo": [],
        "tiers": [
            { "tier": "core", "rollup": "core/planning/state.json", "summary": null }
        ]
    });
    write_json(root, "planning/state.json", &state);
}

fn write_hq_status_md(root: &Path) {
    let doc = "---\n\
                type: ProjectStatus\n\
                title: HQ status\n\
                description: HQ operating board fixture.\n\
                ---\n\n\
                # HQ Status\n\n\
                <!-- BEGIN generated:hq-board -->\n\
                <!-- END generated:hq-board -->\n";
    write_file(root, "planning/status.md", doc);
}

fn write_tier_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "core",
        "kind": "brain",
        "updated": "2026-09-06",
        "focus": { "now": [], "next": [], "blocked": [] },
        "repos": [],
        "cross_repo": []
    });
    write_json(root, "core/planning/state.json", &state);
}

fn write_tier_status_md(root: &Path) {
    let doc = "---\n\
               type: ProjectStatus\n\
               title: Core Tier status\n\
               description: Core tier rollup fixture.\n\
               ---\n\n\
               # Core Tier Status\n\n\
               <!-- BEGIN generated:tier-rollup -->\n\
               <!-- END generated:tier-rollup -->\n";
    write_file(root, "core/planning/status.md", doc);
}

/// mev's own `planning/state.json`, deliberately stale (empty `focus.now`
/// despite a live `in_progress` block) so a scoped run has a real write to
/// make against mev's own surface, the tier rollup, and the HQ board alike.
fn write_stale_leaf_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "mev",
        "kind": "project",
        "updated": "2026-09-06",
        "focus": { "now": [], "next": [], "blocked": [] },
        "tracks": [
            {
                "title": "Phase 1",
                "blocks": [
                    { "id": "MEV.1.A", "title": "mev block A", "status": "in_progress" }
                ]
            }
        ]
    });
    write_json(root, "core/mev/planning/state.json", &state);
}

fn write_leaf_status_md(root: &Path) {
    let doc = "---\n\
               type: ProjectStatus\n\
               title: mev status\n\
               description: Status fixture for mev.\n\
               timestamp: \"2026-09-06T12:00:00Z\"\n\
               ---\n\n\
               # Status\n";
    write_file(root, "core/mev/planning/status.md", doc);
}

fn write_project_cache_doc(root: &Path) {
    let doc = "---\n\
               type: ProjectStatus\n\
               title: mev cache\n\
               description: Project cache fixture for mev.\n\
               ---\n\n\
               # mev\n\n\
               <!-- BEGIN generated:project-cache -->\n\
               <!-- END generated:project-cache -->\n";
    write_file(root, "docs/projects/mev.md", doc);
}

fn write_fixture(root: &Path) {
    write_brain_toml(root);
    write_hq_state(root);
    write_hq_status_md(root);
    write_tier_state(root);
    write_tier_status_md(root);
    write_stale_leaf_state(root);
    write_leaf_status_md(root);
    write_project_cache_doc(root);
}

fn resolve_mev_scope(root: &Path) -> mev::brain::config::ScopeDependencySet {
    let config = mev::brain::config::load_brain_config(&root.join("brain.toml")).unwrap();
    config.scope_dependencies("mev").expect("mev is registered")
}

fn shared_surface_diags(report: &mev::Report) -> Vec<&mev::Diagnostic> {
    report
        .diagnostics
        .iter()
        .filter(|d| d.locator == "W_EMIT_SCOPE_SHARED_SURFACE")
        .collect()
}

// ---------------------------------------------------------------------------
// 1. A scoped plan targeting a shared surface emits exactly one diagnostic
//    per such action, naming the path — here both mev's tier rollup
//    (core/planning/status.md) and the HQ board (planning/status.md).
// ---------------------------------------------------------------------------

#[test]
fn scoped_write_touching_shared_surfaces_emits_one_diagnostic_each() {
    let dir = temp_dir("shared-surfaces-emit");
    write_fixture(&dir);

    let scope = resolve_mev_scope(&dir);
    let report = mev::emit_state(&dir, true, Some(&scope)).expect("scoped emit should not error");

    let diags = shared_surface_diags(&report);

    let tier_rollup_abs = dir.join("core/planning/status.md");
    let hq_board_abs = dir.join("planning/status.md");

    let named: Vec<&Path> = diags.iter().map(|d| d.file.as_path()).collect();
    assert!(
        named.iter().any(|p| *p == tier_rollup_abs),
        "expected a W_EMIT_SCOPE_SHARED_SURFACE diagnostic naming the tier rollup \
         '{}'; got diagnostics: {diags:#?}",
        tier_rollup_abs.display()
    );
    assert!(
        named.iter().any(|p| *p == hq_board_abs),
        "expected a W_EMIT_SCOPE_SHARED_SURFACE diagnostic naming the HQ board \
         '{}'; got diagnostics: {diags:#?}",
        hq_board_abs.display()
    );

    // At least one diagnostic per shared surface actually written. More than
    // one is expected and correct here: several planners splice independent
    // sentinel regions into the SAME physical file (e.g. the HQ board's
    // hq-board and unified-board sections both live in `planning/status.md`),
    // each producing its own `EmitAction` against that path — and the
    // acceptance criterion is "one diagnostic per planned action", not one
    // per file. Every named diagnostic must still be one of exactly the two
    // shared surfaces; nothing else.
    let tier_hits = named.iter().filter(|p| **p == tier_rollup_abs).count();
    let hq_hits = named.iter().filter(|p| **p == hq_board_abs).count();
    assert!(
        tier_hits >= 1,
        "expected at least one diagnostic for the tier rollup; got {tier_hits}"
    );
    assert!(
        hq_hits >= 1,
        "expected at least one diagnostic for the HQ board; got {hq_hits}"
    );
    assert_eq!(
        tier_hits + hq_hits,
        named.len(),
        "every W_EMIT_SCOPE_SHARED_SURFACE diagnostic must name one of the two shared \
         surfaces and nothing else; got: {diags:#?}"
    );

    // The message names the scope it was invoked under.
    for d in &diags {
        assert!(
            d.message.contains("mev"),
            "diagnostic message should name the scope ('mev'); got: {}",
            d.message
        );
    }

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 2. A scoped plan touching only the repo's OWN surfaces emits none — the
//    diagnostic tracks sharedness, not merely the presence of a scope. We
//    force this by pre-settling the tier rollup and HQ board so mev's scoped
//    run has nothing new to write to them (no EmitAction survives for those
//    paths), while mev's own leaf state.json is re-staled so its own surface
//    still produces a real write.
// ---------------------------------------------------------------------------

#[test]
fn scoped_write_touching_only_own_surfaces_emits_none() {
    let dir = temp_dir("own-surfaces-only");
    write_fixture(&dir);

    // Settle the whole corpus first so the tier rollup and HQ board already
    // reflect mev's block and a second scoped run has no diff to write there.
    mev::emit_state(&dir, true, None).expect("initial unscoped settle should not error");

    // Re-introduce staleness only in mev's own leaf state.json.
    write_stale_leaf_state(&dir);

    let scope = resolve_mev_scope(&dir);
    let report = mev::emit_state(&dir, true, Some(&scope)).expect("scoped emit should not error");

    let diags = shared_surface_diags(&report);
    assert!(
        diags.is_empty(),
        "a scoped run touching only mev's own surfaces must emit no \
         W_EMIT_SCOPE_SHARED_SURFACE diagnostics; got: {diags:#?}"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 3. THE POSITIVE CONTROL: an unscoped plan over the same (freshly re-staled)
//    fixture emits none of these diagnostics, and its action list is exactly
//    what it always was — a full-corpus rewrite. Without this control, a
//    change that emitted the diagnostic unconditionally (ignoring `scope`)
//    would still pass test 1 above.
// ---------------------------------------------------------------------------

#[test]
fn unscoped_write_emits_no_shared_surface_diagnostics() {
    let dir = temp_dir("unscoped-control");
    write_fixture(&dir);

    let report = mev::emit_state(&dir, true, None).expect("unscoped emit should not error");

    let diags = shared_surface_diags(&report);
    assert!(
        diags.is_empty(),
        "an unscoped run must never emit W_EMIT_SCOPE_SHARED_SURFACE; got: {diags:#?}"
    );

    // And the unscoped path still corrects every surface it always has —
    // proof the positive control's fixture is capable of producing the
    // diagnostic's target actions in the first place (they are just unscoped
    // here), not merely a fixture too inert to exercise anything.
    let hq_board = fs::read_to_string(dir.join("planning/status.md")).unwrap();
    assert!(
        hq_board.contains("MEV.1.A"),
        "unscoped run must still update the HQ board; got:\n{hq_board}"
    );
    let tier_rollup = fs::read_to_string(dir.join("core/planning/status.md")).unwrap();
    assert!(
        tier_rollup.contains("MEV.1.A"),
        "unscoped run must still update the tier rollup; got:\n{tier_rollup}"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 4. The scoped action LIST is unchanged by this task: the same four target
//    surfaces (own state.json, own status.md, cache doc, tier rollup, HQ
//    board — five paths since a tier rollup exists for mev) still survive
//    `filter_plan_by_scope`/get written, exactly as before this task. This
//    is the guard that the task reports rather than refuses.
// ---------------------------------------------------------------------------

#[test]
fn scoped_write_still_writes_every_scope_target_unchanged() {
    let dir = temp_dir("action-list-unchanged");
    write_fixture(&dir);

    let scope = resolve_mev_scope(&dir);
    let report = mev::emit_state(&dir, true, Some(&scope)).expect("scoped emit should not error");
    let errors: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.severity == mev::Severity::Error)
        .collect();
    assert!(
        errors.is_empty(),
        "scoped fixture emit should have no errors; got: {errors:#?}"
    );

    // mev's own state.json corrected.
    let mev_state: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(dir.join("core/mev/planning/state.json")).unwrap(),
    )
    .unwrap();
    let now = mev_state["focus"]["now"].as_array().unwrap();
    assert!(
        now.iter().any(|b| b["id"].as_str() == Some("MEV.1.A")),
        "mev's own state.json must still be corrected by a scoped run; got: {now:?}"
    );

    // mev's cache doc updated.
    let mev_cache = fs::read_to_string(dir.join("docs/projects/mev.md")).unwrap();
    assert!(
        mev_cache.contains("MEV.1.A"),
        "mev's cache doc must still be updated by a scoped run; got:\n{mev_cache}"
    );

    // core's tier rollup (shared, but still WRITTEN — this task reports, does not refuse).
    let core_rollup = fs::read_to_string(dir.join("core/planning/status.md")).unwrap();
    assert!(
        core_rollup.contains("MEV.1.A"),
        "core's tier rollup must still be written by a scoped run; got:\n{core_rollup}"
    );

    // HQ board (shared, but still WRITTEN).
    let hq_board = fs::read_to_string(dir.join("planning/status.md")).unwrap();
    assert!(
        hq_board.contains("MEV.1.A"),
        "HQ board must still be written by a scoped run; got:\n{hq_board}"
    );

    // And it still carries this task's diagnostics for both shared surfaces
    // (possibly more than one each — several planners splice independent
    // sentinel regions into the same physical file) and nothing else.
    let diags = shared_surface_diags(&report);
    let tier_rollup_abs = dir.join("core/planning/status.md");
    let hq_board_abs = dir.join("planning/status.md");
    assert!(
        diags.iter().any(|d| d.file == tier_rollup_abs),
        "expected at least one diagnostic naming the tier rollup; got: {diags:#?}"
    );
    assert!(
        diags.iter().any(|d| d.file == hq_board_abs),
        "expected at least one diagnostic naming the HQ board; got: {diags:#?}"
    );
    assert!(
        diags
            .iter()
            .all(|d| d.file == tier_rollup_abs || d.file == hq_board_abs),
        "every W_EMIT_SCOPE_SHARED_SURFACE diagnostic must name one of the two shared \
         surfaces; got: {diags:#?}"
    );

    let _ = fs::remove_dir_all(&dir);
}
