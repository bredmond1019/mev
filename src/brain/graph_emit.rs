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

use crate::brain::graph::{EdgeKind, EdgeResolution, GraphArtifact, Node, resolve_edge};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// The complete graph-export envelope for a Brain corpus crawl.
///
/// Serialises to JSON for consumption by the orchestrator's Postgres edges loader.
#[derive(Debug, Serialize)]
pub struct GraphExport {
    /// Schema version — currently `"2"`.
    pub version: String,
    /// Display path of the HQ root used for the crawl.
    pub root: String,
    /// All graph nodes, in walk order.
    pub nodes: Vec<Node>,
    /// All graph edges, in walk order, carrying resolved target fields.
    pub edges: Vec<ExportedEdge>,
    /// `scope:stem` for every corpus file with no authored `doc_id`, sorted for
    /// deterministic output.
    pub leaves: Vec<String>,
}

/// One exported graph edge, augmented with the [`resolve_edge`] outcome.
///
/// `to_ref` stays raw as-authored in every case. `target_node_id`/`target_doc_id`
/// are both `Some` when the edge resolves to a real node, and both `None` when it
/// is dangling or resolves to a leaf (doc-id-less file) — this mirrors
/// `check_graph`'s `E_GRAPH_DANGLING_RELATED`/`W_GRAPH_LEAF_TARGET` classification
/// by construction, since both call [`resolve_edge`].
///
/// Export-local — deliberately not added to `graph.rs`'s shared `Edge` struct,
/// which also backs `generate-graph` HTML and `check_graph` and must stay unchanged.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ExportedEdge {
    /// The referring node's canonical `scope:doc_id`.
    pub from: String,
    /// The raw, as-authored `related:` reference (bare or qualified).
    pub to_ref: String,
    /// Edge type.
    pub kind: EdgeKind,
    /// Qualified `scope:doc_id` of the resolved target node, or `None` if the
    /// edge is dangling or targets a leaf.
    pub target_node_id: Option<String>,
    /// The resolved target node's authored `doc_id`, or `None` if the edge is
    /// dangling or targets a leaf.
    pub target_doc_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Build a [`GraphExport`] from a pre-built [`GraphArtifact`].
///
/// `root` is the HQ directory that was crawled; it is stored as a display string in the
/// envelope header and is not used to access the filesystem.
///
/// Nodes are cloned directly from `artifact.graph` (already deterministic walk order).
/// Each edge is resolved via [`resolve_edge`] (the same pure function `check_graph`
/// uses) to populate `target_node_id`/`target_doc_id` — both `Some` on `Resolved`,
/// both `None` on `LeafTarget`/`Dangling`. `leaves` is `artifact.leaf_keys` collected
/// into a `Vec<String>` and sorted.
pub fn build_graph_export(root: &Path, artifact: &GraphArtifact) -> GraphExport {
    let mut leaves: Vec<String> = artifact.leaf_keys.iter().cloned().collect();
    leaves.sort();

    let edges: Vec<ExportedEdge> = artifact
        .graph
        .edges
        .iter()
        .map(|edge| {
            let (target_node_id, target_doc_id) = match resolve_edge(artifact, edge) {
                EdgeResolution::Resolved { node_id, doc_id } => (Some(node_id), Some(doc_id)),
                EdgeResolution::LeafTarget { .. } | EdgeResolution::Dangling { .. } => (None, None),
            };
            ExportedEdge {
                from: edge.from.clone(),
                to_ref: edge.to_ref.clone(),
                kind: edge.kind.clone(),
                target_node_id,
                target_doc_id,
            }
        })
        .collect();

    GraphExport {
        version: "2".to_string(),
        root: root.display().to_string(),
        nodes: artifact.graph.nodes.clone(),
        edges,
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

        assert_eq!(export.version, "2");
        assert_eq!(export.root, "/hq");
        assert_eq!(export.nodes.len(), 2);
        assert_eq!(export.edges.len(), 1);
        assert_eq!(export.edges[0].from, "brain:alpha");
        assert_eq!(export.edges[0].to_ref, "beta");
        assert_eq!(
            export.edges[0].target_node_id,
            Some("brain:beta".to_string()),
            "bare `beta` ref resolves to the beta node in the referrer's own scope"
        );
        assert_eq!(export.edges[0].target_doc_id, Some("beta".to_string()));
        assert_eq!(
            export.leaves,
            vec!["brain:a-leaf".to_string(), "brain:z-leaf".to_string()],
            "leaves must be sorted"
        );
    }

    #[test]
    fn dangling_and_leaf_edges_have_null_target_fields() {
        let dir = TempDir::new().unwrap();
        let e1 = make_entry(
            &dir,
            "brain",
            "a.md",
            "---\ndoc_id: alpha\nrelated:\n  - missing\n  - z-leaf\n---",
        );
        let e2 = make_entry(&dir, "brain", "z-leaf.md", "# No frontmatter");
        let corpus = corpus_from(vec![e1, e2]);
        let config = BrainConfig::default();
        let artifact = build_graph(&corpus, &config);

        let root = std::path::Path::new("/hq");
        let export = build_graph_export(root, &artifact);

        assert_eq!(export.edges.len(), 2);

        let dangling = export
            .edges
            .iter()
            .find(|e| e.to_ref == "missing")
            .expect("dangling edge present");
        assert_eq!(dangling.target_node_id, None);
        assert_eq!(dangling.target_doc_id, None);

        let leaf = export
            .edges
            .iter()
            .find(|e| e.to_ref == "z-leaf")
            .expect("leaf-target edge present");
        assert_eq!(leaf.target_node_id, None);
        assert_eq!(leaf.target_doc_id, None);
    }

    #[test]
    fn empty_corpus_produces_empty_vecs() {
        let corpus = corpus_from(vec![]);
        let config = BrainConfig::default();
        let artifact = build_graph(&corpus, &config);

        let root = std::path::Path::new("/hq");
        let export = build_graph_export(root, &artifact);

        assert_eq!(export.version, "2");
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
        assert_eq!(value["version"], "2");
        assert!(
            value["edges"][0].get("target_node_id").is_some(),
            "target_node_id key present on exported edge"
        );
        assert!(
            value["edges"][0].get("target_doc_id").is_some(),
            "target_doc_id key present on exported edge"
        );
    }
}
