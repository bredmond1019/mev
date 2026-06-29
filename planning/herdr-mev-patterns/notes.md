---
type: Note
title: Herdr Patterns for mev — Graph Integrity, Crawling & Future Validation
description: Detailed analysis of herdr source patterns applicable to mev. Block J (graph integrity) has been shipped — this documents which patterns it implemented, which remain for blocks Q/R/S, watch mode, and manifest-driven validation extensibility.
doc_id: herdr-mev-patterns
layer: [factory]
project: mev
status: active
keywords: [knowledge-graph, validation, crawling, manifest-driven, herdr, patterns]
related: [herdr-bella-console-research, master-plan, status]
---

# Herdr Patterns for mev — Graph Integrity, Crawling & Future Validation

> **Status:** active — Block J shipped; research complete. Patterns marked **DONE** are live in
> `src/brain/graph.rs`. Patterns marked **FUTURE** apply to blocks Q/R/S, watch mode, and
> manifest-driven validation extensibility.
> **Cross-reference:** `planning/herdr-bella-console-research/notes.md` (bastion/TUI focus;
> the prior Opus session that produced the companion note for bastion).
> **Promote with:** reference this note when planning Block Q (manifest emit), Block R (graph
> emit), or any future `--watch` / manifest-driven validation work.

---

## What & Why

Herdr (`/Users/brandon/Dev/agentic-portfolio/herdr/`) is a "tmux for AI agents" in Rust that
solves the same class of structural problems mev faces — hierarchical scope, flat registry +
O(1) lookup, edge resolution, lint dispatch, and manifest-driven rule evaluation. A parallel
Explore-agent session (2026-06-29) deep-read its source across four subsystems
(`detect/`, `api/`, `workspace.rs`, `integration/`) to extract applicable patterns.

**Herdr is reference-only — do not add it as a dependency** (AGPL-3.0, vendored C libs,
single-author, transport mismatch with BastionUI). All patterns are reimplemented clean. The
full "why not to build on herdr" analysis is in the companion note at
`planning/herdr-bella-console-research/notes.md`.

This note covers the **mev-specific angle only**: which patterns herdr offered, whether Block J
already implemented them, and what remains.

---

## Context & Background

### Block J — what shipped

`2.J-graph-integrity` completed 2026-06-29 as a git worktree (`trees/2.J-graph-integrity-flow`,
PR pending). The implementation lives in `src/brain/graph.rs` (667 lines) and `src/lib.rs`
(public API additions). Integration tests in `tests/brain_graph.rs` (378 lines).

**CLI surface added:**
```
mev validate-brain [path] --graph
```
Runs OKF frontmatter pass + corpus crawl + graph build + graph integrity check, all emitted
into one `Report`. Works with existing `--json` flag for machine-readable output.

**Public API added (`src/lib.rs`):**
```rust
pub use brain::graph::{Graph, build_graph, check_graph};
pub fn validate_brain_graph(root: &Path) -> anyhow::Result<Report>
```

**Core module (`src/brain/graph.rs`):**

| Type | Role |
|---|---|
| `EdgeKind` | `Copy`-able enum, `Serialize`, `rename_all = "snake_case"`. Single variant `Related` today; designed for typed-edge extension (supersedes, depends-on). |
| `Edge { from, to_ref, kind }` | Directed edge. `from` = canonical `scope:doc_id`; `to_ref` = as-authored raw ref string; `kind = EdgeKind`. Serializable (D4 artifact). |
| `Node { id, scope, doc_id, rel }` | `id = "scope:doc_id"`. `scope` denormalized from corpus. Serializable (D4 artifact). |
| `Graph { nodes, edges }` | The D4 serializable/emittable artifact; Phase 3B Block R loads this into Postgres. |
| `DocMeta { doc_id, related }` | D5 seam — the only struct that knows metadata comes from frontmatter. |
| `RawFrontmatter` | Minimal serde target for frontmatter parsing (`doc_id`, `related` only). |
| `GraphArtifact { graph, node_map, leaf_keys }` | Build output: the emittable `Graph` + lookup structures. Not serialized. |
| `read_doc_metadata(entry) -> DocMeta` | **D5 seam** — single site that calls `extract_frontmatter` and parses graph-relevant fields. Future foreign-format extractors replace this one function. |
| `build_graph(corpus, config) -> GraphArtifact` | Two-pass: (1) index all nodes, collect pending edges; (2) build edges from pending list. |
| `check_graph(artifact) -> Vec<Diagnostic>` | Uniqueness lint + edge resolution + leaf-target lint. Does not re-walk the corpus. |

