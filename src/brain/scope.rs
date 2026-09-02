//! Scope-unit registry and resolver for the multi-root Brain corpus.
//!
//! Each `[[repos]]` entry in `brain.toml` defines a **scope unit** identified by a
//! stable `slug` and anchored at a `repo_path` relative to the HQ root.  The root unit
//! (`repo_path = "."`) is the fallback scope for any file not claimed by a longer prefix.
//!
//! Public API:
//! - [`scope_units`] — enumerate all `(slug, repo_path)` pairs from the registry.
//! - [`scope_for`]   — resolve a relative path to its owning unit's slug.
//! - [`owning_unit`] — resolve a relative path to its `(slug, repo_path)` owning pair.

use std::path::{Path, PathBuf};

use crate::brain::config::BrainConfig;

// ---------------------------------------------------------------------------
// Public helpers
// ---------------------------------------------------------------------------

/// Return all `(slug, repo_path)` pairs registered in the config's `[[repos]]` table.
///
/// The pairs are returned in declaration order (same as `brain.toml`).
pub fn scope_units(config: &BrainConfig) -> Vec<(String, String)> {
    config
        .repos
        .iter()
        .map(|r| (r.slug.clone(), r.repo_path.clone()))
        .collect()
}

/// Resolve `rel` (a path relative to the HQ root) to the stable slug of its owning scope unit.
///
/// **Algorithm:** among all units whose `repo_path` is a proper path prefix of `rel` (using
/// [`Path::strip_prefix`] so `core/mev-extra` does *not* match a `core/mev` unit), pick the one
/// with the longest `repo_path` component count.  The root unit (`repo_path = "."`) is excluded
/// from prefix matching but used as the fallback when no other unit claims `rel`.
///
/// Returns the owning unit's `slug`.  If no root unit is registered (unusual fixture) and no
/// prefix matches, returns `"brain"` as a hard-coded safety fallback.
pub fn scope_for(rel: &Path, config: &BrainConfig) -> String {
    owning_unit(rel, config).0
}

