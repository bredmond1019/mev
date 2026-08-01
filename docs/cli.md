---
type: Reference
title: mev CLI Reference
description: Full reference for the mev command-line interface — subcommands, flags, defaults, exit codes, and output formats
doc_id: cli-reference
layer: [factory]
project: mev
status: active
keywords: [cli, validate, validate-brain, json, exit-codes, mev]
related: [architecture, brain-toml-config, okf-schema]
---

# mev CLI Reference

## Synopsis

```
mev [--json] <subcommand> [args]
```

## Global flags

| Flag | Description |
|---|---|
| `--json` | Emit a machine-readable JSON envelope to stdout instead of the human summary. Exit code behaviour is unchanged — exit 1 on any error-severity diagnostic. |

## Subcommands

### `validate [path]`

Validate the learn-ai content tree.

```bash
mev validate [path]
```

| Argument | Default | Description |
|---|---|---|
| `path` | `../learn-ai/content/learn` | Path to the content root |

Checks each file in the tree against the learn-ai frontmatter schema and JSON struct constraints (`LearnAiValidator`).

**Examples:**

```bash
# Default path
mev validate

# Explicit path
mev validate ~/Dev/learn-ai/content/learn

# Machine-readable output
mev --json validate
```

---

### `validate-brain [--sync] [--graph] [--state] [--links] [--structure] [path]`

Validate the Bastion Brain repo for OKF frontmatter compliance, and optionally check cross-repo sync watermark integrity, global knowledge-graph integrity, state.json schema and block-dependency graph integrity, link integrity, or structural `index.md` coverage.

```bash
mev validate-brain [--sync] [--graph] [--state] [--links] [--structure] [path]
```

| Argument / Flag | Default | Description |
|---|---|---|
| `path` | `..` | Path to the company-brain repo root |
| `--sync` | off | Also run the cross-repo sync watermark check (see below) |
| `--graph` | off | Also run the global `scope:doc_id` knowledge-graph integrity check (see below). Takes precedence over `--sync` when both flags are present — `--graph` is a superset. |
| `--state` | off | Also run the `planning/state.json` schema and cross-repo block-dependency graph integrity check (see below). Takes precedence over `--graph` and `--sync` in the dispatch chain. |
| `--structure` | off | Also run the bidirectional `index.md` ↔ directory structural coverage check (see below) — flag corpus files not referenced by their directory's `index.md`, and `index.md` rows pointing at a nonexistent target. Takes precedence over `--state`, `--graph`, and `--sync` in the dispatch chain. |
| `--links` | off | Also run the link-integrity pass (see below) — flag dead markdown links, broken `file://` URIs, dangling `[[wikilinks]]`, and references to moved/deleted paths. Takes the highest precedence overall — over `--structure`, `--state`, `--graph`, and `--sync` in the dispatch chain. |

Resolves `brain.toml` by walking up from `path`. If no `brain.toml` is found, a fatal `Error`-severity diagnostic with locator `E_CONFIG_NOT_FOUND` is emitted and the process exits 1.

See [`brain.toml` config](brain-toml.md) for the configuration format and [OKF schema](okf-schema.md) for what is validated.

#### `--sync` — cross-repo watermark check

When `--sync` is passed, `mev` runs the full OKF schema pass first, then appends a second pass that compares watermarks for every `[[repos]]` entry in `brain.toml`:

