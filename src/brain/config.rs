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

/// `[attention]` section of `brain.toml` — per-`kind` staleness thresholds
/// (in days) for the Attention surface.
///
/// A carryover/backlog item is "stale" (surfaced on the Attention board and via
/// the `W_STATE_*_STALE` warnings) once its age exceeds the threshold for its
/// kind, unless it is currently snoozed. An absent `[attention]` table yields
/// all defaults via [`Default`]. Both the validator and the emit planner read
/// this one struct, so the board shows exactly what the warnings fire on.
#[derive(Debug, Clone, Deserialize)]
pub struct AttentionThresholds {
    /// `carryover` kind `env` — transient environmental caveats (default 3).
    #[serde(default = "default_env_days")]
    pub env_days: i64,
    /// `carryover` kind `deferred` — untickted follow-ons (default 5).
    #[serde(default = "default_deferred_days")]
    pub deferred_days: i64,
    /// `carryover` kind `known_issue` (default 10).
    #[serde(default = "default_known_issue_days")]
    pub known_issue_days: i64,
    /// `carryover` kind `constraint` (default 10).
    #[serde(default = "default_constraint_days")]
    pub constraint_days: i64,
    /// `backlog[]` `idea`/`ready` nodes (default 7).
    #[serde(default = "default_backlog_days")]
    pub backlog_days: i64,
}

fn default_env_days() -> i64 {
    3
}
fn default_deferred_days() -> i64 {
    5
}
fn default_known_issue_days() -> i64 {
    10
}
fn default_constraint_days() -> i64 {
    10
}
fn default_backlog_days() -> i64 {
    7
}

impl Default for AttentionThresholds {
    fn default() -> Self {
        Self {
            env_days: default_env_days(),
            deferred_days: default_deferred_days(),
            known_issue_days: default_known_issue_days(),
            constraint_days: default_constraint_days(),
            backlog_days: default_backlog_days(),
        }
    }
}

impl AttentionThresholds {
    /// The staleness threshold (in days) for a `carryover` entry of the given
    /// `kind`. An unrecognised kind falls back to the most conservative
    /// (longest) carryover threshold so novel kinds surface, but not eagerly.
    pub fn carryover_threshold(&self, kind: &str) -> i64 {
        match kind {
            "env" => self.env_days,
            "deferred" => self.deferred_days,
            "known_issue" => self.known_issue_days,
            "constraint" => self.constraint_days,
            _ => self
                .known_issue_days
                .max(self.constraint_days)
                .max(self.deferred_days)
                .max(self.env_days),
        }
    }
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
    /// `[attention]` section — staleness thresholds for the Attention surface.
    #[serde(default)]
    pub attention: AttentionThresholds,
    /// `[[repos]]` entries.
    #[serde(default)]
    pub repos: Vec<RepoEntry>,
}

impl BrainConfig {
    /// Derive the valid `project` vocabulary as the set of `[[repos]]` slugs.
    pub fn projects(&self) -> Vec<&str> {
        self.repos.iter().map(|r| r.slug.as_str()).collect()
    }

    /// Look up a `[[repos]]` entry by slug.
    fn repo(&self, slug: &str) -> Option<&RepoEntry> {
        self.repos.iter().find(|r| r.slug == slug)
    }