**Graph diagnostics vocabulary:**
| Code | Severity | Meaning |
|---|---|---|
| `E_GRAPH_DUPLICATE_DOC_ID` | Error | Two nodes share a canonical `scope:doc_id` |
| `E_GRAPH_DANGLING_RELATED` | Error | `related:` entry resolves to nothing in corpus |
| `W_GRAPH_LEAF_TARGET` | Warning | `related:` entry resolves to a corpus file with no `doc_id` |

**Resolution rules (as shipped):**
- Bare ref (no `:`) → qualified to referrer's own scope: `"beta"` from `brain:alpha` → looks up `"brain:beta"`
- Qualified ref (contains `:`) → used as-is: `"mev:target"` → looks up `"mev:target"`
- A bare ref naming a `doc_id` that exists in *another* scope is correctly flagged as dangling (does NOT search cross-scope)

**`GraphArtifact` internal layout:**
```rust
pub struct GraphArtifact {
    pub graph: Graph,                       // emittable (Serialize)
    pub node_map: HashMap<String, usize>,   // canonical_id → graph.nodes index (O(1))
    pub leaf_keys: HashSet<String>,         // scope:stem for files with no doc_id
}
```

`node_map` keyed by the full `"scope:doc_id"` string — one flat HashMap covers the whole
multi-scope corpus with O(1) lookup. No nested-by-scope structure was needed.

**`CorpusEntry` in `src/brain/crawl.rs` already carries `scope: String`** (denormalized from
the scope registry at crawl time, added in `2.J-corpus-crawl`). `Node` re-exposes scope for
graph consumers without needing to re-resolve.

---

## Key Information — Herdr Pattern Analysis

### Pattern 1 — Tree + Flat Registry Duality

**Herdr source:** `herdr/src/workspace.rs`, `herdr/src/layout.rs`

Herdr's hierarchy (Workspace → Tab → Pane) uses a BSP tree for structure and flat
`HashMap<PaneId, PaneState>` for state access. Two registries per workspace:
- `panes: HashMap<PaneId, PaneState>` (tab-level, O(1) state access)
- `public_pane_numbers: HashMap<PaneId, usize>` (workspace-level, stable user-facing numbers)

The tree is structural only; all state is flat. Invariant enforced in tests: layout pane IDs
== panes HashMap keys (exact set equality).

**Key types:**
- `TileLayout { root: Node, focus: PaneId }` — BSP tree
- `Node::Split { direction, ratio, first: Box<Node>, second: Box<Node> }` / `Node::Pane(PaneId)`
- `TileLayout::panes(area) -> Vec<PaneInfo>` — walks tree to compute positions
- `TileLayout::splits(area) -> Vec<SplitBorder>` — split boundaries for drag-resize

**Status for mev: DONE (Block J).** Block J implemented the same duality via `GraphArtifact`:
- `graph.nodes: Vec<Node>` — ordered list (structural)
- `node_map: HashMap<String, usize>` — canonical_id → index (O(1) flat registry)
- `leaf_keys: HashSet<String>` — second flat registry for leaves

The key insight adopted: separate the emittable artifact (`Graph`) from the lookup structures
(`node_map`, `leaf_keys`). `check_graph` takes `&GraphArtifact`, never re-walks the corpus.

---

### Pattern 2 — Manifest-Driven Validation Rules

