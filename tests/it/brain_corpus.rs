//! Integration tests — multi-root corpus crawl over a fixture tree.
//!
//! Each test builds a temporary HQ-root fixture that mirrors a realistic company-brain
//! layout with multiple registered units (`brain`, `core`, `mev`), then calls
//! [`mev::brain::crawl::crawl_corpus`] and asserts on the returned [`Corpus`].
//!
//! The unit-level helpers (`is_corpus_member`, `scope_for`, etc.) are tested in their
//! respective source files; these tests exercise the end-to-end walk behaviour over a
//! real filesystem tree, including:
//!
//! - Positive cases: files under `planning/`, `docs/`, and root `README.md`/`CLAUDE.md`
//!   for each registered unit.
//! - Negative cases: `sdlc/`, `archive/`, `trees/` subtrees pruned by `skip_dirs`;
//!   `handoff.md` and `_`-prefixed files; `src/` files; stray root `.md` files; files
//!   under an unregistered nested dir.
//! - Scope assertions: each corpus entry carries the stable slug of its owning unit.
//! - Serialization: the `Corpus` round-trips cleanly through `serde_json`.

use std::fs;
use std::path::Path;

use mev::brain::config::{BrainConfig, CrawlConfig, RepoEntry, VocabConfig};
use mev::brain::crawl::{Corpus, crawl_corpus};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create a file and all parent directories, writing optional content.
fn write(root: &Path, rel: &str, content: &[u8]) {
    let full = root.join(rel);
    fs::create_dir_all(full.parent().unwrap()).unwrap();
    fs::write(&full, content).unwrap();
}

/// Create a fresh temp dir (removing any leftovers from a prior run).
fn temp_dir(suffix: &str) -> std::path::PathBuf {
    let dir = mev::testsupport::unique_temp_dir(&format!("mev-brain-corpus-{suffix}"));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Build a three-unit [`BrainConfig`] with `brain` (`.`), `core` (`core`), and
/// `mev` (`core/mev`), plus the standard bloat `skip_dirs` from the spec.
///
/// This is the canonical fixture config for all tests in this module.
fn three_unit_config() -> BrainConfig {
    BrainConfig {
        conformance_writers: Vec::new(),
        permission_profiles: Default::default(),
        attention: Default::default(),
        history: Default::default(),
        carryover: Default::default(),
        vocab: VocabConfig::default(),
        crawl: CrawlConfig {
            skip_dirs: vec![
                "target".to_string(),
                "node_modules".to_string(),
                ".git".to_string(),
                ".claude".to_string(),
                ".repo-backups".to_string(),
                ".agent".to_string(),
                ".agents".to_string(),
                "archive".to_string(),
                "archived".to_string(),
                "trees".to_string(),
                "sdlc".to_string(),
                "venv".to_string(),
                ".venv".to_string(),
            ],
        },
        repos: vec![
            RepoEntry {
                slug: "brain".to_string(),
                tier: "primary".to_string(),
                repo_path: ".".to_string(),
                status_file: String::new(),
                cache_doc: String::new(),
                heading: String::new(),
                prefix: None,
            },
            RepoEntry {
                slug: "core".to_string(),
                tier: "tier".to_string(),
                repo_path: "core".to_string(),
                status_file: String::new(),
                cache_doc: String::new(),
                heading: String::new(),
                prefix: None,
            },
            RepoEntry {
                slug: "mev".to_string(),
                tier: "primary".to_string(),
                repo_path: "core/mev".to_string(),
                status_file: String::new(),
                cache_doc: String::new(),
                heading: String::new(),
                prefix: None,
            },
        ],
    }
}

/// Collect all relative-path strings from a corpus for easy assertion.
fn rels(corpus: &Corpus) -> Vec<String> {
    corpus
        .entries
        .iter()
        .map(|e| e.rel.display().to_string())
        .collect()
}

/// Find the scope for a given relative path in the corpus, or panic.
fn scope_of<'a>(corpus: &'a Corpus, rel_path: &str) -> &'a str {
    corpus
        .entries
        .iter()
        .find(|e| e.rel.display().to_string() == rel_path)
        .unwrap_or_else(|| panic!("entry not found for rel path: {rel_path}"))
        .scope
        .as_str()
}

