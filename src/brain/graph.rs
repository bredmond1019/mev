//! Global `scope:doc_id` knowledge-graph model and construction (Phase 3, Block J).
//!
//! This module builds the **serializable, emittable** knowledge graph over the Brain
//! corpus produced by `crawl_corpus` (Block J-crawl).  The graph is authored-only:
//! nodes and edges come exclusively from frontmatter fields, never from inference.
//!
//! # Separation of concerns
//!
//! - [`build_graph`] → [`GraphArtifact`]: walks the corpus **once**, reading
//!   metadata from `entry.metadata` (populated once by `crawl_corpus` — D5
//!   extract-once refactor), and returns both the `Graph` (D4 serializable,
//!   emittable artifact) and the lookup structures required by the integrity checks.
//! - [`check_graph`] accepts the built [`GraphArtifact`] and returns
//!   `Vec<Diagnostic>` without re-walking the corpus.
//!
//! # Forward-compat
//!
//! - **D4** — `Graph`, `Node`, `Edge`, `EdgeKind` all derive `serde::Serialize` so
//!   the validated graph is byte-for-byte the artifact Phase 3B Block R emits to Postgres.
//! - **D5** — frontmatter is parsed exactly once in `crawl_corpus` and carried on
//!   `CorpusEntry.metadata`; no module re-reads files for frontmatter independently.
//!
//! # Authored-only guarantee (D5)
//!
//! No node or edge is inferred, proposed, or auto-applied.  Nodes and edges are built
//! solely from confirmed authored metadata (frontmatter today; reviewed sidecars later).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::Diagnostic;
use crate::brain::config::BrainConfig;
use crate::brain::crawl::Corpus;

// ---------------------------------------------------------------------------
// Graph model (D4 serializable, emittable artifact)
// ---------------------------------------------------------------------------

/// The kind of a directed edge in the knowledge graph.
///
/// `Related` is the only variant today, sourced from the `related:` frontmatter list.
/// Typed edges (`supersedes`, `depends-on`, `parent`) extend this enum in a later block
/// without reshaping `Edge` or `check_graph`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    Related,
}

/// A directed edge in the knowledge graph.
///
/// `from` and `to_ref` are both *as-authored* (from `related:` entries); normalisation to
/// qualified `scope:doc_id` happens in `check_graph` at resolution time.  `kind` carries
/// the discriminant so typed edges extend the same emitted schema with no reshape.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Edge {
    /// Canonical id of the source node (`scope:doc_id`).
    pub from: String,
    /// The raw `related:` entry as authored (bare or `scope:doc_id`).
    pub to_ref: String,
    /// Edge type.
    pub kind: EdgeKind,
}

/// A graph node — a Brain corpus file with an authored `doc_id`.
///
/// Files **without** a `doc_id` are leaves; they are tracked in
/// [`GraphArtifact::leaf_keys`] for the leaf-target lint, but never become nodes.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Node {
    /// Canonical id: `scope:doc_id`.
    pub id: String,
    /// Owning scope slug (from the corpus registry).
    pub scope: String,
    /// Authored `doc_id` (location-independent frontmatter field).
    pub doc_id: String,
    /// Path of the file relative to the HQ crawl root.
    pub rel: PathBuf,
}

/// The serializable, emittable knowledge graph — the D4 artifact.
///
/// Produced by [`build_graph`] and consumed (read-only) by `check_graph`.
/// Phase 3B Block R loads this into Postgres alongside embeddings.
#[derive(Debug, Default, serde::Serialize)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

// ---------------------------------------------------------------------------
// Graph construction
// ---------------------------------------------------------------------------

/// All data produced by [`build_graph`]: the serializable graph artifact plus the
/// lookup structures required by `check_graph`.
pub struct GraphArtifact {
    /// The serializable, emittable graph (D4 artifact).
    pub graph: Graph,
    /// `canonical_id → node index` for O(1) resolution in `check_graph`.
    pub node_map: HashMap<String, usize>,
    /// `scope:stem` for every corpus file that has **no** authored `doc_id`.
    /// Used by the leaf-target lint (`W_GRAPH_LEAF_TARGET`).
    pub leaf_keys: HashSet<String>,
}