**Herdr source:** `herdr/src/detect/manifest.rs`, `herdr/src/detect/manifests/*.toml`

TOML manifests define validation rules with recursive boolean gates, region scoping, and
priority ordering. This is an entire validation DSL expressed in TOML rather than Rust.

**Core structs:**
```rust
struct AgentManifest {
    id: String,
    version: Option<ManifestVersion>,
    aliases: Vec<String>,
    rules: Vec<ManifestRule>,     // up to 128 per manifest
}
struct ManifestRule {
    id: String,
    state: Option<ManifestState>, // maps to severity in mev terms
    priority: i32,                // higher wins; first match selected
    region: String,               // which subsection of content to test
    contains: Vec<String>,        // case-insensitive substring (AND)
    regex: Vec<String>,           // full-text regex (case-sensitive, AND)
    line_regex: Vec<String>,      // any-line regex (AND)
    all: Vec<ManifestGate>,       // AND gates (recursive)
    any: Vec<ManifestGate>,       // OR gates (recursive)
    not: Vec<ManifestGate>,       // NOT gates (recursive)
}
```

**Matching semantics:** Within a rule/gate, ALL of: all `contains` found + all `regex` match
+ all `line_regex` match at least one line + all `all` gates pass + if `any` gates exist then
at least one passes + none of `not` gates pass.

**Region vocabulary (subsection targeting):**
| Region | Selects |
|---|---|
| `whole_recent` | Entire content |
| `after_last_horizontal_rule` | After last `─` line |
| `bottom_non_empty_lines(n)` | Last n non-empty lines |
| `after_last_prompt_marker` | After last `›` line |
| `prompt_box_body` | Between two horizontal rules |
| `osc_title` | Terminal OSC title string |

**Compilation:** Regex patterns compiled at manifest load time; matching is pure regex.Apply().
Hot-reload via `RwLock<ManifestCache>` + `reload_manifests()`.

**Safety limits** (all validated at parse time):
- `MAX_RULES_PER_MANIFEST = 128`
- `MAX_GATE_DEPTH = 8` (stack-overflow prevention)
- `MAX_TOTAL_GATES = 512`, `MAX_TOTAL_MATCHERS = 1024`, `MAX_MATCHER_CHARS = 512`

**Three-tier loading:** bundled via `include_str!()` → remote in `~/.config/herdr/` → local
override. Local shadows remote which shadows bundled.

**Rich diagnostics:** `DetectionExplain { matched_rule, evaluated_rules, evidence: RuleEvidence }`
— which rule fired, all evaluated rules, and region preview. Analogous to mev's `Diagnostic`
but richer (includes which pattern matched and what the region content looked like).

**TOML examples:**
```toml
# claude.toml — complex nesting: any + all + line_regex
[[rules]]
id = "bash_permission_prompt"
state = "blocked"
priority = 850
region = "whole_recent"
contains = ["do you want to proceed?"]
any = [
  { contains = ["bash command"] },
  { contains = ["bash("] },
]
all = [
  { any = [
    { line_regex = ['(?i)^\s*❯?\s*yes\b'] },
    { line_regex = ['(?i)^\s*1\.\s*yes\b'] },
  ] }
]

# codex.toml — NOT gates
[[rules]]
id = "osc_title_idle"
state = "idle"
priority = 100
region = "osc_title"
regex = ['\S']
not = [
  { regex = ['^[\x{2800}-\x{28FF}]'] },
  { contains = ["Action Required"] },
]
```

**File references:**
- `herdr/src/detect/manifest.rs` — `AgentManifest`, `ManifestRule`, `ManifestGate`,
  `compile_gate()`, `compiled_gate_matches()`, `parse_manifest()`, `reload_manifests()`
- `herdr/src/detect/mod.rs` — `AgentState`, orchestration
- `herdr/src/detect/manifests/claude.toml` — any+all+line_regex nesting
- `herdr/src/detect/manifests/codex.toml` — NOT gates
- `herdr/src/detect/manifests/gemini.toml` — deeply nested all-inside-any