/// Resolve `rel` to its owning `(slug, repo_path)` pair.
///
/// See [`scope_for`] for the resolution algorithm.
pub fn owning_unit(rel: &Path, config: &BrainConfig) -> (String, String) {
    let mut best: Option<(String, String, usize)> = None; // (slug, repo_path, prefix_component_count)

    for unit in &config.repos {
        let repo_path = unit.repo_path.trim();

        // The root unit is the fallback — skip it during prefix matching.
        if repo_path == "." || repo_path.is_empty() {
            continue;
        }

        let unit_path = PathBuf::from(repo_path);

        // Use Path::strip_prefix for exact component matching (avoids "core/mev-extra" matching "core/mev").
        if rel.strip_prefix(&unit_path).is_ok() {
            let depth = unit_path.components().count();
            let better = match &best {
                None => true,
                Some((_, _, d)) => depth > *d,
            };
            if better {
                best = Some((unit.slug.clone(), unit.repo_path.clone(), depth));
            }
        }
    }

    if let Some((slug, repo_path, _)) = best {
        return (slug, repo_path);
    }

    // No prefix match — use the root unit as fallback.
    for unit in &config.repos {
        let rp = unit.repo_path.trim();
        if rp == "." || rp.is_empty() {
            return (unit.slug.clone(), unit.repo_path.clone());
        }
    }

    // Hard-coded safety fallback (should not occur with a well-formed brain.toml).
    ("brain".to_string(), ".".to_string())
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::config::{BrainConfig, CrawlConfig, RepoEntry, VocabConfig};

    /// Build a test config with brain (root), core (tier), and mev (core/mev) units.
    fn three_unit_config() -> BrainConfig {
        BrainConfig {
            surface_allowlist: Default::default(),
            conformance_writers: Vec::new(),
            contracts: Vec::new(),
            permission_profiles: Default::default(),
            attention: Default::default(),
            history: Default::default(),
            carryover: Default::default(),
            vocab: VocabConfig::default(),
            crawl: CrawlConfig::default(),
            repos: vec![
                RepoEntry {
                    public: false,
                    slug: "brain".to_string(),
                    tier: "primary".to_string(),
                    repo_path: ".".to_string(),
                    status_file: String::new(),
                    cache_doc: String::new(),
                    heading: String::new(),
                    prefix: None,
                },
                RepoEntry {
                    public: false,
                    slug: "core".to_string(),
                    tier: "tier".to_string(),
                    repo_path: "core".to_string(),
                    status_file: String::new(),
                    cache_doc: String::new(),
                    heading: String::new(),
                    prefix: None,
                },
                RepoEntry {
                    public: false,
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

    #[test]
    fn scope_units_returns_all_pairs() {
        let cfg = three_unit_config();
        let units = scope_units(&cfg);
        assert_eq!(units.len(), 3);
        assert!(units.iter().any(|(s, p)| s == "brain" && p == "."));
        assert!(units.iter().any(|(s, p)| s == "core" && p == "core"));
        assert!(units.iter().any(|(s, p)| s == "mev" && p == "core/mev"));
    }

    #[test]
    fn mev_file_resolves_to_mev() {
        let cfg = three_unit_config();
        let rel = Path::new("core/mev/planning/x.md");
        assert_eq!(scope_for(rel, &cfg), "mev");
    }

    #[test]
    fn core_docs_file_resolves_to_core() {
        let cfg = three_unit_config();
        let rel = Path::new("core/docs/x.md");
        assert_eq!(scope_for(rel, &cfg), "core");
    }

    #[test]
    fn planning_file_resolves_to_brain() {
        let cfg = three_unit_config();
        let rel = Path::new("planning/x.md");
        assert_eq!(scope_for(rel, &cfg), "brain");
    }

    #[test]
    fn readme_resolves_to_brain() {
        let cfg = three_unit_config();
        let rel = Path::new("README.md");
        assert_eq!(scope_for(rel, &cfg), "brain");
    }

    #[test]
    fn longest_prefix_wins_mev_over_core() {
        let cfg = three_unit_config();
        // core/mev/... should match "mev" (longer prefix) not "core".
        let rel = Path::new("core/mev/docs/guide.md");
        assert_eq!(scope_for(rel, &cfg), "mev");
    }

    #[test]
    fn no_false_prefix_match_mev_extra() {
        // "core/mev-extra/planning/x.md" must NOT match the "core/mev" unit.
        let cfg = three_unit_config();
        let rel = Path::new("core/mev-extra/planning/x.md");
        // "core/mev" does not match via strip_prefix; "core" does.
        assert_eq!(scope_for(rel, &cfg), "core");
    }

    #[test]
    fn owning_unit_returns_repo_path() {
        let cfg = three_unit_config();
        let rel = Path::new("core/mev/planning/status.md");
        let (slug, repo_path) = owning_unit(rel, &cfg);
        assert_eq!(slug, "mev");
        assert_eq!(repo_path, "core/mev");
    }

    #[test]
    fn stability_slug_unchanged_after_simulated_tier_rename() {
        // Simulate renaming the tier path from "core" to "workspace" — the mev slug
        // is stable and still resolves by slug even though the outer tier changed.
        let mut cfg = three_unit_config();
        // Rename the "core" tier unit's repo_path to "workspace".
        cfg.repos[1].repo_path = "workspace".to_string();
        // Also update the mev unit to "workspace/mev" (consistent rename).
        cfg.repos[2].repo_path = "workspace/mev".to_string();

        let rel_renamed = Path::new("workspace/mev/planning/x.md");
        let (slug, _) = owning_unit(rel_renamed, &cfg);
        // The slug is still "mev" — keyed by slug, not by path.
        assert_eq!(slug, "mev");
    }

    #[test]
    fn two_unit_config_brain_fallback() {
        // Minimal two-unit config (brain + mev) — a brain-level file falls back to brain.
        let cfg = BrainConfig {
            attention: Default::default(),
            history: Default::default(),
            carryover: Default::default(),
            repos: vec![
                RepoEntry {
                    public: false,
                    slug: "brain".to_string(),
                    repo_path: ".".to_string(),
                    tier: String::new(),
                    status_file: String::new(),
                    cache_doc: String::new(),
                    heading: String::new(),
                    prefix: None,
                },
                RepoEntry {
                    public: false,
                    slug: "mev".to_string(),
                    repo_path: "core/mev".to_string(),
                    tier: String::new(),
                    status_file: String::new(),
                    cache_doc: String::new(),
                    heading: String::new(),
                    prefix: None,
                },
            ],
            ..BrainConfig::default()
        };
        assert_eq!(scope_for(Path::new("docs/index.md"), &cfg), "brain");
        assert_eq!(
            scope_for(Path::new("core/mev/planning/status.md"), &cfg),
            "mev"
        );
    }
}