// ---------------------------------------------------------------------------
// Core inclusion tests
// ---------------------------------------------------------------------------

/// Positive cases: planning/, docs/, and root README/CLAUDE across all three units
/// should all appear in the corpus with the correct owning scope.
#[test]
fn includes_all_unit_planning_docs_and_root_files() {
    let root = temp_dir("include-all");
    let cfg = three_unit_config();

    // brain unit (HQ root)
    write(&root, "README.md", b"");
    write(&root, "CLAUDE.md", b"");
    write(&root, "planning/status.md", b"");
    write(&root, "docs/index.md", b"");

    // core unit
    write(&root, "core/README.md", b"");
    write(&root, "core/CLAUDE.md", b"");
    write(&root, "core/planning/context.md", b"");
    write(&root, "core/docs/guide.md", b"");

    // mev unit (longest-prefix wins over core)
    write(&root, "core/mev/README.md", b"");
    write(&root, "core/mev/CLAUDE.md", b"");
    write(&root, "core/mev/planning/master-plan.md", b"");
    write(&root, "core/mev/docs/api.md", b"");

    let (corpus, diags) = crawl_corpus(&root, &cfg);
    assert!(diags.is_empty(), "expected no diagnostics: {diags:?}");

    let paths = rels(&corpus);

    // brain files
    assert!(
        paths.contains(&"README.md".to_string()),
        "brain README.md missing"
    );
    assert!(
        paths.contains(&"CLAUDE.md".to_string()),
        "brain CLAUDE.md missing"
    );
    assert!(
        paths.contains(&"planning/status.md".to_string()),
        "brain planning/status.md missing"
    );
    assert!(
        paths.contains(&"docs/index.md".to_string()),
        "brain docs/index.md missing"
    );

    // core files
    assert!(
        paths.contains(&"core/README.md".to_string()),
        "core README.md missing"
    );
    assert!(
        paths.contains(&"core/CLAUDE.md".to_string()),
        "core CLAUDE.md missing"
    );
    assert!(
        paths.contains(&"core/planning/context.md".to_string()),
        "core planning/context.md missing"
    );
    assert!(
        paths.contains(&"core/docs/guide.md".to_string()),
        "core docs/guide.md missing"
    );

    // mev files
    assert!(
        paths.contains(&"core/mev/README.md".to_string()),
        "mev README.md missing"
    );
    assert!(
        paths.contains(&"core/mev/CLAUDE.md".to_string()),
        "mev CLAUDE.md missing"
    );
    assert!(
        paths.contains(&"core/mev/planning/master-plan.md".to_string()),
        "mev planning/master-plan.md missing"
    );
    assert!(
        paths.contains(&"core/mev/docs/api.md".to_string()),
        "mev docs/api.md missing"
    );

    assert_eq!(
        corpus.entries.len(),
        12,
        "expected exactly 12 entries, got: {paths:?}"
    );

    let _ = fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// Scope-resolution tests
// ---------------------------------------------------------------------------

/// Each corpus entry must carry the stable slug of its owning unit.
#[test]
fn entries_carry_correct_owning_scope() {
    let root = temp_dir("scope-check");
    let cfg = three_unit_config();

    write(&root, "planning/status.md", b"");
    write(&root, "README.md", b"");
    write(&root, "core/planning/context.md", b"");
    write(&root, "core/docs/overview.md", b"");
    write(&root, "core/mev/planning/spec.md", b"");
    write(&root, "core/mev/docs/guide.md", b"");

    let (corpus, diags) = crawl_corpus(&root, &cfg);
    assert!(diags.is_empty(), "expected no diagnostics: {diags:?}");

    assert_eq!(scope_of(&corpus, "planning/status.md"), "brain");
    assert_eq!(scope_of(&corpus, "README.md"), "brain");
    assert_eq!(scope_of(&corpus, "core/planning/context.md"), "core");
    assert_eq!(scope_of(&corpus, "core/docs/overview.md"), "core");
    assert_eq!(scope_of(&corpus, "core/mev/planning/spec.md"), "mev");
    assert_eq!(scope_of(&corpus, "core/mev/docs/guide.md"), "mev");

    let _ = fs::remove_dir_all(&root);
}

/// A file under `core/mev/` resolves to `mev` (longest-prefix), not `core`.
#[test]
fn longest_prefix_wins_mev_over_core() {
    let root = temp_dir("longest-prefix");
    let cfg = three_unit_config();

    write(&root, "core/mev/planning/status.md", b"");
    write(&root, "core/planning/status.md", b"");

    let (corpus, diags) = crawl_corpus(&root, &cfg);
    assert!(diags.is_empty(), "expected no diagnostics: {diags:?}");

    assert_eq!(
        scope_of(&corpus, "core/mev/planning/status.md"),
        "mev",
        "core/mev/ files must resolve to 'mev', not 'core'"
    );
    assert_eq!(
        scope_of(&corpus, "core/planning/status.md"),
        "core",
        "core/ files (outside mev) must resolve to 'core'"
    );

    let _ = fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// Negative (exclusion) tests
// ---------------------------------------------------------------------------

/// Files under `sdlc/`, `archive/`, and `trees/` are pruned by `skip_dirs`.
#[test]
fn excludes_skip_dir_subtrees() {
    let root = temp_dir("skip-dirs");
    let cfg = three_unit_config();

    // Positive control: a corpus member that must survive.
    write(&root, "planning/status.md", b"");

    // Bloat subtrees — must be pruned entirely.
    write(&root, "sdlc/some-workflow/planning/task.md", b"");
    write(&root, "archive/old/planning/notes.md", b"");
    write(&root, "trees/some-worktree/planning/status.md", b"");

    let (corpus, diags) = crawl_corpus(&root, &cfg);
    assert!(diags.is_empty(), "expected no diagnostics: {diags:?}");

    let paths = rels(&corpus);
    assert_eq!(
        paths,
        vec!["planning/status.md"],
        "only the positive control must survive; got: {paths:?}"
    );

    let _ = fs::remove_dir_all(&root);
}

/// `handoff.md` is excluded even when inside a `planning/` subtree.
#[test]
fn excludes_handoff_md() {
    let root = temp_dir("handoff");
    let cfg = three_unit_config();

    write(&root, "planning/handoff.md", b"");
    write(&root, "planning/status.md", b"");

    let (corpus, diags) = crawl_corpus(&root, &cfg);
    assert!(diags.is_empty(), "expected no diagnostics: {diags:?}");

    let paths = rels(&corpus);
    assert!(
        !paths.contains(&"planning/handoff.md".to_string()),
        "handoff.md must be excluded"
    );
    assert!(
        paths.contains(&"planning/status.md".to_string()),
        "status.md must be included"
    );

    let _ = fs::remove_dir_all(&root);
}

/// `_`-prefixed files are excluded even when inside a `planning/` subtree.
#[test]
fn excludes_underscore_prefixed_files() {
    let root = temp_dir("underscore");
    let cfg = three_unit_config();

    write(&root, "planning/_draft.md", b"");
    write(&root, "planning/_working-notes.md", b"");
    write(&root, "planning/status.md", b"");

    let (corpus, diags) = crawl_corpus(&root, &cfg);
    assert!(diags.is_empty(), "expected no diagnostics: {diags:?}");

    let paths = rels(&corpus);
    assert!(
        !paths.iter().any(|p| p.contains("_draft")),
        "_draft.md must be excluded"
    );
    assert!(
        !paths.iter().any(|p| p.contains("_working")),
        "_working-notes.md must be excluded"
    );
    assert!(
        paths.contains(&"planning/status.md".to_string()),
        "status.md must be included"
    );

    let _ = fs::remove_dir_all(&root);
}

/// Files under `src/` of any unit are excluded (not planning/ or docs/).
#[test]
fn excludes_src_files() {
    let root = temp_dir("src-files");
    let cfg = three_unit_config();

    // src files in each unit — must all be excluded.
    write(&root, "src/lib.md", b"");
    write(&root, "core/src/notes.md", b"");
    write(&root, "core/mev/src/notes.md", b"");

    // A corpus member to confirm the crawl is working.
    write(&root, "planning/status.md", b"");

    let (corpus, diags) = crawl_corpus(&root, &cfg);
    assert!(diags.is_empty(), "expected no diagnostics: {diags:?}");

    let paths = rels(&corpus);
    assert!(
        !paths.iter().any(|p| p.contains("/src/")),
        "src/ files must be excluded; got: {paths:?}"
    );
    assert!(
        !paths.iter().any(|p| p.starts_with("src/")),
        "root src/ file must be excluded; got: {paths:?}"
    );
    assert!(
        paths.contains(&"planning/status.md".to_string()),
        "planning/status.md must be included"
    );

    let _ = fs::remove_dir_all(&root);
}

/// Stray root-level `.md` files (not `README.md` or `CLAUDE.md`) are excluded.
#[test]
fn excludes_stray_root_md_files() {
    let root = temp_dir("stray-root");
    let cfg = three_unit_config();

    write(&root, "NOTES.md", b""); // stray — excluded
    write(&root, "random.md", b""); // stray — excluded
    write(&root, "README.md", b""); // corpus root file — included
    write(&root, "CLAUDE.md", b""); // corpus root file — included

    let (corpus, diags) = crawl_corpus(&root, &cfg);
    assert!(diags.is_empty(), "expected no diagnostics: {diags:?}");

    let paths = rels(&corpus);
    assert!(
        !paths.contains(&"NOTES.md".to_string()),
        "NOTES.md must be excluded"
    );
    assert!(
        !paths.contains(&"random.md".to_string()),
        "random.md must be excluded"
    );
    assert!(
        paths.contains(&"README.md".to_string()),
        "README.md must be included"
    );
    assert!(
        paths.contains(&"CLAUDE.md".to_string()),
        "CLAUDE.md must be included"
    );
    assert_eq!(
        corpus.entries.len(),
        2,
        "expected exactly 2 entries; got: {paths:?}"
    );

    let _ = fs::remove_dir_all(&root);
}

/// Files inside a subdirectory that is neither `planning/` nor `docs/` (and not a
/// unit-root `README.md`/`CLAUDE.md`) are excluded — even if they are deeply nested.
#[test]
fn excludes_files_under_unregistered_nested_dirs() {
    let root = temp_dir("unregistered-dir");
    let cfg = three_unit_config();

    // Files in unregistered dirs at the brain unit root.
    write(&root, "assets/image.md", b"");
    write(&root, "tests/fixture.md", b"");
    write(&root, "scripts/helper.md", b"");

    // Corpus member to confirm crawl is functioning.
    write(&root, "planning/status.md", b"");

    let (corpus, diags) = crawl_corpus(&root, &cfg);
    assert!(diags.is_empty(), "expected no diagnostics: {diags:?}");

    let paths = rels(&corpus);
    assert!(
        !paths.iter().any(|p| p.starts_with("assets/")),
        "assets/ must be excluded; got: {paths:?}"
    );
    assert!(
        !paths.iter().any(|p| p.starts_with("tests/")),
        "tests/ must be excluded; got: {paths:?}"
    );
    assert!(
        !paths.iter().any(|p| p.starts_with("scripts/")),
        "scripts/ must be excluded; got: {paths:?}"
    );
    assert!(
        paths.contains(&"planning/status.md".to_string()),
        "planning/status.md must be included"
    );

    let _ = fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// Full fixture test — all positive + all negative cases together
// ---------------------------------------------------------------------------

/// End-to-end fixture: build a full multi-unit HQ tree with both positive corpus members
/// and all spec-listed negative cases, then assert the exact expected set is returned.
#[test]
fn full_fixture_exact_corpus_set() {
    let root = temp_dir("full-fixture");
    let cfg = three_unit_config();

    // --- Positive cases (should be included) ---

    // brain unit
    write(&root, "README.md", b"");
    write(&root, "CLAUDE.md", b"");
    write(&root, "planning/status.md", b"");
    write(&root, "planning/decisions/D1-foo.md", b"");
    write(&root, "docs/index.md", b"");
    write(&root, "docs/projects/mev.md", b"");

    // core unit
    write(&root, "core/README.md", b"");
    write(&root, "core/CLAUDE.md", b"");
    write(&root, "core/planning/context.md", b"");
    write(&root, "core/docs/guide.md", b"");

    // mev unit
    write(&root, "core/mev/README.md", b"");
    write(&root, "core/mev/CLAUDE.md", b"");
    write(&root, "core/mev/planning/master-plan.md", b"");
    write(&root, "core/mev/docs/api.md", b"");

    // --- Negative cases (should be excluded) ---

    // Skip-dir subtrees
    write(&root, "sdlc/some-workflow/planning/task.md", b"");
    write(&root, "archive/old/docs/notes.md", b"");
    write(&root, "trees/some-worktree/planning/status.md", b"");

    // Ephemeral files inside planning/
    write(&root, "planning/handoff.md", b"");
    write(&root, "planning/_draft.md", b"");

    // src/ files in each unit
    write(&root, "core/mev/src/notes.md", b"");
    write(&root, "core/src/internal.md", b"");

    // Stray root .md (not README/CLAUDE)
    write(&root, "NOTES.md", b"");

    // File under unregistered nested dir at brain root
    write(&root, "assets/image.md", b"");

    let (corpus, diags) = crawl_corpus(&root, &cfg);
    assert!(diags.is_empty(), "expected no diagnostics; got: {diags:?}");

    let paths = rels(&corpus);

    // Verify all positive cases are present.
    let expected_present = [
        "README.md",
        "CLAUDE.md",
        "planning/status.md",
        "planning/decisions/D1-foo.md",
        "docs/index.md",
        "docs/projects/mev.md",
        "core/README.md",
        "core/CLAUDE.md",
        "core/planning/context.md",
        "core/docs/guide.md",
        "core/mev/README.md",
        "core/mev/CLAUDE.md",
        "core/mev/planning/master-plan.md",
        "core/mev/docs/api.md",
    ];
    for p in &expected_present {
        assert!(
            paths.contains(&p.to_string()),
            "expected corpus member missing: {p}; got: {paths:?}"
        );
    }

    // Verify all negative cases are absent.
    let expected_absent = [
        "sdlc/some-workflow/planning/task.md",
        "archive/old/docs/notes.md",
        "trees/some-worktree/planning/status.md",
        "planning/handoff.md",
        "planning/_draft.md",
        "core/mev/src/notes.md",
        "core/src/internal.md",
        "NOTES.md",
        "assets/image.md",
    ];
    for p in &expected_absent {
        assert!(
            !paths.contains(&p.to_string()),
            "excluded file unexpectedly present: {p}; got: {paths:?}"
        );
    }

    // Exact count: 14 positive cases.
    assert_eq!(
        corpus.entries.len(),
        14,
        "expected exactly 14 corpus entries; got: {paths:?}"
    );

    let _ = fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// Scope correctness in the full fixture
// ---------------------------------------------------------------------------

/// Confirm scope is correct for every entry in the full fixture.
#[test]
fn full_fixture_scope_correctness() {
    let root = temp_dir("full-scope");
    let cfg = three_unit_config();

    write(&root, "README.md", b"");
    write(&root, "planning/status.md", b"");
    write(&root, "docs/index.md", b"");
    write(&root, "core/README.md", b"");
    write(&root, "core/planning/context.md", b"");
    write(&root, "core/docs/guide.md", b"");
    write(&root, "core/mev/README.md", b"");
    write(&root, "core/mev/planning/spec.md", b"");
    write(&root, "core/mev/docs/api.md", b"");

    let (corpus, diags) = crawl_corpus(&root, &cfg);
    assert!(diags.is_empty(), "expected no diagnostics: {diags:?}");

    // brain-scoped
    for rel in &["README.md", "planning/status.md", "docs/index.md"] {
        assert_eq!(
            scope_of(&corpus, rel),
            "brain",
            "expected scope 'brain' for {rel}"
        );
    }
    // core-scoped
    for rel in &[
        "core/README.md",
        "core/planning/context.md",
        "core/docs/guide.md",
    ] {
        assert_eq!(
            scope_of(&corpus, rel),
            "core",
            "expected scope 'core' for {rel}"
        );
    }
    // mev-scoped
    for rel in &[
        "core/mev/README.md",
        "core/mev/planning/spec.md",
        "core/mev/docs/api.md",
    ] {
        assert_eq!(
            scope_of(&corpus, rel),
            "mev",
            "expected scope 'mev' for {rel}"
        );
    }

    let _ = fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// Serialization round-trip
// ---------------------------------------------------------------------------

/// `Corpus` must serialize cleanly to JSON with correct field names.
#[test]
fn corpus_serializes_to_json() {
    let root = temp_dir("serialize");
    let cfg = three_unit_config();

    write(&root, "planning/status.md", b"");
    write(&root, "core/mev/docs/api.md", b"");

    let (corpus, diags) = crawl_corpus(&root, &cfg);
    assert!(diags.is_empty(), "expected no diagnostics: {diags:?}");

    let json = serde_json::to_string(&corpus).expect("corpus must serialize to JSON");
    assert!(json.contains("\"entries\""), "JSON must have 'entries' key");
    assert!(
        json.contains("\"scope\""),
        "JSON must have 'scope' per entry"
    );
    assert!(json.contains("\"stem\""), "JSON must have 'stem' per entry");
    assert!(
        json.contains("\"brain\""),
        "JSON must contain 'brain' scope"
    );
    assert!(json.contains("\"mev\""), "JSON must contain 'mev' scope");

    let _ = fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// CLAUDE.md inclusion (Task 3: no longer blocklisted)
// ---------------------------------------------------------------------------

/// `CLAUDE.md` at unit roots must be included in the corpus (it was blocklisted before
/// Task 3 removed it from the file blocklist).
#[test]
fn claude_md_is_included_at_all_unit_roots() {
    let root = temp_dir("claude-md");
    let cfg = three_unit_config();

    write(&root, "CLAUDE.md", b"");
    write(&root, "core/CLAUDE.md", b"");
    write(&root, "core/mev/CLAUDE.md", b"");

    let (corpus, diags) = crawl_corpus(&root, &cfg);
    assert!(diags.is_empty(), "expected no diagnostics: {diags:?}");

    let paths = rels(&corpus);
    assert!(
        paths.contains(&"CLAUDE.md".to_string()),
        "brain CLAUDE.md must be in corpus"
    );
    assert!(
        paths.contains(&"core/CLAUDE.md".to_string()),
        "core CLAUDE.md must be in corpus"
    );
    assert!(
        paths.contains(&"core/mev/CLAUDE.md".to_string()),
        "mev CLAUDE.md must be in corpus"
    );

    let _ = fs::remove_dir_all(&root);
}
