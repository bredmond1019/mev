//! Integration tests for `mev emit-state --scope <repo>` (ticket
//! `ticket-emit-state-scope-and-lock`, task 3).
//!
//! Builds a fixture brain with an HQ root, two tier sub-brains ("core" and
//! "business"), and three leaf project repos — two under "core" ("mev",
//! "bella") and one under "business" ("bastiel") — each wired to a
//! sentinel-bearing `status.md`/cache doc exactly like the existing
//! `mv4e_ripple` full-corpus fixture in `tests/brain_emit.rs`, so a scoped
//! `emit_state` call has real derived surfaces to write into.
//!
//! Every leaf repo starts *stale* (its cached `state.json` focus and its
//! `docs/projects/<slug>.md` cache doc disagree with its own `tracks[]`), so
//! a run that visits a repo is forced to produce an observable diff and a
//! run that does not visit a repo leaves it exactly as stale as it started
//! — the core assertion this ticket is about.

use std::fs;
use std::path::{Path, PathBuf};

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("mev-emit-state-scope-{tag}"));
    let _ = fs::remove_dir_all(&d);
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

fn read(root: &Path, rel: &str) -> String {
    fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("failed to read {rel}: {e}"))
}

/// `brain.toml`: HQ root ("brain") + two tier-container self-entries ("core",
/// "business") + three leaf repos ("mev", "bella" under core; "bastiel"
/// under business). Mirrors the shape `BrainConfig::scope_dependencies`'s own
/// unit tests (`src/brain/config.rs`) and `filter_plan_by_scope`'s
/// (`tests/brain_emit.rs::task2_scope_filter`) use, plus a second core-tier
/// leaf ("bella") so the byte-identical assertion covers a *sibling in the
/// same tier* as the scoped repo, not just an unrelated tier.
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

[[repos]]
slug = "bella"
tier = "core"
repo_path = "core/bella"
status_file = "core/bella/planning/status.md"
cache_doc = "docs/projects/bella.md"
heading = "bella"

[[repos]]
slug = "business"
tier = "_root"
repo_path = "business"
status_file = "business/planning/status.md"
cache_doc = "docs/projects/business.md"
heading = "Business Tier"

[[repos]]
slug = "bastiel"
tier = "business"
repo_path = "business/bastiel"
status_file = "business/bastiel/planning/status.md"
cache_doc = "docs/projects/bastiel.md"
heading = "bastiel"
"#;
    fs::write(root.join("brain.toml"), toml.as_bytes()).unwrap();
}

/// HQ `planning/state.json` (kind:"brain", repo "brain" matches no declared
/// tier -> TierScope::All) pointing at both tier sub-brains.
fn write_hq_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "brain",
        "kind": "brain",
        "updated": "2026-07-27",
        "focus": { "now": [], "next": [], "blocked": [] },
        "repos": [],
        "cross_repo": [],
        "tiers": [
            { "tier": "core", "rollup": "core/planning/state.json", "summary": null },
            { "tier": "business", "rollup": "business/planning/state.json", "summary": null }
        ]
    });
    write_json(root, "planning/state.json", &state);
}

/// HQ `status.md` carrying the HQ_BOARD sentinel.
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

/// A tier sub-brain's own `planning/state.json` (kind:"brain").
fn write_tier_state(root: &Path, tier_slug: &str) {
    let state = serde_json::json!({
        "repo": tier_slug,
        "kind": "brain",
        "updated": "2026-07-27",
        "focus": { "now": [], "next": [], "blocked": [] },
        "repos": [],
        "cross_repo": []
    });
    write_json(root, &format!("{tier_slug}/planning/state.json"), &state);
}

/// A tier sub-brain's `status.md` carrying the TIER_ROLLUP sentinel.
fn write_tier_status_md(root: &Path, tier_slug: &str, title: &str) {
    let doc = format!(
        "---\n\
         type: ProjectStatus\n\
         title: {title} status\n\
         description: {title} tier rollup fixture.\n\
         ---\n\n\
         # {title} Status\n\n\
         <!-- BEGIN generated:tier-rollup -->\n\
         <!-- END generated:tier-rollup -->\n"
    );
    write_file(root, &format!("{tier_slug}/planning/status.md"), &doc);
}

