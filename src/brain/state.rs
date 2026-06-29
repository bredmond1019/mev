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
}
