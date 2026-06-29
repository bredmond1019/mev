//! `state.json` serde model and JSON loader for `mev validate-brain --state`.
//!
//! Phase 3, Block P: schema validation of each repo's `planning/state.json` and the
//! cross-repo block-dependency graph integrity check.
//!
//! This module provides:
//! - The serde structs mirroring `state-schema.md` (all collections default-empty,
//!   extra fields tolerated via `deny_unknown_fields` omitted).
//! - [`load_state`] — read a `state.json` file and surface parse failures as
//!   [`StateLoadError`] so the caller can emit `E_STATE_MALFORMED_JSON`.
//!
//! Diagnostic locator codes emitted by later tasks that build on this foundation:
//! - `E_STATE_MALFORMED_JSON` — file is not parseable JSON.
//! - `E_STATE_SCHEMA_MISSING_FIELD` — a required key is absent.
//! - `E_STATE_SCHEMA_BAD_KIND` — `kind` ∉ `{project, brain}`.
//! - `E_STATE_SCHEMA_BAD_STATUS` — a `status` value ∉ the enum.
//! - `E_STATE_SCHEMA_BAD_BLOCKED_BY` — a `blocked_by[]` entry has an unknown `type`.
//! - `E_STATE_DUPLICATE_BLOCK_ID` — two `tracks[]` blocks in one repo share an `id`.
//! - `E_STATE_DANGLING_FOCUS` — a focus entry's `block` is absent from `tracks[]`.
//! - `E_STATE_DANGLING_BLOCKED_BY` — a cross-repo dependency block doesn't exist.
//! - `E_STATE_UNKNOWN_REPO` — a `blocked_by` / `cross_repo` entry names unknown repo.
//! - `E_STATE_DANGLING_CROSS_REPO` — a brain `cross_repo[]` endpoint doesn't resolve.
//! - `W_STATE_ROLLUP_DRIFT` — brain `repos[]` headline drifted from child `focus`.
//! - `W_STATE_FILE_MISSING` — a registered repo has no `planning/state.json`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::Diagnostic;
use crate::brain::config::BrainConfig;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur when loading a `planning/state.json` file.
#[derive(Debug, Error)]
pub enum StateLoadError {
    /// The file could not be read from disk.
    #[error("could not read {path}: {source}")]
    Io {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// The file contents are not valid JSON (maps to `E_STATE_MALFORMED_JSON`).
    #[error("could not parse {path} as JSON: {source}")]
    Parse {
        path: std::path::PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

// ---------------------------------------------------------------------------
// BlockedBy — internally tagged enum on `type`
// ---------------------------------------------------------------------------

/// A single entry in a `blocked_by[]` array.
///
/// Tagged by the `"type"` field.  Unknown `type` values are rejected by serde
/// (no `#[serde(other)]`), which is surfaced as `StateLoadError::Parse` →
/// locator code `E_STATE_SCHEMA_BAD_BLOCKED_BY`.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BlockedBy {
    /// A dependency on another block (may be cross-repo).
    Block {
        /// Slug of the owning repo.
        repo: String,
        /// Canonical block ID (e.g. `BA.11.C`).
        id: String,
        /// Optional gloss explaining the dependency.
        #[serde(default)]
        what: Option<String>,
    },
    /// An environmental / external dependency (not a tracked block).
    External {
        /// Human description of the external dependency.
        what: String,
    },
}

// ---------------------------------------------------------------------------
// Block — lenient superset across now/next/blocked variants
// ---------------------------------------------------------------------------

/// One entry in a `focus.now`, `focus.next`, or `focus.blocked` array.
///
/// Fields are a lenient union of all three variants so the same struct works
/// everywhere:
/// - `now` items: `block`, `title`, `status`, `note?`, `repo?`
/// - `next` items: `block`, `title`, `repo?` (no `status`)
/// - `blocked` items: `block`, `title`, `blocked_by[]`, `repo?`
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Block {
    /// Canonical block ID.
    pub block: String,
    /// Brief human description.
    pub title: String,
    /// Lifecycle status (present on `now` and `blocked` entries).
    #[serde(default)]
    pub status: Option<String>,
    /// Optional in-flight context note.
    #[serde(default)]
    pub note: Option<String>,
    /// Cross-repo source repo slug (used in brain `focus` entries).
    #[serde(default)]
    pub repo: Option<String>,
    /// What this block is waiting on (present on `blocked` entries).
    #[serde(default)]
    pub blocked_by: Vec<BlockedBy>,
}

// ---------------------------------------------------------------------------
// Focus
// ---------------------------------------------------------------------------

/// The `focus` object — what's now, next, and blocked in a repo.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct Focus {
    /// Blocks currently in progress.
    #[serde(default)]
    pub now: Vec<Block>,
    /// Blocks queued for next (ordered).
    #[serde(default)]
    pub next: Vec<Block>,
    /// Blocks waiting on something.
    #[serde(default)]
    pub blocked: Vec<Block>,
}

// ---------------------------------------------------------------------------
// Track / TrackBlock — leaf roadmap catalog
// ---------------------------------------------------------------------------

/// One block entry inside a `tracks[]` phase/wave.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TrackBlock {
    /// Canonical block ID.
    pub id: String,
    /// Brief human description.
    pub title: String,
    /// Lifecycle status.
    #[serde(default)]
    pub status: Option<String>,
}

/// One phase/wave entry in a leaf repo's `tracks[]`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Track {
    /// Phase or wave name.
    pub title: String,
    /// Ordered blocks in this phase.
    #[serde(default)]
    pub blocks: Vec<TrackBlock>,
}

// ---------------------------------------------------------------------------
// RepoRollup — brain `repos[]` child headline cache
// ---------------------------------------------------------------------------

/// One child repo's cached headline in a brain `repos[]` entry.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RepoRollup {
    /// Child repo slug.
    pub repo: String,
    /// Tier classification (e.g. `"core"`, `"portfolio"`).
    #[serde(default)]
    pub tier: Option<String>,
    /// Cached `focus.now` from the child.
    #[serde(default)]
    pub now: Vec<Block>,
    /// Cached `focus.next` from the child.
    #[serde(default)]
    pub next: Vec<Block>,
    /// Cached `focus.blocked` from the child.
    #[serde(default)]
    pub blocked: Vec<Block>,
}

// ---------------------------------------------------------------------------
// CrossRepoEdge / Endpoint — brain `cross_repo[]`
// ---------------------------------------------------------------------------

/// One endpoint of a cross-repo dependency edge.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Endpoint {
    /// Repo slug.
    pub repo: String,
    /// Canonical block ID.
    pub block: String,
}

/// A directed cross-repo dependency edge in a brain `cross_repo[]`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CrossRepoEdge {
    /// Source endpoint (the dependent block).
    pub from: Endpoint,
    /// Target endpoint (the dependency).
    pub to: Endpoint,
    /// Optional explanation of why this edge exists.
    #[serde(default)]
    pub note: Option<String>,
}

// ---------------------------------------------------------------------------
// TierEntry — HQ `tiers[]`
// ---------------------------------------------------------------------------

/// One tier pointer in the HQ brain `tiers[]`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TierEntry {
    /// Tier name (e.g. `"core"`).
    pub tier: String,
    /// Path or slug to the tier sub-brain, or `null`.
    #[serde(default)]
    pub rollup: Option<String>,
    /// One-line summary of the tier's current state.
    #[serde(default)]
    pub summary: Option<String>,
}

// ---------------------------------------------------------------------------
// StateFile — top-level structure
// ---------------------------------------------------------------------------

/// The deserialized contents of a `planning/state.json` file.
///
/// Both leaf (`kind:"project"`) and brain (`kind:"brain"`) variants are covered:
/// - Leaf adds `tracks[]`.
/// - Brain adds `repos[]`, `cross_repo[]`; HQ brain also adds `tiers[]`.
///
/// All optional collections default to empty; extra unknown fields are tolerated
/// (serde default behaviour without `deny_unknown_fields`).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StateFile {
    /// Repo slug identifying this file's owner.
    pub repo: String,
    /// File variant: `"project"` or `"brain"`.
    pub kind: String,
    /// Freshness date string (checked for presence only; format is `MV.3.M`'s job).
    pub updated: String,
    /// Current work status snapshot.
    #[serde(default)]
    pub focus: Focus,
    /// Roadmap catalog (leaf repos).
    #[serde(default)]
    pub tracks: Vec<Track>,
    /// Child-repo headline cache (brain files).
    #[serde(default)]
    pub repos: Vec<RepoRollup>,
    /// Directed cross-repo dependency edges (brain files).
    #[serde(default)]
    pub cross_repo: Vec<CrossRepoEdge>,
    /// Tier pointers (HQ brain only).
    #[serde(default)]
    pub tiers: Vec<TierEntry>,
    /// Optional top-level annotation note (seen in HQ state.json).
    #[serde(default)]
    pub note: Option<String>,
}

