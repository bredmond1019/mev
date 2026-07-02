# Worklog — 3B.R-graph-emit

## Task 1 — PASSED (1 attempt)
What: Added src/brain/graph_emit.rs with GraphExport {version, root, nodes, edges, leaves} and build_graph_export(root, &GraphArtifact) -> GraphExport, reusing Node/Edge from graph.rs, sorting leaves for determinism, with 3 unit tests (node/edge/leaf mapping, empty corpus, JSON round-trip); registered pub mod graph_emit in src/brain/mod.rs.
Decisions: Cloned artifact.graph.nodes/edges directly rather than re-deriving, per the spec's 'pure compiler' guidance; leaves built via HashSet -> Vec then .sort() for deterministic output as required by AC
Validated: gating checks (fast tripwire)