**Status for mev: FUTURE.** Block J hardcodes all lint rules in Rust (duplicate detection,
edge resolution, leaf warning). Manifest-driven rules would allow: different OKF schemas per
scope unit, configurable `--graph` lint rules (e.g. required-edge types per document type),
and rule updates without recompilation. Not needed until the rule surface grows significantly.

**When to apply:** If the number of OKF lint rules grows to the point where Rust becomes
unwieldy, or if different Brain scope units need different validation rules (e.g. decisions
must have `supersedes`, plans must have `depends`).

---

### Pattern 3 — Enum-Based Registry + Match Dispatch

**Herdr source:** `herdr/src/api/schema/integrations.rs`, `herdr/src/integration/targets.rs`

`IntegrationTarget` is a `Copy` enum (13 variants) used as a dispatch key throughout the
integration system. No trait objects, no dynamic dispatch, no runtime registration.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationTarget { Pi, Claude, Codex, /* ... 13 variants */ }

// All dispatch via match:
fn integration_target_label(target: IntegrationTarget) -> &'static str {
    match target { IntegrationTarget::Claude => "claude", /* ... */ }
}
fn integration_target_command_names(target: IntegrationTarget) -> &'static [&'static str] {
    match target { IntegrationTarget::Kilo => &["kilo", "kilo-code"], /* ... */ }
}
```

Per-agent version constants for forward-compat: `CLAUDE_INTEGRATION_VERSION: u32 = 7`.

**File references:**
- `herdr/src/api/schema/integrations.rs` — `IntegrationTarget` enum
- `herdr/src/integration/targets.rs` — `install_target_inner()` full match dispatch
- `herdr/src/integration/env.rs` — per-agent path resolution helpers
- `herdr/src/integration/mod.rs` — per-agent version constants

**Status for mev: DONE (Block J — partially).** `EdgeKind` enum mirrors this pattern exactly:
```rust
// src/brain/graph.rs
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind { Related }
```
Single variant today; future typed edges (`Supersedes`, `DependsOn`, `Parent`) extend the
enum without reshaping `Edge` or `check_graph`. The `check_graph` function already implicitly
dispatches by EdgeKind via the `Edge` struct — adding typed-edge lints is a match arm addition.

**When to apply:** Next time a new edge type is needed. The pattern is already in place.

---

### Pattern 4 — Wait/Polling Primitive

**Herdr source:** `herdr/src/api/wait.rs`, `herdr/SKILL.md`

```rust
// Polling loop core (herdr/src/api/wait.rs):
let deadline = timeout_ms.map(|ms| Instant::now() + Duration::from_millis(ms));
loop {
    let text = read_pane_content(pane_id, source, lines, strip_ansi);
    if let Some(matched) = match_output(&text, &params.r#match, regex.as_ref()) {
        return Ok(OutputMatched { pane_id, revision, matched_line: matched, read });
    }
    if deadline.is_some_and(|d| Instant::now() >= d) {
        return Err("timeout");
    }
    std::thread::sleep(CONNECTION_POLL_INTERVAL); // 100ms
}

