//! Manifest emitter for the Brain corpus (Phase 3B, Block Q).
//!
//! Converts the crawled [`Corpus`] into a [`Manifest`] — a canonical, JSON-serializable
//! file list with per-file OKF metadata.  The manifest is the single source of truth
//! consumed by `index_brain.py` so "what's validated == what's embedded" holds by
//! construction.
//!
//! Design principles:
//! - **Pure output** — `build_manifest` does not write to disk.  The caller serialises the
//!   result to stdout (or a file) as needed.  Consistent with the D4 pure-compiler model.
//! - **Graceful degradation** — entries without parseable frontmatter appear in the manifest
//!   with `null` metadata fields; the OKF validator will report the error separately.
//! - **No re-crawl** — this module consumes a pre-built `&Corpus` rather than calling
//!   `crawl_corpus` itself.  Callers that already have a `Corpus` (e.g. the combined
//!   validate-and-manifest flow) avoid a second disk walk.

use std::path::Path;

use serde::Serialize;

use crate::brain::crawl::Corpus;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A single file entry in the emitted manifest.
///
/// `rel` and `scope` are always populated (they come from the crawl, not the frontmatter).
/// All OKF-derived fields are `Option` — absent when the file has no parseable frontmatter.
#[derive(Debug, Clone, Serialize)]
pub struct ManifestEntry {
    /// Path relative to the HQ crawl root (forward-slash separated on all platforms).
    pub rel: String,
    /// Stable scope slug of the owning registry unit (e.g. `"brain"`, `"mev"`).
    pub scope: String,
    /// OKF `doc_id` field; `None` if not present in frontmatter.
    pub doc_id: Option<String>,
    /// OKF `type` field; serialized as `doc_type` to avoid the Rust keyword.
    #[serde(rename = "doc_type")]
    pub doc_type: Option<String>,
    /// OKF `title` field.
    pub title: Option<String>,
    /// OKF `description` field.
    pub description: Option<String>,
    /// OKF `layer` field (closed-set list).
    pub layer: Option<Vec<String>>,
    /// OKF `project` field.
    pub project: Option<String>,
    /// OKF `status` field.
    pub status: Option<String>,
    /// OKF `keywords` field (3–7 free-form topic terms).
    pub keywords: Option<Vec<String>>,
    /// OKF `related` field.
    pub related: Option<Vec<String>>,
    /// OKF `synced_from` watermark.
    pub synced_from: Option<String>,
}