    /// Derive the set of derived surfaces a given repo `slug` feeds, for
    /// `emit-state --scope <slug>` (ticket `ticket-emit-state-scope-and-lock`).
    ///
    /// Purely mechanical over the `[[repos]]` registry — no hardcoded repo list:
    ///
    /// - **`own_state_json`** — `{repo_path}/planning/state.json`, mirroring the
    ///   fixed suffix [`crate::brain::state::discover_state_files`] uses for every
    ///   leaf repo (independent of the `status_file` field, which already embeds
    ///   the same `{repo_path}` prefix for its own `status.md` sibling).
    /// - **`cache_doc`** — the entry's own `cache_doc` field, verbatim.
    /// - **`tier_rollup_status_file`** — the `status_file` of the `[[repos]]` entry
    ///   whose `slug` equals this repo's `tier` value (a "tier-container
    ///   self-entry", e.g. `slug = "core"`, `tier = "core"` on the leaf that feeds
    ///   it — mirroring the self-entry convention `tier_scope_for` and
    ///   `discover_state_files` already use, where a tier container is a
    ///   `[[repos]]` entry whose `slug` names the tier). `None` when no entry's
    ///   `slug` matches the tier (e.g. a `tier = "_root"` entry such as the HQ
    ///   root or a tier-container's own entry, which has no *further* tier to
    ///   roll into — it feeds the HQ board directly instead).
    /// - **`hq_board_status_file`** — the `status_file` of the `[[repos]]` entry
    ///   with `repo_path == "."` (the HQ root). Every repo feeds the HQ board.
    ///
    /// Returns [`ScopeError::UnknownSlug`] (carrying every valid slug, in
    /// registry order) when `slug` is not registered.
    pub fn scope_dependencies(&self, slug: &str) -> Result<ScopeDependencySet, ScopeError> {
        let repo = self.repo(slug).ok_or_else(|| ScopeError::UnknownSlug {
            slug: slug.to_string(),
            valid_slugs: self.projects().into_iter().map(str::to_string).collect(),
        })?;

        let own_state_json = PathBuf::from(&repo.repo_path)
            .join("planning")
            .join("state.json");

        let cache_doc = PathBuf::from(&repo.cache_doc);

        let tier_rollup_status_file = self
            .repo(&repo.tier)
            .filter(|tier_repo| tier_repo.slug != repo.slug)
            .map(|tier_repo| PathBuf::from(&tier_repo.status_file));

        let hq_repo = self
            .repos
            .iter()
            .find(|r| r.repo_path == "." || r.repo_path.is_empty())
            .ok_or(ScopeError::NoHqRoot)?;
        let hq_board_status_file = PathBuf::from(&hq_repo.status_file);

        Ok(ScopeDependencySet {
            own_state_json,
            cache_doc,
            tier_rollup_status_file,
            hq_board_status_file,
        })
    }
}

/// The derived surfaces a single repo (resolved via [`BrainConfig::scope_dependencies`])
/// feeds — the set `emit-state --scope <repo>` must regenerate and nothing else.
///
/// All paths are relative to the brain root (as every `[[repos]]` field already is).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeDependencySet {
    /// `{repo_path}/planning/state.json` — the repo's own leaf state file.
    pub own_state_json: PathBuf,
    /// The repo's `cache_doc` (e.g. `docs/projects/<slug>.md`).
    pub cache_doc: PathBuf,
    /// The `status_file` of the repo's tier container, if the tier resolves to a
    /// registered `[[repos]]` entry distinct from this repo itself. `None` for a
    /// tier-container (or root) entry that has no further tier to roll into.
    pub tier_rollup_status_file: Option<PathBuf>,
    /// The HQ root's `status_file` — the Operating Board target every repo feeds.
    pub hq_board_status_file: PathBuf,
}

impl ScopeDependencySet {
    /// Every target path in this set, resolved against `root`, as the absolute
    /// paths an `emit-state` planner would produce for the *same* fields
    /// (`root.join(<relative field>)`) — see [`BrainConfig::scope_dependencies`]'s
    /// doc comment for why each field's construction already matches its
    /// planner's own path-building.
    pub fn absolute_targets(&self, root: &Path) -> Vec<PathBuf> {
        let mut targets = vec![
            root.join(&self.own_state_json),
            root.join(&self.cache_doc),
            root.join(&self.hq_board_status_file),
        ];
        if let Some(tier) = &self.tier_rollup_status_file {
            targets.push(root.join(tier));
        }
        targets
    }

    /// Is `path` (already an absolute path, as every `EmitAction::path` is) one
    /// of this scope's target surfaces under `root`?
    pub fn allows(&self, root: &Path, path: &Path) -> bool {
        self.absolute_targets(root).iter().any(|t| t == path)
    }
}

