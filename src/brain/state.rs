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

use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

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
}
