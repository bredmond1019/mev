//! Graph-export envelope for the `scope:doc_id` knowledge graph (Phase 3B, Block R).
//!
//! Converts a built [`GraphArtifact`] (from [`crate::brain::graph::build_graph`]) into a
//! [`GraphExport`] — a canonical, JSON-serializable envelope with a `version`/`root` header,
//! mirroring [`crate::brain::manifest::Manifest`]. Consumed by the orchestrator to load nodes
//! and edges into a Postgres edges table beside `brain_documents` (D4).
//!
//! Design principles (shared with `manifest.rs`):
//! - **Pure output** — [`build_graph_export`] does not write to disk or a DB. The caller
//!   serialises the result to stdout (or a file) as needed.
//! - **No re-derivation** — nodes and edges are cloned straight from `artifact.graph` in
//!   walk order; nothing is re-walked or re-inferred here.
//! - **Deterministic leaves** — `leaves` is a sorted `Vec<String>` (from `artifact.leaf_keys`,
//!   a `HashSet`) so repeated runs over an unchanged corpus emit byte-identical output.

use std::path::Path;

use serde::Serialize;

use crate::brain::graph::{Edge, GraphArtifact, Node};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// The complete graph-export envelope for a Brain corpus crawl.
///
/// Serialises to JSON for consumption by the orchestrator's Postgres edges loader.
#[derive(Debug, Serialize)]
pub struct GraphExport {
    /// Schema version — currently `"1"`.
    pub version: String,
    /// Display path of the HQ root used for the crawl.
    pub root: String,
    /// All graph nodes, in walk order.
    pub nodes: Vec<Node>,
    /// All graph edges, in walk order.
    pub edges: Vec<Edge>,
    /// `scope:stem` for every corpus file with no authored `doc_id`, sorted for
    /// deterministic output.
    pub leaves: Vec<String>,
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Build a [`GraphExport`] from a pre-built [`GraphArtifact`].
///
/// `root` is the HQ directory that was crawled; it is stored as a display string in the
/// envelope header and is not used to access the filesystem.
///
/// Nodes and edges are cloned directly from `artifact.graph` (already deterministic walk
/// order); `leaves` is `artifact.leaf_keys` collected into a `Vec<String>` and sorted.
pub fn build_graph_export(root: &Path, artifact: &GraphArtifact) -> GraphExport {
    let mut leaves: Vec<String> = artifact.leaf_keys.iter().cloned().collect();
    leaves.sort();

    GraphExport {
        version: "1".to_string(),
        root: root.display().to_string(),
        nodes: artifact.graph.nodes.clone(),
        edges: artifact.graph.edges.clone(),
        leaves,
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::config::BrainConfig;
    use crate::brain::crawl::{Corpus, CorpusEntry};
    use crate::brain::graph::build_graph;
    use crate::brain::okf::OkfFrontmatter;
    use crate::shared::extract_frontmatter;
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// Write a `.md` file and return a `CorpusEntry` with `metadata` pre-parsed from
    /// the file contents (mirrors what `crawl_corpus` does — D5 extract-once).
    fn make_entry(dir: &TempDir, scope: &str, filename: &str, contents: &str) -> CorpusEntry {
        let path = dir.path().join(filename);
        std::fs::write(&path, contents).unwrap();
        let rel = PathBuf::from(filename);
        let stem = std::path::Path::new(filename)
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let metadata = extract_frontmatter(contents)
            .and_then(|yaml| serde_yaml::from_str::<OkfFrontmatter>(yaml).ok());
        CorpusEntry {
            path,
            rel,
            stem,
            scope: scope.to_string(),
            metadata,
        }
    }

    fn corpus_from(entries: Vec<CorpusEntry>) -> Corpus {
        Corpus { entries }
    }

    #[test]
    fn maps_nodes_edges_and_sorted_leaves() {
        let dir = TempDir::new().unwrap();
        let e1 = make_entry(
            &dir,
            "brain",
            "a.md",
            "---\ndoc_id: alpha\nrelated:\n  - beta\n---",
        );
        let e2 = make_entry(&dir, "brain", "b.md", "---\ndoc_id: beta\n---");
        // Two leaves, deliberately out of alpha order to exercise sorting.
        let e3 = make_entry(&dir, "brain", "z-leaf.md", "# No frontmatter");
        let e4 = make_entry(&dir, "brain", "a-leaf.md", "# No frontmatter");
        let corpus = corpus_from(vec![e1, e2, e3, e4]);
        let config = BrainConfig::default();
        let artifact = build_graph(&corpus, &config);

        let root = std::path::Path::new("/hq");
        let export = build_graph_export(root, &artifact);

        assert_eq!(export.version, "1");
        assert_eq!(export.root, "/hq");
        assert_eq!(export.nodes.len(), 2);
        assert_eq!(export.edges.len(), 1);
        assert_eq!(export.edges[0].from, "brain:alpha");
        assert_eq!(export.edges[0].to_ref, "beta");
        assert_eq!(
            export.leaves,
            vec!["brain:a-leaf".to_string(), "brain:z-leaf".to_string()],
            "leaves must be sorted"
        );
    }

    #[test]
    fn empty_corpus_produces_empty_vecs() {
        let corpus = corpus_from(vec![]);
        let config = BrainConfig::default();
        let artifact = build_graph(&corpus, &config);

        let root = std::path::Path::new("/hq");
        let export = build_graph_export(root, &artifact);

        assert_eq!(export.version, "1");
        assert!(export.nodes.is_empty());
        assert!(export.edges.is_empty());
        assert!(export.leaves.is_empty());
    }

    #[test]
    fn graph_export_serializes_and_round_trips() {
        let dir = TempDir::new().unwrap();
        let entry = make_entry(
            &dir,
            "brain",
            "doc.md",
            "---\ndoc_id: my-doc\nrelated:\n  - other\n---",
        );
        let corpus = corpus_from(vec![entry]);
        let config = BrainConfig::default();
        let artifact = build_graph(&corpus, &config);

        let root = std::path::Path::new("/hq");
        let export = build_graph_export(root, &artifact);

        let json = serde_json::to_string(&export).expect("export must serialize to JSON");
        let value: serde_json::Value =
            serde_json::from_str(&json).expect("export JSON must be valid");

        assert!(value.get("version").is_some(), "version key present");
        assert!(value.get("root").is_some(), "root key present");
        assert!(value.get("nodes").is_some(), "nodes key present");
        assert!(value.get("edges").is_some(), "edges key present");
        assert!(value.get("leaves").is_some(), "leaves key present");
        assert_eq!(value["version"], "1");
    }
}