/// The complete manifest for a Brain corpus crawl.
///
/// Serialises to JSON for consumption by `index_brain.py` and other orchestrator tooling.
#[derive(Debug, Serialize)]
pub struct Manifest {
    /// Schema version — currently `"1"`.
    pub version: String,
    /// Display path of the HQ root used for the crawl.
    pub root: String,
    /// All corpus entries, in walk order.
    pub entries: Vec<ManifestEntry>,
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Build a [`Manifest`] from a pre-crawled [`Corpus`].
///
/// `root` is the HQ directory that was crawled; it is stored as a display string in the
/// manifest header and is not used to access the filesystem.
///
/// Each [`crate::brain::crawl::CorpusEntry`] is mapped to a [`ManifestEntry`]:
/// - `rel` and `scope` come from the entry directly.
/// - OKF metadata fields are extracted from `entry.metadata` (the D5 extract-once field);
///   if `metadata` is `None`, all metadata fields on the `ManifestEntry` are `None`.
pub fn build_manifest(root: &Path, corpus: &Corpus) -> Manifest {
    let entries = corpus
        .entries
        .iter()
        .map(|entry| {
            // Use forward slashes for cross-platform JSON portability.
            let rel = entry
                .rel
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/");

            let (
                doc_id,
                doc_type,
                title,
                description,
                layer,
                project,
                status,
                keywords,
                related,
                synced_from,
            ) = match &entry.metadata {
                Some(meta) => (
                    meta.doc_id.clone(),
                    meta.type_.clone(),
                    meta.title.clone(),
                    meta.description.clone(),
                    meta.layer.clone(),
                    meta.project.clone(),
                    meta.status.clone(),
                    meta.keywords.clone(),
                    meta.related.clone(),
                    meta.synced_from.clone(),
                ),
                None => (None, None, None, None, None, None, None, None, None, None),
            };

            ManifestEntry {
                rel,
                scope: entry.scope.clone(),
                doc_id,
                doc_type,
                title,
                description,
                layer,
                project,
                status,
                keywords,
                related,
                synced_from,
            }
        })
        .collect();

    Manifest {
        version: "1".to_string(),
        root: root.display().to_string(),
        entries,
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::crawl::{Corpus, CorpusEntry};
    use crate::brain::okf::OkfFrontmatter;
    use std::path::PathBuf;

    fn make_entry_with_meta(rel: &str, scope: &str, doc_id: &str, title: &str) -> CorpusEntry {
        CorpusEntry {
            path: PathBuf::from(format!("/hq/{rel}")),
            rel: PathBuf::from(rel),
            stem: PathBuf::from(rel)
                .file_stem()
                .unwrap()
                .to_string_lossy()
                .to_string(),
            scope: scope.to_string(),
            metadata: Some(OkfFrontmatter {
                type_: Some("ProjectStatus".to_string()),
                title: Some(title.to_string()),
                description: Some("A description.".to_string()),
                doc_id: Some(doc_id.to_string()),
                layer: Some(vec!["brain".to_string()]),
                project: Some("mev".to_string()),
                status: Some("active".to_string()),
                keywords: Some(vec!["foo".to_string(), "bar".to_string()]),
                related: None,
                synced_from: None,
            }),
        }
    }

    fn make_entry_no_meta(rel: &str, scope: &str) -> CorpusEntry {
        CorpusEntry {
            path: PathBuf::from(format!("/hq/{rel}")),
            rel: PathBuf::from(rel),
            stem: PathBuf::from(rel)
                .file_stem()
                .unwrap()
                .to_string_lossy()
                .to_string(),
            scope: scope.to_string(),
            metadata: None,
        }
    }

    #[test]
    fn build_manifest_maps_entries_correctly() {
        let corpus = Corpus {
            entries: vec![
                make_entry_with_meta("planning/status.md", "brain", "mev-status", "MEV Status"),
                make_entry_no_meta("README.md", "brain"),
            ],
        };
        let root = std::path::Path::new("/hq");
        let manifest = build_manifest(root, &corpus);

        assert_eq!(manifest.version, "1");
        assert_eq!(manifest.entries.len(), 2);

        // Entry with metadata.
        let status = &manifest.entries[0];
        assert_eq!(status.rel, "planning/status.md");
        assert_eq!(status.scope, "brain");
        assert_eq!(status.doc_id.as_deref(), Some("mev-status"));
        assert_eq!(status.doc_type.as_deref(), Some("ProjectStatus"));
        assert_eq!(status.title.as_deref(), Some("MEV Status"));
        assert_eq!(
            status.layer.as_deref(),
            Some(vec!["brain".to_string()].as_slice())
        );

        // Entry without metadata.
        let readme = &manifest.entries[1];
        assert_eq!(readme.rel, "README.md");
        assert_eq!(readme.scope, "brain");
        assert!(readme.doc_id.is_none());
        assert!(readme.doc_type.is_none());
        assert!(readme.title.is_none());
    }

    #[test]
    fn manifest_serializes_to_valid_json() {
        let corpus = Corpus {
            entries: vec![
                make_entry_with_meta("docs/index.md", "brain", "brain-index", "Brain Index"),
                make_entry_no_meta("README.md", "brain"),
            ],
        };
        let root = std::path::Path::new("/hq");
        let manifest = build_manifest(root, &corpus);

        let json = serde_json::to_string(&manifest).expect("manifest must serialize to JSON");

        // Top-level keys present.
        assert!(json.contains("\"version\""), "version key must be present");
        assert!(json.contains("\"root\""), "root key must be present");
        assert!(json.contains("\"entries\""), "entries key must be present");

        // Entry with metadata has doc_type (renamed from type_).
        assert!(
            json.contains("\"doc_type\""),
            "doc_type must appear in JSON (renamed from type_)"
        );

        // null fields appear for the no-metadata entry.
        assert!(
            json.contains("null"),
            "null fields must appear for entries without frontmatter"
        );

        // Round-trips through serde_json::from_str as a Value.
        let _: serde_json::Value =
            serde_json::from_str(&json).expect("manifest JSON must be valid");
    }

    #[test]
    fn empty_corpus_produces_empty_manifest() {
        let corpus = Corpus { entries: vec![] };
        let root = std::path::Path::new("/hq");
        let manifest = build_manifest(root, &corpus);

        assert_eq!(manifest.version, "1");
        assert!(manifest.entries.is_empty());

        let json = serde_json::to_string(&manifest).unwrap();
        let _: serde_json::Value = serde_json::from_str(&json).unwrap();
    }
}
