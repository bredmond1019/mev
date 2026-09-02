//! Discover path-dependent consumers of mev from the corpus.
//!
//! `ticket-consumer-compile-gate` and `ticket-consumer-dependency-parity` both need the same
//! answer to "which repos depend on mev via a path dependency" — the compile gate to know
//! which test targets to build, the dependency-parity check to know whose `Cargo.lock` to
//! read. This module is the one implementation both reuse; a second discovery path would let
//! the two checks silently disagree about what a consumer is.
//!
//! A consumer is any `[[repos]]` entry in `brain.toml` whose `Cargo.toml` declares a path
//! dependency that *resolves* to mev's own crate directory — matched by canonicalized path,
//! not by the literal string `"../mev"`, so a differently-nested consumer (or a workspace
//! declaring the dependency under `[workspace.dependencies]` rather than `[dependencies]`,
//! as `engine-rs` does) is still found. A registered repo with no `Cargo.toml` is simply not
//! a consumer and is skipped without error.

use std::path::{Path, PathBuf};

use super::super::config::BrainConfig;

/// mev's own crate directory, stamped at compile time — the target every path dependency
/// below is checked against. Same pattern as `toolchain.rs`'s `STAMPED_SOURCE_DIR`.
const MEV_CRATE_DIR: &str = env!("CARGO_MANIFEST_DIR");

/// The `Cargo.toml` tables that can carry a path dependency on mev. `workspace.dependencies`
/// covers `engine-rs`-style workspaces, where member crates pull `mev = { workspace = true }`
/// and the actual `path = "../mev"` lives only at the workspace root.
const DEPENDENCY_TABLE_KEYS: &[&[&str]] = &[
    &["dependencies"],
    &["dev-dependencies"],
    &["build-dependencies"],
    &["workspace", "dependencies"],
];

/// One discovered path-dependent consumer of mev.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsumerRepo {
    /// The `[[repos]]` slug (e.g. `"bastion"`, `"engine-rs"`).
    pub slug: String,
    /// Absolute path to the consumer's repo root.
    pub repo_path: PathBuf,
    /// Absolute path to the consumer's top-level `Cargo.toml`.
    pub cargo_toml: PathBuf,
    /// Absolute path to the consumer's `Cargo.lock` (may not exist on disk; callers that need
    /// to know check for themselves rather than this module guessing on their behalf).
    pub cargo_lock: PathBuf,
}

/// Discover every `[[repos]]` entry in `config` whose `Cargo.toml` (resolved relative to
/// `root`) declares a path dependency resolving to mev's own crate directory.
///
/// Never hard-codes a consumer name: this scans the full `[[repos]]` registry so a repo that
/// adds the dependency edge tomorrow is discovered with no code change here.
pub(crate) fn discover_mev_consumers(root: &Path, config: &BrainConfig) -> Vec<ConsumerRepo> {
    let mev_dir = canonicalize_lossy(Path::new(MEV_CRATE_DIR));

    let mut consumers = Vec::new();
    for repo in &config.repos {
        if repo.repo_path.is_empty() {
            continue;
        }
        let repo_dir = root.join(&repo.repo_path);
        let cargo_toml = repo_dir.join("Cargo.toml");
        if !cargo_toml.is_file() {
            // Not a Rust repo (or none checked out here) -> simply not a consumer.
            continue;
        }
        if canonicalize_lossy(&repo_dir) == mev_dir {
            // mev is not its own consumer.
            continue;
        }

        let Ok(text) = std::fs::read_to_string(&cargo_toml) else {
            continue;
        };
        let Ok(doc) = text.parse::<toml::Value>() else {
            continue;
        };

        if depends_on_mev(&doc, &repo_dir, &mev_dir) {
            consumers.push(ConsumerRepo {
                slug: repo.slug.clone(),
                repo_path: repo_dir.clone(),
                cargo_toml,
                cargo_lock: repo_dir.join("Cargo.lock"),
            });
        }
    }
    consumers
}

/// True if any of `doc`'s dependency tables (see [`DEPENDENCY_TABLE_KEYS`]) has an entry named
/// `mev` — or any entry at all whose `path` value resolves to `mev_dir`, so a consumer that
/// renames the dependency (`mev-cli = { path = "../mev" }`) is still found by resolution, not
/// by name.
fn depends_on_mev(doc: &toml::Value, repo_dir: &Path, mev_dir: &Path) -> bool {
    for table_path in DEPENDENCY_TABLE_KEYS {
        let Some(table) = walk_table(doc, table_path) else {
            continue;
        };
        for entry in table.values() {
            let Some(path_str) = entry.get("path").and_then(toml::Value::as_str) else {
                continue;
            };
            let resolved = canonicalize_lossy(&repo_dir.join(path_str));
            if resolved == *mev_dir {
                return true;
            }
        }
    }
    false
}

/// Walk a dotted key path (e.g. `["workspace", "dependencies"]`) down a parsed TOML document,
/// returning the table found there, if any and if it is in fact a table.
fn walk_table<'a>(
    doc: &'a toml::Value,
    path: &[&str],
) -> Option<&'a toml::map::Map<String, toml::Value>> {
    let mut current = doc;
    for key in path {
        current = current.get(key)?;
    }
    current.as_table()
}

