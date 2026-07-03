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

// The `GraphExport`/`ExportedEdge` model and the `build_graph_export` builder now live
// in `okf_core::graph_emit` (BA.15.12/D16 convergence) — this module re-exports them so
// every existing consumer (`crate::lib`, this file's own tests) keeps resolving the same
// names. `okf_core::graph_emit::build_graph_export` consumes `okf_core::graph::resolve_edge`
// against the same `GraphArtifact` shape `crate::brain::graph::build_graph` produces, so no
// mev-specific adaptation is needed here.
pub use okf_core::{ExportedEdge, GraphExport, build_graph_export};

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
