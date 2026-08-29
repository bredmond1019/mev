//! Block record parsing — the authored definition of one block of work (D65).
//!
//! Mirrors `.claude/workflows/block.schema.json` (base-template's contract for
//! `planning/blocks/<BlockID>.json`). This module provides:
//!
//! - [`BlockRecord`] — the serde-deserialized shape of one record. Deliberately
//!   tolerant of unknown top-level fields (no `#[serde(deny_unknown_fields)]`)
//!   because the schema is expected to grow (D65 consequences) and a stricter
//!   parse would turn a schema addition into a spurious parse failure here.
//! - [`discover_block_records`] — reads every `planning/blocks/*.json` file
//!   under a given repo root and returns one [`BlockRecordFile`] per file
//!   found, parse errors included rather than short-circuited.
//!
//! Task 2 of `MV.ticket.block-record-validation` adds the `W_BLOCK_*`
//! diagnostic checks over these types; task 3 wires discovery into
//! `validate-brain --state`.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// One `depends_on[]` edge.
///
/// Only the fields the block-record checks need (`type`/`exit`/`start`, for
/// the "operator edge missing its `exit` or `start`" check) are modeled by
/// name; every other field on any edge shape (`repo`, `id`, `why`, `slug`,
/// `what`, `digest`, ...) round-trips through `extra` via `#[serde(flatten)]`
/// rather than being individually declared, since this ticket's checks never
/// need them.
#[derive(Debug, Clone, Deserialize)]
pub struct DependsOnEdge {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub exit: Option<String>,
    #[serde(default)]
    pub start: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// `files.new[]` / `files.modified[]` — loosely typed as raw JSON values
/// since no check in this ticket inspects individual file entries.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct BlockFiles {
    #[serde(default)]
    pub new: Vec<serde_json::Value>,
    #[serde(default)]
    pub modified: Vec<serde_json::Value>,
}

/// A block record — the deserialized shape of one
/// `planning/blocks/<BlockID>.json` file.
///
/// Fields the `W_BLOCK_*` checks (task 2) must be able to find missing or
/// empty — `why`, `description`, `out_of_scope` — are modeled as `Option`
/// with `#[serde(default)]` even though the schema marks them required: a
/// record that omits one entirely must still deserialize so the check can
/// report it, rather than failing at parse time and reporting nothing more
/// specific than "invalid JSON".
///
/// No `#[serde(deny_unknown_fields)]`: unknown top-level keys are ignored,
/// not rejected, so a schema addition doesn't turn into a parse failure here.
#[derive(Debug, Clone, Deserialize)]
pub struct BlockRecord {
    pub id: String,
    pub repo: String,
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    pub what: String,
    #[serde(default)]
    pub why: Option<String>,
    pub sdlc_workflow: String,
    pub model: String,
    #[serde(default)]
    pub files: Option<BlockFiles>,
    #[serde(default)]
    pub out_of_scope: Option<Vec<String>>,
    #[serde(default)]
    pub acceptance_criteria: Vec<serde_json::Value>,
    pub spec_dir: String,
    #[serde(default)]
    pub depends_on: Vec<DependsOnEdge>,
    #[serde(default)]
    pub created: Option<String>,
    #[serde(default)]
    pub updated: Option<String>,
}

/// One `planning/blocks/*.json` file discovered on disk, whether or not it
/// parsed successfully.
///
/// Carries `filename_stem` alongside `parsed` (rather than making the caller
/// re-derive it from `path`) because the filename/id mismatch check compares
/// the stem against `BlockRecord::id` and both must survive independently of
/// whether parsing succeeded.
#[derive(Debug)]
pub struct BlockRecordFile {
    /// Absolute or repo-root-relative path this record was read from, as
    /// passed to [`discover_block_records`].
    pub path: PathBuf,
    /// The filename without its `.json` extension, e.g. `"MV.ticket.foo"`
    /// for `planning/blocks/MV.ticket.foo.json`.
    pub filename_stem: String,
    /// `Ok` on a successful parse; `Err` with a human-readable message
    /// (read failure or JSON/schema-shape error) otherwise. Kept as `String`
    /// rather than `serde_json::Error` so callers (including tests) can
    /// assert on message content without needing to reconstruct a
    /// `serde_json::Error`.
    pub parsed: Result<BlockRecord, String>,
}

impl BlockRecordFile {
    /// True when the filename stem does not equal the record's own `id`.
    ///
    /// Only meaningful for a successfully parsed record — returns `false`
    /// (nothing to compare) when `parsed` is `Err`, since a parse failure is
    /// already its own diagnostic and re-reporting a filename mismatch on
    /// top of it would be noise about a record whose `id` couldn't even be
    /// read.
    pub fn filename_id_mismatch(&self) -> bool {
        match &self.parsed {
            Ok(record) => record.id != self.filename_stem,
            Err(_) => false,
        }
    }
}

