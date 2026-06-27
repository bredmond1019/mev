//! `brain.toml` config loading for the Brain OKF validator.
//!
//! Provides [`BrainConfig`], the deserialized form of `brain.toml` (the shared
//! HQ-root config file), along with [`find_brain_config`] (walk-up resolver) and
//! [`load_brain_config`] (parse from a given path).
//!
//! Used by [`crate::brain::BrainValidator`] to supply controlled-vocabulary sets
//! (`[vocab]` layer/status lists), crawl skip-dirs (`[crawl].skip_dirs`), and the
//! set of valid project slugs (derived from `[[repos]]` entries) — replacing all
//! hardcoded `is_valid_*` match arms.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur when resolving or parsing `brain.toml`.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// `brain.toml` was not found by walking up from the start path.
    #[error("brain.toml not found: walked up from {start} and reached filesystem root")]
    NotFound { start: PathBuf },

    /// The file was found but could not be read.
    #[error("could not read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// The file was read but could not be parsed as TOML.
    #[error("could not parse {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
}

// ---------------------------------------------------------------------------
// Config structs
// ---------------------------------------------------------------------------

/// `[vocab]` section of `brain.toml`.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct VocabConfig {
    /// Valid `layer` values (closed set).
    #[serde(default)]
    pub layer: Vec<String>,
    /// Valid `status` values (closed set).
    #[serde(default)]
    pub status: Vec<String>,
}

/// `[crawl]` section of `brain.toml`.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct CrawlConfig {
    /// Directory names to skip during crawl.
    #[serde(default)]
    pub skip_dirs: Vec<String>,
}

/// One `[[repos]]` entry in `brain.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct RepoEntry {
    /// Short identifier used as the project vocabulary slug.
    pub slug: String,
    /// Tier classification (e.g. `primary`, `secondary`).
    #[serde(default)]
    pub tier: String,
    /// Path to the repo relative to HQ root.
    #[serde(default)]
    pub repo_path: String,
    /// Path to the status file within the repo.
    #[serde(default)]
    pub status_file: String,
    /// Path to the brain cache doc for this repo.
    #[serde(default)]
    pub cache_doc: String,
    /// Heading used in the brain README quick-status table.
    #[serde(default)]
    pub heading: String,
}

/// Top-level `brain.toml` config.
///
/// The `[[repos]]` manifest is fully stored (for Block N's `--sync` check) but
/// Block M only uses `repos` to derive the valid `project` vocabulary via
/// [`BrainConfig::projects`].
#[derive(Debug, Clone, Deserialize, Default)]
pub struct BrainConfig {
    /// `[vocab]` section.
    #[serde(default)]
    pub vocab: VocabConfig,
    /// `[crawl]` section.
    #[serde(default)]
    pub crawl: CrawlConfig,
    /// `[[repos]]` entries.
    #[serde(default)]
    pub repos: Vec<RepoEntry>,
}

impl BrainConfig {
    /// Derive the valid `project` vocabulary as the set of `[[repos]]` slugs.
    pub fn projects(&self) -> Vec<&str> {
        self.repos.iter().map(|r| r.slug.as_str()).collect()
    }
}

// ---------------------------------------------------------------------------
// Resolvers
// ---------------------------------------------------------------------------

/// Parse `brain.toml` from an explicit path.
///
/// Returns a typed [`ConfigError`] on read or parse failure.
pub fn load_brain_config(path: &Path) -> Result<BrainConfig, ConfigError> {
    let contents = std::fs::read_to_string(path).map_err(|e| ConfigError::Read {
        path: path.to_path_buf(),
        source: e,
    })?;
    toml::from_str(&contents).map_err(|e| ConfigError::Parse {
        path: path.to_path_buf(),
        source: e,
    })
}

/// Walk up from `start`, looking for `brain.toml`, and return the parsed config.
///
/// Starts at `start` itself, then walks to each parent in turn until a `brain.toml`
/// is found or the filesystem root is reached (in which case [`ConfigError::NotFound`]
/// is returned).
pub fn find_brain_config(start: &Path) -> Result<BrainConfig, ConfigError> {
    let mut current = start.to_path_buf();
    loop {
        let candidate = current.join("brain.toml");
        if candidate.exists() {
            return load_brain_config(&candidate);
        }
        match current.parent() {
            Some(parent) => current = parent.to_path_buf(),
            None => {
                return Err(ConfigError::NotFound {
                    start: start.to_path_buf(),
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Unit tests (inline)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("brain.toml")
    }

    #[test]
    fn load_fixture_parses_vocab_config() {
        let cfg = load_brain_config(&fixture_path()).expect("should parse fixture");
        assert!(
            !cfg.vocab.layer.is_empty(),
            "expected at least one layer value"
        );
        assert!(
            !cfg.vocab.status.is_empty(),
            "expected at least one status value"
        );
    }

    #[test]
    fn load_fixture_parses_crawl_config() {
        let cfg = load_brain_config(&fixture_path()).expect("should parse fixture");
        assert!(
            !cfg.crawl.skip_dirs.is_empty(),
            "expected at least one skip_dir"
        );
    }

    #[test]
    fn load_fixture_parses_two_repos() {
        let cfg = load_brain_config(&fixture_path()).expect("should parse fixture");
        assert_eq!(cfg.repos.len(), 2, "expected exactly two [[repos]] entries");
    }

    #[test]
    fn projects_returns_repo_slugs() {
        let cfg = load_brain_config(&fixture_path()).expect("should parse fixture");
        let slugs = cfg.projects();
        assert!(slugs.contains(&"brain"), "expected slug 'brain'");
        assert!(slugs.contains(&"mev"), "expected slug 'mev'");
    }
}