// Match conditions:
enum OutputMatch {
    Substring { value: String },
    Regex { value: String },
}
```

**Key constants:** `CONNECTION_POLL_INTERVAL = 100ms`, `APP_RESPONSE_TIMEOUT = 5s`.

**CLI surface:** `herdr wait output <pane> --match "ready" --timeout 30000 --regex`

**Status for mev: FUTURE.** mev has no `--watch` mode. When added, this pattern is the
implementation model: poll the corpus/file-system at a constant interval, match condition
= "any file mtime changed", timeout = optional. A lighter variant: `mev wait-clean` that
blocks until `validate_brain` passes (polling loop + deadline).

---

### Pattern 5 — Revision Tracking

**Herdr source:** `herdr/src/api/schema/panes.rs`, `herdr/src/api/schema/agents.rs`

```rust
struct PaneInfo {
    // ...
    revision: u64,  // increments on pane output change
}
struct PaneReadResult {
    // ...
    revision: u64,
    truncated: bool,
}
```

Subscribers skip re-read if `revision <= last_known_revision`. Enables incremental updates
without full re-scan. `PaneOutputChanged { pane_id, workspace_id, revision }` events let
subscribers know a re-read is needed without transmitting the content.

**Status for mev: FUTURE.** Currently mev does a full corpus crawl + validate on every run.
When `--watch` mode is added, attaching `content_hash: u64` (or `mtime: SystemTime`) to
`CorpusEntry` enables skipping unchanged files. The revision on `CorpusEntry` would be
computed once at crawl time and compared against a cached previous value.

**Note:** `CorpusEntry` already carries `scope: String` (denormalized at crawl time). Adding
`content_hash` follows the same pattern — one extra field computed once, read many times.

---

### Pattern 6 — Event Ring Buffer (for Block R graph emit)

**Herdr source:** implied from `herdr/src/api/schema/events.rs`, subscriptions system

```rust
const MAX_EVENTS: usize = 512;
struct EventHubState {
    next_sequence: u64,                   // always incrementing
    events: Vec<(u64, EventEnvelope)>,
}
// Subscriber pattern:
for (sequence, event) in event_hub.events_after(last_sequence) {
    last_sequence = sequence;
    if event matches filter { yield event }
}
```

Pure pull model: subscribers store `last_sequence` and poll `events_after()`. No push.
Ring buffer capped at 512; oldest events evicted. Monotonic sequence = subscriber can always
catch up without gaps.

**Status for mev: FUTURE (Block R).** When mev emits graph-change events to the orchestrator
(Block R — graph emit → Postgres edges table), this ring buffer pattern over the existing
`--json` stdout pipeline would allow incremental graph updates rather than full re-emit.

---

### Pattern 7 — Denormalized Parent IDs in Entity Model

**Herdr source:** `herdr/src/api/schema/panes.rs`

Every `PaneInfo` carries `workspace_id` and `tab_id` — no graph traversal needed to answer
"which workspace does this pane belong to?" Every `AgentInfo` adds a third level
(`workspace_id`, `tab_id`, `pane_id`).

**Status for mev: DONE (Block J — partially, on Node; not on CorpusEntry).** `Node` carries
`scope: String` denormalized from the corpus. `CorpusEntry` in `src/brain/crawl.rs` already
carries `scope: String` (added in Block J-crawl) — so the full denormalization chain exists:
CorpusEntry.scope → Node.scope → Edge.from (canonical id encodes scope).

Future consideration: if `CorpusEntry` needs more denormalized fields for Block Q (manifest
emit), the pattern is already established — compute once at crawl time in `crawl_corpus`,
store on the entry.

---

### Pattern 8 — Entity Hierarchy + API Query Model (for Block R)

**Herdr source:** `herdr/src/api/schema/` (panes.rs, tabs.rs, workspaces.rs, agents.rs,
events.rs)

Herdr's 4-level hierarchy (Workspace → Tab → Pane → Agent) with:
- List operations returning `Vec<EntityInfo>`
- Get operations returning `EntityInfo` by id
- Event subscription (ring buffer, poll-based)
- `wait` primitives for synchronization

The `PaneLayoutSnapshot` uses a recursive tree structure:
```rust
enum LayoutNode {
    Split { direction, ratio, first: Box<LayoutNode>, second: Box<LayoutNode> },
    Pane { pane_id, label, cwd, command, env },
}
```

**Status for mev: FUTURE (Block R).** When mev exposes a graph query surface (Block R), the
entity model to borrow: `NodeInfo`, `EdgeInfo` list/get operations over the same JSON schema
as the `--json` output. The `Graph { nodes, edges }` struct shipped in Block J is already
the right shape for this.

---

## Open Questions

None — research is complete and Block J is shipped. Remaining patterns are deferred until the
relevant block becomes active. Re-read this note when starting Block Q (manifest emit), Block R
(graph emit), or any `--watch` / manifest-driven validation work.

---

## Rough Scope

### Patterns already implemented (Block J) — no action needed

| Pattern | Where in code |
|---|---|
| Tree + flat registry (`GraphArtifact.graph` + `node_map` + `leaf_keys`) | `src/brain/graph.rs:165–243` |
| `EdgeKind` Copy enum + match dispatch | `src/brain/graph.rs:47–51` |
| `Edge { from, to_ref, kind }` serializable struct | `src/brain/graph.rs:58–66` |
| `Node { id, scope, doc_id, rel }` serializable struct | `src/brain/graph.rs:72–82` |
| Two-phase build (index all nodes, then resolve edges) | `src/brain/graph.rs:191–244` |
| D5 frontmatter seam (`read_doc_metadata`) | `src/brain/graph.rs:126–157` |
| `--graph` CLI flag | `src/main.rs:48–51`, `src/lib.rs:202–232` |
| Scope denormalization on `CorpusEntry` | `src/brain/crawl.rs:55` |

### Patterns remaining for future blocks

| Pattern | Applies to | Effort |
|---|---|---|
| Manifest-driven validation (Pattern 2) | Future OKF rule extensibility, or typed-edge lint rules | Med — new module `src/brain/rules/` + TOML schema |
| Wait/polling primitive (Pattern 4) | `--watch` mode, `mev wait-clean` | Low — thin loop over existing validate fn |
| Revision/content-hash tracking (Pattern 5) | `--watch` incremental re-validation | Low-Med — add field to `CorpusEntry`, cache previous values |
| Event ring buffer (Pattern 6) | Block R (graph emit → orchestrator) | Med — new event channel over `--json` |
| Graph query API (Pattern 8) | Block R (Postgres edges + bastion/MCP structural queries) | High — new `mev serve` or graph emit format |

### Typed edge extension (nearest next step for this module)

When the first non-`Related` edge type is needed (e.g. `Supersedes` for decision chains,
`DependsOn` for planning blocks), the change is:
1. Add variant to `EdgeKind` enum in `src/brain/graph.rs:49`
2. Add frontmatter field parsing in `RawFrontmatter` + `DocMeta`
3. Add lint arm in `check_graph` for the new edge type
4. Update `docs/okf-schema.md` with the new field
5. Tests in `tests/brain_graph.rs` for the new edge variant

`src/main.rs` and `src/lib.rs` do not need to change.

---

## References

- **mev Block J source:** `src/brain/graph.rs` — `EdgeKind`, `Edge`, `Node`, `Graph`,
  `GraphArtifact`, `DocMeta`, `read_doc_metadata`, `build_graph`, `check_graph`
- **mev public API:** `src/lib.rs` — `validate_brain_graph`, `Graph`, `build_graph`, `check_graph`
- **mev CLI:** `src/main.rs:36–52` — `--graph` flag on `validate-brain` subcommand
- **mev integration tests:** `tests/brain_graph.rs` — 378 lines, multi-unit fixture
- **Herdr detect engine:** `herdr/src/detect/manifest.rs`, `herdr/src/detect/manifests/`
- **Herdr workspace hierarchy:** `herdr/src/workspace.rs`, `herdr/src/layout.rs`
- **Herdr wait primitive:** `herdr/src/api/wait.rs`
- **Herdr integration registry:** `herdr/src/integration/registry.rs`,
  `herdr/src/api/schema/integrations.rs`
- **Herdr API schema:** `herdr/src/api/schema/` (panes.rs, tabs.rs, events.rs)
- **Companion note (bastion/TUI focus):** `planning/herdr-bella-console-research/notes.md`
  (in the brain repo at `/Users/brandon/Dev/agentic-portfolio/`)
- **Governing decisions:** D4 (corpus engine + knowledge graph emittable artifact),
  D5 (heterogeneous-format ingest, the `read_doc_metadata` seam)
