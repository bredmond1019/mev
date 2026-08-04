//! Append-only revision history for files `mev` overwrites.
//!
//! `apply_plan()` in `src/brain/emit.rs` is the single write point for the entire
//! binary: `emit-state`, the doc materializer, the Opportunity family, and the index
//! reconciler all route through it. Historically it was one bare `std::fs::write`
//! with no snapshot — a logic bug in a derivation (e.g. `derive_rollup` silently
//! dropping a repo from a rollup) could overwrite good authored content with a wrong
//! derived result, and the prior content was simply gone. This is the incident class
//! this module closes: before any overwrite of an *existing* file, the writer records
//! the file's prior content as a new revision here, so a bad derived write is
//! recoverable rather than destructive.
//!
//! Storage layout: revisions for `<dir>/<name>` live in `<dir>/.mev-history/<name>/`,
//! one file per revision named `<seq:06>__<utc-rfc3339-basic>`, holding the
//! snapshotted content verbatim. `seq` is monotonic per file, derived as `(max
//! existing seq) + 1` by reading the directory on each call — never by rewriting a
//! separate counter file, so a crash between writes can never desynchronize a counter
//! from reality. Snapshots are append-only: once written, a revision file is never
//! reopened for writing except by [`prune`], which only ever deletes whole files.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::Utc;
use thiserror::Error;

/// Name of the per-directory history root, relative to the file's own directory.
const HISTORY_DIR_NAME: &str = ".mev-history";

/// A single recorded revision of a file's prior content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revision {
    /// Monotonic sequence number for this file, starting at 1.
    pub seq: u32,
    /// UTC timestamp the revision was recorded at, filename-safe RFC 3339 (no colons).
    pub recorded_at: String,
    /// Size of the snapshotted content, in bytes.
    pub bytes: u64,
}

/// Errors returned by this module's functions.
#[derive(Debug, Error)]
pub enum HistoryError {
    /// An I/O operation on the history store failed.
    #[error("history I/O error for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    /// A `read_revision` (or restore) named a `seq` that has no snapshot on disk.
    #[error("no revision {seq} recorded for {path}")]
    NoSuchRevision { path: PathBuf, seq: u32 },
}

/// The history directory for `path`: `<dir>/.mev-history/<name>/`.
///
/// Exposed for tests/diagnostics, mirroring `LockGuard::path()`.
pub fn history_dir(path: &Path) -> PathBuf {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    dir.join(HISTORY_DIR_NAME).join(name)
}

/// Filename-safe UTC timestamp: RFC 3339 with colons replaced so the result is a
/// valid filename component on every platform this runs on.
fn filename_safe_timestamp() -> String {
    Utc::now().to_rfc3339().replace(':', "-")
}

/// Parse a revision filename of the form `<seq:06>__<timestamp>` into `(seq, timestamp)`.
fn parse_revision_filename(name: &str) -> Option<(u32, String)> {
    let (seq_part, ts_part) = name.split_once("__")?;
    let seq = seq_part.parse::<u32>().ok()?;
    Some((seq, ts_part.to_string()))
}

/// Record `prior` as the next revision of `path`.
///
/// Creates the history directory if it does not yet exist. `seq` is derived as `(max
/// existing seq) + 1` by reading the directory fresh each call — append-only: an
/// existing snapshot file is never opened for writing.
pub fn record_revision(path: &Path, prior: &[u8]) -> Result<Revision, HistoryError> {
    let dir = history_dir(path);
    fs::create_dir_all(&dir).map_err(|source| HistoryError::Io {
        path: dir.clone(),
        source,
    })?;

    let next_seq = next_seq_for(&dir)?;
    let recorded_at = filename_safe_timestamp();
    let filename = format!("{next_seq:06}__{recorded_at}");
    let revision_path = dir.join(&filename);

    fs::write(&revision_path, prior).map_err(|source| HistoryError::Io {
        path: revision_path.clone(),
        source,
    })?;

    Ok(Revision {
        seq: next_seq,
        recorded_at,
        bytes: prior.len() as u64,
    })
}