/// Build the global `scope:doc_id` knowledge graph over the supplied corpus.
///
/// For each [`CorpusEntry`]:
/// - Reads `doc_id` and `related` from `entry.metadata` (D5 extract-once — populated
///   by `crawl_corpus`; no file I/O here).
/// - Files **with** a non-empty `doc_id` become [`Node`]s; their `related:` entries
///   become [`Edge`]s with `kind = EdgeKind::Related`.
/// - Files **without** a `doc_id` are leaves — tracked in `leaf_keys` (`scope:stem`)
///   for the leaf-target lint; never added as nodes or uniqueness-checked.
///
/// # Arguments
/// - `corpus` — the owned [`Corpus`] from `crawl_corpus`.
/// - `_config` — reserved for future per-scope overrides; unused today.
///
/// # Returns
/// A [`GraphArtifact`] carrying the [`Graph`], `node_map`, and `leaf_keys`.
/// The `_config` parameter is retained for future per-scope overrides.
pub fn build_graph(corpus: &Corpus, _config: &BrainConfig) -> GraphArtifact {
    let mut nodes: Vec<Node> = Vec::new();
    let mut edges: Vec<Edge> = Vec::new();
    let mut node_map: HashMap<String, usize> = HashMap::new();
    let mut leaf_keys: HashSet<String> = HashSet::new();

    // First pass: build nodes and collect per-node related entries.
    // We need to build the node map before creating edges so we can store edges
    // alongside nodes; we accumulate (canonical_id, to_ref) pairs for the edge pass.
    let mut pending_edges: Vec<(String, Vec<String>)> = Vec::new();

    for entry in &corpus.entries {
        // D5 extract-once: read from entry.metadata (populated by crawl_corpus).
        // Graceful degradation: None metadata → leaf (same behaviour as before).
        let doc_id = entry
            .metadata
            .as_ref()
            .and_then(|m| m.doc_id.clone())
            .filter(|s| !s.trim().is_empty());
        let related = entry
            .metadata
            .as_ref()
            .and_then(|m| m.related.clone())
            .unwrap_or_default();

        match doc_id {
            Some(doc_id) => {
                let canonical_id = format!("{}:{}", entry.scope, doc_id);
                let node_idx = nodes.len();
                node_map.insert(canonical_id.clone(), node_idx);
                nodes.push(Node {
                    id: canonical_id.clone(),
                    scope: entry.scope.clone(),
                    doc_id,
                    rel: entry.rel.clone(),
                });
                if !related.is_empty() {
                    pending_edges.push((canonical_id, related));
                }
            }
            None => {
                // Leaf: track by scope:stem for the W_GRAPH_LEAF_TARGET lint.
                let leaf_key = format!("{}:{}", entry.scope, entry.stem);
                leaf_keys.insert(leaf_key);
            }
        }
    }

    // Second pass: build edges from collected (from, to_refs) pairs.
    for (from_id, to_refs) in pending_edges {
        for to_ref in to_refs {
            edges.push(Edge {
                from: from_id.clone(),
                to_ref,
                kind: EdgeKind::Related,
            });
        }
    }

    GraphArtifact {
        graph: Graph { nodes, edges },
        node_map,
        leaf_keys,
    }
}

// ---------------------------------------------------------------------------
// Graph integrity checks
// ---------------------------------------------------------------------------