/// A leaf project repo's `planning/state.json`, deliberately **stale**: its
/// cached `focus.now` is empty even though `tracks[]` has a live
/// `in_progress` block, forcing `plan_state_json` to produce a real rewrite
/// when (and only when) that repo is actually visited.
fn write_stale_leaf_state(root: &Path, repo_path: &str, repo_slug: &str, block_id: &str) {
    let state = serde_json::json!({
        "repo": repo_slug,
        "kind": "project",
        "updated": "2026-07-27",
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
/// `timestamp` watermark `plan_project_caches` reconciles the cache doc's
/// `synced_from` field against.
fn write_leaf_status_md(root: &Path, repo_path: &str, repo_slug: &str) {
    let doc = format!(
        "---\n\
         type: ProjectStatus\n\
         title: {repo_slug} status\n\
         description: Status fixture for {repo_slug}.\n\
         timestamp: \"2026-07-27T12:00:00Z\"\n\
         ---\n\n\
         # Status\n"
    );
    write_file(root, &format!("{repo_path}/planning/status.md"), &doc);
}

/// A leaf project repo's brain cache doc, carrying the PROJECT_CACHE
/// sentinel — empty, so a visit produces an observable splice.
fn write_project_cache_doc(root: &Path, repo_slug: &str) {
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

/// Every file path in the fixture, relative to `root` — used to snapshot the
/// whole corpus before/after a run for the byte-identical assertion.
fn all_fixture_files() -> Vec<&'static str> {
    vec![
        "planning/state.json",
        "planning/status.md",
        "core/planning/state.json",
        "core/planning/status.md",
        "business/planning/state.json",
        "business/planning/status.md",
        "core/mev/planning/state.json",
        "core/mev/planning/status.md",
        "docs/projects/mev.md",
        "core/bella/planning/state.json",
        "core/bella/planning/status.md",
        "docs/projects/bella.md",
        "business/bastiel/planning/state.json",
        "business/bastiel/planning/status.md",
        "docs/projects/bastiel.md",
    ]
}

/// Build the full stale fixture: HQ + both tier sub-brains + three stale
/// leaf repos (mev, bella under "core"; bastiel under "business").
fn write_fixture(root: &Path) {
    write_brain_toml(root);

    write_hq_state(root);
    write_hq_status_md(root);

    write_tier_state(root, "core");
    write_tier_status_md(root, "core", "Core Tier");

    write_tier_state(root, "business");
    write_tier_status_md(root, "business", "Business Tier");

    write_stale_leaf_state(root, "core/mev", "mev", "MEV.1.A");
    write_leaf_status_md(root, "core/mev", "mev");
    write_project_cache_doc(root, "mev");

    write_stale_leaf_state(root, "core/bella", "bella", "BELLA.1.A");
    write_leaf_status_md(root, "core/bella", "bella");
    write_project_cache_doc(root, "bella");

    write_stale_leaf_state(root, "business/bastiel", "bastiel", "BASTIEL.1.A");
    write_leaf_status_md(root, "business/bastiel", "bastiel");
    write_project_cache_doc(root, "bastiel");
}

fn snapshot(root: &Path) -> Vec<(String, Vec<u8>)> {
    all_fixture_files()
        .into_iter()
        .map(|rel| (rel.to_string(), fs::read(root.join(rel)).unwrap()))
        .collect()
}

fn resolve_mev_scope(root: &Path) -> mev::brain::config::ScopeDependencySet {
    let config = mev::brain::config::load_brain_config(&root.join("brain.toml")).unwrap();
    config.scope_dependencies("mev").expect("mev is registered")
}

// ---------------------------------------------------------------------------
// (a) `--scope mev --write` modifies mev's derived files and the rollups it
//     feeds (own state.json, cache doc, core's tier rollup, HQ board).
// ---------------------------------------------------------------------------

#[test]
fn scoped_write_modifies_scoped_repo_and_the_rollups_it_feeds() {
    let dir = temp_dir("modifies-own-surfaces");
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

    // mev's own leaf state.json is corrected: focus.now now carries MEV.1.A.
    let mev_state: serde_json::Value =
        serde_json::from_str(&read(&dir, "core/mev/planning/state.json")).unwrap();
    let now = mev_state["focus"]["now"].as_array().unwrap();
    assert!(
        now.iter().any(|b| b["id"].as_str() == Some("MEV.1.A")),
        "mev's own state.json must be corrected by a scoped run; got: {now:?}"
    );

    // mev's cache doc got its focus line spliced in.
    let mev_cache = read(&dir, "docs/projects/mev.md");
    assert!(
        mev_cache.contains("MEV.1.A"),
        "mev's cache doc must be updated by a scoped run; got:\n{mev_cache}"
    );

    // core's tier rollup (the rollup mev feeds) reflects mev's corrected row.
    let core_rollup = read(&dir, "core/planning/status.md");
    assert!(
        core_rollup.contains("MEV.1.A"),
        "core's tier rollup must reflect mev's corrected state; got:\n{core_rollup}"
    );

    // The HQ board (every repo's ultimate rollup) reflects mev's block.
    let hq_board = read(&dir, "planning/status.md");
    assert!(
        hq_board.contains("MEV.1.A"),
        "HQ board must reflect mev's corrected state; got:\n{hq_board}"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// (b) every unrelated repo's derived files are BYTE-IDENTICAL before and
//     after a scoped run — the core assertion of the ticket.
// ---------------------------------------------------------------------------

#[test]
fn scoped_write_leaves_unrelated_repos_byte_identical() {
    let dir = temp_dir("byte-identical");
    write_fixture(&dir);

    let before = snapshot(&dir);

    let scope = resolve_mev_scope(&dir);
    mev::emit_state(&dir, true, Some(&scope)).expect("scoped emit should not error");

    let after = snapshot(&dir);
    let before_map: std::collections::HashMap<_, _> = before.into_iter().collect();
    let after_map: std::collections::HashMap<_, _> = after.into_iter().collect();

    // Files a scoped `--scope mev` run is expected to touch.
    let expected_changed = [
        "core/mev/planning/state.json",
        "core/mev/planning/status.md",
        "docs/projects/mev.md",
        "core/planning/status.md",
        "planning/status.md",
    ];

    for rel in all_fixture_files() {
        let changed = before_map[rel] != after_map[rel];
        if expected_changed.contains(&rel) {
            assert!(
                changed,
                "'{rel}' is one of mev's scope surfaces and must have changed, but is byte-identical"
            );
        } else {
            assert!(
                !changed,
                "'{rel}' is NOT in mev's scope and must be byte-identical before/after \
                 a `--scope mev` run, but it changed"
            );
        }
    }

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// (c) after a scoped run, the tier rollup and HQ board carry the scoped
//     repo's new state while preserving every other repo's previously
//     emitted rows — a scoped run must never blank a repo it did not visit.
// ---------------------------------------------------------------------------

#[test]
fn scoped_run_preserves_unvisited_rows_in_rollups_it_writes() {
    let dir = temp_dir("preserve-unvisited-rows");
    write_fixture(&dir);

    // First, settle the whole corpus with an unscoped write so bella's row
    // in the core tier rollup starts out correct (BELLA.1.A present) rather
    // than stale — otherwise a "row not blanked" assertion would be
    // indistinguishable from "row still stale, never computed at all".
    mev::emit_state(&dir, true, None).expect("initial unscoped settle should not error");

    // Re-introduce staleness only in mev's own leaf state.json, leaving
    // bella's already-settled files untouched, then run a scoped write
    // targeting only mev.
    write_stale_leaf_state(&dir, "core/mev", "mev", "MEV.1.A");

    let scope = resolve_mev_scope(&dir);
    mev::emit_state(&dir, true, Some(&scope)).expect("second scoped emit should not error");

    // core's tier rollup — the file the scoped run DID write — must carry
    // both mev's freshly corrected row AND bella's previously-settled row.
    let core_rollup = read(&dir, "core/planning/status.md");
    assert!(
        core_rollup.contains("MEV.1.A"),
        "core tier rollup must carry mev's corrected row after the scoped run; got:\n{core_rollup}"
    );
    assert!(
        core_rollup.contains("BELLA.1.A"),
        "core tier rollup must still carry bella's row — a scoped run must never blank \
         a repo it did not visit; got:\n{core_rollup}"
    );

    // The HQ board likewise still carries bastiel's settled block even
    // though the scoped run never touched the business tier.
    let hq_board = read(&dir, "planning/status.md");
    assert!(
        hq_board.contains("BASTIEL.1.A"),
        "HQ board must still carry bastiel's settled block after a scoped mev run; got:\n{hq_board}"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// (d) an unscoped `--write` over the same fixture produces the result it
//     does today — the default path is untouched by `--scope`'s existence.
// ---------------------------------------------------------------------------

#[test]
fn unscoped_write_still_regenerates_every_repo() {
    let dir = temp_dir("unscoped-default-path");
    write_fixture(&dir);

    let before = snapshot(&dir);

    let report = mev::emit_state(&dir, true, None).expect("unscoped emit should not error");
    let errors: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.severity == mev::Severity::Error)
        .collect();
    assert!(
        errors.is_empty(),
        "unscoped fixture emit should have no errors; got: {errors:#?}"
    );

    let after = snapshot(&dir);
    let before_map: std::collections::HashMap<_, _> = before.into_iter().collect();
    let after_map: std::collections::HashMap<_, _> = after.into_iter().collect();

    // Every leaf repo's own state.json and cache doc must have been
    // corrected — not just mev's, unlike the scoped run above.
    for (leaf_path, leaf_cache) in [
        ("core/mev/planning/state.json", "docs/projects/mev.md"),
        ("core/bella/planning/state.json", "docs/projects/bella.md"),
        (
            "business/bastiel/planning/state.json",
            "docs/projects/bastiel.md",
        ),
    ] {
        assert_ne!(
            before_map[leaf_path], after_map[leaf_path],
            "'{leaf_path}' must be corrected by an unscoped run"
        );
        assert_ne!(
            before_map[leaf_cache], after_map[leaf_cache],
            "'{leaf_cache}' must be corrected by an unscoped run"
        );
    }

    // Both tier rollups and the HQ board must reflect every repo.
    let core_rollup = read(&dir, "core/planning/status.md");
    assert!(core_rollup.contains("MEV.1.A") && core_rollup.contains("BELLA.1.A"));

    let business_rollup = read(&dir, "business/planning/status.md");
    assert!(business_rollup.contains("BASTIEL.1.A"));

    let hq_board = read(&dir, "planning/status.md");
    for id in ["MEV.1.A", "BELLA.1.A", "BASTIEL.1.A"] {
        assert!(
            hq_board.contains(id),
            "HQ board must contain '{id}' after an unscoped run; got:\n{hq_board}"
        );
    }

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// An unknown `--scope` slug is rejected before any write is attempted.
// ---------------------------------------------------------------------------

#[test]
fn unknown_scope_slug_is_rejected_with_valid_slugs() {
    let dir = temp_dir("unknown-scope-slug");
    write_fixture(&dir);

    let config = mev::brain::config::load_brain_config(&dir.join("brain.toml")).unwrap();
    let err = config
        .scope_dependencies("nonexistent")
        .expect_err("nonexistent must not resolve");

    match err {
        mev::brain::config::ScopeError::UnknownSlug { slug, valid_slugs } => {
            assert_eq!(slug, "nonexistent");
            assert_eq!(
                valid_slugs,
                vec!["brain", "core", "mev", "bella", "business", "bastiel"]
            );
        }
        other => panic!("expected ScopeError::UnknownSlug, got {other:?}"),
    }

    let _ = fs::remove_dir_all(&dir);
}
