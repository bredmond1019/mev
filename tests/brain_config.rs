//! Integration tests for `src/brain/config.rs` — BrainConfig loading and walk-up resolver.

use std::path::PathBuf;

use mev::brain::config::{ConfigError, find_brain_config, load_brain_config};

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
    let tmp = std::env::temp_dir().join("mev-find-brain-config-no-toml-test");
    let _ = std::fs::remove_dir_all(&tmp);
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

#[test]
fn load_brain_config_returns_err_for_missing_file() {
    let result = load_brain_config(&PathBuf::from("/nonexistent/path/brain.toml"));
    assert!(result.is_err(), "should fail for nonexistent file");
    match result {
        Err(ConfigError::Read { .. }) => {}
        other => panic!("expected Read error, got {other:?}"),
    }
}
