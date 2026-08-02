//! Integration tests for `src/brain/config.rs` — BrainConfig loading and walk-up resolver.

use std::path::PathBuf;

use mev::brain::config::{ConfigError, find_brain_config, find_brain_root, load_brain_config};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("brain.toml")
}

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

// ---------------------------------------------------------------------------
// load_brain_config tests
// ---------------------------------------------------------------------------

#[test]
fn fixture_parses_vocab_layer_list() {
    let cfg = load_brain_config(&fixture_path()).expect("should parse fixture");
    assert!(
        cfg.vocab.layer.contains(&"brain".to_string()),
        "expected 'brain' in layer list, got {:?}",
        cfg.vocab.layer
    );
    assert!(
        cfg.vocab.layer.contains(&"factory".to_string()),
        "expected 'factory' in layer list"
    );
}

#[test]
fn fixture_parses_vocab_status_list() {
    let cfg = load_brain_config(&fixture_path()).expect("should parse fixture");
    assert!(
        cfg.vocab.status.contains(&"active".to_string()),
        "expected 'active' in status list"
    );
    assert!(
        cfg.vocab.status.contains(&"archived".to_string()),
        "expected 'archived' in status list"
    );
}

#[test]
fn fixture_parses_crawl_skip_dirs() {
    let cfg = load_brain_config(&fixture_path()).expect("should parse fixture");
    assert!(
        cfg.crawl.skip_dirs.contains(&"target".to_string()),
        "expected 'target' in skip_dirs"
    );
    assert!(
        cfg.crawl.skip_dirs.contains(&".git".to_string()),
        "expected '.git' in skip_dirs"
    );
}

#[test]
fn fixture_parses_two_repo_entries() {
    let cfg = load_brain_config(&fixture_path()).expect("should parse fixture");
    assert_eq!(
        cfg.repos.len(),
        2,
        "expected exactly two [[repos]] entries, got {}",
        cfg.repos.len()
    );
    let slugs: Vec<&str> = cfg.repos.iter().map(|r| r.slug.as_str()).collect();
    assert!(slugs.contains(&"brain"), "expected slug 'brain'");
    assert!(slugs.contains(&"mev"), "expected slug 'mev'");
}

#[test]
fn repo_entry_fields_are_populated() {
    let cfg = load_brain_config(&fixture_path()).expect("should parse fixture");
    let brain_entry = cfg.repos.iter().find(|r| r.slug == "brain").unwrap();
    assert_eq!(brain_entry.tier, "primary");
    assert!(!brain_entry.repo_path.is_empty());
    assert!(!brain_entry.cache_doc.is_empty());
}

#[test]
fn repo_entry_parses_declared_prefix() {
    let cfg = load_brain_config(&fixture_path()).expect("should parse fixture");
    let mev_entry = cfg.repos.iter().find(|r| r.slug == "mev").unwrap();
    assert_eq!(
        mev_entry.prefix.as_deref(),
        Some("MV"),
        "a declared prefix must parse to Some(..)"
    );
}

#[test]
fn repo_entry_omitted_prefix_is_none_not_an_error() {
    // An entry with no `prefix` key must load cleanly and yield `None` — a missing
    // prefix degrades to "no prefix-stripped candidate", it is never a diagnostic.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("brain.toml");
    std::fs::write(
        &path,
        r#"[[repos]]
slug = "noprefix"
tier = "core"
repo_path = "core/noprefix"
"#,
    )
    .unwrap();

    let cfg = load_brain_config(&path).expect("a brain.toml with no prefix must still parse");
    assert_eq!(cfg.repos.len(), 1);
    assert!(
        cfg.repos[0].prefix.is_none(),
        "an omitted prefix must be None"
    );
}

// ---------------------------------------------------------------------------
// BrainConfig::projects() tests
// ---------------------------------------------------------------------------

#[test]
fn projects_returns_slugs_from_repos() {
    let cfg = load_brain_config(&fixture_path()).expect("should parse fixture");
    let projects = cfg.projects();
    assert!(
        projects.contains(&"brain"),
        "expected 'brain' in projects()"
    );
    assert!(projects.contains(&"mev"), "expected 'mev' in projects()");
}

#[test]
fn projects_length_matches_repos_count() {
    let cfg = load_brain_config(&fixture_path()).expect("should parse fixture");
    assert_eq!(
        cfg.projects().len(),
        cfg.repos.len(),
        "projects() length should match repos count"
    );
}

// ---------------------------------------------------------------------------
// find_brain_config tests
// ---------------------------------------------------------------------------

#[test]
fn find_brain_config_resolves_from_fixture_dir() {
    // The fixture dir contains brain.toml directly — find_brain_config should find it at level 0.
    let cfg = find_brain_config(&fixture_dir()).expect("should find brain.toml in fixture dir");
    assert!(
        cfg.vocab.layer.contains(&"brain".to_string()),
        "resolved config should have 'brain' layer"
    );
}

#[test]
fn find_brain_config_resolves_by_walking_up_from_subdirectory() {
    // Create a subdirectory inside the fixture dir and resolve from there.
    let subdir = fixture_dir().join("subdir-for-walk-up-test");
    let _ = std::fs::remove_dir_all(&subdir);
    std::fs::create_dir_all(&subdir).expect("could not create test subdirectory");

    let cfg =
        find_brain_config(&subdir).expect("should find brain.toml by walking up from subdirectory");
    assert!(
        cfg.vocab.layer.contains(&"brain".to_string()),
        "walked-up config should contain 'brain' layer"
    );

    let _ = std::fs::remove_dir_all(&subdir);
}