/// Compute the next `seq` for `dir` by scanning existing revision filenames.
fn next_seq_for(dir: &Path) -> Result<u32, HistoryError> {
    let mut max_seq = 0u32;
    let entries = fs::read_dir(dir).map_err(|source| HistoryError::Io {
        path: dir.to_path_buf(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| HistoryError::Io {
            path: dir.to_path_buf(),
            source,
        })?;
        let name = entry.file_name().to_string_lossy().to_string();
        if let Some((seq, _)) = parse_revision_filename(&name) {
            max_seq = max_seq.max(seq);
        }
    }
    Ok(max_seq + 1)
}

/// List all revisions recorded for `path`, ascending by `seq`.
///
/// Returns an empty `Vec` (not an error) when the history directory does not exist —
/// "no history yet" is a normal state, not a failure.
pub fn list_revisions(path: &Path) -> Result<Vec<Revision>, HistoryError> {
    let dir = history_dir(path);
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut revisions = Vec::new();
    let entries = fs::read_dir(&dir).map_err(|source| HistoryError::Io {
        path: dir.clone(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| HistoryError::Io {
            path: dir.clone(),
            source,
        })?;
        let name = entry.file_name().to_string_lossy().to_string();
        let Some((seq, recorded_at)) = parse_revision_filename(&name) else {
            continue;
        };
        let bytes = entry
            .metadata()
            .map_err(|source| HistoryError::Io {
                path: entry.path(),
                source,
            })?
            .len();
        revisions.push(Revision {
            seq,
            recorded_at,
            bytes,
        });
    }

    revisions.sort_by_key(|r| r.seq);
    Ok(revisions)
}

/// Read the raw content of revision `seq` for `path`.
pub fn read_revision(path: &Path, seq: u32) -> Result<Vec<u8>, HistoryError> {
    let dir = history_dir(path);
    let entries = fs::read_dir(&dir).map_err(|source| {
        if source.kind() == io::ErrorKind::NotFound {
            // Surface as NoSuchRevision rather than an IO error: from the caller's
            // perspective "no history dir" and "no matching seq" are the same thing.
            return HistoryError::NoSuchRevision {
                path: path.to_path_buf(),
                seq,
            };
        }
        HistoryError::Io {
            path: dir.clone(),
            source,
        }
    })?;

    for entry in entries {
        let entry = entry.map_err(|source| HistoryError::Io {
            path: dir.clone(),
            source,
        })?;
        let name = entry.file_name().to_string_lossy().to_string();
        if let Some((found_seq, _)) = parse_revision_filename(&name)
            && found_seq == seq
        {
            return fs::read(entry.path()).map_err(|source| HistoryError::Io {
                path: entry.path(),
                source,
            });
        }
    }

    Err(HistoryError::NoSuchRevision {
        path: path.to_path_buf(),
        seq,
    })
}

/// Delete the oldest revisions of `path` beyond the newest `keep`. Returns how many
/// were removed.
///
/// `keep == 0` is a no-op guard — prune never removes every revision by accident of
/// a zero/misconfigured `keep`.
pub fn prune(path: &Path, keep: usize) -> Result<usize, HistoryError> {
    if keep == 0 {
        return Ok(0);
    }

    let mut revisions = list_revisions(path)?;
    if revisions.len() <= keep {
        return Ok(0);
    }

    // Ascending by seq already (list_revisions guarantees this); oldest-first means
    // the ones to remove are the leading `revisions.len() - keep` entries.
    revisions.sort_by_key(|r| r.seq);
    let remove_count = revisions.len() - keep;
    let dir = history_dir(path);

    for revision in revisions.iter().take(remove_count) {
        let filename = format!("{:06}__{}", revision.seq, revision.recorded_at);
        let revision_path = dir.join(filename);
        fs::remove_file(&revision_path).map_err(|source| HistoryError::Io {
            path: revision_path,
            source,
        })?;
    }

    Ok(remove_count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_target() -> PathBuf {
        let dir = crate::testsupport::unique_temp_dir("mev-history-test");
        fs::create_dir_all(&dir).unwrap();
        dir.join("state.json")
    }

    #[test]
    fn record_revision_seq_starts_at_one_and_increments() {
        let target = temp_target();

        let r1 = record_revision(&target, b"first").unwrap();
        assert_eq!(r1.seq, 1);

        let r2 = record_revision(&target, b"second").unwrap();
        assert_eq!(r2.seq, 2);

        let r3 = record_revision(&target, b"third").unwrap();
        assert_eq!(r3.seq, 3);

        let _ = fs::remove_dir_all(history_dir(&target).parent().unwrap());
    }

    #[test]
    fn snapshots_round_trip_byte_exactly_including_non_utf8() {
        let target = temp_target();
        let non_utf8: &[u8] = &[0xff, 0xfe, 0x00, 0x01, b'h', b'i'];

        let r = record_revision(&target, non_utf8).unwrap();
        let read_back = read_revision(&target, r.seq).unwrap();

        assert_eq!(read_back, non_utf8);
        let _ = fs::remove_dir_all(history_dir(&target).parent().unwrap());
    }

    #[test]
    fn list_revisions_on_fresh_path_returns_empty() {
        let target = temp_target();
        let revisions = list_revisions(&target).unwrap();
        assert!(revisions.is_empty());
        let _ = fs::remove_dir_all(target.parent().unwrap());
    }

    #[test]
    fn prune_drops_oldest_and_keeps_newest() {
        let target = temp_target();
        for i in 0..5 {
            record_revision(&target, format!("content-{i}").as_bytes()).unwrap();
        }

        let removed = prune(&target, 2).unwrap();
        assert_eq!(removed, 3);

        let remaining = list_revisions(&target).unwrap();
        assert_eq!(remaining.len(), 2);
        assert_eq!(remaining[0].seq, 4);
        assert_eq!(remaining[1].seq, 5);

        let _ = fs::remove_dir_all(history_dir(&target).parent().unwrap());
    }

    #[test]
    fn prune_with_keep_at_or_above_count_removes_nothing() {
        let target = temp_target();
        record_revision(&target, b"one").unwrap();
        record_revision(&target, b"two").unwrap();

        let removed = prune(&target, 5).unwrap();
        assert_eq!(removed, 0);
        assert_eq!(list_revisions(&target).unwrap().len(), 2);

        let removed_exact = prune(&target, 2).unwrap();
        assert_eq!(removed_exact, 0);

        let _ = fs::remove_dir_all(history_dir(&target).parent().unwrap());
    }

    #[test]
    fn prune_keep_zero_is_a_no_op_guard() {
        let target = temp_target();
        record_revision(&target, b"one").unwrap();
        record_revision(&target, b"two").unwrap();

        let removed = prune(&target, 0).unwrap();
        assert_eq!(removed, 0);
        assert_eq!(
            list_revisions(&target).unwrap().len(),
            2,
            "keep == 0 must never remove everything"
        );

        let _ = fs::remove_dir_all(history_dir(&target).parent().unwrap());
    }

    #[test]
    fn read_revision_on_missing_seq_yields_no_such_revision() {
        let target = temp_target();
        record_revision(&target, b"one").unwrap();

        let err = read_revision(&target, 99).unwrap_err();
        match err {
            HistoryError::NoSuchRevision { seq, .. } => assert_eq!(seq, 99),
            other => panic!("expected NoSuchRevision, got {other:?}"),
        }

        let _ = fs::remove_dir_all(history_dir(&target).parent().unwrap());
    }

    #[test]
    fn read_revision_on_path_with_no_history_dir_yields_no_such_revision() {
        let target = temp_target();
        let err = read_revision(&target, 1).unwrap_err();
        match err {
            HistoryError::NoSuchRevision { seq, .. } => assert_eq!(seq, 1),
            other => panic!("expected NoSuchRevision, got {other:?}"),
        }
        let _ = fs::remove_dir_all(target.parent().unwrap());
    }

    #[test]
    fn history_dir_layout_matches_convention() {
        let target = PathBuf::from("/tmp/example/state.json");
        let dir = history_dir(&target);
        assert_eq!(dir, PathBuf::from("/tmp/example/.mev-history/state.json"));
    }
}