/// Canonicalize `path`, falling back to the path as-given when it does not exist (yet, or in a
/// test fixture) or canonicalization otherwise fails — comparison still works via string
/// equality on the un-canonicalized form in that case, it just loses symlink resolution.
fn canonicalize_lossy(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::config::RepoEntry;

    fn config_with_repos(repos: Vec<RepoEntry>) -> BrainConfig {
        BrainConfig {
            repos,
            ..Default::default()
        }
    }

    fn repo_entry(slug: &str, repo_path: &str) -> RepoEntry {
        RepoEntry {
            public: false,
            slug: slug.to_string(),
            tier: "secondary".to_string(),
            repo_path: repo_path.to_string(),
            status_file: String::new(),
            cache_doc: String::new(),
            heading: String::new(),
            prefix: None,
        }
    }

    /// Build a fixture corpus under a tempdir: `mev/` (the "real" mev crate mimicked by a
    /// bare Cargo.toml at a known relative path), one consumer that path-depends on it, one
    /// non-Rust repo (no Cargo.toml), and one Rust repo with no dependency on mev at all.
    fn fixture_corpus() -> (tempfile::TempDir, PathBuf) {
        let root = tempfile::tempdir().expect("tempdir");
        let root_path = root.path().to_path_buf();

        // Consumer with a plain [dependencies] path entry, spelled differently than "../mev"
        // (nested one level deeper) to prove resolution, not string matching.
        let consumer_dir = root_path.join("consumer-a");
        std::fs::create_dir_all(&consumer_dir).unwrap();
        std::fs::write(
            consumer_dir.join("Cargo.toml"),
            format!(
                "[package]\nname = \"consumer-a\"\nversion = \"0.1.0\"\n\n[dependencies]\nmev = {{ path = \"{}\" }}\n",
                MEV_CRATE_DIR
            ),
        )
        .unwrap();

        // Consumer with the dependency under [workspace.dependencies] (engine-rs shape).
        let workspace_consumer_dir = root_path.join("consumer-workspace");
        std::fs::create_dir_all(&workspace_consumer_dir).unwrap();
        std::fs::write(
            workspace_consumer_dir.join("Cargo.toml"),
            format!(
                "[workspace]\nmembers = []\n\n[workspace.dependencies]\nmev = {{ path = \"{}\" }}\n",
                MEV_CRATE_DIR
            ),
        )
        .unwrap();

        // Rust repo that does NOT depend on mev.
        let unrelated_dir = root_path.join("unrelated-rust-repo");
        std::fs::create_dir_all(&unrelated_dir).unwrap();
        std::fs::write(
            unrelated_dir.join("Cargo.toml"),
            "[package]\nname = \"unrelated\"\nversion = \"0.1.0\"\n\n[dependencies]\nserde = \"1\"\n",
        )
        .unwrap();

        // Non-Rust repo: no Cargo.toml at all.
        let non_rust_dir = root_path.join("non-rust-repo");
        std::fs::create_dir_all(&non_rust_dir).unwrap();
        std::fs::write(non_rust_dir.join("README.md"), "not rust\n").unwrap();

        (root, root_path)
    }

    #[test]
    fn discovers_exactly_the_path_dependent_consumer() {
        let (_tmp, root) = fixture_corpus();
        let config = config_with_repos(vec![
            repo_entry("consumer-a", "consumer-a"),
            repo_entry("unrelated-rust-repo", "unrelated-rust-repo"),
            repo_entry("non-rust-repo", "non-rust-repo"),
        ]);

        let found = discover_mev_consumers(&root, &config);

        assert_eq!(
            found.len(),
            1,
            "expected exactly one consumer, got {found:?}"
        );
        assert_eq!(found[0].slug, "consumer-a");
    }

    #[test]
    fn workspace_dependencies_table_is_also_discovered() {
        let (_tmp, root) = fixture_corpus();
        let config =
            config_with_repos(vec![repo_entry("consumer-workspace", "consumer-workspace")]);

        let found = discover_mev_consumers(&root, &config);

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].slug, "consumer-workspace");
    }

    #[test]
    fn non_rust_repo_is_silently_skipped_not_an_error() {
        let (_tmp, root) = fixture_corpus();
        let config = config_with_repos(vec![repo_entry("non-rust-repo", "non-rust-repo")]);

        let found = discover_mev_consumers(&root, &config);

        assert!(found.is_empty());
    }

    #[test]
    fn rust_repo_without_mev_dependency_is_not_a_consumer() {
        let (_tmp, root) = fixture_corpus();
        let config = config_with_repos(vec![repo_entry(
            "unrelated-rust-repo",
            "unrelated-rust-repo",
        )]);

        let found = discover_mev_consumers(&root, &config);

        assert!(found.is_empty());
    }

    #[test]
    fn consumer_result_carries_absolute_manifest_and_lock_paths() {
        let (_tmp, root) = fixture_corpus();
        let config = config_with_repos(vec![repo_entry("consumer-a", "consumer-a")]);

        let found = discover_mev_consumers(&root, &config);

        assert_eq!(found.len(), 1);
        assert!(found[0].cargo_toml.ends_with("consumer-a/Cargo.toml"));
        assert!(found[0].cargo_lock.ends_with("consumer-a/Cargo.lock"));
    }

    #[test]
    fn empty_repo_path_is_skipped() {
        let (_tmp, root) = fixture_corpus();
        let config = config_with_repos(vec![repo_entry("blank", "")]);

        let found = discover_mev_consumers(&root, &config);

        assert!(found.is_empty());
    }
}