/// Errors from [`BrainConfig::scope_dependencies`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ScopeError {
    /// The requested `--scope` slug is not a registered `[[repos]]` entry.
    #[error("unknown --scope slug '{slug}'; valid slugs: {valid_slugs:?}")]
    UnknownSlug {
        slug: String,
        valid_slugs: Vec<String>,
    },
    /// The registry has no `[[repos]]` entry with `repo_path == "."` (the HQ
    /// root), so the HQ board target cannot be derived. This should not occur
    /// with a well-formed `brain.toml`.
    #[error("brain.toml has no [[repos]] entry with repo_path = \".\" (HQ root)")]
    NoHqRoot,
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

/// Walk up from `start`, looking for `brain.toml`, and return its containing directory.
///
/// Starts at `start` itself, then walks to each parent in turn until a `brain.toml`
/// is found or the filesystem root is reached (in which case [`ConfigError::NotFound`]
/// is returned).
pub fn find_brain_root(start: &Path) -> Result<PathBuf, ConfigError> {
    let mut current = start.canonicalize().unwrap_or_else(|_| start.to_path_buf());
    loop {
        if current.join("brain.toml").exists() {
            return Ok(current);
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

/// Walk up from `start`, looking for `brain.toml`, and return the parsed config.
///
/// Starts at `start` itself, then walks to each parent in turn until a `brain.toml`
/// is found or the filesystem root is reached (in which case [`ConfigError::NotFound`]
/// is returned).
pub fn find_brain_config(start: &Path) -> Result<BrainConfig, ConfigError> {
    let root = find_brain_root(start)?;
    load_brain_config(&root.join("brain.toml"))
}

/// Best-effort check: is `path` inside a *linked* git worktree (as opposed to a
/// repo's main working tree)?
///
/// Runs `git -C <path> rev-parse --git-dir` and `git -C <path> rev-parse
/// --git-common-dir`. In a repo's main working tree these resolve to the same
/// directory; in a linked worktree (created via `git worktree add`) `--git-dir`
/// points at `<main-repo>/.git/worktrees/<name>` while `--git-common-dir` points
/// at the main repo's `.git`, so they differ.
///
/// This is a *safety check*, not a hard requirement: any failure to invoke git
/// at all (not a git repo, git missing from `PATH`, a git error) must resolve to
/// `false` (fail open) rather than propagating an error — many existing tests
/// use a bare [`tempfile::tempdir`] (no `git init`) as a brain root, and this
/// function must not treat that as a worktree.
pub fn is_linked_worktree(path: &Path) -> bool {
    let git_dir = match run_git_rev_parse(path, "--git-dir") {
        Some(dir) => dir,
        None => return false,
    };
    let git_common_dir = match run_git_rev_parse(path, "--git-common-dir") {
        Some(dir) => dir,
        None => return false,
    };

    let resolve = |raw: &str| -> PathBuf {
        let candidate = PathBuf::from(raw);
        let joined = if candidate.is_absolute() {
            candidate
        } else {
            path.join(candidate)
        };
        joined.canonicalize().unwrap_or(joined)
    };

    resolve(&git_dir) != resolve(&git_common_dir)
}

/// Run `git -C <path> rev-parse <arg>` and return its trimmed stdout, or `None`
/// if git could not be invoked or exited non-zero.
fn run_git_rev_parse(path: &Path, arg: &str) -> Option<String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .arg("rev-parse")
        .arg(arg)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
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

    #[test]
    fn attention_thresholds_default_when_section_absent() {
        // The fixture brain.toml has no [attention] table → all defaults.
        let cfg = load_brain_config(&fixture_path()).expect("should parse fixture");
        assert_eq!(cfg.attention.env_days, 3);
        assert_eq!(cfg.attention.deferred_days, 5);
        assert_eq!(cfg.attention.known_issue_days, 10);
        assert_eq!(cfg.attention.constraint_days, 10);
        assert_eq!(cfg.attention.backlog_days, 7);
        // Per-kind lookup + unknown-kind fallback (most conservative).
        assert_eq!(cfg.attention.carryover_threshold("env"), 3);
        assert_eq!(cfg.attention.carryover_threshold("deferred"), 5);
        assert_eq!(cfg.attention.carryover_threshold("mystery"), 10);
    }

    #[test]
    fn attention_thresholds_partial_override_keeps_other_defaults() {
        let toml = r#"
[attention]
deferred_days = 2
"#;
        let cfg: BrainConfig = toml::from_str(toml).expect("parse");
        assert_eq!(cfg.attention.deferred_days, 2);
        assert_eq!(cfg.attention.env_days, 3, "unset field keeps its default");
        assert_eq!(cfg.attention.backlog_days, 7);
    }

    fn run_git(dir: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .status()
            .expect("failed to invoke git");
        assert!(status.success(), "git {:?} failed in {:?}", args, dir);
    }

    #[test]
    fn is_linked_worktree_false_for_main_tree() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo_path = dir.path();
        run_git(repo_path, &["init", "-q"]);
        run_git(repo_path, &["config", "user.email", "test@example.com"]);
        run_git(repo_path, &["config", "user.name", "Test"]);
        std::fs::write(repo_path.join("README.md"), "hello").expect("write file");
        run_git(repo_path, &["add", "README.md"]);
        run_git(repo_path, &["commit", "-q", "-m", "initial commit"]);

        assert!(
            !is_linked_worktree(repo_path),
            "main working tree should not be reported as a linked worktree"
        );
    }

    #[test]
    fn is_linked_worktree_true_for_linked_worktree() {
        let main_dir = tempfile::tempdir().expect("tempdir");
        let main_path = main_dir.path();
        run_git(main_path, &["init", "-q"]);
        run_git(main_path, &["config", "user.email", "test@example.com"]);
        run_git(main_path, &["config", "user.name", "Test"]);
        std::fs::write(main_path.join("README.md"), "hello").expect("write file");
        run_git(main_path, &["add", "README.md"]);
        run_git(main_path, &["commit", "-q", "-m", "initial commit"]);

        let worktree_parent = tempfile::tempdir().expect("tempdir");
        let worktree_path = worktree_parent.path().join("wt");
        run_git(
            main_path,
            &[
                "worktree",
                "add",
                worktree_path.to_str().expect("utf8 path"),
            ],
        );

        assert!(
            is_linked_worktree(&worktree_path),
            "linked worktree should be reported as such"
        );
    }

    #[test]
    fn is_linked_worktree_false_for_non_git_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(
            !is_linked_worktree(dir.path()),
            "a plain non-git directory must fail open to false"
        );
    }

    // -----------------------------------------------------------------------
    // BrainConfig::scope_dependencies (ticket-emit-state-scope-and-lock task 1)
    // -----------------------------------------------------------------------

    fn repo_entry(slug: &str, tier: &str, repo_path: &str) -> RepoEntry {
        RepoEntry {
            slug: slug.to_string(),
            tier: tier.to_string(),
            repo_path: repo_path.to_string(),
            status_file: format!("{repo_path}/planning/status.md"),
            cache_doc: format!("docs/projects/{slug}.md"),
            heading: slug.to_string(),
        }
    }

    /// Mirrors the real HQ `brain.toml` shape: an HQ root, two tier-container
    /// self-entries (`core`, `business`), and one leaf repo under each tier.
    fn scoped_fixture_config() -> BrainConfig {
        BrainConfig {
            vocab: VocabConfig::default(),
            crawl: CrawlConfig::default(),
            attention: AttentionThresholds::default(),
            repos: vec![
                RepoEntry {
                    slug: "brain".to_string(),
                    tier: "_root".to_string(),
                    repo_path: ".".to_string(),
                    status_file: "planning/status.md".to_string(),
                    cache_doc: "README.md".to_string(),
                    heading: "Company Brain".to_string(),
                },
                repo_entry("core", "_root", "core"),
                repo_entry("mev", "core", "core/mev"),
                repo_entry("business", "_root", "business"),
                repo_entry("bastiel", "business", "business/bastiel"),
            ],
        }
    }

    #[test]
    fn scope_dependencies_valid_core_tier_slug() {
        let cfg = scoped_fixture_config();
        let deps = cfg.scope_dependencies("mev").expect("mev is registered");

        assert_eq!(
            deps.own_state_json,
            PathBuf::from("core/mev/planning/state.json")
        );
        assert_eq!(deps.cache_doc, PathBuf::from("docs/projects/mev.md"));
        assert_eq!(
            deps.tier_rollup_status_file,
            Some(PathBuf::from("core/planning/status.md")),
            "mev's tier 'core' resolves to the 'core' tier-container entry"
        );
        assert_eq!(
            deps.hq_board_status_file,
            PathBuf::from("planning/status.md")
        );
    }

    #[test]
    fn scope_dependencies_valid_business_tier_slug() {
        let cfg = scoped_fixture_config();
        let deps = cfg
            .scope_dependencies("bastiel")
            .expect("bastiel is registered");

        assert_eq!(
            deps.own_state_json,
            PathBuf::from("business/bastiel/planning/state.json")
        );
        assert_eq!(deps.cache_doc, PathBuf::from("docs/projects/bastiel.md"));
        assert_eq!(
            deps.tier_rollup_status_file,
            Some(PathBuf::from("business/planning/status.md")),
            "bastiel's tier 'business' resolves to the 'business' tier-container entry"
        );
        assert_eq!(
            deps.hq_board_status_file,
            PathBuf::from("planning/status.md")
        );
    }

    #[test]
    fn scope_dependencies_tier_container_self_entry_has_no_further_rollup() {
        // 'core' itself has tier = "_root", which names no [[repos]] entry, so
        // it feeds the HQ board directly rather than a further tier rollup.
        let cfg = scoped_fixture_config();
        let deps = cfg.scope_dependencies("core").expect("core is registered");
        assert_eq!(deps.tier_rollup_status_file, None);
    }

    #[test]
    fn scope_dependencies_unknown_slug_carries_valid_slugs() {
        let cfg = scoped_fixture_config();
        let err = cfg
            .scope_dependencies("nonexistent")
            .expect_err("nonexistent is not registered");

        match err {
            ScopeError::UnknownSlug { slug, valid_slugs } => {
                assert_eq!(slug, "nonexistent");
                assert_eq!(
                    valid_slugs,
                    vec!["brain", "core", "mev", "business", "bastiel"]
                );
            }
            other => panic!("expected ScopeError::UnknownSlug, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // ScopeDependencySet::allows / absolute_targets (task 2)
    // -----------------------------------------------------------------------

    #[test]
    fn scope_allows_own_state_json_cache_doc_and_hq_board() {
        let cfg = scoped_fixture_config();
        let deps = cfg.scope_dependencies("mev").expect("mev is registered");
        let root = PathBuf::from("/hq");

        assert!(deps.allows(&root, &root.join("core/mev/planning/state.json")));
        assert!(deps.allows(&root, &root.join("docs/projects/mev.md")));
        assert!(deps.allows(&root, &root.join("core/planning/status.md")));
        assert!(deps.allows(&root, &root.join("planning/status.md")));
    }

    #[test]
    fn scope_disallows_unrelated_repo_paths() {
        let cfg = scoped_fixture_config();
        let deps = cfg.scope_dependencies("mev").expect("mev is registered");
        let root = PathBuf::from("/hq");

        assert!(!deps.allows(&root, &root.join("business/bastiel/planning/state.json")));
        assert!(!deps.allows(&root, &root.join("docs/projects/bastiel.md")));
        assert!(!deps.allows(&root, &root.join("business/planning/status.md")));
    }

    #[test]
    fn scope_allows_no_tier_rollup_when_none() {
        // A tier-container self-entry has no further tier rollup — its target
        // set has three entries, not four, and never allows a path that isn't
        // one of them.
        let cfg = scoped_fixture_config();
        let deps = cfg.scope_dependencies("core").expect("core is registered");
        let root = PathBuf::from("/hq");

        assert_eq!(deps.absolute_targets(&root).len(), 3);
        assert!(!deps.allows(&root, &root.join("core/planning/status.md")));
    }
}