#[test]
fn find_brain_config_returns_err_when_no_brain_toml() {
    // Create a temporary directory with no brain.toml in any ancestor up to the FS root.
    // We use a path that is guaranteed not to have a brain.toml ancestor: /tmp itself.
    // However, to be robust, we check that the error variant is NotFound.
    let tmp = mev::testsupport::unique_temp_dir("mev-find-brain-config-no-toml-test");
    std::fs::create_dir_all(&tmp).expect("could not create temp dir");

    // Only treat as the expected error if tmp's ancestors don't contain brain.toml.
    // In a typical CI environment /tmp does not have brain.toml.
    let result = find_brain_config(&tmp);

    // If the environment happens to have a brain.toml somewhere above /tmp (unlikely but
    // possible on developer machines), skip the assertion rather than fail.
    match result {
        Err(ConfigError::NotFound { .. }) => {
            // Expected: no brain.toml found up the chain.
        }
        Ok(_) => {
            // brain.toml found in an ancestor of /tmp — skip assertion (dev machine).
            eprintln!(
                "SKIP: find_brain_config found a brain.toml above /tmp — skipping NotFound assertion"
            );
        }
        Err(e) => {
            panic!("expected NotFound, got unexpected error: {e}");
        }
    }

    let _ = std::fs::remove_dir_all(&tmp);
}

// ---------------------------------------------------------------------------
// find_brain_root tests
// ---------------------------------------------------------------------------

#[test]
fn find_brain_root_returns_fixture_dir_when_brain_toml_is_there() {
    // Passing the fixture dir (which contains brain.toml) should return that dir, canonicalized.
    let root = find_brain_root(&fixture_dir()).expect("should find brain.toml in fixture dir");
    assert!(
        root.join("brain.toml").exists(),
        "returned root should contain brain.toml; got {root:?}"
    );
    // The returned path should canonicalize to the same location as fixture_dir().
    assert_eq!(
        root.canonicalize().unwrap(),
        fixture_dir().canonicalize().unwrap(),
        "find_brain_root should return the directory containing brain.toml"
    );
}

#[test]
fn find_brain_root_walks_up_from_subdirectory() {
    // Create a nested subdirectory inside the fixture dir. find_brain_root should walk up
    // and return the fixture dir (where brain.toml lives), not the subdir.
    let subdir = fixture_dir().join("root-walk-up-subdir");
    let _ = std::fs::remove_dir_all(&subdir);
    std::fs::create_dir_all(&subdir).expect("could not create test subdirectory");

    let root = find_brain_root(&subdir).expect("should find brain.toml by walking up");
    assert!(
        root.join("brain.toml").exists(),
        "walked-up root should contain brain.toml; got {root:?}"
    );
    assert_eq!(
        root.canonicalize().unwrap(),
        fixture_dir().canonicalize().unwrap(),
        "walk-up should stop at the directory containing brain.toml"
    );

    let _ = std::fs::remove_dir_all(&subdir);
}

#[test]
fn find_brain_root_canonicalizes_relative_input() {
    // Passing a relative path that resolves to the fixture dir should still find brain.toml.
    // We change into the fixture dir and pass "." to exercise the canonicalize branch.
    let original = std::env::current_dir().unwrap();
    std::env::set_current_dir(fixture_dir()).expect("could not cd to fixture dir");

    let root = find_brain_root(std::path::Path::new("."))
        .expect("find_brain_root('.') should find brain.toml after canonicalize");
    assert!(
        root.join("brain.toml").exists(),
        "canonicalized root should contain brain.toml; got {root:?}"
    );

    std::env::set_current_dir(original).expect("could not restore cwd");
}

#[test]
fn find_brain_root_returns_err_when_no_brain_toml() {
    let tmp = mev::testsupport::unique_temp_dir("mev-find-brain-root-no-toml");
    std::fs::create_dir_all(&tmp).expect("could not create temp dir");

    let result = find_brain_root(&tmp);
    match result {
        Err(ConfigError::NotFound { .. }) => {
            // Expected on most machines.
        }
        Ok(root) => {
            // A brain.toml exists somewhere above /tmp on this machine — skip.
            eprintln!(
                "SKIP: find_brain_root found brain.toml above temp dir at {root:?} — skipping NotFound assertion"
            );
        }
        Err(e) => panic!("expected NotFound, got unexpected error: {e}"),
    }

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn find_brain_config_delegates_to_find_brain_root_correctly() {
    // Confirm that find_brain_config (which now delegates to find_brain_root) still
    // returns a correctly parsed config when called from the fixture dir.
    let cfg =
        find_brain_config(&fixture_dir()).expect("should resolve and parse via find_brain_root");
    assert!(
        cfg.vocab.layer.contains(&"brain".to_string()),
        "config via find_brain_root should have 'brain' layer"
    );
    assert!(
        !cfg.repos.is_empty(),
        "config via find_brain_root should have at least one repo entry"
    );
}

#[test]
fn load_brain_config_returns_err_for_missing_file() {
    let result = load_brain_config(&PathBuf::from("/nonexistent/path/brain.toml"));
    assert!(result.is_err(), "should fail for nonexistent file");
    match result {
        Err(ConfigError::Read { .. }) => {}
        other => panic!("expected Read error, got {other:?}"),
    }
}
