//! Integration tests for the graph-export CLI driver (`graph_brain`) and emitter.
//!
//! Each test builds a small temp-dir fixture with a `brain.toml` and a handful of `.md` files,
//! calls [`mev::graph_brain`], and asserts on the returned [`mev::GraphExport`].

use std::fs;
use std::path::Path;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create a file at `root/rel` (creating parent dirs as needed) with `content`.
fn write_file(root: &Path, rel: &str, content: &str) {
    let full = root.join(rel);
    fs::create_dir_all(full.parent().unwrap()).unwrap();
    fs::write(&full, content.as_bytes()).unwrap();
}

/// Create a fresh temp dir (removing any leftovers from a prior run).
fn temp_dir(suffix: &str) -> std::path::PathBuf {
    let dir = mev::testsupport::unique_temp_dir(&format!("mev-brain-graph-emit-{suffix}"));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Write a minimal `brain.toml` registering one scope unit (`brain` → `.`).
fn write_brain_toml(root: &Path) {
    let toml = r#"[vocab]
layer = ["brain", "engine", "factory", "console", "surface", "infra", "business", "content", "meta"]
status = ["active", "draft", "deprecated", "superseded", "archived"]

[crawl]
skip_dirs = ["target", "node_modules", ".git"]

[[repos]]
slug = "brain"
tier = "primary"
repo_path = "."
status_file = "planning/status.md"
cache_doc = "docs/projects/brain.md"
heading = "Brain"
"#;
    fs::write(root.join("brain.toml"), toml.as_bytes()).unwrap();
}

/// A minimal OKF doc with the given `doc_id` and `title`, optionally with `related:` entries.
fn okf_doc(doc_id: &str, title: &str, related: &[&str]) -> String {
    let related_block = if related.is_empty() {
        String::new()
    } else {
        let items = related
            .iter()
            .map(|r| format!("  - {r}"))
            .collect::<Vec<_>>()
            .join("\n");
        format!("related:\n{items}\n")
    };
    format!(
        "---\ntype: Reference\ntitle: {title}\ndescription: Test doc {doc_id}.\ndoc_id: {doc_id}\nproject: brain\nstatus: active\n{related_block}---\n\nBody.\n"
    )
}

/// A markdown file with no frontmatter (becomes a leaf).
fn no_frontmatter_doc() -> &'static str {
    "# Just a heading\n\nNo frontmatter here.\n"
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// A small corpus with two linked nodes and one leaf yields the expected node count, an
/// edge from → to_ref, and the leaf present in `leaves`.
#[test]
fn graph_brain_nodes_edges_and_leaf() {
    let dir = temp_dir("nodes-edges-leaf");
    write_brain_toml(&dir);

    write_file(
        &dir,
        "planning/status.md",
        &okf_doc("brain-status", "Brain Status", &["brain-index"]),
    );
    write_file(
        &dir,
        "docs/index.md",
        &okf_doc("brain-index", "Brain Index", &[]),
    );
    write_file(&dir, "README.md", no_frontmatter_doc());

    let export = mev::graph_brain(&dir).expect("graph_brain must succeed");

    assert_eq!(
        export.nodes.len(),
        2,
        "expected 2 nodes (the two doc_id files), got {}: {:#?}",
        export.nodes.len(),
        export.nodes.iter().map(|n| &n.id).collect::<Vec<_>>()
    );

    let status_node = export
        .nodes
        .iter()
        .find(|n| n.doc_id == "brain-status")
        .expect("brain-status node must be present");
    assert_eq!(status_node.id, "brain:brain-status");
    assert_eq!(status_node.scope, "brain");

    assert_eq!(export.edges.len(), 1, "expected exactly one edge");
    let edge = &export.edges[0];
    assert_eq!(edge.from, "brain:brain-status");
    assert_eq!(edge.to_ref, "brain-index");

    assert!(
        export.leaves.contains(&"brain:README".to_string()),
        "leafless README.md must appear in leaves, got: {:?}",
        export.leaves
    );

    let _ = fs::remove_dir_all(&dir);
}

/// The export round-trips: `serde_json::to_string` parses back as a `serde_json::Value` with
/// `version == "2"` and `nodes`/`edges`/`leaves` arrays.
#[test]
fn graph_brain_export_round_trips_as_json() {
    let dir = temp_dir("round-trip");
    write_brain_toml(&dir);

    write_file(
        &dir,
        "planning/status.md",
        &okf_doc("brain-status", "Brain Status", &[]),
    );

    let export = mev::graph_brain(&dir).expect("graph_brain must succeed");
    let json = serde_json::to_string(&export).expect("export must serialize to JSON");

    let value: serde_json::Value = serde_json::from_str(&json).expect("JSON must be valid");
    assert_eq!(value["version"], "2");
    assert!(value["nodes"].is_array(), "nodes must be an array");
    assert!(value["edges"].is_array(), "edges must be an array");
    assert!(value["leaves"].is_array(), "leaves must be an array");

    let _ = fs::remove_dir_all(&dir);
}

/// A `related:` edge to a `doc_id` node is present in `edges`.
#[test]
fn graph_brain_related_edge_present() {
    let dir = temp_dir("related-edge");
    write_brain_toml(&dir);

    write_file(&dir, "docs/a.md", &okf_doc("doc-a", "Doc A", &["doc-b"]));
    write_file(&dir, "docs/b.md", &okf_doc("doc-b", "Doc B", &[]));

    let export = mev::graph_brain(&dir).expect("graph_brain must succeed");

    let found = export
        .edges
        .iter()
        .any(|e| e.from == "brain:doc-a" && e.to_ref == "doc-b");
    assert!(
        found,
        "expected an edge brain:doc-a -> doc-b, got: {:#?}",
        export.edges
    );

    let _ = fs::remove_dir_all(&dir);
}

/// `graph_brain` errors (mentioning `brain.toml`) when no `brain.toml` is present.
#[test]
fn graph_brain_errors_without_brain_toml() {
    let dir = temp_dir("no-toml");
    // Intentionally do NOT write brain.toml.
    write_file(&dir, "planning/status.md", "no frontmatter");

    let result = mev::graph_brain(&dir);
    assert!(
        result.is_err(),
        "expected Err when brain.toml is absent, got Ok"
    );
    let msg = format!("{:#}", result.unwrap_err());
    assert!(
        msg.contains("brain.toml"),
        "error message should mention brain.toml, got: {msg}"
    );

    let _ = fs::remove_dir_all(&dir);
}

/// Parity: `build_graph_export`'s resolved `target_node_id`/`target_doc_id` fields agree,
/// edge by edge, with `check_graph`'s diagnostics — both are driven by the same
/// `resolve_edge` function (Phase 3B, Block V).
///
/// The synthetic corpus carries one of each edge shape:
/// - `doc-a` -> `doc-b` (bare, same-scope `related:`) resolves to a real node.
/// - `doc-a` -> `leaf-file` (bare `related:`) resolves to a doc-id-less leaf file.
/// - `doc-a` -> `missing` (bare `related:`) resolves to nothing at all (dangling).
#[test]
fn export_resolution_matches_check_graph_diagnostics() {
    use mev::brain::config::find_brain_config;
    use mev::brain::crawl::crawl_corpus;
    use mev::{build_graph, build_graph_export, check_graph};

    let dir = temp_dir("parity");
    write_brain_toml(&dir);

    write_file(
        &dir,
        "docs/a.md",
        &okf_doc("doc-a", "Doc A", &["doc-b", "leaf-file", "missing"]),
    );
    write_file(&dir, "docs/b.md", &okf_doc("doc-b", "Doc B", &[]));
    write_file(&dir, "docs/leaf-file.md", no_frontmatter_doc());

    let config = find_brain_config(&dir).expect("brain.toml must be found");
    let (corpus, _crawl_diags) = crawl_corpus(&dir, &config);
    let artifact = build_graph(&corpus, &config);

    let diags = check_graph(&artifact);
    let export = build_graph_export(&dir, &artifact);

    assert_eq!(export.edges.len(), 3, "expected exactly 3 edges");

    for edge in &export.edges {
        let dangling_diag = diags.iter().any(|d| {
            d.locator == "E_GRAPH_DANGLING_RELATED" && d.message.contains(edge.to_ref.as_str())
        });
        let leaf_diag = diags.iter().any(|d| {
            d.locator == "W_GRAPH_LEAF_TARGET" && d.message.contains(edge.to_ref.as_str())
        });

        match edge.to_ref.as_str() {
            "doc-b" => {
                assert!(
                    edge.target_node_id.is_some() && edge.target_doc_id.is_some(),
                    "resolved edge to doc-b must have non-null target fields, got: {edge:?}"
                );
                assert_eq!(
                    edge.target_node_id,
                    Some("brain:doc-b".to_string()),
                    "target_node_id must be the qualified scope:doc_id"
                );
                assert_eq!(
                    edge.target_doc_id,
                    Some("doc-b".to_string()),
                    "target_doc_id must be the target's authored doc_id"
                );
                assert!(
                    !dangling_diag && !leaf_diag,
                    "resolved edge must have no dangling/leaf diagnostic, got diags: {diags:#?}"
                );
            }
            "leaf-file" => {
                assert!(
                    edge.target_node_id.is_none() && edge.target_doc_id.is_none(),
                    "leaf-target edge must have null target fields, got: {edge:?}"
                );
                assert!(
                    leaf_diag && !dangling_diag,
                    "leaf-target edge must have exactly a W_GRAPH_LEAF_TARGET diagnostic, got diags: {diags:#?}"
                );
            }
            "missing" => {
                assert!(
                    edge.target_node_id.is_none() && edge.target_doc_id.is_none(),
                    "dangling edge must have null target fields, got: {edge:?}"
                );
                assert!(
                    dangling_diag && !leaf_diag,
                    "dangling edge must have exactly an E_GRAPH_DANGLING_RELATED diagnostic, got diags: {diags:#?}"
                );
            }
            other => panic!("unexpected edge to_ref: {other}"),
        }
    }

    let _ = fs::remove_dir_all(&dir);
}