/// Check the built graph for integrity violations: duplicate canonical ids,
/// dangling `related:` edges, leaf-target warnings, and isolated nodes.
///
/// Accepts the [`GraphArtifact`] produced by [`build_graph`] — does **not**
/// re-walk the corpus.
///
/// Returns a `Vec<Diagnostic>` using the graph locator vocabulary:
/// - `E_GRAPH_DUPLICATE_DOC_ID` — two or more nodes share one canonical `scope:doc_id`.
/// - `E_GRAPH_DANGLING_RELATED` — a `related:` entry resolves to no node and no leaf.
/// - `W_GRAPH_LEAF_TARGET` — a `related:` entry resolves to a real file with no `doc_id`.
/// - `W_GRAPH_ISOLATED_NODE` — a node (has `doc_id`) has zero outbound `related:` edges.
pub fn check_graph(artifact: &GraphArtifact) -> Vec<Diagnostic> {
    let mut diags: Vec<Diagnostic> = Vec::new();

    // --- Uniqueness: duplicate canonical ids ---
    // node_map was built from the first-occurrence of each canonical_id.
    // Detect duplicates by scanning all nodes and counting id occurrences.
    let mut id_to_rels: HashMap<&str, Vec<&std::path::Path>> = HashMap::new();
    for node in &artifact.graph.nodes {
        id_to_rels
            .entry(node.id.as_str())
            .or_default()
            .push(&node.rel);
    }
    for (canonical_id, rels) in &id_to_rels {
        if rels.len() >= 2 {
            let paths: Vec<String> = rels.iter().map(|r| r.display().to_string()).collect();
            diags.push(Diagnostic::error(
                rels[0],
                "E_GRAPH_DUPLICATE_DOC_ID",
                format!(
                    "duplicate canonical id `{canonical_id}` claimed by: {}",
                    paths.join(", ")
                ),
            ));
        }
    }

    // --- Edge resolution ---
    for edge in &artifact.graph.edges {
        // Determine the referrer's scope from the from-node.
        let from_scope: &str = artifact
            .node_map
            .get(edge.from.as_str())
            .and_then(|&idx| artifact.graph.nodes.get(idx))
            .map(|n| n.scope.as_str())
            .unwrap_or("");

        let referrer_rel: &std::path::Path = artifact
            .node_map
            .get(edge.from.as_str())
            .and_then(|&idx| artifact.graph.nodes.get(idx))
            .map(|n| n.rel.as_path())
            .unwrap_or_else(|| std::path::Path::new(""));

        // Normalise to_ref: if it contains ':' it is already qualified; otherwise
        // qualify it within the referrer's scope.
        let qualified: String = if edge.to_ref.contains(':') {
            edge.to_ref.clone()
        } else {
            format!("{from_scope}:{}", edge.to_ref)
        };

        if artifact.node_map.contains_key(qualified.as_str()) {
            // Resolved to a node — OK.
            continue;
        }

        // Not a node: check if it's a known leaf.
        if artifact.leaf_keys.contains(&qualified) {
            diags.push(Diagnostic::warning(
                referrer_rel,
                "W_GRAPH_LEAF_TARGET",
                format!(
                    "related target `{}` (resolved: `{qualified}`) is a leaf file (no `doc_id`)",
                    edge.to_ref
                ),
            ));
        } else {
            diags.push(Diagnostic::error(
                referrer_rel,
                "E_GRAPH_DANGLING_RELATED",
                format!(
                    "related target `{}` (resolved: `{qualified}`) does not exist in the corpus",
                    edge.to_ref
                ),
            ));
        }
    }

    // --- Isolated nodes: a doc_id-bearing node with zero outbound related: edges ---
    let referring_ids: HashSet<&str> = artifact
        .graph
        .edges
        .iter()
        .map(|e| e.from.as_str())
        .collect();
    for node in &artifact.graph.nodes {
        if !referring_ids.contains(node.id.as_str()) {
            diags.push(Diagnostic::warning(
                &node.rel,
                "W_GRAPH_ISOLATED_NODE",
                format!(
                    "`{}` has a doc_id but no `related:` entries — zero outbound edges",
                    node.id
                ),
            ));
        }
    }

    diags
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::config::BrainConfig;
    use crate::brain::crawl::{Corpus, CorpusEntry};
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
        // D5: parse frontmatter once here just as crawl_corpus does.
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

    // -----------------------------------------------------------------------
    // build_graph — nodes
    // -----------------------------------------------------------------------

    #[test]
    fn authored_doc_becomes_node_with_canonical_id() {
        let dir = TempDir::new().unwrap();
        let entry = make_entry(&dir, "brain", "status.md", "---\ndoc_id: status\n---\nbody");
        let corpus = corpus_from(vec![entry]);
        let artifact = build_graph(&corpus, &BrainConfig::default());
        assert_eq!(artifact.graph.nodes.len(), 1);
        assert_eq!(artifact.graph.nodes[0].id, "brain:status");
        assert_eq!(artifact.graph.nodes[0].scope, "brain");
        assert_eq!(artifact.graph.nodes[0].doc_id, "status");
    }

    #[test]
    fn no_doc_id_file_is_leaf_not_node() {
        let dir = TempDir::new().unwrap();
        let entry = make_entry(&dir, "mev", "README.md", "# README\nNo frontmatter");
        let corpus = corpus_from(vec![entry]);
        let artifact = build_graph(&corpus, &BrainConfig::default());
        assert!(artifact.graph.nodes.is_empty(), "no nodes from leaf file");
        assert!(
            artifact.leaf_keys.contains("mev:README"),
            "leaf tracked as mev:README"
        );
    }

    #[test]
    fn related_entries_become_related_edges() {
        let dir = TempDir::new().unwrap();
        let entry = make_entry(
            &dir,
            "brain",
            "a.md",
            "---\ndoc_id: alpha\nrelated:\n  - beta\n  - gamma\n---",
        );
        let corpus = corpus_from(vec![entry]);
        let artifact = build_graph(&corpus, &BrainConfig::default());
        assert_eq!(artifact.graph.edges.len(), 2);
        assert_eq!(artifact.graph.edges[0].from, "brain:alpha");
        assert_eq!(artifact.graph.edges[0].to_ref, "beta");
        assert_eq!(artifact.graph.edges[0].kind, EdgeKind::Related);
        assert_eq!(artifact.graph.edges[1].to_ref, "gamma");
    }

    #[test]
    fn same_doc_id_different_scopes_are_two_distinct_nodes() {
        let dir = TempDir::new().unwrap();
        let e1 = make_entry(&dir, "brain", "knowledge.md", "---\ndoc_id: knowledge\n---");
        let e2 = make_entry(&dir, "mev", "knowledge2.md", "---\ndoc_id: knowledge\n---");
        let corpus = corpus_from(vec![e1, e2]);
        let artifact = build_graph(&corpus, &BrainConfig::default());
        assert_eq!(artifact.graph.nodes.len(), 2);
        let ids: Vec<&str> = artifact.graph.nodes.iter().map(|n| n.id.as_str()).collect();
        assert!(ids.contains(&"brain:knowledge"));
        assert!(ids.contains(&"mev:knowledge"));
    }

    // -----------------------------------------------------------------------
    // Serialization (D4 emittable artifact)
    // -----------------------------------------------------------------------

    #[test]
    fn graph_serializes_to_json_round_trip() {
        let dir = TempDir::new().unwrap();
        let entry = make_entry(
            &dir,
            "brain",
            "doc.md",
            "---\ndoc_id: my-doc\nrelated:\n  - other\n---",
        );
        let corpus = corpus_from(vec![entry]);
        let artifact = build_graph(&corpus, &BrainConfig::default());
        let json = serde_json::to_string(&artifact.graph).expect("serialization must succeed");
        assert!(json.contains("brain:my-doc"), "id in JSON: {json}");
        assert!(json.contains("other"), "to_ref in JSON: {json}");
        assert!(json.contains("related"), "kind in JSON: {json}");
    }

    // -----------------------------------------------------------------------
    // check_graph — duplicate detection
    // -----------------------------------------------------------------------

    #[test]
    fn duplicate_canonical_id_produces_error() {
        // Two distinct CorpusEntry items with the same scope+doc_id.
        // We fake two nodes with the same id by building the artifact manually.
        let dir = TempDir::new().unwrap();
        let e1 = make_entry(&dir, "brain", "a.md", "---\ndoc_id: shared\n---");
        // Give the second file a different filename so it's a separate entry.
        let e2 = make_entry(&dir, "brain", "b.md", "---\ndoc_id: shared\n---");
        let corpus = corpus_from(vec![e1, e2]);
        let artifact = build_graph(&corpus, &BrainConfig::default());

        // node_map only keeps first occurrence, but nodes vec has both
        // because build_graph inserts them both.
        // We must verify check_graph catches the duplicate.
        let diags = check_graph(&artifact);
        let errors: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_GRAPH_DUPLICATE_DOC_ID")
            .collect();
        assert_eq!(
            errors.len(),
            1,
            "expected one duplicate error, got: {diags:?}"
        );
        assert!(errors[0].message.contains("brain:shared"));
    }

    #[test]
    fn same_doc_id_different_scopes_no_error() {
        let dir = TempDir::new().unwrap();
        let e1 = make_entry(&dir, "brain", "k.md", "---\ndoc_id: knowledge\n---");
        let e2 = make_entry(&dir, "mev", "k2.md", "---\ndoc_id: knowledge\n---");
        let corpus = corpus_from(vec![e1, e2]);
        let artifact = build_graph(&corpus, &BrainConfig::default());
        let diags = check_graph(&artifact);
        let dup_errors: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_GRAPH_DUPLICATE_DOC_ID")
            .collect();
        assert!(
            dup_errors.is_empty(),
            "no dup errors for different scopes: {diags:?}"
        );
    }

    #[test]
    fn bare_edge_resolving_to_node_in_same_scope_is_ok() {
        let dir = TempDir::new().unwrap();
        let e1 = make_entry(
            &dir,
            "brain",
            "a.md",
            "---\ndoc_id: alpha\nrelated:\n  - beta\n---",
        );
        let e2 = make_entry(&dir, "brain", "b.md", "---\ndoc_id: beta\n---");
        let corpus = corpus_from(vec![e1, e2]);
        let artifact = build_graph(&corpus, &BrainConfig::default());
        let diags = check_graph(&artifact);
        // beta has no related: of its own, so it legitimately trips
        // W_GRAPH_ISOLATED_NODE; that's not what this test is about.
        let resolution_diags: Vec<_> = diags
            .iter()
            .filter(|d| d.locator != "W_GRAPH_ISOLATED_NODE")
            .collect();
        assert!(
            resolution_diags.is_empty(),
            "bare in-scope edge should be OK: {resolution_diags:?}"
        );
    }

    #[test]
    fn qualified_cross_scope_edge_resolving_is_ok() {
        let dir = TempDir::new().unwrap();
        let e1 = make_entry(
            &dir,
            "brain",
            "a.md",
            "---\ndoc_id: alpha\nrelated:\n  - mev:target\n---",
        );
        let e2 = make_entry(&dir, "mev", "t.md", "---\ndoc_id: target\n---");
        let corpus = corpus_from(vec![e1, e2]);
        let artifact = build_graph(&corpus, &BrainConfig::default());
        let diags = check_graph(&artifact);
        // target has no related: of its own, so it legitimately trips
        // W_GRAPH_ISOLATED_NODE; that's not what this test is about.
        let resolution_diags: Vec<_> = diags
            .iter()
            .filter(|d| d.locator != "W_GRAPH_ISOLATED_NODE")
            .collect();
        assert!(
            resolution_diags.is_empty(),
            "qualified cross-scope edge should be OK: {resolution_diags:?}"
        );
    }

    #[test]
    fn typo_in_related_produces_dangling_error() {
        let dir = TempDir::new().unwrap();
        let e1 = make_entry(
            &dir,
            "brain",
            "a.md",
            "---\ndoc_id: alpha\nrelated:\n  - typo-nonexistent\n---",
        );
        let corpus = corpus_from(vec![e1]);
        let artifact = build_graph(&corpus, &BrainConfig::default());
        let diags = check_graph(&artifact);
        let dangling: Vec<_> = diags
            .iter()
            .filter(|d| {
                d.locator == "E_GRAPH_DANGLING_RELATED" && d.severity == crate::Severity::Error
            })
            .collect();
        assert_eq!(dangling.len(), 1, "expected dangling error: {diags:?}");
        assert!(dangling[0].message.contains("typo-nonexistent"));
    }

    #[test]
    fn related_to_leaf_produces_warning_not_error() {
        let dir = TempDir::new().unwrap();
        // e1 references e2 which has no doc_id → e2 is a leaf
        let e1 = make_entry(
            &dir,
            "brain",
            "a.md",
            "---\ndoc_id: alpha\nrelated:\n  - leaf-stem\n---",
        );
        let e2 = make_entry(&dir, "brain", "leaf-stem.md", "# No frontmatter");
        let corpus = corpus_from(vec![e1, e2]);
        let artifact = build_graph(&corpus, &BrainConfig::default());
        let diags = check_graph(&artifact);
        let warnings: Vec<_> = diags
            .iter()
            .filter(|d| d.severity == crate::Severity::Warning)
            .collect();
        let errors: Vec<_> = diags
            .iter()
            .filter(|d| d.severity == crate::Severity::Error)
            .collect();
        assert_eq!(warnings.len(), 1, "expected leaf warning: {diags:?}");
        assert!(errors.is_empty(), "no errors for leaf target: {diags:?}");
        assert!(warnings[0].message.contains("leaf-stem"));
    }

    #[test]
    fn bare_ref_naming_other_scope_id_is_dangling() {
        // A bare `related:` entry that matches a doc_id in a *different* scope
        // but not in the referrer's own scope must be flagged E_GRAPH_DANGLING_RELATED.
        // Bare refs are always qualified to the *from-node's* scope first; they do NOT
        // search across scopes.
        let dir = TempDir::new().unwrap();
        // brain-scope file references "mev-target" (bare)
        let e1 = make_entry(
            &dir,
            "brain",
            "a.md",
            "---\ndoc_id: alpha\nrelated:\n  - mev-target\n---",
        );
        // mev-scope has a node with that doc_id — but bare ref won't find it
        let e2 = make_entry(&dir, "mev", "t.md", "---\ndoc_id: mev-target\n---");
        let corpus = corpus_from(vec![e1, e2]);
        let artifact = build_graph(&corpus, &BrainConfig::default());
        let diags = check_graph(&artifact);
        // bare "mev-target" resolves to "brain:mev-target" which does not exist
        let dangling: Vec<_> = diags
            .iter()
            .filter(|d| {
                d.locator == "E_GRAPH_DANGLING_RELATED" && d.severity == crate::Severity::Error
            })
            .collect();
        assert_eq!(
            dangling.len(),
            1,
            "bare ref to other-scope id should be dangling: {diags:?}"
        );
        assert!(
            dangling[0].message.contains("mev-target"),
            "message names the target: {:?}",
            dangling[0].message
        );
    }

    #[test]
    fn node_with_no_related_produces_isolated_warning() {
        let dir = TempDir::new().unwrap();
        let e1 = make_entry(&dir, "brain", "a.md", "---\ndoc_id: alpha\n---");
        let corpus = corpus_from(vec![e1]);
        let artifact = build_graph(&corpus, &BrainConfig::default());
        let diags = check_graph(&artifact);
        let isolated: Vec<_> = diags
            .iter()
            .filter(|d| {
                d.locator == "W_GRAPH_ISOLATED_NODE" && d.severity == crate::Severity::Warning
            })
            .collect();
        assert_eq!(isolated.len(), 1, "expected isolated warning: {diags:?}");
        assert!(isolated[0].message.contains("brain:alpha"));
    }

    #[test]
    fn node_with_related_is_not_isolated() {
        let dir = TempDir::new().unwrap();
        let e1 = make_entry(
            &dir,
            "brain",
            "a.md",
            "---\ndoc_id: alpha\nrelated:\n  - beta\n---",
        );
        let e2 = make_entry(&dir, "brain", "b.md", "---\ndoc_id: beta\n---");
        let corpus = corpus_from(vec![e1, e2]);
        let artifact = build_graph(&corpus, &BrainConfig::default());
        let diags = check_graph(&artifact);
        // alpha has an outbound related: entry — must not be flagged isolated.
        let alpha_isolated = diags
            .iter()
            .any(|d| d.locator == "W_GRAPH_ISOLATED_NODE" && d.message.contains("brain:alpha"));
        assert!(
            !alpha_isolated,
            "alpha has an outbound edge, should not be isolated: {diags:?}"
        );
        // beta has no related: of its own — must be flagged isolated.
        let beta_isolated = diags
            .iter()
            .any(|d| d.locator == "W_GRAPH_ISOLATED_NODE" && d.message.contains("brain:beta"));
        assert!(
            beta_isolated,
            "beta has zero outbound edges, should be isolated: {diags:?}"
        );
    }
}