/// Read every `planning/blocks/*.json` file under `repo_root` and return one
/// [`BlockRecordFile`] per file found.
///
/// A repo with no `planning/blocks/` directory (the common case across the
/// fleet today, per the ticket's testing strategy) returns an empty `Vec`
/// rather than an error — the caller (task 3's `validate-brain --state`
/// wiring) must treat "no directory" identically to "directory with no
/// files": zero diagnostics, no error.
///
/// Files are returned in sorted filename order for deterministic output.
/// Non-`.json` files in the directory are ignored; a file that fails to read
/// or fails to parse still yields a `BlockRecordFile` with `parsed = Err(..)`
/// rather than being silently skipped.
pub fn discover_block_records(repo_root: &Path) -> Vec<BlockRecordFile> {
    let blocks_dir = repo_root.join("planning").join("blocks");

    let entries = match fs::read_dir(&blocks_dir) {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };

    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file() && path.extension().and_then(|ext| ext.to_str()) == Some("json")
        })
        .collect();
    paths.sort();

    paths
        .into_iter()
        .map(|path| {
            let filename_stem = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("")
                .to_string();

            let parsed = fs::read_to_string(&path)
                .map_err(|err| format!("could not read {}: {err}", path.display()))
                .and_then(|contents| {
                    serde_json::from_str::<BlockRecord>(&contents)
                        .map_err(|err| format!("could not parse {}: {err}", path.display()))
                });

            BlockRecordFile {
                path,
                filename_stem,
                parsed,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, contents: &str) {
        fs::write(dir.join(name), contents).expect("write fixture file");
    }

    /// A minimal but fully valid record — every schema-required field
    /// present and non-empty. Used both as a deserialization smoke test and
    /// as the base a few tests below tweak from.
    const VALID_RECORD: &str = r#"{
        "id": "MV.ticket.example",
        "repo": "mev",
        "kind": "ticket",
        "title": "Example block",
        "description": "A one-line description of the example block.",
        "what": "The scope of the example block, in implementation terms.",
        "why": "Why this block exists and why now.",
        "sdlc_workflow": "task",
        "model": "sonnet",
        "files": { "new": [], "modified": [] },
        "out_of_scope": ["Anything not listed above"],
        "acceptance_criteria": ["It works"],
        "spec_dir": "planning/MV.ticket.example/",
        "created": "2026-08-16",
        "updated": "2026-08-16"
    }"#;

    #[test]
    fn valid_record_deserializes() {
        let record: BlockRecord =
            serde_json::from_str(VALID_RECORD).expect("valid record should deserialize");
        assert_eq!(record.id, "MV.ticket.example");
        assert_eq!(record.repo, "mev");
        assert_eq!(record.why.as_deref(), Some("Why this block exists and why now."));
        assert_eq!(
            record.out_of_scope.as_deref(),
            Some(&["Anything not listed above".to_string()][..])
        );
    }

    #[test]
    fn record_with_unknown_extra_field_still_deserializes() {
        let with_extra = VALID_RECORD.replacen(
            "\"id\": \"MV.ticket.example\",",
            "\"id\": \"MV.ticket.example\", \"totally_new_field\": {\"nested\": true},",
            1,
        );
        let record: BlockRecord = serde_json::from_str(&with_extra)
            .expect("unknown top-level field must not fail deserialization");
        assert_eq!(record.id, "MV.ticket.example");
    }

    #[test]
    fn record_missing_why_still_deserializes_as_none() {
        // `why` is schema-required but must not be a hard parse failure here
        // — the checks (task 2) need to observe "missing" as data, not as a
        // deserialization error with no locator.
        let without_why = VALID_RECORD.replacen("\"why\": \"Why this block exists and why now.\",", "", 1);
        let record: BlockRecord =
            serde_json::from_str(&without_why).expect("missing `why` should still deserialize");
        assert_eq!(record.why, None);
    }

    #[test]
    fn discover_returns_empty_vec_when_no_blocks_dir() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let files = discover_block_records(tmp.path());
        assert!(
            files.is_empty(),
            "a repo with no planning/blocks/ dir must yield no records and no error"
        );
    }

    #[test]
    fn discover_finds_json_files_sorted_and_ignores_other_extensions() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let blocks_dir = tmp.path().join("planning").join("blocks");
        fs::create_dir_all(&blocks_dir).expect("mkdir");
        write(&blocks_dir, "MV.ticket.b.json", VALID_RECORD);
        write(&blocks_dir, "MV.ticket.a.json", VALID_RECORD);
        write(&blocks_dir, "README.md", "not a block record");

        let files = discover_block_records(tmp.path());
        let names: Vec<String> = files
            .iter()
            .map(|f| f.path.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["MV.ticket.a.json", "MV.ticket.b.json"]);
    }

    #[test]
    fn filename_id_mismatch_detected() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let blocks_dir = tmp.path().join("planning").join("blocks");
        fs::create_dir_all(&blocks_dir).expect("mkdir");
        // File is named "...wrong-name.json" but the record's own id is
        // "MV.ticket.example" (from VALID_RECORD) — a mismatch.
        write(&blocks_dir, "MV.ticket.wrong-name.json", VALID_RECORD);

        let files = discover_block_records(tmp.path());
        assert_eq!(files.len(), 1);
        assert!(files[0].filename_id_mismatch());
    }

    #[test]
    fn filename_id_match_not_flagged() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let blocks_dir = tmp.path().join("planning").join("blocks");
        fs::create_dir_all(&blocks_dir).expect("mkdir");
        write(&blocks_dir, "MV.ticket.example.json", VALID_RECORD);

        let files = discover_block_records(tmp.path());
        assert_eq!(files.len(), 1);
        assert!(!files[0].filename_id_mismatch());
    }

    #[test]
    fn malformed_json_yields_err_not_panic() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let blocks_dir = tmp.path().join("planning").join("blocks");
        fs::create_dir_all(&blocks_dir).expect("mkdir");
        write(&blocks_dir, "MV.ticket.broken.json", "{ not valid json");

        let files = discover_block_records(tmp.path());
        assert_eq!(files.len(), 1);
        assert!(files[0].parsed.is_err());
        assert!(!files[0].filename_id_mismatch());
    }
}
