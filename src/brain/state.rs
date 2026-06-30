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
//! - `W_STATE_FOCUS_DRIFT` — a leaf file's `focus` snapshot has drifted from the tracks[] derivation.
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
    /// Canonical block ID. (`#[serde(alias)]` keeps v1 `"block"`-keyed files readable
    /// through the v2 transition; the canonical authored key is `id`.)
    #[serde(alias = "block")]
    pub id: String,
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
    /// Lifecycle status (authored: `open`/`in_progress`/`closed` — `blocked` is derived,
    /// enforced in task 2).
    #[serde(default)]
    pub status: Option<String>,
    /// The block's full dependency edges (the authoritative DAG). Same forms as [`BlockedBy`].
    #[serde(default)]
    pub depends_on: Vec<BlockedBy>,
    /// Execution-order rank for "what's next" (orthogonal to track grouping).
    #[serde(default)]
    pub wave: Option<i64>,
    /// Backlog-promotion provenance, when this block came from a backlog item.
    #[serde(default)]
    pub origin: Option<Origin>,
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
    #[serde(alias = "block")]
    pub id: String,
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
// Origin — backlog→block promotion provenance (v2)
// ---------------------------------------------------------------------------

/// Provenance pointer on a block that was promoted from a backlog item.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Origin {
    /// Origin kind — `"backlog"` today.
    #[serde(rename = "type")]
    pub kind: String,
    /// The originating backlog node's stable `slug` key.
    pub slug: String,
}

// ---------------------------------------------------------------------------
// Backlog — HQ queued-ideas graph node (v2)
// ---------------------------------------------------------------------------

/// One entry in the HQ brain `backlog[]` — a queued idea as a graph node.
///
/// `slug` is the stable node key. `depends_on` reuses [`BlockedBy`] (the same
/// edge form as blocks). On promotion the node persists with
/// `status:"promoted"` + a `block` pointer; the resulting block carries an
/// [`Origin`] back-pointer.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Backlog {
    /// Stable node key (the notes-dir slug).
    pub slug: String,
    /// Human description.
    pub title: String,
    /// Repo the item will land in when promoted (or `"cross-repo"`).
    pub repo: String,
    /// Item kind (`improvement` / `feature` / `chore` / `decision` / …).
    #[serde(rename = "type")]
    pub kind: String,
    /// Lifecycle status: `idea` / `ready` / `promoted` (validated in task 2).
    pub status: String,
    /// What the idea is gated on — same edge forms as a block's `depends_on`.
    #[serde(default)]
    pub depends_on: Vec<BlockedBy>,
    /// Set only when `status == "promoted"`: the ID of the block it became.
    #[serde(default)]
    pub block: Option<String>,
    /// Path to the pre-plan notes doc.
    #[serde(default)]
    pub notes: Option<String>,
}

// ---------------------------------------------------------------------------
// Carryover — durable caveats / follow-ons (v3)
// ---------------------------------------------------------------------------

/// The scope of a `carryover[]` entry.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct CarryoverScope {
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub tier: Option<String>,
    #[serde(default)]
    pub cross_repo: Option<bool>,
}

/// A durable caveat, known issue, environmental note, or deferred follow-on.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Carryover {
    /// Stable node key.
    pub slug: String,
    /// Where it applies.
    pub scope: CarryoverScope,
    /// Item kind (`constraint`, `known_issue`, `env`, `deferred`).
    pub kind: String,
    /// The caveat / follow-on text.
    pub text: String,
    /// Optional related edges (same forms as blocked_by).
    #[serde(default)]
    pub related: Vec<BlockedBy>,
    /// Human-readable condition under which this entry should be deleted.
    #[serde(default)]
    pub clears_when: Option<String>,
    /// Date recorded (YYYY-MM-DD).
    pub created: String,
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
    /// HQ queued-ideas graph (brain HQ only; empty elsewhere).
    #[serde(default)]
    pub backlog: Vec<Backlog>,
    /// Durable caveats and follow-ons.
    #[serde(default)]
    pub carryover: Vec<Carryover>,
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

/// Valid `status` values for `focus.now` and `focus.blocked` block entries (derived view).
const VALID_STATUSES: &[&str] = &["open", "in_progress", "blocked", "closed"];

/// Valid *authored* `status` values for `tracks[].blocks[]` entries.
///
/// `"blocked"` is intentionally excluded — it is a derived property, never an authored one.
/// Any block authored with `"blocked"` triggers `E_STATE_AUTHORED_BLOCKED`.
const VALID_TRACK_BLOCK_STATUSES: &[&str] = &["open", "in_progress", "closed"];

/// Valid `status` values for `backlog[]` entries (HQ brain only).
const VALID_BACKLOG_STATUSES: &[&str] = &["idea", "ready", "promoted"];