- Reads `timestamp` from `<repo_path>/<status_file>` (the sub-repo's status file).
- Reads `synced_from` from `<cache_doc>` (the brain cache doc for that repo).
- Both values must be present and valid RFC3339 datetimes; they are compared as explicit UTC
  instants (each side normalized via `.to_utc()`), not as raw strings — a `-03:00` watermark and
  a `Z` watermark denoting the same moment are in sync.

A mismatch or missing watermark emits an `Error`-severity diagnostic and causes exit 1.

| Locator | Condition |
|---|---|
| `E_SYNC_FILE_MISSING` | `status_file` or `cache_doc` does not exist, or cannot be read |
| `E_SYNC_WATERMARK_MISSING` | `timestamp` or `synced_from` field is absent from the frontmatter |
| `E_SYNC_WATERMARK_MALFORMED` | A watermark is present but is not a valid RFC3339 datetime |
| `E_SYNC_DRIFT` | Both watermarks parse successfully but denote different instants |

#### `--graph` — knowledge-graph integrity check

When `--graph` is passed, `mev` runs the full OKF schema pass first, then appends a graph integrity pass:

1. Crawls the corpus (same registry-driven walk as the OKF pass).
2. Builds the global `scope:doc_id` knowledge graph from frontmatter (`doc_id` + `related` fields).
3. Runs integrity checks over the built graph.

Files with a `doc_id` become graph nodes; files without one are leaves. All `related:` entries are resolved as either bare `doc_id` refs (resolved within the from-node's scope first) or qualified `scope:doc_id` refs.

| Locator | Severity | Condition |
|---|---|---|
| `E_GRAPH_DUPLICATE_DOC_ID` | Error | Two corpus files in the same scope share a `doc_id`. |
| `related` | Error | A `related:` entry resolves to no node and no leaf (dangling). |
| `related` | Warning | A `related:` entry resolves to a corpus file that has no `doc_id` (leaf target). |

Graph errors (`E_GRAPH_DUPLICATE_DOC_ID`, dangling `related:`) cause exit 1. The leaf-target warning alone does not change the exit code.

`--graph` takes precedence over `--sync` when both flags are given — it is a superset (runs the OKF schema pass that `--sync` also runs, plus the graph pass).

#### `--state` — state.json schema and block-dependency graph check

When `--state` is passed, `mev` runs the full OKF schema pass first, then appends the state-validation pipeline:

1. **Discovery** — finds all `planning/state.json` files: the HQ brain state, each tier sub-brain state (via `tiers[].rollup` in the HQ state), and each leaf project state (via `[[repos]]` in `brain.toml`). A leaf repo whose `brain.toml` `tier` is `"portfolio"` is expected as `kind:"portfolio"` instead of `kind:"project"` — these are terminal repos (published to GitHub, no further planning state), expected to carry a non-empty `note` instead of `tracks[]`, and are skipped entirely by `emit-state`'s wave-table splice (no `master-plan.md` expected). Missing files emit `W_STATE_FILE_MISSING`. If the HQ root's own `state.json` exists but fails to load (parse error), tier sub-brain paths are recovered directly from `brain.toml`'s `[[repos]]` tier config (rather than from the unloadable HQ `tiers[]`) and registered as `expected_kind:"brain"` stubs — this prevents them from falling through to the leaf `[[repos]]` loop and being misclassified as `expected_kind:"project"`. A single `E_STATE_ROOT_LOAD_FAILED` diagnostic names the degraded classification; the root's own detailed `E_STATE_MALFORMED_JSON` remains the actionable error, instead of a cascade of spurious `E_STATE_SCHEMA_BAD_KIND` on every tier.
2. **Load** — deserializes each discovered file. Unparseable files emit `E_STATE_MALFORMED_JSON`, which now includes the underlying `serde_json::Error` detail (offending field/type and line:column), not just the generic message.
3. **Schema ring** — checks field validity within each file (kind membership, status enum values, `blocked_by` well-formedness, kind-appropriate sections). In v2 schema files: validates `depends_on[]` entry well-formedness on track blocks, rejects authored `status:"blocked"` (derived, not authored), and validates `backlog[].status` membership.
4. **Graph** — builds the cross-repo block-dependency graph from all loaded files (v2: DAG edges sourced from `tracks[].blocks[].depends_on[]`) and checks it for integrity violations, including cycle detection over the `depends_on` DAG and backlog-node integrity.
5. **Status consistency** — checks that a `closed` block does not depend (via `depends_on`) on a block that is not yet `closed`.
6. **Rollup** — checks that brain `repos[]` headline entries (now/next) match their children's actual `focus` values.
7. **Focus drift** — recomputes the expected `focus` from authored `tracks[]` and warns when the stored `focus` disagrees (warning-only; exit code is unchanged).

`--state` takes precedence over `--graph` and `--sync` in the dispatch chain — when `--state` is present, neither `--graph` nor `--sync` are separately invoked. `--structure` takes precedence over `--state`, `--graph`, and `--sync`. `--links` takes the highest precedence overall; when `--links` is present, `--structure`, `--state`, `--graph`, and `--sync` are not separately invoked.

| Locator | Severity | Condition |
|---|---|---|
| `W_STATE_FILE_MISSING` | Warning | A registered repo has no `planning/state.json` |
| `E_STATE_MALFORMED_JSON` | Error | A state.json file is not valid JSON or does not match the expected schema; message includes the underlying serde error (field/type + line:column) |
| `E_STATE_ROOT_LOAD_FAILED` | Error | The HQ root `state.json` exists but failed to load; tier sub-brain classification is degraded (recovered from `brain.toml` instead of the unloadable root) — see the root's own `E_STATE_MALFORMED_JSON` for the actual cause |
| `E_STATE_SCHEMA_BAD_KIND` | Error | `kind` is not one of `project`, `brain`, or `portfolio` |
| `E_STATE_SCHEMA_MISSING_FIELD` | Error/Warning | A required field is absent or a kind-appropriate section is missing |
| `E_STATE_SCHEMA_BAD_STATUS` | Error | A `status` value is not in the allowed enum |
| `E_STATE_SCHEMA_BAD_BLOCKED_BY` | Error | A `blocked_by[]` entry has an unknown or malformed `type` |
| `E_STATE_DUPLICATE_BLOCK_ID` | Error | Two `tracks[]` blocks in the same repo share an `id` |
| `E_STATE_DANGLING_FOCUS` | Error | A leaf repo focus entry's `block` is absent from `tracks[]` |
| `E_STATE_UNKNOWN_REPO` | Error | A `blocked_by` or `cross_repo` edge names an unknown repo |
| `E_STATE_DANGLING_BLOCKED_BY` | Error | A cross-repo block dependency's block does not exist in the named repo |
| `E_STATE_DANGLING_CROSS_REPO` | Error | A brain `cross_repo[]` edge's endpoint does not resolve to a known block |
| `W_STATE_ROLLUP_DRIFT` | Warning | Brain `repos[]` headline differs from the child repo's actual `focus` |
| `E_STATE_CYCLE` | Error | A `depends_on` edge forms a cycle; the cycle path is named in the message |
| `E_STATE_AUTHORED_BLOCKED` | Error | A `tracks[].blocks[].status` is `"blocked"` — `blocked` is derived, not authored. (`"deferred"`, by contrast, **is** a legal authored status.) |
| `E_STATE_STATUS_INCONSISTENT` | Error | A `closed` block has a `type:block` `depends_on` target that is not `closed` |
| `E_STATE_DANGLING_PROMOTION` | Error | A `status:"promoted"` backlog node's `block` pointer resolves to no `tracks[]` node |
| `E_STATE_PRIORITY_RANGE` | Error | A `priority` value is not in 0..=3 |
| `E_STATE_DUE_FORMAT` | Error | A `due` value is not a valid YYYY-MM-DD date |
| `E_STATE_SDLC_WORKFLOW_ENUM` | Error | An `sdlc_workflow` value is not in `{none, patch, task, run, flow}` |
| `E_STATE_MODEL_ENUM` | Error | A `model` value is not in `{sonnet, gemini-pro, gemini-flash, either}` |
| `E_STATE_DATE_FORMAT` | Error | A carryover/backlog `created` / `reviewed` / `snoozed_until` value is not a valid `YYYY-MM-DD` (or RFC3339) date |
| `W_STATE_FOCUS_DRIFT` | Warning | Stored `focus` disagrees with the derivation from `tracks[]`; exit code is unchanged |
| `W_STATE_CARRYOVER_STALE` | Warning | A `carryover[]` entry has aged past its per-`kind` `[attention]` threshold and is not snoozed; exit code is unchanged |
| `W_STATE_BACKLOG_STALE` | Warning | An HQ `backlog[]` `idea`/`ready` node has aged past the `[attention]` backlog threshold and is not snoozed; exit code is unchanged |
| `W_DISTILL_STALE` | Warning | A D35-distilled `knowledge.md`/`memory.md` entry has aged past its `[attention]` `knowledge_days`/`memory_days` threshold (`check_distill_staleness`); exit code is unchanged |

#### `--structure` — structural `index.md` coverage check

When `--structure` is passed, `mev` runs the full OKF schema pass first, then appends the structural coverage pass (D17 / CLAUDE.md Standing Rule 7):

1. Crawls the corpus (same registry-driven walk as the OKF pass).
2. Locates every directory's `index.md` corpus member and its direct-child corpus entries (siblings of that `index.md`; subdirectories are excluded — they are covered by their own `index.md`).
3. Extracts every markdown `[text](path)` link and `file://` URI from each `index.md` and resolves it against that `index.md`'s directory.
4. Checks both directions: every direct-child file must be referenced by the `index.md` (orphan detection), and every resolved `index.md` link that lands inside the corpus root must exist on disk (dangling-row detection).

Directories with no `index.md` corpus member are skipped entirely — no coverage obligation, so no orphan diagnostics. `[[wikilink]]` targets, external (`http(s)://`, `mailto:`, etc.) links, and links that resolve outside the corpus root are ignored (owned elsewhere / out of scope for this check).

| Locator | Severity | Condition |
|---|---|---|
| `E_STRUCT_ORPHAN_FILE` | Error | A corpus file in a directory is not referenced by that directory's `index.md`. Located at the orphan file. |
| `E_STRUCT_DANGLING_ROW` | Error | An `index.md` row (markdown or `file://` link) resolves to a target inside the corpus root that does not exist on disk. Located at the `index.md`. |

Any error-severity diagnostic causes exit 1.

`--structure` takes precedence over `--state`, `--graph`, and `--sync` in the dispatch chain — when `--structure` is present, none of those are separately invoked. `--links` takes precedence over `--structure`.

#### `--links` — link-integrity pass

When `--links` is passed, `mev` runs the full OKF schema pass first, then appends a link-integrity pass:

1. **Extract** — parses every corpus file for markdown `[text](path)` inline links, `file://` URIs, and `[[wikilink]]` references. External links (`http://`, `https://`, `mailto:`, `tel:`, protocol-relative `//`) and pure in-page anchors (`#section`) are unconditionally skipped.
2. **Resolve** — checks each local reference on disk:
   - Relative markdown links are resolved against the referring file's directory.
   - `file://` URIs are resolved to absolute paths.
   - `[[wikilinks]]` are matched against the set of authored `doc_id`s in the corpus.
3. **Moved-reference re-check** — reads `.brain-moves-pending` from the brain root (optional/ephemeral; if missing, no diagnostics are added). Each line is `<ISO-date> <path...>`; the pass flags any corpus reference that still targets a moved or deleted path.

The pass is **read-only** — it never mutates the corpus (D25).

| Locator | Severity | Condition |
|---|---|---|
| `E_LINK_DEAD_MARKDOWN` | Error | A markdown `[text](path)` link's resolved path does not exist on disk |
| `E_LINK_DEAD_FILE_URI` | Error | A `file://` URI's resolved path does not exist on disk |
| `E_LINK_DANGLING_WIKILINK` | Error | A `[[wikilink]]` target slug is not present in the corpus `doc_id` set |
| `E_LINK_MOVED_REFERENCE` | Error | A markdown or `file://` reference still points at a path listed in `.brain-moves-pending` |

Any error-severity diagnostic causes exit 1.

**Examples:**

```bash
# Default: validates OKF frontmatter in the sibling brain repo at ..
mev validate-brain

# OKF pass + sync watermark check
mev validate-brain --sync

# Explicit path with sync check
mev validate-brain --sync ~/Dev/agentic-portfolio

# OKF pass + knowledge-graph integrity check
mev validate-brain --graph

# Explicit path with graph check
mev validate-brain --graph ~/Dev/agentic-portfolio

# OKF pass + state.json schema and block-dependency graph check
mev validate-brain --state

# Explicit path with state check
mev validate-brain --state ~/Dev/agentic-portfolio

# Machine-readable output (consumed by the Brain RAG indexer)
mev --json validate-brain ~/Dev/agentic-portfolio

# Machine-readable output including sync diagnostics
mev --json validate-brain --sync ~/Dev/agentic-portfolio

# Machine-readable output including graph diagnostics
mev --json validate-brain --graph ~/Dev/agentic-portfolio

# Machine-readable output including state diagnostics
mev --json validate-brain --state ~/Dev/agentic-portfolio

# OKF pass + link-integrity check
mev validate-brain --links

# Explicit path with link-integrity check
mev validate-brain --links ~/Dev/agentic-portfolio

# Machine-readable output including link diagnostics
mev --json validate-brain --links ~/Dev/agentic-portfolio

# OKF pass + structural index.md coverage check
mev validate-brain --structure

# Explicit path with structural coverage check
mev validate-brain --structure ~/Dev/agentic-portfolio

# Machine-readable output including structural diagnostics
mev --json validate-brain --structure ~/Dev/agentic-portfolio
```

---

### `generate-graph [--out] [path]`

Generate an interactive HTML visualization of the Bastion Brain knowledge graph.

```bash
mev generate-graph [--out <dir>] [path]
```

| Argument / Flag | Default | Description |
|---|---|---|
| `path` | `.` | Path to search from when locating `brain.toml` (walks up to find it) |
| `--out` | `<brain_root>/planning/doc-graph` | The output directory to write the graph files (`graph.md` and `graph.html`) to |

Resolves `brain.toml` by walking up from `path`. If no `brain.toml` is found, the process exits 1.

The output is an interactive `vis.js` physics simulation that visualizes all `scope:doc_id` nodes and their `related:` edges across the entire portfolio. It includes color coding by repository scope, node sizing based on connectivity (hub nodes), hover tooltips, and a dynamic search and filtering UI.

**Examples:**

```bash
# Generate the graph in the default location (planning/doc-graph)
mev generate-graph

# Generate the graph from an explicit brain root
mev generate-graph ~/Dev/agentic-portfolio

# Generate the graph to a custom output directory
mev generate-graph --out /tmp/my-graph
```

---

### `manifest [--pretty] [path]`

Emit a JSON manifest of every file in the Brain corpus.

```bash
mev manifest [--pretty] [path]
```

| Argument / Flag | Default | Description |
|---|---|---|
| `path` | `.` | Path to search from when locating `brain.toml` (walks up to find it) |
| `--pretty` | off | Emit pretty-printed (indented) JSON instead of compact JSON |

Resolves `brain.toml` by walking up from `path`. If no `brain.toml` is found, the process
exits 1 with an error message on stderr.

The output is the manifest JSON written directly to stdout — there is no `--json` envelope
wrapper; the output *is* JSON.

#### Output shape

```json
{
  "version": "1",
  "root": "/path/to/brain",
  "entries": [
    {
      "rel": "planning/status.md",
      "scope": "brain",
      "doc_id": "mev-status",
      "doc_type": "ProjectStatus",
      "title": "MEV Status",
      "description": "Current project state for the mev validator.",
      "layer": ["factory"],
      "project": "mev",
      "status": "active",
      "keywords": ["mev", "status", "validator"]
    },
    {
      "rel": "README.md",
      "scope": "brain",
      "doc_id": null,
      "doc_type": null,
      "title": null,
      "description": null,
      "layer": null,
      "project": null,
      "status": null,
      "keywords": null
    }
  ]
}
```

| Field | Type | Description |
|---|---|---|
| `version` | string | Schema version — currently `"1"` |
| `root` | string | Display path of the HQ root used for the crawl |
| `entries` | array | All corpus files, in walk order |
| `entries[].rel` | string | Path relative to the HQ crawl root (forward-slash separated) |
| `entries[].scope` | string | Stable scope slug of the owning registry unit (e.g. `"brain"`, `"mev"`) |
| `entries[].doc_id` | string \| null | OKF `doc_id` field; `null` when not present in frontmatter |
| `entries[].doc_type` | string \| null | OKF `type` field (serialized as `doc_type`); `null` when absent |
| `entries[].title` | string \| null | OKF `title` field; `null` when absent |
| `entries[].description` | string \| null | OKF `description` field; `null` when absent |
| `entries[].layer` | array \| null | OKF `layer` field (closed-set list); `null` when absent |
| `entries[].project` | string \| null | OKF `project` field; `null` when absent |
| `entries[].status` | string \| null | OKF `status` field; `null` when absent |
| `entries[].keywords` | array \| null | OKF `keywords` field (3–7 free-form terms); `null` when absent |

Files without parseable frontmatter appear in the manifest with all metadata fields set to
`null` (graceful degradation — the OKF validator reports the error separately).

#### Exit codes

| Code | Meaning |
|---|---|
| `0` | Manifest emitted successfully |
| `1` | `brain.toml` not found, or a runtime error prevented crawl completion |

**Examples:**

```bash
# Compact JSON from the current directory
mev manifest

# Compact JSON from an explicit brain root
mev manifest ~/Dev/agentic-portfolio

# Pretty-printed JSON
mev manifest --pretty

# Pretty-printed JSON from an explicit brain root
mev manifest --pretty ~/Dev/agentic-portfolio

# Pipe compact JSON into jq
mev manifest | jq '.entries | length'
```

---

### `emit-graph [--pretty] [path]`

Emit the `scope:doc_id` knowledge graph — authored nodes, `related:` edges, and marked leaves — as a canonical JSON artifact.

```bash
mev emit-graph [--pretty] [path]
```

| Argument / Flag | Default | Description |
|---|---|---|
| `path` | `.` | Path to search from when locating `brain.toml` (walks up to find it) |
| `--pretty` | off | Emit pretty-printed (indented) JSON instead of compact JSON |

Resolves `brain.toml` by walking up from `path`. If no `brain.toml` is found, the process
exits 1 with an error message on stderr (mentioning `brain.toml`).

The output is the graph-export JSON written directly to stdout — there is no `--json` envelope
wrapper; the output *is* JSON. `mev emit-graph` is a **pure emit**: it does not write to any
file or database, and it does not re-derive or re-walk the graph — it reuses the same
`build_graph` pass that backs `mev validate-brain --graph`.

This is distinct from `generate-graph` (above), which renders an interactive HTML/`vis.js`
visualization for humans. `emit-graph` produces a machine-readable JSON artifact intended for
the orchestrator to load into a Postgres edges table beside `brain_documents` (D4).

#### Output shape

```json
{
  "version": "2",
  "root": "/path/to/brain",
  "nodes": [
    {
      "id": "brain:alpha",
      "scope": "brain",
      "doc_id": "alpha",
      "rel": "docs/alpha.md"
    }
  ],
  "edges": [
    {
      "from": "brain:alpha",
      "to_ref": "beta",
      "kind": "related",
      "target_node_id": "brain:beta",
      "target_doc_id": "beta"
    },
    {
      "from": "brain:alpha",
      "to_ref": "missing",
      "kind": "related",
      "target_node_id": null,
      "target_doc_id": null
    }
  ],
  "leaves": ["brain:a-leaf", "brain:z-leaf"]
}
```

| Field | Type | Description |
|---|---|---|
| `version` | string | Schema version — currently `"2"` |
| `root` | string | Display path of the HQ root used for the crawl |
| `nodes` | array | Every corpus file with an authored `doc_id`, in walk order — one node per `scope:doc_id` |
| `nodes[].id` | string | Canonical node id: `scope:doc_id` |
| `nodes[].scope` | string | Owning scope slug (from the corpus registry) |
| `nodes[].doc_id` | string | Authored `doc_id` (location-independent frontmatter field) |
| `nodes[].rel` | string | Path of the file relative to the HQ crawl root |
| `edges` | array | Every authored `related:` entry, in walk order |
| `edges[].from` | string | Canonical id of the source node (`scope:doc_id`) |
| `edges[].to_ref` | string | The raw `related:` entry as authored (bare or `scope:doc_id`) — not yet resolved/normalized |
| `edges[].kind` | string | Edge type; currently only `"related"` |
| `edges[].target_node_id` | string \| null | Qualified `scope:doc_id` of the resolved target node; non-null when the edge resolves to a real node, `null` when it is dangling or targets a leaf (doc-id-less file) |
| `edges[].target_doc_id` | string \| null | Authored `doc_id` of the resolved target node; non-null exactly when `target_node_id` is non-null |
| `leaves` | array | `scope:stem` for every corpus file with **no** authored `doc_id`, sorted for deterministic output |

`leaves` is always sorted, so repeated runs over an unchanged corpus emit byte-identical
output.

#### Exit codes

| Code | Meaning |
|---|---|
| `0` | Graph emitted successfully |
| `1` | `brain.toml` not found, or a runtime error prevented crawl completion |

**Examples:**

```bash
# Compact JSON from the current directory
mev emit-graph

# Compact JSON from an explicit brain root
mev emit-graph ~/Dev/agentic-portfolio

# Pretty-printed JSON
mev emit-graph --pretty

# Pipe compact JSON into jq for summary counts
mev emit-graph | jq '{nodes: (.nodes|length), edges: (.edges|length), leaves: (.leaves|length)}'
```

---

### `emit-block-graph [flags] [path]`

Emit the corpus-wide block-dependency graph — every discovered `planning/state.json` block,
enriched with derived attention/priority/topology fields, filtered by an optional scope — as a
JSON artifact.

```bash
mev emit-block-graph [--scope <hq|tier|repo|epic>] [--tier <NAME>] [--epic <SLUG>]
                      [--repo <SLUG>] [--include-closed] [--include-boundary]
                      [--max-nodes <N>] [--pretty] [path]
```

| Argument / Flag | Default | Description |
|---|---|---|
| `path` | `.` | Path to search from when locating `brain.toml` (walks up to find it) |
| `--scope <hq\|tier\|repo\|epic>` | `hq` | Scope mode — see the table below |
| `--tier <NAME>` | `core` | Tier name to scope to; consulted only when `--scope tier` is given |
| `--epic <SLUG>` | unset | Epic slug to project onto; required when `--scope epic` is given. Overrides `--tier`/`--repo` rather than intersecting with them |
| `--repo <SLUG>` | unset | Repo slug to intersect against; required when `--scope repo` is given |
| `--include-closed` | off | Include `closed`-lane blocks in the exported node set |
| `--include-boundary` | off | Re-add direct neighbours of the in-scope set as boundary nodes (`in_scope: false`); retains edges that cross the scope boundary |
| `--max-nodes <N>` | unset (no truncation) | Cap the exported node list at `N` nodes (topo-ordered); sets `truncated: true` when the pre-truncation node count exceeds `N` |
| `--pretty` | off | Emit pretty-printed (indented) JSON instead of compact JSON |

`--scope` selects the mode:

| `--scope` | Meaning | Companion flag |
|---|---|---|
| `hq` (default) | Every repo (`TierScope::All`) | — |
| `tier` | Repos in `--tier` | `--tier <NAME>` (default `core`) |
| `repo` | `TierScope::All` intersected with a single repo | `--repo <SLUG>` (required) |
| `epic` | Epic projection; overrides `--tier`/`--repo` | `--epic <SLUG>` (required) |

Resolves `brain.toml` by walking up from `path`. If no `brain.toml` is found, the process
exits 1 with an error message on stderr (mentioning `brain.toml`).

`mev emit-block-graph` is a **pure emit**: nothing is ever written to disk, no cache, no
side effects. It does not re-derive the block graph — it is a serializer over
`mev::block_graph_brain` / `build_block_graph_export`, and the output is emitted **verbatim**:
no post-processing, no field reordering, no added or dropped keys. This is the CLI companion
to bastion's `GET /api/blocks/graph` (`BA.17.A`) — node counts for a given scope must match
that endpoint's.

#### Output shape

```json
{
  "version": "1",
  "root": "/path/to/brain",
  "scope": {
    "tier": null,
    "epic": null,
    "repo": null,
    "include_closed": false,
    "include_boundary": false
  },
  "nodes": [
    {
      "key": "repo:BLOCK-ID",
      "repo": "repo",
      "id": "BLOCK-ID",
      "title": "...",
      "status": "open",
      "lane": "next",
      "track": "Phase 1",
      "wave": 1,
      "priority": 2,
      "effective_priority": 2,
      "due": null,
      "epics": [],
      "layer": 0,
      "topo_index": 0,
      "ready": true,
      "in_cycle": false,
      "in_scope": true,
      "external_deps": [],
      "unmet_count": 0,
      "dependent_count": 0,
      "last_touched": null
    }
  ],
  "edges": [
    {
      "from": "repo:BLOCK-ID",
      "to_ref": "repo:OTHER-ID",
      "kind": "blocked_by",
      "target_node_id": "repo:OTHER-ID",
      "blocking": true
    }
  ],
  "cycles": [],
  "total_nodes": 1,
  "truncated": false
}
```

#### Field guide

`version`, `root`, `scope` (an echo of the resolved scope request), `nodes`, `edges`, `cycles`
(over the **full corpus**, never the scoped subgraph), `total_nodes` (node count before any
`--max-nodes` truncation), and `truncated`.

Every node carries the full-corpus derivations that back the state-graph views:

| Field | Description |
|---|---|
| `lane` | Derived attention lane: `now` / `next` / `blocked` / `deferred` / `closed` / `other` |
| `layer` | Longest path over resolved `depends_on` edges (`0` = no resolved prerequisites) |
| `topo_index` | Position in the full-corpus topological order |
| `effective_priority` | Effective priority; absent when it never lands in the real `0..=3` range |
| `in_scope` | `true` for a scope survivor, `false` for a node re-added only as an `--include-boundary` neighbour |
| `unmet_count` | Count of unmet dependencies for a `blocked` node; `0` for every other lane |
| `dependent_count` | Corpus-wide count of in-corpus blocks whose `BlockedBy` edges point at this node (`CrossRepo` edges excluded). Computed over the **full corpus before scope filtering**, exactly like `layer`, `topo_index`, and `effective_priority` — so it is **identical for a given node across a scoped and an unscoped export**, and reports `0` (never absent, never a sentinel) for a node nothing depends on |
| `last_touched` | Derived — never authored in any `state.json` — from the block's own on-disk SDLC run artifacts: `<spec-folder>/sdlc/sdlc-{flow,task,run,}-state.json` (all four kinds are read). The **newest** `updated_at` wins across every matched spec folder and every state-file kind, including folders under `planning/archive/`. Computed over the **full corpus before scope filtering**, exactly like `dependent_count` — so it is **identical for a given node across a scoped and an unscoped export**. `null` means the block has **never been worked**, not that it was worked long ago — no sentinel date and no `state.json.updated` fallback is ever substituted for a missing run |

#### Exit codes

| Code | Meaning |
|---|---|
| `0` | Graph emitted successfully |
| `1` | `brain.toml` not found, `--scope epic` given without `--epic`, `--scope repo` given without `--repo`, an unknown or blank `--epic` slug, or a serialization/runtime error |

**Examples:**

```bash
# Compact JSON, whole corpus, from the current directory
mev emit-block-graph

# Pretty-printed JSON from an explicit brain root
mev emit-block-graph --pretty ~/Dev/agentic-portfolio

# Scope to one tier
mev emit-block-graph --scope tier --tier core

# Scope to one repo
mev emit-block-graph --scope repo --repo mev

# Project onto one epic
mev emit-block-graph --scope epic --epic bastion-tui

# Include closed blocks and boundary neighbours
mev emit-block-graph --include-closed --include-boundary

# Cap the node list and check whether it truncated
mev emit-block-graph --max-nodes 50 | jq '.truncated'

# Summary counts via jq (the program-plan smoke check)
mev emit-block-graph --pretty ~/Dev/agentic-portfolio | jq '{v:.version, n:(.nodes|length), e:(.edges|length), cycles:(.cycles|length), truncated}'
```

---

### `emit-state [--write] [path]`

Regenerate all derived views in the Brain corpus from the authored `tracks[]` DAG and write them in place (with `--write`) or report what would change (dry-run, without `--write`).

`mev emit-state` is the **single derivation engine** that `/log-work` shells out to for regenerating leaf `focus` fields, the brain `repos[]` / `cross_repo[]` rollup, brain `focus`, the master-plan wave/dependency tables, the per-project cache docs (focus line + `synced_from` watermark), the tier sub-brain rollup tables, the HQ Operating Board, and the HQ unified priority board. Because the validator's `check_focus_drift` and `check_rollup` share the same `derive_focus` / `derive_rollup` functions, running `mev emit-state --write` followed by `mev validate-brain --state` on the same corpus will report zero `W_STATE_FOCUS_DRIFT` and zero `W_STATE_ROLLUP_DRIFT` — the emit is, by construction, the fixed point of the drift check across every generated surface.

```bash
mev emit-state [--write] [--scope <repo>] [path]
```

| Argument / Flag | Default | Description |
|---|---|---|
| `path` | `.` | Path to search from when locating `brain.toml` (walks up to find it) |
| `--write` | off | Write the derived views in place. Without this flag the command is a dry-run |
| `--scope <repo>` | unset (whole corpus) | Limit regeneration to one repo's own derived surfaces plus the rollups it feeds — nothing else. Omit for today's default full-corpus behaviour, byte-for-byte unchanged. |

#### `--scope <repo>` — per-repo regeneration

Unscoped, `emit-state --write` regenerates the derived surfaces of **every** registered repo on
every run — a single `/log-work` in one sub-repo dirties dozens of files across the whole corpus.
`--scope <repo>` narrows a `--write` to exactly the surfaces that one repo feeds, derived
mechanically from the `[[repos]]` registry in `brain.toml` (never hardcoded):

- the repo's own leaf `planning/state.json`,
- its `cache_doc` (e.g. `docs/projects/<slug>.md`),
- its tier container's rollup `status.md` (the `[[repos]]` entry whose `slug` matches this repo's
  `tier`), when that tier resolves to a distinct registered entry, and
- the HQ root's `status.md` (the Operating Board) — every repo feeds this one.

Every other repo's files are left byte-identical. A scoped run never blanks or truncates a repo it
did not visit — rollups preserve every row it didn't touch. An unknown `--scope` slug fails fast
with `E_EMIT_UNKNOWN_SCOPE`, naming every valid slug, before any planner runs or any file is
touched.

**Operating guidance:** reach for `--scope <repo>` from a single sub-repo's own workflow commands
(`/log-work`, `/start-block`, `/blocked`, …) where only that repo's state changed — it keeps the
diff local and avoids cross-repo churn when several agents are working concurrently. Leave the
periodic full reconciliation (cron-bound `routine.sh`, or any run meant to catch drift across the
whole corpus) unscoped — only a full run recomputes every cross-repo edge (`cross_repo[]`, the
unified board's cross-repo priority sort, epic relationships) that a single repo's scope cannot see.

```bash
# Regenerate only mev's own derived surfaces (its state.json, cache doc, tier rollup, HQ board)
mev emit-state --scope mev --write

# Unknown slug: fails fast, names the valid slugs, writes nothing
mev emit-state --scope not-a-repo --write
```

#### Advisory lock on `--write`

`--write` (scoped or not) takes an exclusive advisory lockfile at `<root>/.mev-emit.lock` for the
duration of the run, recording the owning pid. This guards against concurrent `emit-state --write`
invocations interleaving writes to the same derived file — a real risk given how many workflow
commands shell out to `emit-state --write`, and one `E_EMIT_LINKED_WORKTREE` does not cover: that
guard only catches a linked git *worktree*, not the symlinked `planning/` vaults (D46) two agents
in different sub-repos can both be writing through at once.

A second concurrent `--write` polls briefly for the lock to free up, then fails with
`E_EMIT_LOCK_HELD` (naming the holder's pid) and writes nothing rather than interleaving. A
lockfile whose owning process is no longer alive is reclaimed automatically instead of blocking
forever. Dry-run (no `--write`) never takes the lock and is unaffected by contention.

#### Derived views updated

- **Leaf `state.json`** (`kind == "project"`): regenerates `focus` — `now` = blocks with `status: in_progress`; `next` = ready open blocks in `wave` order; `blocked` = open blocks with an unmet `depends_on`, each carrying the unmet `blocked_by[]` subset. Authored `tracks[]` and all other fields survive the round-trip unchanged.
- **Brain `state.json`** (`kind == "brain"`): regenerates `repos[]`, `cross_repo[]` (cross-repo `depends_on` edges), and the brain file's own `focus`. Authored `tracks[]`, `backlog[]`, and `tiers[]` are left untouched.
  - `repos[]` is **tier-scoped**: a brain file whose `repo` slug matches a `tier` value in `brain.toml` (e.g. `core`) scopes to only that tier's `[[repos]]`; a brain file whose `repo` matches no tier (the HQ root) scopes to every repo. See `tier_scope_for`.
  - `repos[]` is **non-destructive**: for each in-scope repo, if a loadable child `state.json` exists, its headline is derived as before (`RepoRollup.tier` populated from config); if not, but the brain file already carries a `repos[]` entry for that slug, the entry is **preserved verbatim** (with `tier` backfilled); only when neither exists is a tier-tagged empty stub emitted. A malformed or not-yet-authored child `state.json` can therefore never silently drop a repo out of the rollup.
  - `focus.now/next/blocked` is derived as the **repo-tagged union** of the in-scope children's own derived `focus` (each block carries its source `repo`), in config-repo order then within-child order, deduplicated by `(repo, id)`. Repos with no loadable child contribute nothing to `focus` (they still surface in `repos[]` via the preserve/stub branch).
- **`master-plan.md` wave tables**: splices a rendered wave/dependency Markdown table between the `<!-- BEGIN generated:wave-table -->` and `<!-- END generated:wave-table -->` sentinels. All narrative lines outside the sentinels are preserved verbatim. Re-running the emit is idempotent — if the splice produces no change, no `EmitAction` is recorded.
- **Project-cache docs** (`docs/projects/<slug>.md`, one per leaf project repo): splices the derived focus headline into the `<!-- BEGIN generated:project-cache -->` / `<!-- END generated:project-cache -->` sentinels and reconciles the doc's OKF frontmatter `synced_from` field to the child `state.json`'s `updated` watermark. A repo with no matching `[[repos]]` entry, or whose entry has a blank `cache_doc`, is silently skipped (nothing to target).
- **Tier rollup tables** (each tier sub-brain's sibling `status.md`): splices a rendered per-repo now/next/blocked rollup table into the `<!-- BEGIN generated:tier-rollup -->` / `<!-- END generated:tier-rollup -->` sentinels. Only brain files scoped to a single tier (`tier_scope_for` resolves to `TierScope::Tier`) are targeted — the HQ root (`TierScope::All`) is skipped by this planner.
- **HQ Operating Board** (the HQ brain's `status.md`): splices a rendered NOW/NEXT/BLOCKED board across every registered repo into the `<!-- BEGIN generated:hq-board -->` / `<!-- END generated:hq-board -->` sentinels.
- **HQ unified priority board** (the same HQ brain's `status.md`, independent sentinel region): splices a priority-ranked NOW/NEXT/BLOCKED/DUE-SOON board into the `<!-- BEGIN generated:unified-board -->` / `<!-- END generated:unified-board -->` sentinels. Rows are tagged `[BIZ]`/`[ENG]` by the source repo's configured tier; `NEXT` is stably re-sorted by `(effective priority asc, due asc)` (absent values last, wave order as the implicit tiebreak). Effective priority (MV.7.A) is computed by `effective_priorities` via reverse-topological `min`-propagation over the `depends_on` DAG, so a block with no own priority that gates a hotter dependent inherits that dependent's priority and floats to the top instead of sorting last; it falls back to the block's own raw `priority` when no hotter dependent exists. `DUE-SOON` lists blocks due within 14 days (overdue included and annotated) sorted by due date ascending.
- **Attention board** (every brain-level `status.md`, tier-scoped): splices the stale-item board into the `<!-- BEGIN generated:attention -->` / `<!-- END generated:attention -->` sentinels. Unlike the boards above (HQ root only), this emits for **both** scopes: the HQ root (`TierScope::All`) unions `carryover[]` from every loaded repo/tier plus the whole HQ `backlog[]`; each tier sub-brain (`TierScope::Tier`) shows its own tier's leaf-repo carryover (plus the tier brain's own) and the HQ backlog nodes whose `repo` belongs to that tier. Four lanes — Stale carryover · Aging backlog · Orphaned captures · Stale distilled knowledge — each row `[<repo>]`-tagged and sorted oldest-first, showing only items past their `[attention]` threshold (the visible twin of `W_STATE_CARRYOVER_STALE`/`W_STATE_BACKLOG_STALE`/`W_DISTILL_STALE`). The fourth lane (distill-freshness-lane) reads each repo's `knowledge.md`/`memory.md` once (cached across boards) and lists D35-distilled entries whose `distill_stale_age` exceeds the `[attention]` `knowledge_days`/`memory_days` threshold, capped at 10 rows per board with an "…and N more" tail — the same predicate `check_distill_staleness` fires `W_DISTILL_STALE` on, so the board never shows an entry the warning didn't also flag.
- **Portfolio `state.json`** (`kind == "portfolio"`): not regenerated at all (no `focus` to derive — these are terminal repos), and skipped entirely by the wave-table splice pass — no `master-plan.md` is expected, so no `W_EMIT_NO_SENTINEL` is raised for these repos.

All of the project-cache, tier-rollup, HQ-board, and unified-board planners share the same fixed-point and sentinel-safety guarantees as the wave-table splice: a target document missing its sentinel pair produces a `W_EMIT_NO_SENTINEL` warning and is left untouched, and re-running the emit over already-emitted content produces no further `EmitAction`/`I_EMIT_WROTE`.

#### Sentinel contract

Every planner splices into its own named marker, using the same `<!-- BEGIN generated:<marker> --> ... <!-- END generated:<marker> -->` pair format. The wave-table example:

```markdown
<!-- BEGIN generated:wave-table -->
| Wave | Block | Title | Status | Depends on |
| --- | --- | --- | --- | --- |
... (generated rows) ...
<!-- END generated:wave-table -->
```

The other planners use their own markers in the same document types: `project-cache` (leaf `docs/projects/<slug>.md`), `tier-rollup` (tier sub-brain `status.md`), `hq-board` (HQ `status.md`), `unified-board` (the same HQ `status.md`, an independent sentinel region alongside `hq-board`), and `attention` (every brain-level `status.md` — HQ and each tier sub-brain — an independent sentinel region).

- Both `BEGIN` and `END` sentinels must be present and balanced; a missing or unbalanced pair causes a `W_EMIT_NO_SENTINEL` warning and the file is skipped — sentinels are never invented into arbitrary prose.
- Re-splicing an already-emitted table is idempotent.

#### Diagnostic codes

| Locator | Severity | Condition |
|---|---|---|
| `W_EMIT_DRY_RUN` | Warning | Planned action (dry-run only; no file written) |
| `I_EMIT_WROTE` | Warning | File written (`--write` mode) |
| `W_EMIT_NO_SENTINEL` | Warning | A target document is missing its marker's sentinel pair (`wave-table`, `project-cache`, `tier-rollup`, `hq-board`, `unified-board`, or `attention`); file skipped |
| `E_EMIT_WRITE_FAILED` | Error | IO error writing a file; causes exit 1 |
| `E_CONFIG_NOT_FOUND` | Error | `brain.toml` could not be located by walking up from `path`; causes exit 1 |
| `E_EMIT_LINKED_WORKTREE` | Error | `--write` invoked from inside a linked git worktree; causes exit 1 |
| `E_EMIT_INCOMPLETE_CORPUS` | Error | `--write` refused because one or more discovered `state.json` files failed to load; causes exit 1 |
| `E_EMIT_UNKNOWN_SCOPE` | Error | `--scope` names a slug with no matching `[[repos]]` entry in `brain.toml`; the message names every valid slug; causes exit 1 |
| `E_EMIT_LOCK_HELD` | Error | `--write` could not acquire the advisory lock at `<root>/.mev-emit.lock` within the timeout because another live process already holds it (names the holder pid); causes exit 1. A lockfile whose owning process is no longer alive is reclaimed automatically instead of blocking forever. Dry-run never takes the lock. |

`--write` refuses to run when `path` resolves to a linked git worktree (e.g. `trees/<slug>/` under a
repo that already has its own main working tree) — `emit-state` resolves every repo's derived-file
paths from `brain.toml`, not from CWD, so writing from a worktree would silently regenerate the
**main checkout's** files instead of the worktree's own copy. The command prints an error naming
the worktree path and exits non-zero (`E_EMIT_LINKED_WORKTREE`) without writing anything. Dry-run
(no `--write`) is read-only and is never gated — it still succeeds from inside a worktree. Run
`--write` from the repo's main working tree instead.

`--write` also refuses to run when the corpus is incomplete: if any discovered `state.json` fails
to load (an `E_STATE_MALFORMED_JSON` diagnostic), every derived view is a cross-repo union
(`repos[]`/`cross_repo[]`, tier rollups, HQ/unified/epic boards, master-plan and epic sequence
tables) — regenerating them from a partial corpus would silently erase the missing repo(s) from
every surface, and rewriting `cross_repo[]` would delete the dangling references that are the only
evidence of the failure. The command pushes `E_EMIT_INCOMPLETE_CORPUS` alongside the underlying
`E_STATE_MALFORMED_JSON` cause, writes nothing, and exits non-zero. Dry-run is unaffected — it is
the diagnostic tool for exactly this situation, and still runs every planner and reports the
`W_EMIT_DRY_RUN` actions that would have been taken. Fix the load failure named by
`E_STATE_MALFORMED_JSON`, then re-run `--write`.

**Examples:**

```bash
# Dry-run from the current directory (reports planned changes, writes nothing)
mev emit-state

# Dry-run from an explicit brain root
mev emit-state ~/Dev/agentic-portfolio

# Write derived views in place
mev emit-state --write

# Write derived views from an explicit brain root
mev emit-state --write ~/Dev/agentic-portfolio

# Write only mev's own derived surfaces (leaf state.json, cache doc, tier rollup, HQ board)
mev emit-state --scope mev --write

# Machine-readable dry-run output
mev --json emit-state

# Machine-readable write output
mev --json emit-state --write ~/Dev/agentic-portfolio
```

---

### `defer-epic <slug> [--write] [path]` · `resume-epic <slug> [--write] [path]` · `sync-epics [--write] [path]`

Park and un-park a whole initiative, keeping the HQ `epics[]` registry status and
its member blocks' authored statuses in agreement.

An epic is "parked" when its registry `status` is `paused` **and** its unfinished
member blocks are `deferred`. Those two can drift — a `paused` epic whose blocks
are still `open` keeps flooding `focus.next` even though you consider the
initiative shelved. These commands move both together.

| Command | Registry | Member blocks |
|---|---|---|
| `defer-epic <slug>` | → `paused` | `open` → `deferred` |
| `resume-epic <slug>` | → `active` | `deferred` → `open` |
| `sync-epics` | fully-deferred epics → `paused` | stragglers in a `paused` epic → `deferred` |

**`in_progress` blocks are never touched**, in either direction. Parking work you
are mid-block on is far more likely to be a mistake than an intent, so it is left
alone and reported as `W_EPIC_SKIPPED_IN_PROGRESS`. `closed` blocks are likewise
never reopened.

**`sync-epics` never un-defers anything.** An `active` epic with *some* deferred
blocks is a perfectly normal state (you parked two of nine). Un-parking is always
explicit, via `resume-epic`.

**Dry-run by default**, exactly like `emit-state`: without `--write` the proposed
edits print as `W_EMIT_DRY_RUN` and nothing is touched. A successful `--write`
additionally runs `emit-state --write`, so `focus`, the boards and the rollups are
regenerated in the same invocation instead of being left drifted.

**`--write` takes the same advisory lock `emit-state --write` and
`set-block-status --write` take**, at `<root>/.mev-emit.lock`, before any file is
touched — `defer-epic`, `resume-epic`, and `sync-epics` all share one dispatch
function, so one lock acquisition covers all three. If another live process
already holds it, the command fails with `E_EMIT_LOCK_HELD` (naming the holder's
pid) and writes nothing; a lockfile whose owning process is no longer alive is
reclaimed automatically instead of blocking forever. Dry-run (no `--write`) never
takes the lock and is unaffected by contention.

> **These, plus [`set-block-status`](#set-block-status-repoid-status---write-path),
> are the only commands that write *authored* state.** Everything else mev writes is
> derived. The cascade lives behind an explicit command precisely so `emit-state`
> stays safe to run unattended — see `src/brain/epics.rs` for the full rationale.

```bash
# What would parking the TUI initiative change?
mev defer-epic bastion-tui

# Park it (and regenerate every derived view)
mev defer-epic bastion-tui --write

# Bring it back
mev resume-epic bastion-tui --write

# You deferred blocks by hand; make the registry agree
mev sync-epics --write
```

Exit codes: `0` planned/applied successfully · `1` unknown epic slug
(`E_EPIC_UNKNOWN`), no HQ registry (`E_EPIC_NO_REGISTRY`), an unreadable
state.json (`E_EPIC_INCOMPLETE_CORPUS` on `--write`), the advisory lock already
held (`E_EMIT_LOCK_HELD` on `--write`), or a write failure.

---

### `set-block-status <repo:id> <status> [path] [--write]`

Set **one** block's authored `status` in its repo's `planning/state.json`. The
block-level counterpart to the epic commands above: those move a whole initiative,
this moves exactly one block and nothing else.

**Status only.** Not `priority`, not `due`, not a generic `set-block-field`. The
narrow surface keeps the caller's contract precise; a generic setter would push
per-field validation to runtime.

**The key is always `repo:id`** — e.g. `mev:MV.10.A` — the same
`"{repo_slug}:{block_id}"` form `global_status_map` and `effective_priorities` use.
Block ids are only unique *within* a repo, so an unqualified id is **rejected**
rather than guessed at.

| Status | Meaning |
|---|---|
| `open` | not started, a candidate for `next` |
| `in_progress` | actively being worked (derives into `focus.now`) |
| `deferred` | parked on the back burner (derives into `focus.deferred`) |
| `closed` | done |

> **`blocked` is not authorable, and this command rejects it.** `blocked` is a
> *derived* lane: `emit-state` computes it from a block's unmet `depends_on` edges
> and stamps it onto `focus.blocked[]` entries. Writing it onto a `tracks[]` block
> is exactly what `validate-brain`'s `E_STATE_AUTHORED_BLOCKED` exists to catch, so
> input is validated against `VALID_TRACK_BLOCK_STATUSES` (the four above) and not
> against the wider `VALID_STATUSES`. Passing `blocked` fails with
> `E_BLOCK_BAD_STATUS` and writes nothing.

**Setting a block to the status it already has is a no-op success** — zero actions,
zero diagnostics, exit `0`, nothing written. Running the same `--write` twice leaves
the corpus byte-identical.

**Dry-run by default**, exactly like the epic commands: without `--write` the
proposed edit prints as `W_EMIT_DRY_RUN` and not a byte is touched. A successful
`--write` takes the same advisory lock `emit-state --write` takes, refuses to run
against an incomplete corpus (`E_EMIT_INCOMPLETE_CORPUS`), and then runs
`emit-state --write` so `focus`, the boards and the rollups agree with the new
authored value in the same invocation.

**Who calls this.** The intended caller is an **engine-rs workflow node** invoking
the CLI on bastion-web's behalf — "mark this done", "park this" from the web UI.
`bastion serve` is **read-only by decision (D25)** and stays that way, so the write
lands here in mev, the deterministic writer for the brain corpus. The workflow node
itself is engine-rs work and is not part of this command's contract.

```bash
# What would closing MV.10.A change? (dry run — writes nothing)
mev set-block-status mev:MV.10.A closed ~/Dev/agentic-portfolio

# Apply it, and regenerate every derived view
mev set-block-status mev:MV.10.A closed ~/Dev/agentic-portfolio --write

# Park a single block without touching its epic
mev set-block-status bella:BE.2.C deferred --write

# Machine-readable
mev --json set-block-status mev:MV.10.A in_progress --write
```

Exit codes: `0` planned (dry-run), applied, or already at the target status · `1`
any error-severity diagnostic or a write failure.

| Diagnostic | Cause |
|---|---|
| `E_BLOCK_BAD_KEY` | the key is not `repo:id` (a bare block id, or an empty half) |
| `E_BLOCK_BAD_STATUS` | the status is not one of the four authorable values — this is what rejects `blocked` |
| `E_BLOCK_NOT_FOUND` | no loaded `state.json` owns that `repo:id`; the message lists the known repo slugs when the repo half is the problem |
| `E_EMIT_INCOMPLETE_CORPUS` | `--write` attempted while at least one `state.json` failed to load |
| `E_EMIT_LOCK_HELD` | another mev write holds the brain-root advisory lock |

---

### `doc materialize` · `doc opportunity ingest|set-stage|add-action|merge-contacts`

The generic brain-document materializer (Phase 9, Block MV.9.A) — plans (and, with `--write`,
applies) any of okf-core's three `BrainDocModel` implementors (`Opportunity`,
`LearningArtifact`, `Proposal`) from a raw JSON payload, plus the Opportunity-specific
command family for the `business/docs/opportunities/` corpus.

```bash
mev doc materialize --model <opportunity|learning-artifact|proposal> --input <path.json> [path] [--write]
mev doc opportunity ingest --input <path.json> [--kind company|prospecting-sweep|job-posting] [path] [--write]
mev doc opportunity set-stage <slug> <stage> [path] [--write]
mev doc opportunity add-action <slug> --kind <k> --note <n> [--at <ISO date>] [path] [--write]
mev doc opportunity merge-contacts <slug> --input <path.json> [path] [--write]
```

Every verb resolves its target-corpus root via `find_brain_root` from the optional trailing
`path` argument (default `.`), exactly like `emit-state`. **Dry-run is the default on every
verb** — without `--write`, nothing is touched on disk and every planned action is still
reported; `--write` applies the plan.

#### `doc materialize`

| Flag | Default | Description |
|---|---|---|
| `--model` | *(required)* | Which okf-core model to build: `opportunity` \| `learning-artifact` \| `proposal` |
| `--input` | *(required)* | Path to the JSON payload the model is built from |
| `path` | `.` | Path to search from when locating `brain.toml` |
| `--write` | off | Apply the write; without this the command is a dry-run |

`--model opportunity` dispatches through the same shape auto-detection as `doc opportunity
ingest` (`--kind` is not exposed on this generic verb — use `doc opportunity ingest` when you
need to name it explicitly). `--model learning-artifact` builds via
`LearningArtifact::from_payload(input)`; `--model proposal` reads `company_name` and `roadmap`
off `input` and builds via `Proposal::from_automation_roadmap`. Any other `--model` value pushes
`E_DOC_UNKNOWN_MODEL` and plans nothing.

#### `doc opportunity ingest`

| Flag | Default | Description |
|---|---|---|
| `--input` | *(required)* | Path to a `CompanyBrief` / `ProspectingResult` / job-posting JSON payload |
| `--kind` | auto-detect | `company` \| `prospecting-sweep` \| `job-posting`. Omit to auto-detect from the input's shape (`company_name` present → company; `prospects`/`vertical` present → prospecting-sweep; neither → `E_DOC_UNKNOWN_INPUT_SHAPE`, pass `--kind` explicitly) |
| `path` | `.` | Path to search from when locating `brain.toml` |
| `--write` | off | Apply the write; without this the command is a dry-run |

Creates or updates the target Opportunity document (path derived from its `IndexIntent`, under
`business/docs/opportunities/`) and reconciles that directory's `index.md` table in the same
plan. The raw ingested payload is embedded as the first fenced `json` block in the body.

#### `doc opportunity set-stage <slug> <stage>`

Sets an existing Opportunity's `stage` field. `stage` must be one of the values authored in
`business/docs/pipeline.md`'s `## Stages` line — the vocabulary is **read from that file**, not
compiled into `mev`, per [D58](../../../docs/decisions/D58-pipeline-stage-vocabulary-home.md)
(the file's `## Stages` line is the single source both `mev` and bastion's `parse_stages` read).
As documented today that line resolves to the seven values `identified | researching | contacted
| conversation | proposal-sent | closed-won | closed-lost`, but that list can change without a
`mev` release — only `pipeline.md` needs to change. `pipeline.md` is resolved from the brain root
(walked upward from the target document's path), never from CWD. Any `stage` value outside the
resolved vocabulary pushes `E_DOC_BAD_STAGE` and plans nothing. Re-running with the same stage is
a zero-action no-op (`W_DOC_UNCHANGED`).

Resolving the vocabulary itself can fail independently of the `stage` argument's validity — see
`E_DOC_PIPELINE_ROOT_NOT_FOUND`, `E_DOC_PIPELINE_MD_MISSING`, and
`E_DOC_PIPELINE_STAGES_UNPARSEABLE` in the diagnostics table below. Each names the file (or the
search root) and plans nothing; none panics, and none degrades into flagging every stage as
invalid.

#### `doc opportunity add-action <slug>`

| Flag | Default | Description |
|---|---|---|
| `--kind` | *(required)* | The action's kind (e.g. `email`, `call`, `meeting`) |
| `--note` | *(required)* | A free-form note describing the action |
| `--at` | today | The action's ISO date |

Appends one `{at, kind, note}` entry to the opportunity's `actions[]`. An identical triple
already present is not re-appended — a repeat call is a zero-action no-op.

#### `doc opportunity merge-contacts <slug>`

| Flag | Default | Description |
|---|---|---|
| `--input` | *(required)* | Path to a JSON contact object, or a JSON array of contact objects |

Merges contacts into the opportunity's `contacts[]`, matched on `name`: `emails` / `whatsapp` /
`phones` / `links` are unioned (deduped, order-stable), and `role`/`note` are filled only when
the existing value is empty. An already-merged contact is a zero-action no-op.

#### Shared behaviour across every `doc` verb

- **Linked-worktree write guard:** `--write` from inside a linked git worktree is refused with
  the same guard message `emit-state` uses (`doc` resolves derived-file paths from `brain.toml`,
  not CWD, so a worktree write would silently regenerate the main checkout's files instead).
- Every mutator (`set-stage` / `add-action` / `merge-contacts`) requires the target document to
  already exist; a missing target pushes `E_DOC_NOT_FOUND` and plans nothing.
- `--json` wraps the report in the standard `JsonReport` envelope (see below), labelled
  `doc-materialize`, `doc-opportunity-ingest`, `doc-opportunity-set-stage`,
  `doc-opportunity-add-action`, or `doc-opportunity-merge-contacts`.

#### Diagnostic codes

| Locator | Severity | Condition |
|---|---|---|
| `W_DOC_UNCHANGED` | Warning | Computed content already matches the existing file; no action planned |
| `W_DOC_MISSING_SENTINEL` | Warning | A `BodySection::Generated` section's sentinel pair is absent; that section is left untouched rather than clobbered |
| `W_DOC_INDEX_MISSING` | Warning | The target `index.md` is absent; no index action planned (never creates one) |
| `W_DOC_INDEX_NO_TABLE` | Warning | `index.md` has no parsable table; no index action planned |
| `W_DOC_INDEX_COLUMN_MISMATCH` | Warning | The model's `row_cells` count doesn't match the table's column count; no index action planned |
| `E_DOC_BAD_INDEX_PATH` | Error | The model's `IndexIntent.index_path` has no parent directory component |
| `E_DOC_UNKNOWN_INPUT_SHAPE` | Error | `ingest` input matches neither the company nor the prospecting-sweep shape and `--kind` was not given |
| `E_DOC_UNKNOWN_MODEL` | Error | `materialize --model` is not one of `opportunity` \| `learning-artifact` \| `proposal` |
| `E_DOC_BAD_STAGE` | Error | `set-stage`'s `stage` argument is not in the vocabulary parsed from `business/docs/pipeline.md`'s `## Stages` line (D58) |
| `E_DOC_PIPELINE_ROOT_NOT_FOUND` | Error | No brain root (`brain.toml`) could be located above the target document's path, so `business/docs/pipeline.md` cannot be resolved to validate `stage` |
| `E_DOC_PIPELINE_MD_MISSING` | Error | The brain root was found but `business/docs/pipeline.md` does not exist (or cannot be read) there |
| `E_DOC_PIPELINE_STAGES_UNPARSEABLE` | Error | `business/docs/pipeline.md` exists but has no parseable `## Stages` section (missing heading, or no backtick-delimited tokens before the next heading) |
| `E_DOC_NOT_FOUND` | Error | A mutator's target file is absent or unparsable |
| `W_EMIT_DRY_RUN` / `I_EMIT_WROTE` | Warning | Reused unchanged from `apply_plan`'s write half — see `emit-state` above |

Exit codes: `0` planned (dry-run) or applied successfully with no errors · `1` a
resolution/parse/write failure, a linked-worktree write refusal, or any error-severity
diagnostic (`E_DOC_*` / `E_CONFIG_NOT_FOUND`).

**Examples:**

```bash
# Dry-run: what would ingesting this brief produce?
mev doc opportunity ingest --input company-brief.json

# Apply it
mev doc opportunity ingest --input company-brief.json --write

# Explicit kind
mev doc opportunity ingest --input posting.json --kind job-posting --write

# Move an opportunity forward
mev doc opportunity set-stage acme-co contacted --write

# Log an action
mev doc opportunity add-action acme-co --kind email --note "sent intro" --write

# Merge in a new contact
mev doc opportunity merge-contacts acme-co --input contact.json --write

# Materialize a learning-artifact document from a payload
mev doc materialize --model learning-artifact --input lesson-payload.json --write

# Machine-readable dry-run output
mev --json doc opportunity ingest --input company-brief.json
```

---

## Exit codes

| Code | Meaning |
|---|---|
| `0` | All checks passed (zero error-severity diagnostics) |
| `1` | One or more error-severity diagnostics found, or an unrecoverable runtime error |

Warning-severity diagnostics are reported but do not change the exit code.

---

## Human output format

Without `--json`, `mev` prints a single summary line:

```
validated <path>: <N> error(s), <M> warning(s)
```

Diagnostics are not individually printed in human mode yet — use `--json` to get the full list.

---

## JSON output format (`--json`)

The `--json` flag emits a `JsonReport` envelope:

```json
{
  "validator": "brain",
  "root": "/path/to/repo",
  "errors": 2,
  "warnings": 1,
  "diagnostics": [
    {
      "severity": "error",
      "file": "docs/projects/foo.md",
      "locator": "type",
      "message": "required field 'type' is missing or empty"
    }
  ]
}
```

| Field | Type | Description |
|---|---|---|
| `validator` | string | `"brain"` or `"learn-ai"` |
| `root` | string | Display path of the validated root |
| `errors` | number | Count of error-severity diagnostics |
| `warnings` | number | Count of warning-severity diagnostics |
| `diagnostics` | array | All diagnostics emitted during the run |
| `diagnostics[].severity` | `"error"` \| `"warning"` | Severity level |
| `diagnostics[].file` | string | File path (relative to root where possible) |
| `diagnostics[].locator` | string | In-file locator (e.g. `"type"`, `"layer[0]"`) or `""` for whole-file findings |
| `diagnostics[].message` | string | Human-readable description of the finding |