// ---------------------------------------------------------------------------
// Loader
// ---------------------------------------------------------------------------

/// Read `path` and deserialize it as a [`StateFile`].
///
/// Returns [`StateLoadError::Io`] if the file cannot be read, or
/// [`StateLoadError::Parse`] if the contents are not valid JSON or do not match
/// the [`StateFile`] schema.  The caller maps [`StateLoadError::Parse`] to the
/// `E_STATE_MALFORMED_JSON` diagnostic locator.
pub fn load_state(path: &Path) -> Result<StateFile, StateLoadError> {
    let contents = std::fs::read_to_string(path).map_err(|e| StateLoadError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    serde_json::from_str(&contents).map_err(|e| StateLoadError::Parse {
        path: path.to_path_buf(),
        source: e,
    })
}

// ---------------------------------------------------------------------------
// StateSource — discovery record
// ---------------------------------------------------------------------------

/// Metadata about a discovered `planning/state.json` file.
///
/// Produced by [`discover_state_files`].  The `expected_kind` allows
/// [`check_schema`] to verify that the file's `kind` field matches its
/// structural role (HQ/tier brain vs. leaf project).
#[derive(Debug, Clone)]
pub struct StateSource {
    /// Identifying slug for this source (e.g. `"hq"`, `"core"`, `"mev"`).
    pub repo_slug: String,
    /// Absolute path to the `planning/state.json` file.
    pub abs_path: PathBuf,
    /// Expected `kind` field value: `"brain"` or `"project"`.
    pub expected_kind: &'static str,
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

/// Discover all `planning/state.json` files reachable from `root`.
///
/// Returns `(sources, diagnostics)`:
/// - `sources` — every file that exists, ready to be loaded.
/// - `diagnostics` — one [`Diagnostic`] with locator `W_STATE_FILE_MISSING`
///   per registered path that does not exist on disk (warning severity).
///
/// Discovery strategy (per scoping decision 1 — cross-repo read mode):
/// 1. HQ brain: `root/planning/state.json` (always expected; `kind:"brain"`).
///    If found, the file is loaded internally to enumerate `tiers[]` so that
///    tier sub-brain paths (`tiers[].rollup`) can be discovered.
/// 2. Tier sub-brains: each `tiers[].rollup` path (relative to `root`) that is
///    non-null is expected as a brain-kind file.
/// 3. Leaf repos: each `[[repos]]` entry in `config` whose `repo_path` is not
///    `"."` (the HQ root itself) → `root/{repo_path}/planning/state.json`
///    (`kind:"project"`).
pub fn discover_state_files(
    root: &Path,
    config: &BrainConfig,
) -> (Vec<StateSource>, Vec<Diagnostic>) {
    let mut sources: Vec<StateSource> = Vec::new();
    let mut diags: Vec<Diagnostic> = Vec::new();

    // --- 1. HQ brain state ---
    let hq_path = root.join("planning").join("state.json");
    if hq_path.exists() {
        // Derive the HQ slug from the [[repos]] entry with repo_path="." if
        // present; fall back to "hq".
        let hq_slug = config
            .repos
            .iter()
            .find(|r| r.repo_path == "." || r.repo_path.is_empty())
            .map(|r| r.slug.clone())
            .unwrap_or_else(|| "hq".to_string());

        sources.push(StateSource {
            repo_slug: hq_slug,
            abs_path: hq_path.clone(),
            expected_kind: "brain",
        });

        // --- 2. Tier sub-brains (from HQ tiers[].rollup) ---
        if let Ok(hq_state) = load_state(&hq_path) {
            for tier_entry in &hq_state.tiers {
                let rollup = match &tier_entry.rollup {
                    Some(r) if !r.trim().is_empty() => r.clone(),
                    _ => continue, // null or empty rollup → no state file expected
                };
                let tier_path = root.join(&rollup);
                if tier_path.exists() {
                    sources.push(StateSource {
                        repo_slug: tier_entry.tier.clone(),
                        abs_path: tier_path,
                        expected_kind: "brain",
                    });
                } else {
                    diags.push(Diagnostic::warning(
                        tier_path,
                        "W_STATE_FILE_MISSING",
                        format!(
                            "tier '{}': state.json not found at rollup path '{rollup}'",
                            tier_entry.tier
                        ),
                    ));
                }
            }
        }
    } else {
        diags.push(Diagnostic::warning(
            hq_path,
            "W_STATE_FILE_MISSING",
            "HQ brain: planning/state.json not found at root".to_string(),
        ));
    }

    // --- 3. Leaf repos from [[repos]] config ---
    for repo in &config.repos {
        // Skip the HQ root entry itself.
        if repo.repo_path == "." || repo.repo_path.is_empty() {
            continue;
        }
        let state_path = root
            .join(&repo.repo_path)
            .join("planning")
            .join("state.json");
        if state_path.exists() {
            sources.push(StateSource {
                repo_slug: repo.slug.clone(),
                abs_path: state_path,
                expected_kind: "project",
            });
        } else {
            diags.push(Diagnostic::warning(
                state_path,
                "W_STATE_FILE_MISSING",
                format!(
                    "repo '{}': planning/state.json not found (path: '{}')",
                    repo.slug, repo.repo_path
                ),
            ));
        }
    }

    (sources, diags)
}

// ---------------------------------------------------------------------------
// Schema-ring checks
// ---------------------------------------------------------------------------

/// Valid `status` values for `focus.now` and `focus.blocked` block entries.
const VALID_STATUSES: &[&str] = &["open", "in_progress", "blocked", "closed"];

/// Validate the schema-ring constraints for a successfully-deserialized
/// [`StateFile`].
///
/// Checks performed (all against the deserialized model — JSON structural
/// errors are already surfaced as [`StateLoadError::Parse`] before this
/// function is called):
///
/// 1. **`kind` membership** (`E_STATE_SCHEMA_BAD_KIND`) — `kind` must be
///    `"project"` or `"brain"`.  Also flags if `kind` disagrees with the
///    source's `expected_kind`.
/// 2. **`updated` non-empty** (`E_STATE_SCHEMA_MISSING_FIELD`) — the
///    `updated` string must not be blank (format checked by `MV.3.M`).
/// 3. **`status` enum** (`E_STATE_SCHEMA_BAD_STATUS`) — every `focus.now`
///    and `focus.blocked` entry whose `status` field is present must hold a
///    value in `{open, in_progress, blocked, closed}`.
/// 4. **`blocked_by` well-formedness** (`E_STATE_SCHEMA_BAD_BLOCKED_BY`) —
///    a `{type:"block"}` entry must have non-empty `repo` and `id`.
/// 5. **Kind-appropriate sections** (`E_STATE_SCHEMA_MISSING_FIELD`, warning)
///    — a `project` file is expected to carry `tracks[]`; a `brain` file is
///    expected to carry `repos[]`.
pub fn check_schema(src: &StateSource, file: &StateFile) -> Vec<Diagnostic> {
    let mut diags: Vec<Diagnostic> = Vec::new();
    let path = &src.abs_path;

    // --- 1. kind membership ---
    match file.kind.as_str() {
        "project" | "brain" => {
            // Valid — also check it matches the source's expected kind.
            if file.kind != src.expected_kind {
                diags.push(Diagnostic::error(
                    path,
                    "E_STATE_SCHEMA_BAD_KIND",
                    format!(
                        "kind '{}' does not match expected '{}' for this source",
                        file.kind, src.expected_kind
                    ),
                ));
            }
        }
        other => {
            diags.push(Diagnostic::error(
                path,
                "E_STATE_SCHEMA_BAD_KIND",
                format!("kind '{other}' is not valid; expected 'project' or 'brain'"),
            ));
        }
    }

    // --- 2. updated non-empty ---
    if file.updated.trim().is_empty() {
        diags.push(Diagnostic::error(
            path,
            "E_STATE_SCHEMA_MISSING_FIELD",
            "required field 'updated' is present but empty".to_string(),
        ));
    }

    // --- 3. status enum + 4. blocked_by well-formedness ---
    // Check all focus collections that carry status or blocked_by.
    for block in file.focus.now.iter().chain(file.focus.blocked.iter()) {
        // status enum
        if let Some(status) = &block.status
            && !VALID_STATUSES.contains(&status.as_str())
        {
            diags.push(Diagnostic::error(
                path,
                "E_STATE_SCHEMA_BAD_STATUS",
                format!(
                    "block '{}' has invalid status '{}'; expected one of: {}",
                    block.block,
                    status,
                    VALID_STATUSES.join(", ")
                ),
            ));
        }

        // blocked_by well-formedness
        for bb in &block.blocked_by {
            if let BlockedBy::Block { repo, id, .. } = bb {
                let repo_empty = repo.trim().is_empty();
                let id_empty = id.trim().is_empty();
                if repo_empty || id_empty {
                    diags.push(Diagnostic::error(
                        path,
                        "E_STATE_SCHEMA_BAD_BLOCKED_BY",
                        format!(
                            "blocked_by entry in block '{}' is missing required \
                             field(s): {}",
                            block.block,
                            [repo_empty.then_some("'repo'"), id_empty.then_some("'id'")]
                                .into_iter()
                                .flatten()
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    ));
                }
            }
        }
    }

    // --- 5. kind-appropriate sections (warnings) ---
    if file.kind == "project" && file.tracks.is_empty() {
        diags.push(Diagnostic::warning(
            path,
            "E_STATE_SCHEMA_MISSING_FIELD",
            "project state.json is missing 'tracks[]'; expected a roadmap catalog".to_string(),
        ));
    }
    if file.kind == "brain" && file.repos.is_empty() {
        diags.push(Diagnostic::warning(
            path,
            "E_STATE_SCHEMA_MISSING_FIELD",
            "brain state.json is missing 'repos[]'; expected a child-repo rollup".to_string(),
        ));
    }

    diags
}

// ---------------------------------------------------------------------------
// State graph model (D4 serializable, emittable artifact)
// ---------------------------------------------------------------------------

/// The kind of a directed edge in the state block graph.
///
/// `BlockedBy` edges come from `focus.[].blocked_by[]{type:"block"}` entries.
/// `CrossRepo` edges come from brain-file `cross_repo[]` arrays.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StateEdgeKind {
    /// A `blocked_by` dependency (a block is waiting on another block).
    BlockedBy,
    /// An explicit cross-repo dependency declared in a brain file's `cross_repo[]`.
    CrossRepo,
}

/// A directed edge in the state block graph.
///
/// `from` and `to_ref` are canonical `"repo:id"` keys.  `source_path` is
/// recorded for diagnostic generation but **skipped** in serialization (it is
/// an implementation detail, not part of the D4 artifact).
#[derive(Debug, Clone, Serialize)]
pub struct StateEdge {
    /// `"repo:id"` key of the source block (the dependent / blocked block).
    pub from: String,
    /// `"repo:id"` key of the target block (the dependency / blocker).
    pub to_ref: String,
    /// Edge discriminant.
    pub kind: StateEdgeKind,
    /// Absolute path of the file that authored this edge (skipped in JSON).
    #[serde(skip)]
    pub source_path: PathBuf,
}

/// A graph node — a block registered in a repo's `tracks[]`.
///
/// `source_path` is skipped in serialization for the same reason as
/// [`StateEdge::source_path`].
#[derive(Debug, Clone, Serialize)]
pub struct StateNode {
    /// Canonical key: `"repo:id"`.
    pub key: String,
    /// Repo slug that owns this block.
    pub repo: String,
    /// Canonical block ID (e.g. `"MV.3.P"`).
    pub id: String,
    /// Brief human description.
    pub title: String,
    /// Absolute path of the file that registered this block (skipped in JSON).
    #[serde(skip)]
    pub source_path: PathBuf,
}

/// The serializable, emittable state block graph — the D4 artifact.
///
/// Produced by [`build_state_graph`]; consumed (read-only) by
/// [`check_state_graph`].  The graph is authored-only — no node or edge is
/// inferred.
#[derive(Debug, Default, Serialize)]
pub struct StateGraph {
    /// All blocks registered in any repo's `tracks[]`.
    pub nodes: Vec<StateNode>,
    /// All `blocked_by` block edges and brain `cross_repo[]` edges.
    pub edges: Vec<StateEdge>,
}

// ---------------------------------------------------------------------------
// Graph builder
// ---------------------------------------------------------------------------

/// Build a [`StateGraph`] from the loaded state files.
///
/// # Nodes
/// One [`StateNode`] per `tracks[].blocks[]` entry across all files (keyed
/// `"repo:id"`).
///
/// # Edges
/// - One [`StateEdge`] with `kind: BlockedBy` per `{type:"block"}` entry
///   in any file's `focus.*.blocked_by[]`.  The `from` key is the blocked
///   block's own `"repo:id"` (using the `block.repo` field if present, else
///   the owning file's slug); `to_ref` is `"{blocked_by.repo}:{blocked_by.id}"`.
/// - One [`StateEdge`] with `kind: CrossRepo` per brain-file `cross_repo[]`
///   entry.
///
/// Nodes and edges are emitted even when they are later found to be dangling
/// — the separation of build and check is intentional (see `MV.3.J` pattern).
pub fn build_state_graph(files: &[(StateSource, StateFile)]) -> StateGraph {
    let mut nodes: Vec<StateNode> = Vec::new();
    let mut edges: Vec<StateEdge> = Vec::new();

    for (src, file) in files {
        let path = &src.abs_path;

        // --- Nodes: one per tracks[].blocks[] entry ---
        for track in &file.tracks {
            for block in &track.blocks {
                nodes.push(StateNode {
                    key: format!("{}:{}", src.repo_slug, block.id),
                    repo: src.repo_slug.clone(),
                    id: block.id.clone(),
                    title: block.title.clone(),
                    source_path: path.clone(),
                });
            }
        }

        // --- BlockedBy edges: from focus.*.blocked_by[]{type:block} ---
        let all_focus = file
            .focus
            .now
            .iter()
            .chain(file.focus.next.iter())
            .chain(file.focus.blocked.iter());

        for block in all_focus {
            // The `from` block may live in a child repo (brain focus carries `repo`).
            let from_repo = block.repo.as_deref().unwrap_or(src.repo_slug.as_str());
            let from_key = format!("{}:{}", from_repo, block.block);

            for bb in &block.blocked_by {
                if let BlockedBy::Block { repo, id, .. } = bb {
                    edges.push(StateEdge {
                        from: from_key.clone(),
                        to_ref: format!("{repo}:{id}"),
                        kind: StateEdgeKind::BlockedBy,
                        source_path: path.clone(),
                    });
                }
            }
        }

        // --- CrossRepo edges: from brain cross_repo[] ---
        for edge in &file.cross_repo {
            edges.push(StateEdge {
                from: format!("{}:{}", edge.from.repo, edge.from.block),
                to_ref: format!("{}:{}", edge.to.repo, edge.to.block),
                kind: StateEdgeKind::CrossRepo,
                source_path: path.clone(),
            });
        }
    }

    StateGraph { nodes, edges }
}

// ---------------------------------------------------------------------------
// Graph integrity checks
// ---------------------------------------------------------------------------

/// Run integrity checks over a built [`StateGraph`].
///
/// Checks performed:
///
/// 1. **`E_STATE_DUPLICATE_BLOCK_ID`** — two `tracks[]` blocks in the same
///    repo share an `id`.  Emits one diagnostic per duplicate key (not one
///    per occurrence).
/// 2. **`E_STATE_DANGLING_FOCUS`** — a leaf (`kind:"project"`) file's
///    `focus.now/next/blocked` entry's `block` is absent from that repo's
///    `tracks[]`.  Brain focus entries are cross-repo and intentionally
///    excluded from this check.
/// 3. **`E_STATE_UNKNOWN_REPO`** — a `blocked_by` or `cross_repo` edge
///    references a `repo` that has no discoverable `state.json` (i.e. is not
///    in `files`).
/// 4. **`E_STATE_DANGLING_BLOCKED_BY`** — a `{type:"block",repo,id}` edge's
///    target `id` does not exist in the named repo's `tracks[]` (repo is
///    known, block is not).
/// 5. **`E_STATE_DANGLING_CROSS_REPO`** — a brain `cross_repo[]` edge's
///    `from` or `to` endpoint's block does not exist in the named repo's
///    `tracks[]` (repo is known, block is not).
pub fn check_state_graph(
    graph: &StateGraph,
    files: &[(StateSource, StateFile)],
) -> Vec<Diagnostic> {
    use std::collections::{HashMap, HashSet};

    let mut diags: Vec<Diagnostic> = Vec::new();

    // --- Build lookup structures ---

    // All repo slugs that have a loaded state.json.
    let known_repos: HashSet<&str> = files.iter().map(|(s, _)| s.repo_slug.as_str()).collect();

    // Count occurrences of each "repo:id" key so we can detect duplicates.
    // Also store the path of the *last* occurrence for the diagnostic.
    let mut node_counts: HashMap<&str, (usize, &PathBuf)> = HashMap::new();
    for node in &graph.nodes {
        let entry = node_counts
            .entry(node.key.as_str())
            .or_insert((0, &node.source_path));
        entry.0 += 1;
        entry.1 = &node.source_path;
    }

    // Set of all registered "repo:id" keys (for dangling checks).
    let node_set: HashSet<&str> = node_counts.keys().copied().collect();

    // --- 1. Duplicate block IDs ---
    let mut duplicate_reported: HashSet<&str> = HashSet::new();
    for node in &graph.nodes {
        let key = node.key.as_str();
        if node_counts[key].0 > 1 && !duplicate_reported.contains(key) {
            duplicate_reported.insert(key);
            diags.push(Diagnostic::error(
                &node.source_path,
                "E_STATE_DUPLICATE_BLOCK_ID",
                format!(
                    "duplicate block id '{}' in repo '{}' tracks[]",
                    node.id, node.repo
                ),
            ));
        }
    }

    // --- 2. Dangling focus (leaf files only) ---
    for (src, file) in files {
        if file.kind != "project" {
            continue;
        }
        let path = &src.abs_path;
        let all_focus = file
            .focus
            .now
            .iter()
            .chain(file.focus.next.iter())
            .chain(file.focus.blocked.iter());

        for block in all_focus {
            let key = format!("{}:{}", src.repo_slug, block.block);
            if !node_set.contains(key.as_str()) {
                diags.push(Diagnostic::error(
                    path,
                    "E_STATE_DANGLING_FOCUS",
                    format!(
                        "focus block '{}' is not registered in this repo's tracks[]",
                        block.block
                    ),
                ));
            }
        }
    }

    // --- 3–5. Edge integrity checks ---
    for edge in &graph.edges {
        let path = &edge.source_path;

        match edge.kind {
            StateEdgeKind::BlockedBy => {
                // Parse to_ref as "repo:id".
                let Some((to_repo, to_id)) = edge.to_ref.split_once(':') else {
                    continue;
                };
                if !known_repos.contains(to_repo) {
                    diags.push(Diagnostic::error(
                        path,
                        "E_STATE_UNKNOWN_REPO",
                        format!("blocked_by references unknown repo '{to_repo}'"),
                    ));
                } else if !node_set.contains(edge.to_ref.as_str()) {
                    diags.push(Diagnostic::error(
                        path,
                        "E_STATE_DANGLING_BLOCKED_BY",
                        format!(
                            "blocked_by block '{to_id}' does not exist in repo '{to_repo}' tracks[]"
                        ),
                    ));
                }
            }

            StateEdgeKind::CrossRepo => {
                // Check from endpoint.
                let Some((from_repo, from_id)) = edge.from.split_once(':') else {
                    continue;
                };
                if !known_repos.contains(from_repo) {
                    diags.push(Diagnostic::error(
                        path,
                        "E_STATE_UNKNOWN_REPO",
                        format!("cross_repo 'from' references unknown repo '{from_repo}'"),
                    ));
                } else if !node_set.contains(edge.from.as_str()) {
                    diags.push(Diagnostic::error(
                        path,
                        "E_STATE_DANGLING_CROSS_REPO",
                        format!(
                            "cross_repo 'from' block '{from_id}' does not exist in repo \
                             '{from_repo}' tracks[]"
                        ),
                    ));
                }

                // Check to endpoint.
                let Some((to_repo, to_id)) = edge.to_ref.split_once(':') else {
                    continue;
                };
                if !known_repos.contains(to_repo) {
                    diags.push(Diagnostic::error(
                        path,
                        "E_STATE_UNKNOWN_REPO",
                        format!("cross_repo 'to' references unknown repo '{to_repo}'"),
                    ));
                } else if !node_set.contains(edge.to_ref.as_str()) {
                    diags.push(Diagnostic::error(
                        path,
                        "E_STATE_DANGLING_CROSS_REPO",
                        format!(
                            "cross_repo 'to' block '{to_id}' does not exist in repo \
                             '{to_repo}' tracks[]"
                        ),
                    ));
                }
            }
        }
    }

    diags
}

// ---------------------------------------------------------------------------
// Rollup-drift check (brain files)
// ---------------------------------------------------------------------------

/// Compare a brain `repos[]` headline cache against the children's actual `focus`.
///
/// For each entry in `brain.repos[]`, looks up the child's actual state in
/// `children` (keyed by repo slug).  If the cached `now`/`next`/`blocked`
/// block-id sets don't match the child's live `focus`, emits one
/// `W_STATE_ROLLUP_DRIFT` warning per drifted child.
///
/// Children absent from `children` (no loaded state.json) are silently
/// skipped — `W_STATE_FILE_MISSING` was already emitted during discovery.
///
/// Per scoping decision 4, rollup drift is a **warning** (exit 0); only
/// referential errors are `Error`-severity (exit 1).
///
/// Comparison is over block-id sets only; cosmetic field differences
/// (`title`, `note`) are intentionally ignored.
pub fn check_rollup(
    brain_path: &Path,
    brain: &StateFile,
    children: &std::collections::HashMap<String, StateFile>,
) -> Vec<Diagnostic> {
    use std::collections::HashSet;

    let mut diags: Vec<Diagnostic> = Vec::new();

    for rollup in &brain.repos {
        let child = match children.get(&rollup.repo) {
            Some(c) => c,
            // No state.json loaded for this child — W_STATE_FILE_MISSING was
            // already emitted during discovery; skip silently here.
            None => continue,
        };

        // Compare block-id sets; ignore title/note cosmetic differences.
        let cached_now: HashSet<&str> = rollup.now.iter().map(|b| b.block.as_str()).collect();
        let actual_now: HashSet<&str> = child.focus.now.iter().map(|b| b.block.as_str()).collect();

        let cached_next: HashSet<&str> = rollup.next.iter().map(|b| b.block.as_str()).collect();
        let actual_next: HashSet<&str> =
            child.focus.next.iter().map(|b| b.block.as_str()).collect();

        let cached_blocked: HashSet<&str> =
            rollup.blocked.iter().map(|b| b.block.as_str()).collect();
        let actual_blocked: HashSet<&str> = child
            .focus
            .blocked
            .iter()
            .map(|b| b.block.as_str())
            .collect();

        if cached_now != actual_now
            || cached_next != actual_next
            || cached_blocked != actual_blocked
        {
            // Build a compact diff summary for the warning message.
            let mut diffs: Vec<String> = Vec::new();
            if cached_now != actual_now {
                diffs.push(format!(
                    "now: cached={:?} actual={:?}",
                    sorted_set(&cached_now),
                    sorted_set(&actual_now)
                ));
            }
            if cached_next != actual_next {
                diffs.push(format!(
                    "next: cached={:?} actual={:?}",
                    sorted_set(&cached_next),
                    sorted_set(&actual_next)
                ));
            }
            if cached_blocked != actual_blocked {
                diffs.push(format!(
                    "blocked: cached={:?} actual={:?}",
                    sorted_set(&cached_blocked),
                    sorted_set(&actual_blocked)
                ));
            }

            diags.push(Diagnostic::warning(
                brain_path,
                "W_STATE_ROLLUP_DRIFT",
                format!(
                    "repos[] entry for '{}' has drifted from child's actual focus — {}",
                    rollup.repo,
                    diffs.join("; ")
                ),
            ));
        }
    }

    diags
}

/// Return a deterministically sorted `Vec<&str>` from a `HashSet<&str>` for
/// use in diagnostic messages (avoids non-deterministic output from Set debug).
fn sorted_set<'a>(set: &std::collections::HashSet<&'a str>) -> Vec<&'a str> {
    let mut v: Vec<&str> = set.iter().copied().collect();
    v.sort_unstable();
    v
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Minimal fixture strings (representative of the five live state.json files)
    // -----------------------------------------------------------------------

    /// Minimal leaf state.json (mev / bastion / orchestrator shape).
    fn leaf_json(repo: &str) -> String {
        format!(
            r#"{{
  "repo": "{repo}",
  "kind": "project",
  "updated": "2026-06-29",
  "focus": {{
    "now": [
      {{ "block": "MV.3.J", "title": "Graph integrity", "status": "closed" }}
    ],
    "next": [
      {{ "block": "MV.3.K", "title": "Link integrity" }}
    ],
    "blocked": []
  }},
  "tracks": [
    {{
      "title": "Phase 3",
      "blocks": [
        {{ "id": "MV.3.J", "title": "Graph integrity", "status": "closed" }},
        {{ "id": "MV.3.K", "title": "Link integrity", "status": "open" }}
      ]
    }}
  ]
}}"#
        )
    }

    /// Minimal core-brain state.json shape.
    fn core_brain_json() -> &'static str {
        r#"{
  "repo": "core",
  "kind": "brain",
  "updated": "2026-06-29",
  "focus": {
    "now": [
      { "block": "BA.11.C0", "repo": "bastion", "title": "Manifest engine", "status": "in_progress" }
    ],
    "next": [
      { "block": "BA.11.C", "repo": "bastion", "title": "WebSocket hub" }
    ],
    "blocked": []
  },
  "repos": [
    {
      "repo": "bastion",
      "now": [{ "block": "BA.11.C0", "title": "Manifest engine", "status": "in_progress" }],
      "next": [{ "block": "BA.11.C", "title": "WebSocket hub" }],
      "blocked": []
    }
  ],
  "cross_repo": [
    {
      "from": { "repo": "bastion-ui", "block": "BU.1.A" },
      "to": { "repo": "bastion", "block": "BA.11.C" },
      "note": "Session-control screens need the WS hub."
    }
  ]
}"#
    }

    /// Minimal HQ brain state.json shape (adds `tiers[]` and `note`).
    fn hq_brain_json() -> &'static str {
        r#"{
  "repo": "hq",
  "kind": "brain",
  "updated": "2026-06-29",
  "note": "Top company brain.",
  "focus": {
    "now": [
      { "block": "BA.11.C0", "repo": "bastion", "title": "Manifest engine", "status": "in_progress" }
    ],
    "next": [],
    "blocked": [
      {
        "block": "OR.B",
        "repo": "orchestrator",
        "title": "Semantic Brain Q&A",
        "blocked_by": [
          { "type": "external", "what": "At-home Ollama session" }
        ]
      }
    ]
  },
  "repos": [
    {
      "repo": "bastion",
      "tier": "core",
      "now": [{ "block": "BA.11.C0", "title": "Manifest engine", "status": "in_progress" }],
      "next": [],
      "blocked": []
    }
  ],
  "cross_repo": [],
  "tiers": [
    { "tier": "core", "rollup": "core", "summary": "Primary program — Bastion + tooling." }
  ]
}"#
    }

    // -----------------------------------------------------------------------
    // Deserialize clean: three leaf shapes
    // -----------------------------------------------------------------------

    #[test]
    fn leaf_mev_shape_deserializes_clean() {
        let json = leaf_json("mev");
        let file: StateFile = serde_json::from_str(&json).expect("mev leaf should deserialize");
        assert_eq!(file.repo, "mev");
        assert_eq!(file.kind, "project");
        assert_eq!(file.focus.now.len(), 1);
        assert_eq!(file.focus.next.len(), 1);
        assert_eq!(file.tracks.len(), 1);
        assert_eq!(file.tracks[0].blocks.len(), 2);
    }

    #[test]
    fn leaf_bastion_shape_deserializes_clean() {
        let json = leaf_json("bastion");
        let file: StateFile = serde_json::from_str(&json).expect("bastion leaf should deserialize");
        assert_eq!(file.repo, "bastion");
        assert_eq!(file.kind, "project");
    }

    #[test]
    fn leaf_orchestrator_shape_deserializes_clean() {
        let json = leaf_json("orchestrator");
        let file: StateFile =
            serde_json::from_str(&json).expect("orchestrator leaf should deserialize");
        assert_eq!(file.repo, "orchestrator");
        assert_eq!(file.kind, "project");
    }

    // -----------------------------------------------------------------------
    // Deserialize clean: two brain shapes
    // -----------------------------------------------------------------------

    #[test]
    fn core_brain_shape_deserializes_clean() {
        let file: StateFile =
            serde_json::from_str(core_brain_json()).expect("core brain should deserialize");
        assert_eq!(file.repo, "core");
        assert_eq!(file.kind, "brain");
        assert_eq!(file.repos.len(), 1);
        assert_eq!(file.cross_repo.len(), 1);
        assert!(file.tiers.is_empty());
    }

    #[test]
    fn hq_brain_shape_deserializes_clean() {
        let file: StateFile =
            serde_json::from_str(hq_brain_json()).expect("HQ brain should deserialize");
        assert_eq!(file.repo, "hq");
        assert_eq!(file.kind, "brain");
        assert_eq!(file.tiers.len(), 1);
        assert_eq!(file.tiers[0].tier, "core");
        assert_eq!(file.note.as_deref(), Some("Top company brain."));
    }

    // -----------------------------------------------------------------------
    // BlockedBy variants
    // -----------------------------------------------------------------------

    #[test]
    fn blocked_by_block_type_deserializes() {
        let json =
            r#"{ "type": "block", "repo": "bastion", "id": "BA.11.C", "what": "needs WS hub" }"#;
        let bb: BlockedBy = serde_json::from_str(json).expect("block type should deserialize");
        match bb {
            BlockedBy::Block { repo, id, what } => {
                assert_eq!(repo, "bastion");
                assert_eq!(id, "BA.11.C");
                assert_eq!(what.as_deref(), Some("needs WS hub"));
            }
            _ => panic!("expected BlockedBy::Block"),
        }
    }

    #[test]
    fn blocked_by_external_type_deserializes() {
        let json = r#"{ "type": "external", "what": "At-home Mac Mini session" }"#;
        let bb: BlockedBy = serde_json::from_str(json).expect("external type should deserialize");
        match bb {
            BlockedBy::External { what } => {
                assert_eq!(what, "At-home Mac Mini session");
            }
            _ => panic!("expected BlockedBy::External"),
        }
    }

    #[test]
    fn blocked_by_unknown_type_is_rejected() {
        let json = r#"{ "type": "unknown_type", "what": "some value" }"#;
        let result = serde_json::from_str::<BlockedBy>(json);
        assert!(
            result.is_err(),
            "unknown blocked_by type should be rejected by serde, got: {result:?}"
        );
    }

    // -----------------------------------------------------------------------
    // load_state function tests (temp files)
    // -----------------------------------------------------------------------

    #[test]
    fn load_state_returns_parsed_file_for_valid_json() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        std::fs::write(&path, leaf_json("mev")).unwrap();

        let file = load_state(&path).expect("load_state should succeed");
        assert_eq!(file.repo, "mev");
    }

    #[test]
    fn load_state_surfaces_parse_error_for_malformed_json() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        std::fs::write(&path, b"{ not valid json }").unwrap();

        let result = load_state(&path);
        assert!(
            matches!(result, Err(StateLoadError::Parse { .. })),
            "malformed JSON should produce StateLoadError::Parse, got: {result:?}"
        );
    }

    #[test]
    fn load_state_surfaces_io_error_for_missing_file() {
        let path = std::path::PathBuf::from("/nonexistent/path/state.json");
        let result = load_state(&path);
        assert!(
            matches!(result, Err(StateLoadError::Io { .. })),
            "missing file should produce StateLoadError::Io, got: {result:?}"
        );
    }

    #[test]
    fn load_state_rejects_unknown_blocked_by_type_in_full_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        // A state.json with an unknown blocked_by type in a focus.blocked entry.
        let json = r#"{
  "repo": "test",
  "kind": "project",
  "updated": "2026-06-29",
  "focus": {
    "now": [],
    "next": [],
    "blocked": [
      {
        "block": "TEST.1",
        "title": "Some block",
        "blocked_by": [
          { "type": "mystery_type", "what": "unknown" }
        ]
      }
    ]
  }
}"#;
        std::fs::write(&path, json).unwrap();
        let result = load_state(&path);
        assert!(
            matches!(result, Err(StateLoadError::Parse { .. })),
            "unknown blocked_by type in full file should produce StateLoadError::Parse, got: {result:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Focus defaults
    // -----------------------------------------------------------------------

    #[test]
    fn focus_defaults_to_empty_collections() {
        // A minimal file with no focus sub-fields.
        let json = r#"{
  "repo": "test",
  "kind": "project",
  "updated": "2026-06-29",
  "focus": {}
}"#;
        let file: StateFile = serde_json::from_str(json).expect("should deserialize");
        assert!(file.focus.now.is_empty());
        assert!(file.focus.next.is_empty());
        assert!(file.focus.blocked.is_empty());
    }

    // -----------------------------------------------------------------------
    // Optional collections default to empty
    // -----------------------------------------------------------------------

    #[test]
    fn optional_top_level_collections_default_to_empty() {
        // Minimal file with none of the optional arrays.
        let json = r#"{
  "repo": "test",
  "kind": "project",
  "updated": "2026-06-29"
}"#;
        let file: StateFile = serde_json::from_str(json).expect("should deserialize");
        assert!(file.tracks.is_empty());
        assert!(file.repos.is_empty());
        assert!(file.cross_repo.is_empty());
        assert!(file.tiers.is_empty());
        assert!(file.note.is_none());
    }

    // -----------------------------------------------------------------------
    // Task 2 — discover_state_files tests
    // -----------------------------------------------------------------------

    /// Build a minimal HQ temp dir with two leaf repos and a brain state.json.
    fn build_hq_fixture(dir: &std::path::Path) {
        // HQ brain planning/state.json
        let hq_planning = std::path::Path::new(dir).join("planning");
        std::fs::create_dir_all(&hq_planning).unwrap();
        let hq_state = serde_json::json!({
            "repo": "hq",
            "kind": "brain",
            "updated": "2026-06-29",
            "focus": { "now": [], "next": [], "blocked": [] },
            "repos": [
                { "repo": "alpha", "now": [], "next": [], "blocked": [] }
            ],
            "tiers": [
                { "tier": "core", "rollup": "core/planning/state.json" }
            ]
        });
        std::fs::write(
            hq_planning.join("state.json"),
            serde_json::to_string_pretty(&hq_state).unwrap(),
        )
        .unwrap();

        // Tier sub-brain: core/planning/state.json
        let core_planning = dir.join("core").join("planning");
        std::fs::create_dir_all(&core_planning).unwrap();
        let core_state = serde_json::json!({
            "repo": "core",
            "kind": "brain",
            "updated": "2026-06-29",
            "focus": { "now": [], "next": [], "blocked": [] },
            "repos": [{ "repo": "alpha", "now": [], "next": [], "blocked": [] }]
        });
        std::fs::write(
            core_planning.join("state.json"),
            serde_json::to_string_pretty(&core_state).unwrap(),
        )
        .unwrap();

        // Leaf repo alpha: core/alpha/planning/state.json
        let alpha_planning = dir.join("core").join("alpha").join("planning");
        std::fs::create_dir_all(&alpha_planning).unwrap();
        let alpha_state = serde_json::json!({
            "repo": "alpha",
            "kind": "project",
            "updated": "2026-06-29",
            "focus": { "now": [], "next": [], "blocked": [] },
            "tracks": [{ "title": "Phase 1", "blocks": [{ "id": "AL.1.A", "title": "Start" }] }]
        });
        std::fs::write(
            alpha_planning.join("state.json"),
            serde_json::to_string_pretty(&alpha_state).unwrap(),
        )
        .unwrap();
    }

    fn make_config_with_alpha(alpha_repo_path: &str) -> BrainConfig {
        use crate::brain::config::{BrainConfig, CrawlConfig, RepoEntry, VocabConfig};
        BrainConfig {
            vocab: VocabConfig::default(),
            crawl: CrawlConfig::default(),
            repos: vec![
                RepoEntry {
                    slug: "brain".to_string(),
                    tier: "_root".to_string(),
                    repo_path: ".".to_string(),
                    status_file: String::new(),
                    cache_doc: String::new(),
                    heading: String::new(),
                },
                RepoEntry {
                    slug: "alpha".to_string(),
                    tier: "core".to_string(),
                    repo_path: alpha_repo_path.to_string(),
                    status_file: String::new(),
                    cache_doc: String::new(),
                    heading: String::new(),
                },
            ],
        }
    }

    #[test]
    fn discover_finds_hq_tier_and_leaf_sources() {
        let dir = tempfile::tempdir().expect("tempdir");
        build_hq_fixture(dir.path());
        let config = make_config_with_alpha("core/alpha");

        let (sources, diags) = discover_state_files(dir.path(), &config);

        // 0 warnings expected (all files exist)
        assert!(
            diags.is_empty(),
            "expected no diagnostics for complete fixture, got: {diags:?}"
        );

        // Expect 3 sources: HQ brain, core tier brain, alpha leaf
        assert_eq!(
            sources.len(),
            3,
            "expected 3 sources (hq, core, alpha), got: {sources:?}"
        );

        let slugs: Vec<&str> = sources.iter().map(|s| s.repo_slug.as_str()).collect();
        assert!(
            slugs.contains(&"brain"),
            "expected hq source with slug 'brain'"
        );
        assert!(
            slugs.contains(&"core"),
            "expected tier source with slug 'core'"
        );
        assert!(
            slugs.contains(&"alpha"),
            "expected leaf source with slug 'alpha'"
        );

        // Verify expected_kinds
        for src in &sources {
            match src.repo_slug.as_str() {
                "brain" | "core" => assert_eq!(src.expected_kind, "brain"),
                "alpha" => assert_eq!(src.expected_kind, "project"),
                _ => panic!("unexpected slug: {}", src.repo_slug),
            }
        }
    }

    #[test]
    fn discover_emits_missing_warning_for_absent_leaf() {
        let dir = tempfile::tempdir().expect("tempdir");
        build_hq_fixture(dir.path());
        // Add a second repo entry that has no state.json on disk
        use crate::brain::config::{BrainConfig, CrawlConfig, RepoEntry, VocabConfig};
        let config = BrainConfig {
            vocab: VocabConfig::default(),
            crawl: CrawlConfig::default(),
            repos: vec![
                RepoEntry {
                    slug: "brain".to_string(),
                    tier: "_root".to_string(),
                    repo_path: ".".to_string(),
                    status_file: String::new(),
                    cache_doc: String::new(),
                    heading: String::new(),
                },
                RepoEntry {
                    slug: "alpha".to_string(),
                    tier: "core".to_string(),
                    repo_path: "core/alpha".to_string(),
                    status_file: String::new(),
                    cache_doc: String::new(),
                    heading: String::new(),
                },
                RepoEntry {
                    slug: "missing-repo".to_string(),
                    tier: "core".to_string(),
                    repo_path: "core/missing-repo".to_string(),
                    status_file: String::new(),
                    cache_doc: String::new(),
                    heading: String::new(),
                },
            ],
        };

        let (sources, diags) = discover_state_files(dir.path(), &config);

        // One W_STATE_FILE_MISSING for the missing leaf
        let missing_diags: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "W_STATE_FILE_MISSING")
            .collect();
        assert_eq!(
            missing_diags.len(),
            1,
            "expected exactly one W_STATE_FILE_MISSING, got: {diags:?}"
        );

        // missing-repo is NOT in sources
        assert!(
            !sources.iter().any(|s| s.repo_slug == "missing-repo"),
            "missing-repo should not appear in sources"
        );
    }

    #[test]
    fn discover_emits_missing_warning_for_absent_tier_sub_brain() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Build HQ with a tier that has no actual state.json file on disk
        let hq_planning = dir.path().join("planning");
        std::fs::create_dir_all(&hq_planning).unwrap();
        let hq_state = serde_json::json!({
            "repo": "hq",
            "kind": "brain",
            "updated": "2026-06-29",
            "focus": { "now": [], "next": [], "blocked": [] },
            "repos": [],
            "tiers": [
                { "tier": "ghost-tier", "rollup": "ghost/planning/state.json" }
            ]
        });
        std::fs::write(
            hq_planning.join("state.json"),
            serde_json::to_string_pretty(&hq_state).unwrap(),
        )
        .unwrap();

        let config = BrainConfig::default();
        let (sources, diags) = discover_state_files(dir.path(), &config);

        let missing: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "W_STATE_FILE_MISSING")
            .collect();
        assert_eq!(
            missing.len(),
            1,
            "expected one W_STATE_FILE_MISSING for ghost-tier, got: {diags:?}"
        );

        // HQ itself is in sources
        assert_eq!(sources.len(), 1, "only HQ brain should be in sources");
    }

    // -----------------------------------------------------------------------
    // Task 2 — check_schema tests
    // -----------------------------------------------------------------------

    fn make_source(path: &std::path::Path, kind: &'static str) -> StateSource {
        StateSource {
            repo_slug: "test".to_string(),
            abs_path: path.to_path_buf(),
            expected_kind: kind,
        }
    }

    fn parse_file(json: &str) -> StateFile {
        serde_json::from_str(json).expect("fixture must parse")
    }

    #[test]
    fn check_schema_clean_leaf_emits_no_errors() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        let src = make_source(&path, "project");
        let file = parse_file(&leaf_json("mev"));

        let diags = check_schema(&src, &file);
        let errors: Vec<_> = diags
            .iter()
            .filter(|d| d.severity == crate::Severity::Error)
            .collect();
        assert!(
            errors.is_empty(),
            "clean leaf should produce no errors, got: {diags:?}"
        );
    }

    #[test]
    fn check_schema_bad_kind_emits_e_state_schema_bad_kind() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        let src = make_source(&path, "project");

        let json = r#"{
  "repo": "test",
  "kind": "invalid_kind",
  "updated": "2026-06-29",
  "focus": { "now": [], "next": [], "blocked": [] },
  "tracks": [{ "title": "P1", "blocks": [{ "id": "T.1", "title": "X" }] }]
}"#;
        let file = parse_file(json);
        let diags = check_schema(&src, &file);

        let bad_kind: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_SCHEMA_BAD_KIND")
            .collect();
        assert_eq!(
            bad_kind.len(),
            1,
            "expected exactly one E_STATE_SCHEMA_BAD_KIND, got: {diags:?}"
        );
    }

    #[test]
    fn check_schema_bad_status_emits_e_state_schema_bad_status() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        let src = make_source(&path, "project");

        let json = r#"{
  "repo": "test",
  "kind": "project",
  "updated": "2026-06-29",
  "focus": {
    "now": [
      { "block": "T.1", "title": "Work", "status": "flying" }
    ],
    "next": [],
    "blocked": []
  },
  "tracks": [{ "title": "P1", "blocks": [{ "id": "T.1", "title": "Work" }] }]
}"#;
        let file = parse_file(json);
        let diags = check_schema(&src, &file);

        let bad_status: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_SCHEMA_BAD_STATUS")
            .collect();
        assert_eq!(
            bad_status.len(),
            1,
            "expected exactly one E_STATE_SCHEMA_BAD_STATUS, got: {diags:?}"
        );
    }

    #[test]
    fn check_schema_blocked_by_empty_id_emits_e_state_schema_bad_blocked_by() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        let src = make_source(&path, "project");

        // blocked_by entry with type "block" but empty id — parses fine, fails schema check
        let json = r#"{
  "repo": "test",
  "kind": "project",
  "updated": "2026-06-29",
  "focus": {
    "now": [],
    "next": [],
    "blocked": [
      {
        "block": "T.1",
        "title": "Blocked block",
        "blocked_by": [
          { "type": "block", "repo": "other", "id": "" }
        ]
      }
    ]
  },
  "tracks": [{ "title": "P1", "blocks": [{ "id": "T.1", "title": "Blocked block" }] }]
}"#;
        let file = parse_file(json);
        let diags = check_schema(&src, &file);

        let bad_bb: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_SCHEMA_BAD_BLOCKED_BY")
            .collect();
        assert_eq!(
            bad_bb.len(),
            1,
            "expected exactly one E_STATE_SCHEMA_BAD_BLOCKED_BY for empty id, got: {diags:?}"
        );
    }

    #[test]
    fn check_schema_brain_clean_emits_no_errors() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        let src = make_source(&path, "brain");
        let file = parse_file(core_brain_json());

        let diags = check_schema(&src, &file);
        let errors: Vec<_> = diags
            .iter()
            .filter(|d| d.severity == crate::Severity::Error)
            .collect();
        assert!(
            errors.is_empty(),
            "clean brain should produce no errors, got: {diags:?}"
        );
    }

    #[test]
    fn check_schema_project_missing_tracks_emits_warning() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        let src = make_source(&path, "project");

        let json = r#"{
  "repo": "test",
  "kind": "project",
  "updated": "2026-06-29",
  "focus": { "now": [], "next": [], "blocked": [] }
}"#;
        let file = parse_file(json);
        let diags = check_schema(&src, &file);

        // Should have a warning about missing tracks
        let track_warnings: Vec<_> = diags
            .iter()
            .filter(|d| {
                d.severity == crate::Severity::Warning
                    && d.locator == "E_STATE_SCHEMA_MISSING_FIELD"
            })
            .collect();
        assert!(
            !track_warnings.is_empty(),
            "should warn about missing tracks[], got: {diags:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Task 3 — build_state_graph / check_state_graph tests
    // -----------------------------------------------------------------------

    /// Build a minimal (source, file) pair for use in graph tests.
    fn make_pair(
        dir: &std::path::Path,
        filename: &str,
        kind: &'static str,
        json: &str,
    ) -> (StateSource, StateFile) {
        let path = dir.join(filename);
        std::fs::write(&path, json).unwrap();
        let src = StateSource {
            repo_slug: serde_json::from_str::<serde_json::Value>(json).unwrap()["repo"]
                .as_str()
                .unwrap()
                .to_string(),
            abs_path: path,
            expected_kind: kind,
        };
        let file: StateFile = serde_json::from_str(json).expect("fixture must parse");
        (src, file)
    }

    /// Minimal leaf state.json with one tracks block and a clean focus.
    fn leaf_pair(dir: &std::path::Path, repo: &str, block_id: &str) -> (StateSource, StateFile) {
        let json = format!(
            r#"{{
  "repo": "{repo}",
  "kind": "project",
  "updated": "2026-06-29",
  "focus": {{
    "now": [{{ "block": "{block_id}", "title": "Work", "status": "in_progress" }}],
    "next": [],
    "blocked": []
  }},
  "tracks": [{{
    "title": "Phase 1",
    "blocks": [{{ "id": "{block_id}", "title": "Work", "status": "in_progress" }}]
  }}]
}}"#
        );
        make_pair(dir, &format!("{repo}-state.json"), "project", &json)
    }

    #[test]
    fn build_and_check_clean_two_repo_fixture_passes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pair_a = leaf_pair(dir.path(), "alpha", "AL.1.A");
        let pair_b = leaf_pair(dir.path(), "beta", "BE.1.A");
        let files = vec![pair_a, pair_b];

        let graph = build_state_graph(&files);

        // Two nodes, one per repo.
        assert_eq!(
            graph.nodes.len(),
            2,
            "expected 2 nodes, got: {:?}",
            graph.nodes
        );
        assert_eq!(graph.edges.len(), 0, "expected 0 edges for clean fixture");

        let diags = check_state_graph(&graph, &files);
        let errors: Vec<_> = diags
            .iter()
            .filter(|d| d.severity == crate::Severity::Error)
            .collect();
        assert!(
            errors.is_empty(),
            "clean two-repo fixture should produce no errors, got: {diags:?}"
        );
    }

    #[test]
    fn check_detects_dangling_blocked_by_to_nonexistent_id() {
        let dir = tempfile::tempdir().expect("tempdir");

        // alpha has block AL.1.A in tracks
        let pair_a = leaf_pair(dir.path(), "alpha", "AL.1.A");

        // beta's focus.blocked references alpha's block "AL.1.GHOST" which does NOT exist
        let beta_json = r#"{
  "repo": "beta",
  "kind": "project",
  "updated": "2026-06-29",
  "focus": {
    "now": [],
    "next": [],
    "blocked": [
      {
        "block": "BE.1.A",
        "title": "Blocked work",
        "blocked_by": [
          { "type": "block", "repo": "alpha", "id": "AL.1.GHOST" }
        ]
      }
    ]
  },
  "tracks": [{ "title": "Phase 1", "blocks": [{ "id": "BE.1.A", "title": "Blocked work" }] }]
}"#;
        let pair_b = make_pair(dir.path(), "beta-state.json", "project", beta_json);

        let files = vec![pair_a, pair_b];
        let graph = build_state_graph(&files);
        let diags = check_state_graph(&graph, &files);

        let dangling: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_DANGLING_BLOCKED_BY")
            .collect();
        assert_eq!(
            dangling.len(),
            1,
            "expected exactly one E_STATE_DANGLING_BLOCKED_BY, got: {diags:?}"
        );
    }

    #[test]
    fn check_detects_unknown_repo_in_blocked_by() {
        let dir = tempfile::tempdir().expect("tempdir");

        // Only one repo in the corpus
        let alpha_json = r#"{
  "repo": "alpha",
  "kind": "project",
  "updated": "2026-06-29",
  "focus": {
    "now": [],
    "next": [],
    "blocked": [
      {
        "block": "AL.1.A",
        "title": "Blocked",
        "blocked_by": [
          { "type": "block", "repo": "ghost-repo", "id": "GH.1.X" }
        ]
      }
    ]
  },
  "tracks": [{ "title": "P1", "blocks": [{ "id": "AL.1.A", "title": "Blocked" }] }]
}"#;
        let pair = make_pair(dir.path(), "alpha-state.json", "project", alpha_json);
        let files = vec![pair];

        let graph = build_state_graph(&files);
        let diags = check_state_graph(&graph, &files);

        let unknown: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_UNKNOWN_REPO")
            .collect();
        assert_eq!(
            unknown.len(),
            1,
            "expected exactly one E_STATE_UNKNOWN_REPO, got: {diags:?}"
        );
    }

    #[test]
    fn check_detects_duplicate_block_id_in_tracks() {
        let dir = tempfile::tempdir().expect("tempdir");

        // alpha has block AL.1.A registered TWICE in tracks (two entries with same id)
        let alpha_json = r#"{
  "repo": "alpha",
  "kind": "project",
  "updated": "2026-06-29",
  "focus": { "now": [], "next": [], "blocked": [] },
  "tracks": [
    {
      "title": "Phase 1",
      "blocks": [
        { "id": "AL.1.A", "title": "First" },
        { "id": "AL.1.A", "title": "Duplicate!" }
      ]
    }
  ]
}"#;
        let pair = make_pair(dir.path(), "alpha-state.json", "project", alpha_json);
        let files = vec![pair];

        let graph = build_state_graph(&files);
        // Two nodes with the same key
        assert_eq!(graph.nodes.len(), 2, "builder emits both duplicate nodes");

        let diags = check_state_graph(&graph, &files);

        let dup: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_DUPLICATE_BLOCK_ID")
            .collect();
        assert_eq!(
            dup.len(),
            1,
            "expected exactly one E_STATE_DUPLICATE_BLOCK_ID, got: {diags:?}"
        );
    }

    #[test]
    fn check_detects_dangling_focus_in_leaf_file() {
        let dir = tempfile::tempdir().expect("tempdir");

        // alpha's focus.now references "AL.1.GHOST" which is not in tracks
        let alpha_json = r#"{
  "repo": "alpha",
  "kind": "project",
  "updated": "2026-06-29",
  "focus": {
    "now": [{ "block": "AL.1.GHOST", "title": "Missing", "status": "in_progress" }],
    "next": [],
    "blocked": []
  },
  "tracks": [{ "title": "P1", "blocks": [{ "id": "AL.1.A", "title": "Real block" }] }]
}"#;
        let pair = make_pair(dir.path(), "alpha-state.json", "project", alpha_json);
        let files = vec![pair];

        let graph = build_state_graph(&files);
        let diags = check_state_graph(&graph, &files);

        let dangling_focus: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_DANGLING_FOCUS")
            .collect();
        assert_eq!(
            dangling_focus.len(),
            1,
            "expected exactly one E_STATE_DANGLING_FOCUS, got: {diags:?}"
        );
    }

    #[test]
    fn check_cross_repo_edge_dangling_target_emits_dangling_cross_repo() {
        let dir = tempfile::tempdir().expect("tempdir");

        // alpha has block AL.1.A
        let pair_a = leaf_pair(dir.path(), "alpha", "AL.1.A");

        // brain file with cross_repo edge pointing at "alpha:AL.1.GHOST" (nonexistent)
        let brain_json = r#"{
  "repo": "core",
  "kind": "brain",
  "updated": "2026-06-29",
  "focus": { "now": [], "next": [], "blocked": [] },
  "repos": [{ "repo": "alpha", "now": [], "next": [], "blocked": [] }],
  "cross_repo": [
    {
      "from": { "repo": "alpha", "block": "AL.1.A" },
      "to": { "repo": "alpha", "block": "AL.1.GHOST" }
    }
  ]
}"#;
        let pair_brain = make_pair(dir.path(), "brain-state.json", "brain", brain_json);
        let files = vec![pair_a, pair_brain];

        let graph = build_state_graph(&files);
        let diags = check_state_graph(&graph, &files);

        let dangling_cross: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_DANGLING_CROSS_REPO")
            .collect();
        assert_eq!(
            dangling_cross.len(),
            1,
            "expected exactly one E_STATE_DANGLING_CROSS_REPO for ghost 'to' endpoint, got: {diags:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Task 4 — check_rollup tests
    // -----------------------------------------------------------------------

    /// Build a brain StateFile with a repos[] entry for one child.
    fn brain_with_rollup(
        child_slug: &str,
        now_ids: &[&str],
        next_ids: &[&str],
        blocked_ids: &[&str],
    ) -> StateFile {
        let make_blocks = |ids: &[&str]| -> Vec<Block> {
            ids.iter()
                .map(|id| Block {
                    block: id.to_string(),
                    title: "placeholder".to_string(),
                    status: None,
                    note: None,
                    repo: None,
                    blocked_by: vec![],
                })
                .collect()
        };

        StateFile {
            repo: "hq".to_string(),
            kind: "brain".to_string(),
            updated: "2026-06-29".to_string(),
            focus: Focus::default(),
            tracks: vec![],
            repos: vec![RepoRollup {
                repo: child_slug.to_string(),
                tier: None,
                now: make_blocks(now_ids),
                next: make_blocks(next_ids),
                blocked: make_blocks(blocked_ids),
            }],
            cross_repo: vec![],
            tiers: vec![],
            note: None,
        }
    }

    /// Build a child StateFile with the given focus.
    fn child_with_focus(
        slug: &str,
        now_ids: &[&str],
        next_ids: &[&str],
        blocked_ids: &[&str],
    ) -> StateFile {
        let make_blocks = |ids: &[&str], with_status: bool| -> Vec<Block> {
            ids.iter()
                .map(|id| Block {
                    block: id.to_string(),
                    title: "placeholder".to_string(),
                    status: if with_status {
                        Some("in_progress".to_string())
                    } else {
                        None
                    },
                    note: None,
                    repo: None,
                    blocked_by: vec![],
                })
                .collect()
        };

        StateFile {
            repo: slug.to_string(),
            kind: "project".to_string(),
            updated: "2026-06-29".to_string(),
            focus: Focus {
                now: make_blocks(now_ids, true),
                next: make_blocks(next_ids, false),
                blocked: make_blocks(blocked_ids, false),
            },
            tracks: vec![],
            repos: vec![],
            cross_repo: vec![],
            tiers: vec![],
            note: None,
        }
    }

    #[test]
    fn check_rollup_in_sync_returns_no_diagnostics() {
        let dir = tempfile::tempdir().expect("tempdir");
        let brain_path = dir.path().join("brain-state.json");

        // Brain repos[] cache and child focus are identical (both have ["AL.1.A"]).
        let brain = brain_with_rollup("alpha", &["AL.1.A"], &["AL.1.B"], &[]);
        let child = child_with_focus("alpha", &["AL.1.A"], &["AL.1.B"], &[]);

        let mut children = std::collections::HashMap::new();
        children.insert("alpha".to_string(), child);

        let diags = check_rollup(&brain_path, &brain, &children);
        assert!(
            diags.is_empty(),
            "in-sync rollup should produce no diagnostics, got: {diags:?}"
        );
    }

    #[test]
    fn check_rollup_drifted_child_emits_w_state_rollup_drift() {
        let dir = tempfile::tempdir().expect("tempdir");
        let brain_path = dir.path().join("brain-state.json");

        // Brain cache says child.now = ["AL.1.A"] but child has advanced to ["AL.1.B"].
        let brain = brain_with_rollup("alpha", &["AL.1.A"], &["AL.1.B"], &[]);
        let child = child_with_focus("alpha", &["AL.1.B"], &["AL.1.C"], &[]);

        let mut children = std::collections::HashMap::new();
        children.insert("alpha".to_string(), child);

        let diags = check_rollup(&brain_path, &brain, &children);

        let drift: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "W_STATE_ROLLUP_DRIFT")
            .collect();
        assert_eq!(
            drift.len(),
            1,
            "drifted child should produce exactly one W_STATE_ROLLUP_DRIFT, got: {diags:?}"
        );
        // It must be a Warning, not an Error (decision 4).
        assert_eq!(
            drift[0].severity,
            crate::Severity::Warning,
            "W_STATE_ROLLUP_DRIFT must be Warning severity"
        );
    }

    #[test]
    fn check_rollup_missing_child_is_skipped_silently() {
        let dir = tempfile::tempdir().expect("tempdir");
        let brain_path = dir.path().join("brain-state.json");

        // Brain has a repos[] entry for "alpha" but children map is empty.
        let brain = brain_with_rollup("alpha", &["AL.1.A"], &[], &[]);
        let children: std::collections::HashMap<String, StateFile> =
            std::collections::HashMap::new();

        let diags = check_rollup(&brain_path, &brain, &children);
        assert!(
            diags.is_empty(),
            "missing child (no loaded state.json) should be skipped silently; got: {diags:?}"
        );
    }

    #[test]
    fn check_rollup_only_blocked_drift_emits_warning() {
        let dir = tempfile::tempdir().expect("tempdir");
        let brain_path = dir.path().join("brain-state.json");

        // now/next match but blocked list differs.
        let brain = brain_with_rollup("alpha", &["AL.1.A"], &["AL.1.B"], &[]);
        let child = child_with_focus("alpha", &["AL.1.A"], &["AL.1.B"], &["AL.1.C"]);

        let mut children = std::collections::HashMap::new();
        children.insert("alpha".to_string(), child);

        let diags = check_rollup(&brain_path, &brain, &children);
        let drift: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "W_STATE_ROLLUP_DRIFT")
            .collect();
        assert_eq!(
            drift.len(),
            1,
            "blocked-only drift should also emit W_STATE_ROLLUP_DRIFT, got: {diags:?}"
        );
    }

    #[test]
    fn state_graph_is_serializable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pair_a = leaf_pair(dir.path(), "alpha", "AL.1.A");
        let files = vec![pair_a];

        let graph = build_state_graph(&files);
        let json = serde_json::to_string(&graph).expect("StateGraph must serialize");
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        // Must have "nodes" and "edges" arrays
        assert!(parsed["nodes"].is_array());
        assert!(parsed["edges"].is_array());

        // The node must carry key, repo, id, title but NOT source_path (skipped)
        let node = &parsed["nodes"][0];
        assert_eq!(node["key"], "alpha:AL.1.A");
        assert_eq!(node["repo"], "alpha");
        assert_eq!(node["id"], "AL.1.A");
        assert!(
            node["source_path"].is_null(),
            "source_path should be skipped in JSON"
        );
    }
}