/// Valid `kind` values for `carryover[]` entries.
const VALID_CARRYOVER_KINDS: &[&str] = &["constraint", "known_issue", "env", "deferred"];

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
/// 6. **Authored `tracks[].blocks[].status` not `"blocked"`**
///    (`E_STATE_AUTHORED_BLOCKED`) — `"blocked"` is a derived property; an
///    authored track-block must use `{open, in_progress, closed}` only.
/// 7. **`tracks[].blocks[].depends_on` well-formedness**
///    (`E_STATE_SCHEMA_BAD_BLOCKED_BY`) — a `{type:"block"}` `depends_on`
///    entry must have non-empty `repo` and `id`.
/// 8. **`backlog[].status` enum** (`E_STATE_SCHEMA_BAD_STATUS`) — every
///    backlog node's `status` must be one of `{idea, ready, promoted}`.
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
                    block.id,
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
                            block.id,
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

    // --- 6. Authored track-block status must not be "blocked" (v2: blocked is derived) ---
    // --- 7. depends_on entry well-formedness ---
    for track in &file.tracks {
        for block in &track.blocks {
            // check 6: authored status must not be "blocked"
            if let Some(status) = &block.status {
                if status == "blocked" {
                    diags.push(Diagnostic::error(
                        path,
                        "E_STATE_AUTHORED_BLOCKED",
                        format!(
                            "track block '{}' has authored status 'blocked'; \
                             'blocked' is a derived property — use open/in_progress/closed",
                            block.id
                        ),
                    ));
                } else if !VALID_TRACK_BLOCK_STATUSES.contains(&status.as_str()) {
                    // Also catch other invalid statuses on track blocks.
                    diags.push(Diagnostic::error(
                        path,
                        "E_STATE_SCHEMA_BAD_STATUS",
                        format!(
                            "track block '{}' has invalid status '{}'; expected one of: {}",
                            block.id,
                            status,
                            VALID_TRACK_BLOCK_STATUSES.join(", ")
                        ),
                    ));
                }
            }

            // check 7: depends_on {type:block} entries must have non-empty repo and id
            for dep in &block.depends_on {
                if let BlockedBy::Block { repo, id, .. } = dep {
                    let repo_empty = repo.trim().is_empty();
                    let id_empty = id.trim().is_empty();
                    if repo_empty || id_empty {
                        diags.push(Diagnostic::error(
                            path,
                            "E_STATE_SCHEMA_BAD_BLOCKED_BY",
                            format!(
                                "depends_on entry in track block '{}' is missing required \
                                 field(s): {}",
                                block.id,
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
    }

    // --- 8. backlog[].status enum (HQ brain only) ---
    for item in &file.backlog {
        if !VALID_BACKLOG_STATUSES.contains(&item.status.as_str()) {
            diags.push(Diagnostic::error(
                path,
                "E_STATE_SCHEMA_BAD_STATUS",
                format!(
                    "backlog item '{}' has invalid status '{}'; expected one of: {}",
                    item.slug,
                    item.status,
                    VALID_BACKLOG_STATUSES.join(", ")
                ),
            ));
        }
    }

    // --- 9. carryover[] validation ---
    for item in &file.carryover {
        if !VALID_CARRYOVER_KINDS.contains(&item.kind.as_str()) {
            diags.push(Diagnostic::error(
                path,
                "E_STATE_SCHEMA_BAD_KIND",
                format!(
                    "carryover item '{}' has invalid kind '{}'; expected one of: {}",
                    item.slug,
                    item.kind,
                    VALID_CARRYOVER_KINDS.join(", ")
                ),
            ));
        }
        
        let scope_fields_set = item.scope.repo.is_some() as u8
            + item.scope.tier.is_some() as u8
            + item.scope.cross_repo.is_some() as u8;
            
        if scope_fields_set != 1 {
            diags.push(Diagnostic::error(
                path,
                "E_STATE_SCHEMA_MALFORMED_SCOPE",
                format!(
                    "carryover item '{}' has malformed scope; exactly one of 'repo', 'tier', or 'cross_repo' must be set",
                    item.slug
                ),
            ));
        }

        for dep in &item.related {
            if let BlockedBy::Block { repo, id, .. } = dep {
                let repo_empty = repo.trim().is_empty();
                let id_empty = id.trim().is_empty();
                if repo_empty || id_empty {
                    diags.push(Diagnostic::error(
                        path,
                        "E_STATE_SCHEMA_BAD_BLOCKED_BY",
                        format!(
                            "related entry in carryover item '{}' is missing required \
                             field(s): {}",
                            item.slug,
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
/// - One [`StateEdge`] with `kind: BlockedBy` per `{type:"block"}` entry in
///   any file's `tracks[].blocks[].depends_on[]`.  The `from` key is the
///   owning block's `"repo:id"` (the block that declares the dependency);
///   `to_ref` is `"{dep.repo}:{dep.id}"`.  External entries are skipped —
///   they are leaf constraints, not graph edges.
/// - One [`StateEdge`] with `kind: CrossRepo` per brain-file `cross_repo[]`
///   entry.
///
/// `focus.*.blocked_by[]` is intentionally **not** an edge source in v2 —
/// `focus` is a derived view, not the authoritative DAG.
///
/// Nodes and edges are emitted even when they are later found to be dangling
/// — the separation of build and check is intentional (see `MV.3.J` pattern).
pub fn build_state_graph(files: &[(StateSource, StateFile)]) -> StateGraph {
    let mut nodes: Vec<StateNode> = Vec::new();
    let mut edges: Vec<StateEdge> = Vec::new();

    for (src, file) in files {
        let path = &src.abs_path;

        // --- Nodes + BlockedBy edges: from tracks[].blocks[] ---
        for track in &file.tracks {
            for block in &track.blocks {
                let from_key = format!("{}:{}", src.repo_slug, block.id);

                nodes.push(StateNode {
                    key: from_key.clone(),
                    repo: src.repo_slug.clone(),
                    id: block.id.clone(),
                    title: block.title.clone(),
                    source_path: path.clone(),
                });

                // BlockedBy edges: one per {type:block} depends_on entry.
                // External entries are leaf constraints, not graph edges — skip.
                for dep in &block.depends_on {
                    if let BlockedBy::Block { repo, id, .. } = dep {
                        edges.push(StateEdge {
                            from: from_key.clone(),
                            to_ref: format!("{repo}:{id}"),
                            kind: StateEdgeKind::BlockedBy,
                            source_path: path.clone(),
                        });
                    }
                }
            }
        }

        // --- CrossRepo edges: from brain cross_repo[] ---
        for edge in &file.cross_repo {
            edges.push(StateEdge {
                from: format!("{}:{}", edge.from.repo, edge.from.id),
                to_ref: format!("{}:{}", edge.to.repo, edge.to.id),
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
    let mut node_counts: HashMap<&str, usize> = HashMap::new();
    for node in &graph.nodes {
        *node_counts.entry(node.key.as_str()).or_insert(0) += 1;
    }

    // Set of all registered "repo:id" keys (for dangling checks).
    let node_set: HashSet<&str> = node_counts.keys().copied().collect();

    // --- 1. Duplicate block IDs ---
    let mut duplicate_reported: HashSet<&str> = HashSet::new();
    for node in &graph.nodes {
        let key = node.key.as_str();
        if node_counts[key] > 1 && !duplicate_reported.contains(key) {
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
            let key = format!("{}:{}", src.repo_slug, block.id);
            if !node_set.contains(key.as_str()) {
                diags.push(Diagnostic::error(
                    path,
                    "E_STATE_DANGLING_FOCUS",
                    format!(
                        "focus block '{}' is not registered in this repo's tracks[]",
                        block.id
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
// Status-consistency check (Task 4)
// ---------------------------------------------------------------------------

/// Check that no `closed` block depends on a non-`closed` block.
///
/// A block that declares `status: "closed"` with a `{type:"block"}` entry in its
/// `depends_on[]` that points to a block whose authored status is **not** `"closed"`
/// is inconsistent — the dependency was not complete before the dependent was closed.
///
/// Emits **`E_STATE_STATUS_INCONSISTENT`** for each such pair.  Dependencies whose
/// target does not exist in any loaded file are silently skipped here — they are
/// already reported as `E_STATE_DANGLING_BLOCKED_BY` by [`check_state_graph`].
pub fn check_status_consistency(files: &[(StateSource, StateFile)]) -> Vec<Diagnostic> {
    use std::collections::HashMap;

    // Build a status lookup: "repo:id" → authored status (None = absent = treated as open).
    let mut status_map: HashMap<String, Option<String>> = HashMap::new();
    for (src, file) in files {
        for track in &file.tracks {
            for block in &track.blocks {
                let key = format!("{}:{}", src.repo_slug, block.id);
                status_map.insert(key, block.status.clone());
            }
        }
    }

    let mut diags: Vec<Diagnostic> = Vec::new();

    for (src, file) in files {
        for track in &file.tracks {
            for block in &track.blocks {
                // Only closed blocks participate in this check.
                if block.status.as_deref() != Some("closed") {
                    continue;
                }

                let from_key = format!("{}:{}", src.repo_slug, block.id);

                for dep in &block.depends_on {
                    if let BlockedBy::Block { repo, id, .. } = dep {
                        let dep_key = format!("{repo}:{id}");
                        // If the dep target is not in any loaded file, skip — it will
                        // be reported as E_STATE_DANGLING_BLOCKED_BY by check_state_graph.
                        if let Some(dep_status) = status_map.get(&dep_key) {
                            let dep_is_closed = dep_status.as_deref() == Some("closed");
                            if !dep_is_closed {
                                diags.push(Diagnostic::error(
                                    &src.abs_path,
                                    "E_STATE_STATUS_INCONSISTENT",
                                    format!(
                                        "closed block '{from_key}' has a non-closed depends_on \
                                         target '{dep_key}' (status: {})",
                                        dep_status.as_deref().unwrap_or("open")
                                    ),
                                ));
                            }
                        }
                    }
                }
            }
        }
    }

    diags
}

// ---------------------------------------------------------------------------
// Backlog-node integrity check (Task 4)
// ---------------------------------------------------------------------------

/// Validate referential integrity for `backlog[]` nodes (HQ brain files only).
///
/// Two checks are performed:
///
/// 1. **`E_STATE_DANGLING_BLOCKED_BY`** — a backlog node's `depends_on[]` entry of
///    `{type:"block"}` references a block that is not registered in any loaded
///    repo's `tracks[]`.
///
/// 2. **`E_STATE_DANGLING_PROMOTION`** — a backlog node whose `status` is `"promoted"`
///    carries a `block` pointer (or is missing one) that does not resolve to an
///    existing node in `{backlog.repo}:tracks[]`.
///
/// Backlog nodes with `status` other than `"promoted"` are not checked for
/// promotion integrity (they have no `block` pointer by contract).
pub fn check_backlog_integrity(
    files: &[(StateSource, StateFile)],
    graph: &StateGraph,
) -> Vec<Diagnostic> {
    use std::collections::HashSet;

    // Set of all registered "repo:id" keys.
    let node_set: HashSet<&str> = graph.nodes.iter().map(|n| n.key.as_str()).collect();

    let mut diags: Vec<Diagnostic> = Vec::new();

    for (src, file) in files {
        if file.backlog.is_empty() {
            continue;
        }

        let path = &src.abs_path;

        for backlog_node in &file.backlog {
            // --- 1. Dangling depends_on ---
            for dep in &backlog_node.depends_on {
                if let BlockedBy::Block { repo, id, .. } = dep {
                    let dep_key = format!("{repo}:{id}");
                    if !node_set.contains(dep_key.as_str()) {
                        diags.push(Diagnostic::error(
                            path,
                            "E_STATE_DANGLING_BLOCKED_BY",
                            format!(
                                "backlog node '{}' depends_on references unknown block '{dep_key}'",
                                backlog_node.slug
                            ),
                        ));
                    }
                }
            }

            // --- 2. Orphan promoted node ---
            if backlog_node.status == "promoted" {
                match &backlog_node.block {
                    None => {
                        // Promoted with no block pointer — the promotion target is unknown.
                        diags.push(Diagnostic::error(
                            path,
                            "E_STATE_DANGLING_PROMOTION",
                            format!(
                                "backlog node '{}' has status 'promoted' but no 'block' pointer",
                                backlog_node.slug
                            ),
                        ));
                    }
                    Some(block_id) => {
                        // Promoted and pointing at a block — verify the block exists.
                        let block_key = format!("{}:{block_id}", backlog_node.repo);
                        if !node_set.contains(block_key.as_str()) {
                            diags.push(Diagnostic::error(
                                path,
                                "E_STATE_DANGLING_PROMOTION",
                                format!(
                                    "backlog node '{}' promoted to block '{block_id}' which does \
                                     not exist in '{}' tracks[]",
                                    backlog_node.slug, backlog_node.repo
                                ),
                            ));
                        }
                    }
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
        let cached_now: HashSet<&str> = rollup.now.iter().map(|b| b.id.as_str()).collect();
        let actual_now: HashSet<&str> = child.focus.now.iter().map(|b| b.id.as_str()).collect();

        let cached_next: HashSet<&str> = rollup.next.iter().map(|b| b.id.as_str()).collect();
        let actual_next: HashSet<&str> = child.focus.next.iter().map(|b| b.id.as_str()).collect();

        let cached_blocked: HashSet<&str> = rollup.blocked.iter().map(|b| b.id.as_str()).collect();
        let actual_blocked: HashSet<&str> =
            child.focus.blocked.iter().map(|b| b.id.as_str()).collect();

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

// ---------------------------------------------------------------------------
// Cycle detection
// ---------------------------------------------------------------------------

/// Detect cycles in the `depends_on` edge subgraph of `graph`.
///
/// Performs a DFS over [`StateEdgeKind::BlockedBy`] edges only; [`StateEdgeKind::CrossRepo`]
/// edges are intentionally excluded (they annotate inter-repo intent, not block ordering
/// and are not part of the authoritative DAG).
///
/// On detection of a back-edge, emits **`E_STATE_CYCLE`** naming the cycle path in the
/// form `A → B → C → A`.  Each distinct cycle path is reported once.
///
/// Returns an empty `Vec` when the `depends_on` subgraph is acyclic.
pub fn detect_cycles(graph: &StateGraph) -> Vec<Diagnostic> {
    use std::collections::{HashMap, HashSet};

    // Build adjacency: from_key → Vec<(to_ref, source_path)>.
    // Initialise every node so isolated nodes are visited and early-exited cleanly.
    let mut adj: HashMap<&str, Vec<(&str, &std::path::Path)>> = HashMap::new();
    for node in &graph.nodes {
        adj.entry(node.key.as_str()).or_default();
    }
    for edge in &graph.edges {
        if edge.kind == StateEdgeKind::BlockedBy {
            adj.entry(edge.from.as_str())
                .or_default()
                .push((edge.to_ref.as_str(), edge.source_path.as_path()));
        }
    }

    let mut visited: HashSet<String> = HashSet::new();
    let mut diags: Vec<Diagnostic> = Vec::new();
    let mut reported: HashSet<String> = HashSet::new();

    // Iterate in a deterministic order (node-insertion order from the graph).
    let starts: Vec<String> = graph.nodes.iter().map(|n| n.key.clone()).collect();
    for start in &starts {
        if !visited.contains(start.as_str()) {
            let mut rec_stack: Vec<String> = Vec::new();
            detect_cycles_dfs(
                start.as_str(),
                &adj,
                &mut visited,
                &mut rec_stack,
                &mut diags,
                &mut reported,
            );
        }
    }

    diags
}

/// DFS worker for [`detect_cycles`].
///
/// `rec_stack` tracks the current DFS path (nodes in the "gray" / visiting state).
/// `visited` is the union of gray + black nodes (prevents re-visiting fully-explored nodes).
/// `reported` deduplicates identical cycle-path strings so each cycle is emitted once.
fn detect_cycles_dfs<'a>(
    node: &'a str,
    adj: &std::collections::HashMap<&'a str, Vec<(&'a str, &'a std::path::Path)>>,
    visited: &mut std::collections::HashSet<String>,
    rec_stack: &mut Vec<String>,
    diags: &mut Vec<Diagnostic>,
    reported: &mut std::collections::HashSet<String>,
) {
    visited.insert(node.to_string());
    rec_stack.push(node.to_string());

    if let Some(neighbors) = adj.get(node) {
        for (neighbor, source_path) in neighbors {
            if !visited.contains(*neighbor) {
                detect_cycles_dfs(neighbor, adj, visited, rec_stack, diags, reported);
            } else if let Some(pos) = rec_stack.iter().position(|n| n == neighbor) {
                // Back-edge — `neighbor` is still on the recursion stack.
                // Build the cycle path: stack[pos..] + closing arrow back to neighbor.
                let cycle: Vec<&str> = rec_stack[pos..].iter().map(|s| s.as_str()).collect();
                let path_str = format!("{} \u{2192} {}", cycle.join(" \u{2192} "), neighbor);
                if !reported.contains(&path_str) {
                    reported.insert(path_str.clone());
                    diags.push(Diagnostic::error(
                        *source_path,
                        "E_STATE_CYCLE",
                        format!("cycle detected in depends_on DAG: {path_str}"),
                    ));
                }
            }
        }
    }

    rec_stack.pop();
}

// ---------------------------------------------------------------------------
// Ready-order (reusable — MV.3B.T topo-emitter input)
// ---------------------------------------------------------------------------

/// Compute the wave-ordered list of **ready** `open` blocks across all files.
///
/// A block is *ready* iff:
/// - Its authored status is `"open"` (or absent — treated as open).
/// - It has **zero** `{type:"external"}` `depends_on` entries (external dependencies
///   mean the block is gated on something outside the graph).
/// - Every `{type:"block"}` `depends_on` target has authored status `"closed"`.
///
/// The returned `Vec<String>` lists canonical `"repo:id"` keys ordered by:
/// 1. `wave` ascending (`None` treated as `i64::MAX` — lowest priority, goes last).
/// 2. Track iteration order across `files` (stable tiebreak: position of the containing
///    track, then block array index within that track).
///
/// `graph` is accepted for forward-compatibility — `MV.3B.T` will extend this function
/// to query graph structure; the current implementation derives all information from `files`.
/// Callers should always pass the graph built from the same `files` slice.
///
/// This function is **standalone and public** — do not inline it into any check function.
pub fn ready_order(_graph: &StateGraph, files: &[(StateSource, StateFile)]) -> Vec<String> {
    use std::collections::HashMap;

    // Status lookup: "repo:id" → authored status (None = absent = open).
    let mut status_map: HashMap<String, Option<String>> = HashMap::new();
    for (src, file) in files {
        for track in &file.tracks {
            for block in &track.blocks {
                let key = format!("{}:{}", src.repo_slug, block.id);
                status_map.insert(key, block.status.clone());
            }
        }
    }

    // Collect (wave, iteration_order, "repo:id") for every ready open block.
    let mut ready: Vec<(i64, usize, String)> = Vec::new();
    let mut order: usize = 0;

    for (src, file) in files {
        for track in &file.tracks {
            for block in &track.blocks {
                let current_order = order;
                order += 1;

                // Only open (or status-absent) blocks are candidates.
                let status = block.status.as_deref().unwrap_or("open");
                if status != "open" {
                    continue;
                }

                // Any external dep disqualifies the block (not yet runnable).
                let has_external = block
                    .depends_on
                    .iter()
                    .any(|d| matches!(d, BlockedBy::External { .. }));
                if has_external {
                    continue;
                }

                // All block deps must be closed.
                let all_block_deps_closed = block.depends_on.iter().all(|d| {
                    if let BlockedBy::Block { repo, id, .. } = d {
                        let dep_key = format!("{repo}:{id}");
                        status_map.get(&dep_key).and_then(|s| s.as_deref()) == Some("closed")
                    } else {
                        true // External entries handled above; this branch is unreachable here.
                    }
                });

                if all_block_deps_closed {
                    let wave = block.wave.unwrap_or(i64::MAX);
                    let key = format!("{}:{}", src.repo_slug, block.id);
                    ready.push((wave, current_order, key));
                }
            }
        }
    }

    // Primary sort: wave asc. Tiebreak: iteration order (stable).
    ready.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    ready.into_iter().map(|(_, _, key)| key).collect()
}

// ---------------------------------------------------------------------------
// Focus derivation (MV.3B.T — single-source derivation engine)
// ---------------------------------------------------------------------------

/// The derived view of a file's focus — computed from `tracks[]` by [`derive_focus`].
///
/// All three lists contain canonical block IDs (without the `"repo:"` prefix).
/// `blocked` additionally carries the **unmet subset** of `depends_on` for each blocked
/// block — the unmet items are used by the emitter to populate `focus.blocked[].blocked_by[]`.
#[derive(Debug, Clone, Default)]
pub struct DerivedFocus {
    /// Block IDs with authored `status == "in_progress"`.
    pub now: Vec<String>,
    /// Block IDs that are ready (open, no external deps, all block deps closed), in wave order.
    pub next: Vec<String>,
    /// `(block_id, unmet_deps)` — open blocks with at least one unmet dependency.
    pub blocked: Vec<(String, Vec<BlockedBy>)>,
}

/// Derive the expected `focus` from a file's `tracks[]`.
///
/// This is the **single derivation** used by both [`check_focus_drift`] (the validator)
/// and `mev emit-state` (the writer).  Because both call this function, the validator
/// and the emitter cannot disagree — the emit is, by construction, the fixed point of
/// the drift check.
///
/// Returns an empty [`DerivedFocus`] for files with an empty `tracks[]` (the derivation
/// is undefined when there is no roadmap catalog — typically brain files).
///
/// **Derivation rules:**
/// - `now` — every `tracks[]` block with authored `status == "in_progress"`.
/// - `blocked` — every `tracks[]` block that is `open` and has at least one unmet
///   dependency: any `External` dep, or any `Block` dep whose target is not `closed`.
///   The returned `blocked` entry carries only the **unmet** subset, not the full
///   `depends_on` list.
/// - `next` — every `tracks[]` block returned by [`ready_order`] for this file
///   (open blocks with no external deps and all block deps `closed`), in wave order.
pub fn derive_focus(
    src: &StateSource,
    file: &StateFile,
    graph: &StateGraph,
    files: &[(StateSource, StateFile)],
) -> DerivedFocus {
    use std::collections::HashMap;

    if file.tracks.is_empty() {
        return DerivedFocus::default();
    }

    // Build a status map: "repo:id" → authored status (None = absent = open).
    let mut status_map: HashMap<String, Option<String>> = HashMap::new();
    for (s, f) in files {
        for track in &f.tracks {
            for block in &track.blocks {
                let key = format!("{}:{}", s.repo_slug, block.id);
                status_map.insert(key, block.status.clone());
            }
        }
    }

    let mut now: Vec<String> = Vec::new();
    let mut blocked: Vec<(String, Vec<BlockedBy>)> = Vec::new();

    for track in &file.tracks {
        for block in &track.blocks {
            let authored_status = block.status.as_deref().unwrap_or("open");

            match authored_status {
                "in_progress" => {
                    now.push(block.id.clone());
                }
                "open" => {
                    // Collect the unmet subset of depends_on.
                    let unmet: Vec<BlockedBy> = block
                        .depends_on
                        .iter()
                        .filter(|d| match d {
                            BlockedBy::External { .. } => true,
                            BlockedBy::Block { repo, id, .. } => {
                                let dep_key = format!("{repo}:{id}");
                                status_map.get(&dep_key).and_then(|s| s.as_deref())
                                    != Some("closed")
                            }
                        })
                        .cloned()
                        .collect();
                    if !unmet.is_empty() {
                        blocked.push((block.id.clone(), unmet));
                    }
                    // `closed` and `blocked` (invalid authored) are skipped; they
                    // don't appear in the derived focus.
                }
                _ => {}
            }
        }
    }

    // next = ready_order filtered to this file's blocks (returns canonical "repo:id" keys).
    let ready = ready_order(graph, files);
    let this_prefix = format!("{}:", src.repo_slug);
    let next: Vec<String> = ready
        .into_iter()
        .filter(|key| key.starts_with(&this_prefix))
        .map(|key| key[this_prefix.len()..].to_string())
        .collect();

    DerivedFocus { now, next, blocked }
}

// ---------------------------------------------------------------------------
// Focus-drift check (task 5) — rewritten to delegate to derive_focus
// ---------------------------------------------------------------------------

/// Recompute the expected `focus` from `tracks[]` and warn when it disagrees
/// with the stored `focus` snapshot.
///
/// Delegates to [`derive_focus`] so the validator and the emitter share one
/// derivation and can never disagree.
///
/// Comparison is **block-id sets only** (mirrors [`check_rollup`]'s strategy —
/// cosmetic title/note differences are ignored).
///
/// **Severity:** warning only — focus is a derived view maintained by hand
/// (the warn→error flip is deferred until the `/log-work` writer exists).
/// Drift never causes exit 1.
///
/// Skips files with an empty `tracks[]` (the derivation is undefined when there
/// is no roadmap catalog — typically brain files whose focus comes from
/// aggregated child repos).
pub fn check_focus_drift(
    src: &StateSource,
    file: &StateFile,
    graph: &StateGraph,
    files: &[(StateSource, StateFile)],
) -> Vec<Diagnostic> {
    use std::collections::HashSet;

    // Undefined for files without a roadmap catalog.
    if file.tracks.is_empty() {
        return vec![];
    }

    let derived = derive_focus(src, file, graph, files);

    // Compare stored focus to derived (block-id sets only).
    let stored_now: HashSet<&str> = file.focus.now.iter().map(|b| b.id.as_str()).collect();
    let stored_next: HashSet<&str> = file.focus.next.iter().map(|b| b.id.as_str()).collect();
    let stored_blocked: HashSet<&str> = file.focus.blocked.iter().map(|b| b.id.as_str()).collect();

    let derived_now_str: HashSet<&str> = derived.now.iter().map(|s| s.as_str()).collect();
    let derived_next_str: HashSet<&str> = derived.next.iter().map(|s| s.as_str()).collect();
    let derived_blocked_str: HashSet<&str> =
        derived.blocked.iter().map(|(id, _)| id.as_str()).collect();

    if stored_now == derived_now_str
        && stored_next == derived_next_str
        && stored_blocked == derived_blocked_str
    {
        return vec![];
    }

    // Build a compact diff for the warning message.
    let mut diffs: Vec<String> = Vec::new();
    if stored_now != derived_now_str {
        diffs.push(format!(
            "now: stored={:?} derived={:?}",
            sorted_set(&stored_now),
            sorted_set(&derived_now_str),
        ));
    }
    if stored_next != derived_next_str {
        diffs.push(format!(
            "next: stored={:?} derived={:?}",
            sorted_set(&stored_next),
            sorted_set(&derived_next_str),
        ));
    }
    if stored_blocked != derived_blocked_str {
        diffs.push(format!(
            "blocked: stored={:?} derived={:?}",
            sorted_set(&stored_blocked),
            sorted_set(&derived_blocked_str),
        ));
    }

    vec![Diagnostic::warning(
        &src.abs_path,
        "W_STATE_FOCUS_DRIFT",
        format!(
            "focus snapshot has drifted from tracks[] derivation — {}",
            diffs.join("; ")
        ),
    )]
}

// ---------------------------------------------------------------------------
// Cross-repo edge derivation (MV.3B.T)
// ---------------------------------------------------------------------------

/// Derive cross-repo block dependency edges from all loaded state files.
///
/// For every `tracks[].blocks[].depends_on` entry of `{type:"block"}` where the
/// dependency's `repo` differs from the owning repo, produce a [`CrossRepoEdge`].
/// Same-repo deps and `{type:"external"}` entries are skipped — they are not
/// cross-repo structural edges.
pub fn derive_cross_repo(files: &[(StateSource, StateFile)]) -> Vec<CrossRepoEdge> {
    let mut edges: Vec<CrossRepoEdge> = Vec::new();

    for (src, file) in files {
        for track in &file.tracks {
            for block in &track.blocks {
                for dep in &block.depends_on {
                    if let BlockedBy::Block { repo, id, what } = dep
                        && repo != &src.repo_slug
                    {
                        edges.push(CrossRepoEdge {
                            from: Endpoint {
                                repo: src.repo_slug.clone(),
                                id: block.id.clone(),
                            },
                            to: Endpoint {
                                repo: repo.clone(),
                                id: id.clone(),
                            },
                            note: what.clone(),
                        });
                    }
                }
            }
        }
    }

    edges
}

// ---------------------------------------------------------------------------
// Rollup derivation (MV.3B.T)
// ---------------------------------------------------------------------------

/// Derive the brain `repos[]` rollup from the loaded leaf (`kind == "project"`) state files.
///
/// For each child with `kind == "project"`, calls [`derive_focus`] and maps block IDs back
/// to [`Block`] structs using the child's own `tracks[]` for titles.  The `tier` field is
/// left `None` — it is not derivable from state alone (it comes from the brain config).
///
/// The `graph` and `files` parameters are forwarded to [`derive_focus`] / [`ready_order`]
/// so cross-repo dependency statuses are resolved correctly.
pub fn derive_rollup(
    children: &[(StateSource, StateFile)],
    graph: &StateGraph,
    files: &[(StateSource, StateFile)],
) -> Vec<RepoRollup> {
    children
        .iter()
        .filter(|(_, f)| f.kind == "project")
        .map(|(src, file)| {
            let derived = derive_focus(src, file, graph, files);

            // Build a title lookup from this child's tracks[].
            let mut title_map: std::collections::HashMap<String, String> =
                std::collections::HashMap::new();
            for track in &file.tracks {
                for block in &track.blocks {
                    title_map.insert(block.id.clone(), block.title.clone());
                }
            }
            let title_of = |id: &str| title_map.get(id).cloned().unwrap_or_default();

            let now = derived
                .now
                .iter()
                .map(|id| Block {
                    id: id.clone(),
                    title: title_of(id),
                    status: Some("in_progress".to_string()),
                    note: None,
                    repo: None,
                    blocked_by: Vec::new(),
                })
                .collect();

            let next = derived
                .next
                .iter()
                .map(|id| Block {
                    id: id.clone(),
                    title: title_of(id),
                    status: None,
                    note: None,
                    repo: None,
                    blocked_by: Vec::new(),
                })
                .collect();

            let blocked = derived
                .blocked
                .iter()
                .map(|(id, unmet)| Block {
                    id: id.clone(),
                    title: title_of(id),
                    status: None,
                    note: None,
                    repo: None,
                    blocked_by: unmet.clone(),
                })
                .collect();

            RepoRollup {
                repo: src.repo_slug.clone(),
                tier: None,
                now,
                next,
                blocked,
            }
        })
        .collect()
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
      {{ "id": "MV.3.J", "title": "Graph integrity", "status": "closed" }}
    ],
    "next": [
      {{ "id": "MV.3.K", "title": "Link integrity" }}
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
      { "id": "BA.11.C0", "repo": "bastion", "title": "Manifest engine", "status": "in_progress" }
    ],
    "next": [
      { "id": "BA.11.C", "repo": "bastion", "title": "WebSocket hub" }
    ],
    "blocked": []
  },
  "repos": [
    {
      "repo": "bastion",
      "now": [{ "id": "BA.11.C0", "title": "Manifest engine", "status": "in_progress" }],
      "next": [{ "id": "BA.11.C", "title": "WebSocket hub" }],
      "blocked": []
    }
  ],
  "cross_repo": [
    {
      "from": { "repo": "bastion-ui", "id": "BU.1.A" },
      "to": { "repo": "bastion", "id": "BA.11.C" },
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
      { "id": "BA.11.C0", "repo": "bastion", "title": "Manifest engine", "status": "in_progress" }
    ],
    "next": [],
    "blocked": [
      {
        "id": "OR.B",
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
      "now": [{ "id": "BA.11.C0", "title": "Manifest engine", "status": "in_progress" }],
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
      { "id": "T.1", "title": "Work", "status": "flying" }
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
        "id": "T.1",
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
    "now": [{{ "id": "{block_id}", "title": "Work", "status": "in_progress" }}],
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

        // beta's tracks[].blocks[].depends_on references alpha's block "AL.1.GHOST" which does NOT exist
        let beta_json = r#"{
  "repo": "beta",
  "kind": "project",
  "updated": "2026-06-29",
  "focus": {
    "now": [],
    "next": [],
    "blocked": [
      { "id": "BE.1.A", "title": "Blocked work" }
    ]
  },
  "tracks": [{
    "title": "Phase 1",
    "blocks": [{
      "id": "BE.1.A",
      "title": "Blocked work",
      "depends_on": [
        { "type": "block", "repo": "alpha", "id": "AL.1.GHOST" }
      ]
    }]
  }]
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

        // Only one repo in the corpus; the block's depends_on references an unknown repo.
        let alpha_json = r#"{
  "repo": "alpha",
  "kind": "project",
  "updated": "2026-06-29",
  "focus": {
    "now": [],
    "next": [],
    "blocked": [{ "id": "AL.1.A", "title": "Blocked" }]
  },
  "tracks": [{
    "title": "P1",
    "blocks": [{
      "id": "AL.1.A",
      "title": "Blocked",
      "depends_on": [
        { "type": "block", "repo": "ghost-repo", "id": "GH.1.X" }
      ]
    }]
  }]
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
    "now": [{ "id": "AL.1.GHOST", "title": "Missing", "status": "in_progress" }],
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
      "from": { "repo": "alpha", "id": "AL.1.A" },
      "to": { "repo": "alpha", "id": "AL.1.GHOST" }
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
    // Task 2 — depends_on DAG edges + check_schema v2 checks
    // -----------------------------------------------------------------------

    /// A v2 leaf fixture with `depends_on` wired through `tracks[].blocks[]`.
    fn leaf_with_depends_on(
        dir: &std::path::Path,
        repo: &str,
        block_id: &str,
        dep_repo: &str,
        dep_id: &str,
    ) -> (StateSource, StateFile) {
        let json = format!(
            r#"{{
  "repo": "{repo}",
  "kind": "project",
  "updated": "2026-06-29",
  "focus": {{
    "now": [{{ "id": "{block_id}", "title": "Waiting", "status": "open" }}],
    "next": [],
    "blocked": []
  }},
  "tracks": [{{
    "title": "Phase 1",
    "blocks": [{{
      "id": "{block_id}",
      "title": "Waiting",
      "status": "open",
      "depends_on": [
        {{ "type": "block", "repo": "{dep_repo}", "id": "{dep_id}" }}
      ]
    }}]
  }}]
}}"#
        );
        make_pair(dir, &format!("{repo}-state.json"), "project", &json)
    }

    #[test]
    fn build_depends_on_edges_from_tracks() {
        let dir = tempfile::tempdir().expect("tempdir");

        // alpha has block AL.1.A that depends_on beta:BE.1.A
        let pair_alpha = leaf_with_depends_on(dir.path(), "alpha", "AL.1.A", "beta", "BE.1.A");
        let pair_beta = leaf_pair(dir.path(), "beta", "BE.1.A");
        let files = vec![pair_alpha, pair_beta];

        let graph = build_state_graph(&files);

        // Two nodes (one per repo)
        assert_eq!(
            graph.nodes.len(),
            2,
            "expected 2 nodes, got: {:?}",
            graph.nodes
        );

        // One BlockedBy edge from depends_on
        let bb_edges: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| e.kind == StateEdgeKind::BlockedBy)
            .collect();
        assert_eq!(
            bb_edges.len(),
            1,
            "expected exactly one BlockedBy edge from depends_on, got: {:?}",
            graph.edges
        );
        assert_eq!(bb_edges[0].from, "alpha:AL.1.A");
        assert_eq!(bb_edges[0].to_ref, "beta:BE.1.A");
    }

    #[test]
    fn build_external_depends_on_entries_are_excluded_from_edges() {
        let dir = tempfile::tempdir().expect("tempdir");

        // alpha has a block that depends_on an external constraint only — no graph edges.
        let alpha_json = r#"{
  "repo": "alpha",
  "kind": "project",
  "updated": "2026-06-29",
  "focus": { "now": [], "next": [], "blocked": [] },
  "tracks": [{
    "title": "Phase 1",
    "blocks": [{
      "id": "AL.1.A",
      "title": "Needs hardware",
      "status": "open",
      "depends_on": [
        { "type": "external", "what": "New Mac Mini delivery" }
      ]
    }]
  }]
}"#;
        let pair = make_pair(dir.path(), "alpha-state.json", "project", alpha_json);
        let files = vec![pair];

        let graph = build_state_graph(&files);

        assert_eq!(graph.nodes.len(), 1, "expected 1 node");
        assert_eq!(
            graph.edges.len(),
            0,
            "external depends_on must not produce a graph edge, got: {:?}",
            graph.edges
        );
    }

    #[test]
    fn build_no_edges_from_focus_blocked_by() {
        // Confirm that focus.blocked_by is NOT an edge source in v2.
        let dir = tempfile::tempdir().expect("tempdir");

        // alpha has a focus.blocked entry with blocked_by — should NOT produce an edge.
        let alpha_json = r#"{
  "repo": "alpha",
  "kind": "project",
  "updated": "2026-06-29",
  "focus": {
    "now": [],
    "next": [],
    "blocked": [{
      "id": "AL.1.A",
      "title": "Waiting",
      "blocked_by": [
        { "type": "block", "repo": "beta", "id": "BE.1.A" }
      ]
    }]
  },
  "tracks": [{ "title": "P1", "blocks": [{ "id": "AL.1.A", "title": "Waiting" }] }]
}"#;
        let pair = make_pair(dir.path(), "alpha-state.json", "project", alpha_json);
        let files = vec![pair];

        let graph = build_state_graph(&files);

        assert_eq!(
            graph.edges.len(),
            0,
            "focus.blocked_by must not produce edges in v2; got: {:?}",
            graph.edges
        );
    }

    #[test]
    fn check_schema_authored_blocked_emits_e_state_authored_blocked() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        let src = make_source(&path, "project");

        // A track block with authored status "blocked" — this is illegal in v2.
        let json = r#"{
  "repo": "test",
  "kind": "project",
  "updated": "2026-06-29",
  "focus": { "now": [], "next": [], "blocked": [] },
  "tracks": [{
    "title": "Phase 1",
    "blocks": [
      { "id": "T.1", "title": "Should not be authored blocked", "status": "blocked" }
    ]
  }]
}"#;
        let file = parse_file(json);
        let diags = check_schema(&src, &file);

        let authored_blocked: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_AUTHORED_BLOCKED")
            .collect();
        assert_eq!(
            authored_blocked.len(),
            1,
            "expected exactly one E_STATE_AUTHORED_BLOCKED, got: {diags:?}"
        );
        assert!(
            authored_blocked[0].message.contains("T.1"),
            "E_STATE_AUTHORED_BLOCKED message should name the block id, got: {:?}",
            authored_blocked[0].message
        );
    }

    #[test]
    fn check_schema_bad_backlog_status_emits_e_state_schema_bad_status() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        let src = make_source(&path, "brain");

        // A backlog item with an invalid status value.
        let json = r#"{
  "repo": "hq",
  "kind": "brain",
  "updated": "2026-06-29",
  "focus": { "now": [], "next": [], "blocked": [] },
  "repos": [{ "repo": "some-child", "now": [], "next": [], "blocked": [] }],
  "backlog": [
    {
      "slug": "my-idea",
      "title": "Some idea",
      "repo": "mev",
      "type": "feature",
      "status": "flying"
    }
  ]
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
            "expected exactly one E_STATE_SCHEMA_BAD_STATUS for bad backlog status, got: {diags:?}"
        );
        assert!(
            bad_status[0].message.contains("my-idea"),
            "E_STATE_SCHEMA_BAD_STATUS message should name the backlog slug, got: {:?}",
            bad_status[0].message
        );
    }

    #[test]
    fn check_schema_valid_backlog_statuses_pass() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        let src = make_source(&path, "brain");

        for status in &["idea", "ready", "promoted"] {
            let json = format!(
                r#"{{
  "repo": "hq",
  "kind": "brain",
  "updated": "2026-06-29",
  "focus": {{ "now": [], "next": [], "blocked": [] }},
  "repos": [{{ "repo": "c", "now": [], "next": [], "blocked": [] }}],
  "backlog": [
    {{
      "slug": "item-{status}",
      "title": "Test",
      "repo": "mev",
      "type": "feature",
      "status": "{status}"
    }}
  ]
}}"#
            );
            let file = parse_file(&json);
            let diags = check_schema(&src, &file);
            let bad: Vec<_> = diags
                .iter()
                .filter(|d| {
                    d.locator == "E_STATE_SCHEMA_BAD_STATUS"
                        || d.locator == "E_STATE_AUTHORED_BLOCKED"
                })
                .collect();
            assert!(
                bad.is_empty(),
                "backlog status '{status}' should be valid, got: {diags:?}"
            );
        }
    }

    #[test]
    fn check_schema_clean_v2_file_with_depends_on_passes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        let src = make_source(&path, "project");

        // A clean v2 leaf with depends_on in tracks and backlog-free.
        let json = r#"{
  "repo": "test",
  "kind": "project",
  "updated": "2026-06-29",
  "focus": {
    "now": [{ "id": "T.2", "title": "Active", "status": "in_progress" }],
    "next": [{ "id": "T.1", "title": "Ready" }],
    "blocked": []
  },
  "tracks": [{
    "title": "Phase 1",
    "blocks": [
      { "id": "T.1", "title": "Ready", "status": "open" },
      {
        "id": "T.2",
        "title": "Active",
        "status": "in_progress",
        "depends_on": [
          { "type": "block", "repo": "other", "id": "OT.1.A" }
        ],
        "wave": 1
      }
    ]
  }]
}"#;
        let file = parse_file(json);
        let diags = check_schema(&src, &file);
        let errors: Vec<_> = diags
            .iter()
            .filter(|d| d.severity == crate::Severity::Error)
            .collect();
        assert!(
            errors.is_empty(),
            "clean v2 file with depends_on should produce no schema errors, got: {diags:?}"
        );
    }

    #[test]
    fn check_schema_depends_on_empty_repo_emits_bad_blocked_by() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        let src = make_source(&path, "project");

        // A track block whose depends_on entry has an empty repo field.
        let json = r#"{
  "repo": "test",
  "kind": "project",
  "updated": "2026-06-29",
  "focus": { "now": [], "next": [], "blocked": [] },
  "tracks": [{
    "title": "Phase 1",
    "blocks": [{
      "id": "T.1",
      "title": "Work",
      "depends_on": [
        { "type": "block", "repo": "", "id": "OT.1.A" }
      ]
    }]
  }]
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
            "expected one E_STATE_SCHEMA_BAD_BLOCKED_BY for empty depends_on repo, got: {diags:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Task 3 — detect_cycles tests
    // -----------------------------------------------------------------------

    /// Build a `StateGraph` directly from node keys and BlockedBy edge tuples.
    /// `nodes`: (repo, id); `edges`: (from_key, to_ref).
    /// Uses a stable fake path derived from `from_key` for each edge's `source_path`.
    fn make_cycle_graph(
        dir: &std::path::Path,
        nodes: &[(&str, &str)],
        edges: &[(&str, &str)],
    ) -> StateGraph {
        let node_vec: Vec<StateNode> = nodes
            .iter()
            .map(|(repo, id)| StateNode {
                key: format!("{repo}:{id}"),
                repo: repo.to_string(),
                id: id.to_string(),
                title: format!("{repo}:{id}"),
                source_path: dir.join(format!("{repo}.json")),
            })
            .collect();
        let edge_vec: Vec<StateEdge> = edges
            .iter()
            .map(|(from, to_ref)| {
                let repo = from.split(':').next().unwrap_or("unknown");
                StateEdge {
                    from: from.to_string(),
                    to_ref: to_ref.to_string(),
                    kind: StateEdgeKind::BlockedBy,
                    source_path: dir.join(format!("{repo}.json")),
                }
            })
            .collect();
        StateGraph {
            nodes: node_vec,
            edges: edge_vec,
        }
    }

    #[test]
    fn detect_cycles_simple_two_node_cycle_is_flagged() {
        let dir = tempfile::tempdir().expect("tempdir");
        // a:X depends_on b:Y, b:Y depends_on a:X → cycle
        let graph = make_cycle_graph(
            dir.path(),
            &[("a", "X"), ("b", "Y")],
            &[("a:X", "b:Y"), ("b:Y", "a:X")],
        );

        let diags = detect_cycles(&graph);
        let cycles: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_CYCLE")
            .collect();
        assert_eq!(
            cycles.len(),
            1,
            "expected exactly one E_STATE_CYCLE for two-node cycle, got: {diags:?}"
        );
        // The message should contain both node keys.
        assert!(
            cycles[0].message.contains("a:X") && cycles[0].message.contains("b:Y"),
            "E_STATE_CYCLE message should name the cycle nodes, got: {:?}",
            cycles[0].message
        );
    }

    #[test]
    fn detect_cycles_three_node_cycle_path_is_flagged() {
        let dir = tempfile::tempdir().expect("tempdir");
        // a:X → b:Y → c:Z → a:X
        let graph = make_cycle_graph(
            dir.path(),
            &[("a", "X"), ("b", "Y"), ("c", "Z")],
            &[("a:X", "b:Y"), ("b:Y", "c:Z"), ("c:Z", "a:X")],
        );

        let diags = detect_cycles(&graph);
        let cycles: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_CYCLE")
            .collect();
        assert_eq!(
            cycles.len(),
            1,
            "expected exactly one E_STATE_CYCLE for three-node cycle, got: {diags:?}"
        );
        // All three keys must appear in the message.
        assert!(
            cycles[0].message.contains("a:X")
                && cycles[0].message.contains("b:Y")
                && cycles[0].message.contains("c:Z"),
            "E_STATE_CYCLE message should name all cycle nodes, got: {:?}",
            cycles[0].message
        );
    }

    #[test]
    fn detect_cycles_acyclic_dag_passes() {
        let dir = tempfile::tempdir().expect("tempdir");
        // a:X → b:Y → c:Z (no cycle)
        let graph = make_cycle_graph(
            dir.path(),
            &[("a", "X"), ("b", "Y"), ("c", "Z")],
            &[("a:X", "b:Y"), ("b:Y", "c:Z")],
        );

        let diags = detect_cycles(&graph);
        let cycles: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_CYCLE")
            .collect();
        assert!(
            cycles.is_empty(),
            "acyclic DAG should produce no E_STATE_CYCLE, got: {diags:?}"
        );
    }

    #[test]
    fn detect_cycles_self_loop_is_flagged() {
        let dir = tempfile::tempdir().expect("tempdir");
        // a:X depends_on itself
        let graph = make_cycle_graph(dir.path(), &[("a", "X")], &[("a:X", "a:X")]);

        let diags = detect_cycles(&graph);
        let cycles: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_CYCLE")
            .collect();
        assert_eq!(
            cycles.len(),
            1,
            "self-loop should produce exactly one E_STATE_CYCLE, got: {diags:?}"
        );
        assert!(
            cycles[0].message.contains("a:X"),
            "E_STATE_CYCLE message should name the self-loop node, got: {:?}",
            cycles[0].message
        );
    }

    #[test]
    fn detect_cycles_cross_repo_edges_are_excluded() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Two nodes with a CrossRepo edge — CrossRepo edges must NOT trigger cycle detection.
        let node_vec = vec![
            StateNode {
                key: "a:X".to_string(),
                repo: "a".to_string(),
                id: "X".to_string(),
                title: "X".to_string(),
                source_path: dir.path().join("a.json"),
            },
            StateNode {
                key: "b:Y".to_string(),
                repo: "b".to_string(),
                id: "Y".to_string(),
                title: "Y".to_string(),
                source_path: dir.path().join("b.json"),
            },
        ];
        // CrossRepo edges forming a "cycle" — should be ignored by detect_cycles.
        let edge_vec = vec![
            StateEdge {
                from: "a:X".to_string(),
                to_ref: "b:Y".to_string(),
                kind: StateEdgeKind::CrossRepo,
                source_path: dir.path().join("brain.json"),
            },
            StateEdge {
                from: "b:Y".to_string(),
                to_ref: "a:X".to_string(),
                kind: StateEdgeKind::CrossRepo,
                source_path: dir.path().join("brain.json"),
            },
        ];
        let graph = StateGraph {
            nodes: node_vec,
            edges: edge_vec,
        };

        let diags = detect_cycles(&graph);
        let cycles: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_CYCLE")
            .collect();
        assert!(
            cycles.is_empty(),
            "CrossRepo edges must not trigger E_STATE_CYCLE, got: {diags:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Task 3 — ready_order tests
    // -----------------------------------------------------------------------

    /// Build a minimal (StateSource, StateFile) pair for ready_order testing.
    fn make_ready_pair(
        dir: &std::path::Path,
        repo: &str,
        blocks: &[(&str, Option<&str>, Option<i64>, Vec<BlockedBy>)],
    ) -> (StateSource, StateFile) {
        // blocks: (id, status, wave, depends_on)
        let track_blocks: Vec<TrackBlock> = blocks
            .iter()
            .map(|(id, status, wave, deps)| TrackBlock {
                id: id.to_string(),
                title: id.to_string(),
                status: status.map(|s| s.to_string()),
                depends_on: deps.clone(),
                wave: *wave,
                origin: None,
            })
            .collect();

        let path = dir.join(format!("{repo}-state.json"));
        let file = StateFile {
            repo: repo.to_string(),
            kind: "project".to_string(),
            updated: "2026-06-30".to_string(),
            focus: Focus::default(),
            tracks: vec![Track {
                title: "Phase 1".to_string(),
                blocks: track_blocks,
            }],
            repos: vec![],
            cross_repo: vec![],
            tiers: vec![],
            note: None,
            backlog: vec![],
            carryover: vec![],
        };
        let src = StateSource {
            repo_slug: repo.to_string(),
            abs_path: path,
            expected_kind: "project",
        };
        (src, file)
    }

    #[test]
    fn ready_order_no_deps_open_block_is_ready() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pair = make_ready_pair(
            dir.path(),
            "alpha",
            &[("AL.1.A", Some("open"), None, vec![])],
        );
        let files = vec![pair];
        let graph = build_state_graph(&files);

        let order = ready_order(&graph, &files);
        assert_eq!(
            order,
            vec!["alpha:AL.1.A"],
            "open block with no deps should appear in ready_order, got: {order:?}"
        );
    }

    #[test]
    fn ready_order_absent_status_treated_as_open() {
        let dir = tempfile::tempdir().expect("tempdir");
        // No status field — treated as open.
        let pair = make_ready_pair(dir.path(), "alpha", &[("AL.1.A", None, None, vec![])]);
        let files = vec![pair];
        let graph = build_state_graph(&files);

        let order = ready_order(&graph, &files);
        assert_eq!(
            order,
            vec!["alpha:AL.1.A"],
            "absent-status block should be treated as open and appear in ready_order"
        );
    }

    #[test]
    fn ready_order_closed_block_excluded() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pair = make_ready_pair(
            dir.path(),
            "alpha",
            &[("AL.1.A", Some("closed"), None, vec![])],
        );
        let files = vec![pair];
        let graph = build_state_graph(&files);

        let order = ready_order(&graph, &files);
        assert!(
            order.is_empty(),
            "closed block must not appear in ready_order, got: {order:?}"
        );
    }

    #[test]
    fn ready_order_in_progress_block_excluded() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pair = make_ready_pair(
            dir.path(),
            "alpha",
            &[("AL.1.A", Some("in_progress"), None, vec![])],
        );
        let files = vec![pair];
        let graph = build_state_graph(&files);

        let order = ready_order(&graph, &files);
        assert!(
            order.is_empty(),
            "in_progress block must not appear in ready_order, got: {order:?}"
        );
    }

    #[test]
    fn ready_order_block_with_external_dep_excluded() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ext_dep = BlockedBy::External {
            what: "Mac Mini delivery".to_string(),
        };
        let pair = make_ready_pair(
            dir.path(),
            "alpha",
            &[("AL.1.A", Some("open"), None, vec![ext_dep])],
        );
        let files = vec![pair];
        let graph = build_state_graph(&files);

        let order = ready_order(&graph, &files);
        assert!(
            order.is_empty(),
            "open block with external dep must not appear in ready_order, got: {order:?}"
        );
    }

    #[test]
    fn ready_order_block_with_unclosed_block_dep_excluded() {
        let dir = tempfile::tempdir().expect("tempdir");
        // alpha:AL.1.A depends_on beta:BE.1.A which is open (not closed).
        let block_dep = BlockedBy::Block {
            repo: "beta".to_string(),
            id: "BE.1.A".to_string(),
            what: None,
        };
        let pair_a = make_ready_pair(
            dir.path(),
            "alpha",
            &[("AL.1.A", Some("open"), None, vec![block_dep])],
        );
        // beta:BE.1.A is open (not closed).
        let pair_b = make_ready_pair(
            dir.path(),
            "beta",
            &[("BE.1.A", Some("open"), None, vec![])],
        );
        let files = vec![pair_a, pair_b];
        let graph = build_state_graph(&files);

        let order = ready_order(&graph, &files);
        // beta:BE.1.A is open → alpha:AL.1.A is not ready
        // beta:BE.1.A has no deps → it IS ready
        assert!(
            !order.contains(&"alpha:AL.1.A".to_string()),
            "alpha:AL.1.A should not be ready when its dep is open; order={order:?}"
        );
        assert!(
            order.contains(&"beta:BE.1.A".to_string()),
            "beta:BE.1.A (no deps, open) should be ready; order={order:?}"
        );
    }

    #[test]
    fn ready_order_block_with_closed_dep_is_ready() {
        let dir = tempfile::tempdir().expect("tempdir");
        // alpha:AL.1.A depends_on beta:BE.1.A which is closed → AL.1.A is ready.
        let block_dep = BlockedBy::Block {
            repo: "beta".to_string(),
            id: "BE.1.A".to_string(),
            what: None,
        };
        let pair_a = make_ready_pair(
            dir.path(),
            "alpha",
            &[("AL.1.A", Some("open"), None, vec![block_dep])],
        );
        let pair_b = make_ready_pair(
            dir.path(),
            "beta",
            &[("BE.1.A", Some("closed"), None, vec![])],
        );
        let files = vec![pair_a, pair_b];
        let graph = build_state_graph(&files);

        let order = ready_order(&graph, &files);
        assert!(
            order.contains(&"alpha:AL.1.A".to_string()),
            "alpha:AL.1.A should be ready when its only dep is closed; order={order:?}"
        );
    }

    #[test]
    fn ready_order_wave_ordering_applied() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Three open blocks: wave 3, wave 1, wave 2 → should come out 1, 2, 3.
        let pair = make_ready_pair(
            dir.path(),
            "alpha",
            &[
                ("AL.1.C", Some("open"), Some(3), vec![]),
                ("AL.1.A", Some("open"), Some(1), vec![]),
                ("AL.1.B", Some("open"), Some(2), vec![]),
            ],
        );
        let files = vec![pair];
        let graph = build_state_graph(&files);

        let order = ready_order(&graph, &files);
        assert_eq!(
            order,
            vec!["alpha:AL.1.A", "alpha:AL.1.B", "alpha:AL.1.C"],
            "ready_order should sort by wave ascending, got: {order:?}"
        );
    }

    #[test]
    fn ready_order_none_wave_goes_last() {
        let dir = tempfile::tempdir().expect("tempdir");
        // One block with wave=1, one with no wave → wave=1 should come first.
        let pair = make_ready_pair(
            dir.path(),
            "alpha",
            &[
                ("AL.1.B", Some("open"), None, vec![]), // no wave → last
                ("AL.1.A", Some("open"), Some(1), vec![]),
            ],
        );
        let files = vec![pair];
        let graph = build_state_graph(&files);

        let order = ready_order(&graph, &files);
        assert_eq!(
            order,
            vec!["alpha:AL.1.A", "alpha:AL.1.B"],
            "block with wave=None should sort after wave=1, got: {order:?}"
        );
    }

    #[test]
    fn ready_order_equal_wave_preserves_iteration_order() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Three open blocks all with wave=1 → should preserve array order.
        let pair = make_ready_pair(
            dir.path(),
            "alpha",
            &[
                ("AL.1.A", Some("open"), Some(1), vec![]),
                ("AL.1.B", Some("open"), Some(1), vec![]),
                ("AL.1.C", Some("open"), Some(1), vec![]),
            ],
        );
        let files = vec![pair];
        let graph = build_state_graph(&files);

        let order = ready_order(&graph, &files);
        assert_eq!(
            order,
            vec!["alpha:AL.1.A", "alpha:AL.1.B", "alpha:AL.1.C"],
            "equal-wave blocks should preserve array order, got: {order:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Task 4 — status consistency + backlog integrity tests
    // -----------------------------------------------------------------------

    // Helper: build a (StateSource, StateFile) pair with blocks that have known statuses
    // and depends_on edges. Each block tuple is (id, status, Vec<depends_on>).
    fn make_consistency_pair(
        dir: &std::path::Path,
        repo: &str,
        blocks: &[(&str, Option<&str>, Vec<BlockedBy>)],
    ) -> (StateSource, StateFile) {
        let track_blocks: Vec<TrackBlock> = blocks
            .iter()
            .map(|(id, status, deps)| TrackBlock {
                id: id.to_string(),
                title: id.to_string(),
                status: status.map(|s| s.to_string()),
                depends_on: deps.clone(),
                wave: None,
                origin: None,
            })
            .collect();

        let path = dir.join(format!("{repo}-state.json"));
        let file = StateFile {
            repo: repo.to_string(),
            kind: "project".to_string(),
            updated: "2026-06-30".to_string(),
            focus: Focus::default(),
            tracks: vec![Track {
                title: "Phase 1".to_string(),
                blocks: track_blocks,
            }],
            repos: vec![],
            cross_repo: vec![],
            tiers: vec![],
            note: None,
            backlog: vec![],
            carryover: vec![],
        };
        let src = StateSource {
            repo_slug: repo.to_string(),
            abs_path: path,
            expected_kind: "project",
        };
        (src, file)
    }

    #[test]
    fn check_status_consistency_closed_depends_on_open_emits_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        // alpha has two blocks: AL.1.A (open) and AL.1.B (closed, depends on AL.1.A).
        let dep = BlockedBy::Block {
            repo: "alpha".to_string(),
            id: "AL.1.A".to_string(),
            what: None,
        };
        let pair = make_consistency_pair(
            dir.path(),
            "alpha",
            &[
                ("AL.1.A", Some("open"), vec![]),
                ("AL.1.B", Some("closed"), vec![dep]),
            ],
        );
        let files = vec![pair];
        let diags = check_status_consistency(&files);

        let errs: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_STATUS_INCONSISTENT")
            .collect();
        assert_eq!(
            errs.len(),
            1,
            "closed block depending on open block should emit exactly one \
             E_STATE_STATUS_INCONSISTENT, got: {diags:?}"
        );
    }

    #[test]
    fn check_status_consistency_closed_depends_on_closed_passes() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Both blocks are closed — no inconsistency.
        let dep = BlockedBy::Block {
            repo: "alpha".to_string(),
            id: "AL.1.A".to_string(),
            what: None,
        };
        let pair = make_consistency_pair(
            dir.path(),
            "alpha",
            &[
                ("AL.1.A", Some("closed"), vec![]),
                ("AL.1.B", Some("closed"), vec![dep]),
            ],
        );
        let files = vec![pair];
        let diags = check_status_consistency(&files);

        let errs: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_STATUS_INCONSISTENT")
            .collect();
        assert!(
            errs.is_empty(),
            "closed block depending on closed block should not emit errors, got: {diags:?}"
        );
    }

    #[test]
    fn check_status_consistency_closed_depends_on_in_progress_emits_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let dep = BlockedBy::Block {
            repo: "alpha".to_string(),
            id: "AL.1.A".to_string(),
            what: None,
        };
        let pair = make_consistency_pair(
            dir.path(),
            "alpha",
            &[
                ("AL.1.A", Some("in_progress"), vec![]),
                ("AL.1.B", Some("closed"), vec![dep]),
            ],
        );
        let files = vec![pair];
        let diags = check_status_consistency(&files);

        let errs: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_STATUS_INCONSISTENT")
            .collect();
        assert_eq!(
            errs.len(),
            1,
            "closed block depending on in_progress block should emit E_STATE_STATUS_INCONSISTENT, \
             got: {diags:?}"
        );
    }

    #[test]
    fn check_status_consistency_dangling_dep_is_skipped_silently() {
        let dir = tempfile::tempdir().expect("tempdir");
        // AL.1.B is closed but depends on "alpha:AL.1.GHOST" which is not in any file.
        // Should NOT emit E_STATE_STATUS_INCONSISTENT (that's E_STATE_DANGLING_BLOCKED_BY's job).
        let dep = BlockedBy::Block {
            repo: "alpha".to_string(),
            id: "AL.1.GHOST".to_string(),
            what: None,
        };
        let pair = make_consistency_pair(
            dir.path(),
            "alpha",
            &[("AL.1.B", Some("closed"), vec![dep])],
        );
        let files = vec![pair];
        let diags = check_status_consistency(&files);

        let errs: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_STATUS_INCONSISTENT")
            .collect();
        assert!(
            errs.is_empty(),
            "dangling dep (not in any loaded file) should not produce \
             E_STATE_STATUS_INCONSISTENT (that's check_state_graph's job), got: {diags:?}"
        );
    }

    // Helper: build a (StateSource, StateFile) pair that has a backlog[] array.
    fn make_brain_with_backlog(
        dir: &std::path::Path,
        repo: &str,
        track_blocks: Vec<TrackBlock>,
        backlog_nodes: Vec<Backlog>,
    ) -> (StateSource, StateFile) {
        let path = dir.join(format!("{repo}-state.json"));
        let file = StateFile {
            repo: repo.to_string(),
            kind: "brain".to_string(),
            updated: "2026-06-30".to_string(),
            focus: Focus::default(),
            tracks: if track_blocks.is_empty() {
                vec![]
            } else {
                vec![Track {
                    title: "Phase 1".to_string(),
                    blocks: track_blocks,
                }]
            },
            repos: vec![],
            cross_repo: vec![],
            tiers: vec![],
            note: None,
            backlog: backlog_nodes,
            carryover: vec![],
        };
        let src = StateSource {
            repo_slug: repo.to_string(),
            abs_path: path,
            expected_kind: "brain",
        };
        (src, file)
    }

    #[test]
    fn check_backlog_integrity_dangling_depends_on_emits_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Backlog node depends on "mev:MV.3.GHOST" which doesn't exist in any tracks[].
        let backlog_node = Backlog {
            slug: "add-foo".to_string(),
            title: "Add foo".to_string(),
            repo: "mev".to_string(),
            kind: "feature".to_string(),
            status: "idea".to_string(),
            depends_on: vec![BlockedBy::Block {
                repo: "mev".to_string(),
                id: "MV.3.GHOST".to_string(),
                what: None,
            }],
            block: None,
            notes: None,
        };
        let pair = make_brain_with_backlog(dir.path(), "hq", vec![], vec![backlog_node]);
        let files = vec![pair];
        let graph = build_state_graph(&files);
        let diags = check_backlog_integrity(&files, &graph);

        let errs: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_DANGLING_BLOCKED_BY")
            .collect();
        assert_eq!(
            errs.len(),
            1,
            "backlog node with dangling depends_on should emit E_STATE_DANGLING_BLOCKED_BY, \
             got: {diags:?}"
        );
    }

    #[test]
    fn check_backlog_integrity_promoted_no_block_pointer_emits_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Backlog node with status "promoted" but no block pointer.
        let backlog_node = Backlog {
            slug: "add-bar".to_string(),
            title: "Add bar".to_string(),
            repo: "mev".to_string(),
            kind: "feature".to_string(),
            status: "promoted".to_string(),
            depends_on: vec![],
            block: None, // missing pointer
            notes: None,
        };
        let pair = make_brain_with_backlog(dir.path(), "hq", vec![], vec![backlog_node]);
        let files = vec![pair];
        let graph = build_state_graph(&files);
        let diags = check_backlog_integrity(&files, &graph);

        let errs: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_DANGLING_PROMOTION")
            .collect();
        assert_eq!(
            errs.len(),
            1,
            "promoted backlog node with no block pointer should emit E_STATE_DANGLING_PROMOTION, \
             got: {diags:?}"
        );
    }

    #[test]
    fn check_backlog_integrity_promoted_block_pointer_resolves_to_nothing_emits_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Backlog node with status "promoted" pointing at a block that doesn't exist.
        let backlog_node = Backlog {
            slug: "add-baz".to_string(),
            title: "Add baz".to_string(),
            repo: "mev".to_string(),
            kind: "feature".to_string(),
            status: "promoted".to_string(),
            depends_on: vec![],
            block: Some("MV.3.GHOST".to_string()), // block doesn't exist in mev tracks[]
            notes: None,
        };
        let pair = make_brain_with_backlog(dir.path(), "hq", vec![], vec![backlog_node]);
        let files = vec![pair];
        let graph = build_state_graph(&files);
        let diags = check_backlog_integrity(&files, &graph);

        let errs: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_DANGLING_PROMOTION")
            .collect();
        assert_eq!(
            errs.len(),
            1,
            "promoted backlog node with orphan block pointer should emit \
             E_STATE_DANGLING_PROMOTION, got: {diags:?}"
        );
    }

    #[test]
    fn check_backlog_integrity_clean_promotion_passes() {
        let dir = tempfile::tempdir().expect("tempdir");
        // A promoted backlog node pointing at a real block that carries an origin.
        // The origin back-pointer is structural metadata — the integrity check validates
        // that the block exists in tracks[], not that it carries an origin field.
        let real_block = TrackBlock {
            id: "MV.3.P2".to_string(),
            title: "P2 block".to_string(),
            status: Some("in_progress".to_string()),
            depends_on: vec![],
            wave: Some(1),
            origin: Some(Origin {
                kind: "backlog".to_string(),
                slug: "add-p2".to_string(),
            }),
        };
        let backlog_node = Backlog {
            slug: "add-p2".to_string(),
            title: "Add P2".to_string(),
            repo: "mev".to_string(),
            kind: "feature".to_string(),
            status: "promoted".to_string(),
            depends_on: vec![],
            block: Some("MV.3.P2".to_string()), // resolves to the real block
            notes: None,
        };

        // Two files: a mev leaf (owns the track block) and hq brain (owns the backlog node).
        let mev_path = dir.path().join("mev-state.json");
        let mev_file = StateFile {
            repo: "mev".to_string(),
            kind: "project".to_string(),
            updated: "2026-06-30".to_string(),
            focus: Focus::default(),
            tracks: vec![Track {
                title: "Phase 3".to_string(),
                blocks: vec![real_block],
            }],
            repos: vec![],
            cross_repo: vec![],
            tiers: vec![],
            note: None,
            backlog: vec![],
            carryover: vec![],
        };
        let mev_src = StateSource {
            repo_slug: "mev".to_string(),
            abs_path: mev_path,
            expected_kind: "project",
        };

        let hq_pair = make_brain_with_backlog(dir.path(), "hq", vec![], vec![backlog_node]);
        let files = vec![(mev_src, mev_file), hq_pair];
        let graph = build_state_graph(&files);
        let diags = check_backlog_integrity(&files, &graph);

        let errs: Vec<_> = diags
            .iter()
            .filter(|d| d.severity == crate::Severity::Error)
            .collect();
        assert!(
            errs.is_empty(),
            "clean promotion (block exists in tracks[]) should produce no errors, \
             got: {diags:?}"
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
                    id: id.to_string(),
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
            backlog: vec![],
            carryover: vec![],
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
                    id: id.to_string(),
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
            backlog: vec![],
            carryover: vec![],
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

    // -----------------------------------------------------------------------
    // Task 5 — check_focus_drift tests
    // -----------------------------------------------------------------------

    /// Build a (StateSource, StateFile) pair for focus-drift testing.
    ///
    /// `blocks` = `(id, authored_status, depends_on)`.
    /// `stored_now/next/blocked` are the block IDs stored in the `focus` object.
    fn make_drift_pair(
        dir: &std::path::Path,
        repo: &str,
        blocks: &[(&str, Option<&str>, Vec<BlockedBy>)],
        stored_now: &[&str],
        stored_next: &[&str],
        stored_blocked: &[&str],
    ) -> (StateSource, StateFile) {
        let track_blocks: Vec<TrackBlock> = blocks
            .iter()
            .map(|(id, status, deps)| TrackBlock {
                id: id.to_string(),
                title: id.to_string(),
                status: status.map(|s| s.to_string()),
                depends_on: deps.clone(),
                wave: None,
                origin: None,
            })
            .collect();

        let make_focus_blocks = |ids: &[&str]| -> Vec<Block> {
            ids.iter()
                .map(|id| Block {
                    id: id.to_string(),
                    title: "placeholder".to_string(),
                    status: None,
                    note: None,
                    repo: None,
                    blocked_by: vec![],
                })
                .collect()
        };

        let path = dir.join(format!("{repo}-drift-state.json"));
        let file = StateFile {
            repo: repo.to_string(),
            kind: "project".to_string(),
            updated: "2026-06-30".to_string(),
            focus: Focus {
                now: make_focus_blocks(stored_now),
                next: make_focus_blocks(stored_next),
                blocked: make_focus_blocks(stored_blocked),
            },
            tracks: vec![Track {
                title: "Phase 1".to_string(),
                blocks: track_blocks,
            }],
            repos: vec![],
            cross_repo: vec![],
            tiers: vec![],
            note: None,
            backlog: vec![],
            carryover: vec![],
        };
        let src = StateSource {
            repo_slug: repo.to_string(),
            abs_path: path,
            expected_kind: "project",
        };
        (src, file)
    }

    #[test]
    fn check_focus_drift_in_sync_produces_no_diagnostics() {
        let dir = tempfile::tempdir().expect("tempdir");
        // One in_progress block, one open block (no deps) → it's in next.
        // Stored focus matches exactly.
        let pair = make_drift_pair(
            dir.path(),
            "alpha",
            &[
                ("AL.1.A", Some("in_progress"), vec![]),
                ("AL.1.B", Some("open"), vec![]),
            ],
            &["AL.1.A"], // stored now
            &["AL.1.B"], // stored next
            &[],         // stored blocked
        );
        let files = vec![pair];
        let graph = build_state_graph(&files);
        let (src, file) = &files[0];
        let diags = check_focus_drift(src, file, &graph, &files);

        assert!(
            diags.is_empty(),
            "in-sync focus should produce no diagnostics, got: {diags:?}"
        );
    }

    #[test]
    fn check_focus_drift_now_mismatch_emits_warning() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Block is in_progress in tracks[] but stored focus.now is empty.
        let pair = make_drift_pair(
            dir.path(),
            "alpha",
            &[("AL.1.A", Some("in_progress"), vec![])],
            &[], // stored now — stale (should be ["AL.1.A"])
            &[], // stored next
            &[], // stored blocked
        );
        let files = vec![pair];
        let graph = build_state_graph(&files);
        let (src, file) = &files[0];
        let diags = check_focus_drift(src, file, &graph, &files);

        let drifts: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "W_STATE_FOCUS_DRIFT")
            .collect();
        assert_eq!(
            drifts.len(),
            1,
            "stale now should emit exactly one W_STATE_FOCUS_DRIFT, got: {diags:?}"
        );
        assert_eq!(
            drifts[0].severity,
            crate::Severity::Warning,
            "W_STATE_FOCUS_DRIFT must be Warning severity"
        );
    }

    #[test]
    fn check_focus_drift_next_mismatch_emits_warning() {
        let dir = tempfile::tempdir().expect("tempdir");
        // One open block with no deps → should be in next.
        // Stored focus.next is empty → drift.
        let pair = make_drift_pair(
            dir.path(),
            "alpha",
            &[("AL.1.A", Some("open"), vec![])],
            &[], // stored now
            &[], // stored next — stale (should be ["AL.1.A"])
            &[], // stored blocked
        );
        let files = vec![pair];
        let graph = build_state_graph(&files);
        let (src, file) = &files[0];
        let diags = check_focus_drift(src, file, &graph, &files);

        let drifts: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "W_STATE_FOCUS_DRIFT")
            .collect();
        assert_eq!(
            drifts.len(),
            1,
            "stale next should emit exactly one W_STATE_FOCUS_DRIFT, got: {diags:?}"
        );
    }

    #[test]
    fn check_focus_drift_blocked_mismatch_emits_warning() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Open block with external dep → should be in blocked.
        // Stored focus.blocked is empty → drift.
        let pair = make_drift_pair(
            dir.path(),
            "alpha",
            &[(
                "AL.1.A",
                Some("open"),
                vec![BlockedBy::External {
                    what: "upstream dep".to_string(),
                }],
            )],
            &[], // stored now
            &[], // stored next
            &[], // stored blocked — stale (should be ["AL.1.A"])
        );
        let files = vec![pair];
        let graph = build_state_graph(&files);
        let (src, file) = &files[0];
        let diags = check_focus_drift(src, file, &graph, &files);

        let drifts: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "W_STATE_FOCUS_DRIFT")
            .collect();
        assert_eq!(
            drifts.len(),
            1,
            "stale blocked should emit exactly one W_STATE_FOCUS_DRIFT, got: {diags:?}"
        );
    }

    #[test]
    fn check_focus_drift_drift_is_warning_not_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pair = make_drift_pair(
            dir.path(),
            "alpha",
            &[("AL.1.A", Some("in_progress"), vec![])],
            &[], // stored now — deliberately wrong
            &[],
            &[],
        );
        let files = vec![pair];
        let graph = build_state_graph(&files);
        let (src, file) = &files[0];
        let diags = check_focus_drift(src, file, &graph, &files);

        // No errors should be emitted.
        let errors: Vec<_> = diags
            .iter()
            .filter(|d| d.severity == crate::Severity::Error)
            .collect();
        assert!(
            errors.is_empty(),
            "drift must never produce errors (exit-0 behaviour), got: {diags:?}"
        );
    }

    #[test]
    fn check_focus_drift_empty_tracks_produces_no_diagnostics() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("brain-state.json");
        // Brain file: non-empty focus but no tracks[] — skip.
        let file = StateFile {
            repo: "hq".to_string(),
            kind: "brain".to_string(),
            updated: "2026-06-30".to_string(),
            focus: Focus {
                now: vec![Block {
                    id: "BA.1.A".to_string(),
                    title: "something".to_string(),
                    status: Some("in_progress".to_string()),
                    note: None,
                    repo: Some("bastion".to_string()),
                    blocked_by: vec![],
                }],
                next: vec![],
                blocked: vec![],
            },
            tracks: vec![],
            repos: vec![],
            cross_repo: vec![],
            tiers: vec![],
            note: None,
            backlog: vec![],
            carryover: vec![],
        };
        let src = StateSource {
            repo_slug: "hq".to_string(),
            abs_path: path,
            expected_kind: "brain",
        };
        let files = vec![(src.clone(), file.clone())];
        let graph = build_state_graph(&files);
        let diags = check_focus_drift(&src, &file, &graph, &files);

        assert!(
            diags.is_empty(),
            "empty tracks[] should skip focus drift check, got: {diags:?}"
        );
    }

    #[test]
    fn check_focus_drift_open_with_unclosed_block_dep_goes_to_blocked() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Block B is open but dep A is in_progress (not closed) → B goes in blocked.
        // Stored focus.blocked is empty → drift.
        let dep_a = BlockedBy::Block {
            repo: "alpha".to_string(),
            id: "AL.1.A".to_string(),
            what: None,
        };
        let pair = make_drift_pair(
            dir.path(),
            "alpha",
            &[
                ("AL.1.A", Some("in_progress"), vec![]),
                ("AL.1.B", Some("open"), vec![dep_a]),
            ],
            &["AL.1.A"], // stored now (correct)
            &[],         // stored next (correct — B has unmet dep)
            &[],         // stored blocked — stale (should be ["AL.1.B"])
        );
        let files = vec![pair];
        let graph = build_state_graph(&files);
        let (src, file) = &files[0];
        let diags = check_focus_drift(src, file, &graph, &files);

        let drifts: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "W_STATE_FOCUS_DRIFT")
            .collect();
        assert_eq!(
            drifts.len(),
            1,
            "B blocked by in_progress A should trigger drift, got: {diags:?}"
        );
    }

    #[test]
    fn check_focus_drift_open_with_closed_dep_goes_to_next() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Block B is open; dep A is closed → B is ready (goes in next).
        // Stored focus matches exactly → no drift.
        let dep_a = BlockedBy::Block {
            repo: "alpha".to_string(),
            id: "AL.1.A".to_string(),
            what: None,
        };
        let pair = make_drift_pair(
            dir.path(),
            "alpha",
            &[
                ("AL.1.A", Some("closed"), vec![]),
                ("AL.1.B", Some("open"), vec![dep_a]),
            ],
            &[],         // stored now (A is closed, not in_progress)
            &["AL.1.B"], // stored next (B is ready)
            &[],         // stored blocked
        );
        let files = vec![pair];
        let graph = build_state_graph(&files);
        let (src, file) = &files[0];
        let diags = check_focus_drift(src, file, &graph, &files);

        assert!(
            diags.is_empty(),
            "in-sync focus (B ready after A closed) should produce no diagnostics, \
             got: {diags:?}"
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

    // -----------------------------------------------------------------------
    // Carryover tests
    // -----------------------------------------------------------------------

    #[test]
    fn carryover_array_deserializes() {
        let json = r#"{
  "repo": "bastion",
  "kind": "project",
  "updated": "2026-06-30",
  "carryover": [
    {
      "slug": "some-caveat",
      "scope": { "repo": "bastion" },
      "kind": "constraint",
      "text": "A durable caveat.",
      "created": "2026-06-30"
    }
  ]
}"#;
        let file: StateFile = serde_json::from_str(json).expect("should deserialize");
        assert_eq!(file.carryover.len(), 1);
        assert_eq!(file.carryover[0].slug, "some-caveat");
        assert_eq!(file.carryover[0].scope.repo.as_deref(), Some("bastion"));
        assert!(file.carryover[0].scope.tier.is_none());
        assert_eq!(file.carryover[0].kind, "constraint");
    }

    #[test]
    fn carryover_schema_checks() {
        let json = r#"{
  "repo": "bastion",
  "kind": "project",
  "updated": "2026-06-30",
  "carryover": [
    {
      "slug": "bad-kind",
      "scope": { "repo": "bastion" },
      "kind": "unknown_kind",
      "text": "Bad kind.",
      "created": "2026-06-30"
    },
    {
      "slug": "bad-scope",
      "scope": { "repo": "bastion", "tier": "core" },
      "kind": "known_issue",
      "text": "Malformed scope.",
      "created": "2026-06-30"
    }
  ]
}"#;
        let file: StateFile = serde_json::from_str(json).unwrap();
        let src = StateSource {
            repo_slug: "bastion".to_string(),
            abs_path: PathBuf::from("planning/state.json"),
            expected_kind: "project",
        };
        let diags = check_schema(&src, &file);
        
        let bad_kind = diags.iter().any(|d| d.locator == "E_STATE_SCHEMA_BAD_KIND" && d.message.contains("bad-kind"));
        let bad_scope = diags.iter().any(|d| d.locator == "E_STATE_SCHEMA_MALFORMED_SCOPE" && d.message.contains("bad-scope"));
        
        assert!(bad_kind, "Should flag bad kind");
        assert!(bad_scope, "Should flag malformed scope");
    }
