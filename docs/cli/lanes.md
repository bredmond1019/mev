---
type: Reference
title: mev CLI — graph, lane, and artifact commands
description: The commands that compute what is startable, how work is segmented into lanes, and the graph and consumer artifacts the rest of the fleet reads.
doc_id: cli-lanes
layer: [factory]
project: mev
status: active
keywords: [frontier, lanes, block graph, knowledge graph, consumers]
related: [cli-reference, architecture, cli-state]
---

# mev CLI — graph, lane, and artifact commands

Part of the [CLI reference](../cli.md).

## What this page is for

Blocks depend on other blocks. That makes the corpus a graph, and most scheduling questions are
graph questions: what can start now, what is holding everything else up, which repo's work can run
in parallel with which.

A **lane** is one repo's chain of blocks within a roadmap — the unit an agent session actually
drives. A **segment** is a runnable stretch of a lane. The **frontier** is every block that could
start right now across the whole corpus.

| Command | Answers |
|---|---|
| [`frontier`](#frontier---json-path) | What is startable right now, corpus-wide? |
| [`lanes`](#lanes---json-path) | Which lane segments are available, and what would unblock the most? |
| [`blocks`](#blocks-flags-path) | What's open in this repo/roadmap, what's startable, what does closing this one block free, how deep can I go without leaving this repo? |
| [`emit-block-graph`](#emit-block-graph-flags-path) | The block-dependency graph, as JSON |
| [`emit-graph`](#emit-graph---pretty-path) | The `scope:doc_id` knowledge graph, as JSON |
| [`generate-graph`](#generate-graph---out-path) | The same graph as a browsable HTML page |
| [`check-consumers`](#check-consumers---consumer-slug---json-path) | Does this working tree still compile every repo that depends on it? |
| [`doc`](#doc-materialize--doc-opportunity-ingestset-stageadd-actionmerge-contacts) | Materialize brain documents; manage Opportunity records |

## Quickstart

Run these in a **terminal**. All are read-only except `doc` and `generate-graph`, which write the
artifact you name.

```bash
# What could I start right now?
mev frontier

# Which lane segments are live, and what unblocks the most?
mev lanes

# What's open in mev, ranked by what closing it would free?
mev blocks --repo mev --startable --leverage

# Did my change break a repo that depends on mev?
scripts/check_consumers.sh

# Browsable knowledge graph
mev generate-graph --out graph.html
```

**`check-consumers` has a wrapper, and the wrapper is the one you want.**
[`scripts/check_consumers.sh`](../../scripts/check_consumers.sh) runs the command from source via
`cargo run --release`, applies the waiver list, and prints a coverage line. Running `mev
check-consumers` directly skips all three.

**A green consumer gate means no consumer was *proven* broken** — not that every consumer was
checked. Read the `verified P of N consumers` line, not the exit code.

## Commands

### `frontier [--json] [path]`

Print the corpus-wide lane frontier: one entry per active `(roadmap, lane, segment)`
naming its startable-or-blocked head block — `MV.13.B`, Task 4. Read-only; never writes
`planning/lane-frontier.json` (that write happens only via `mev emit-state --write`,
which runs this same derivation as one of its planners).

```bash
mev frontier [path]
mev frontier --json [path]
```

| Argument / Flag | Default | Description |
|---|---|---|
| `path` | `.` | Path to search from when locating `brain.toml` (walks up to find it) |
| `--json` | off | Emit the `lane-frontier.json` artifact shape (`derived_at`, `entries`, `gate_ranks`) instead of one text line per entry |

#### The consumer contract for HTTP-side closure

**Closure over the block graph MUST run in mev itself, over the untruncated corpus.**
`mev frontier` always builds the in-process graph with `max_nodes: usize::MAX` — never
the HTTP export's truncated default. `mev emit-block-graph` (and bastion's `GET
/api/blocks/graph`, `BA.17.A`) default to `max_nodes=400` against a corpus of ~756
blocks: a client that runs its own closure over that default silently drops gates from
the frontier it computes.

**Any HTTP-side closure — bastion's `/lanes` and concurrency-slot endpoints (`BA.19.C`,
`BA.19.D`) included — MUST send `max_nodes=2000` and hard-fail on `truncated: true`
rather than degrade.** mev cannot gate that half of the contract itself — the evidence
that bastion honours it lives in bastion's own repo, not here. mev's own obligations are
(1) the `ensure_untruncated` refusal, which guarantees `mev frontier`/`mev emit-state`
never computes a frontier over a truncated node set, and (2) this written contract for
every downstream consumer to build against.

#### Text output shape

One line per frontier entry:

```
{roadmap}/{lane}#{segment} {repo}:{id} — startable
{roadmap}/{lane}#{segment} {repo}:{id} — blocked by <reason>[, <reason>...]
```

`<reason>` is each unmet `blocked_by` dependency: `repo:id` for a block dep,
`operator:<slug>` / `approval:<slug>` for a gate, `external:<what>` for an external dep.

#### `--json` output shape

```json
{
  "derived_at": "2026-08-17T13:19:58.661626-03:00",
  "entries": [
    {
      "roadmap": "engine-orchestration",
      "lane": "derive",
      "segment": 0,
      "repo": "mev",
      "key": "mev:MV.13.B",
      "id": "MV.13.B",
      "title": "Frontier computation + gate_rank",
      "status": "in_progress",
      "unmet_blocks": [],
      "unmet_gates": [],
      "startable": true
    }
  ],
  "gate_ranks": [
    {
      "kind": "operator",
      "slug": "operator-fleet-concurrency-live-smoke-test",
      "rank": 1,
      "gates": ["base-template:BT.ticket.heavy-command-signals-rust-build"]
    }
  ]
}
```

`derived_at` is an RFC 3339 timestamp of this run, not of the last `state.json` commit —
lane progress lands live between `/log-work` runs, so a consumer needs this field to
tell how stale the frontier is relative to the corpus it read. `gate_ranks` derives a
rank for operator/approval gates, which are targetless (they gate a block but have no
dependents of their own) and so never receive an `effective_priority` directly: each
gate's rank is the minimum effective priority across every block it gates.

#### Exit codes

| Code | Meaning |
|---|---|
| `0` | Frontier computed and printed |
| `1` | `brain.toml` not found/unreadable, or the in-process graph reported `truncated: true` (should not happen at `max_nodes: usize::MAX`, but this command refuses rather than degrading if it ever does) |

**Examples:**

```bash
# Text frontier from the current directory
mev frontier

# JSON frontier from an explicit brain root
mev frontier --json ~/Dev/agentic-portfolio

# Just the startable heads
mev frontier --json | jq '.entries[] | select(.startable)'
```

---

### `lanes [--json] [path]`

Print six-state lane-segment availability plus lane-level unblock leverage, computed
over the corpus-wide lane frontier — `MV.13.C`, Task 5. Read-only; never writes
`planning/lane-availability.json` (that write happens only via `mev emit-state
--write`, which runs this same derivation as one of its planners).

```bash
mev lanes [path]
mev lanes --json [path]
```

| Argument / Flag | Default | Description |
|---|---|---|
| `path` | `.` | Path to search from when locating `brain.toml` (walks up to find it) |
| `--json` | off | Emit the `lane-availability.json` artifact shape (`derived_at`, `degraded`, `segments`) instead of one text line per segment |

#### The six states

Every lane segment resolves to exactly one of six states, in this fixed precedence
(highest first) — see `docs/architecture.md` for the full rationale:

`done` > `held-block` > `held-operator` > `held-repo-busy` > `held-slot` > `startable`

`held-repo-busy` is derived from exactly one source of lane-liveness truth: the
per-`(repo, roadmap)` orchestration-run record's `lifecycle:` frontmatter
(`planning/orchestration-run/<roadmap>/notes.md`) — never `lane-log.jsonl` or the
`.fleet-locks` fleet-lock registry. `docs/architecture.md` names both rejected
candidates and why.

#### Text output shape

One line per segment:

```
{roadmap}/{lane}#{segment} {repo}:{id} — {availability} (<reason>) frees N lane(s)
```

`{id}` renders as `-` for `done` segments (no live head). The `(<reason>)` clause is
omitted entirely for `startable`/`done`, which need no explanation. `frees N lane(s)`
is always present, including `frees 0 lanes` — the zero case is a real answer, not an
absence.

> **Known caveat — `frees N lane(s)` on a `done` segment is historical, not actionable.**
> A closed segment still reports the lanes it *used to* gate, which are already free, so a
> board that sorts or ranks by `lanes_freed` will float finished work to the top. Filter
> `done` out rather than trusting the number. Tracked as a mev carryover
> (`lanes-freed-nonzero-on-done-segments`); deliberately not folded into
> `MV.ticket.done-segment-discovery`, whose scope was the missing state itself.

#### `--json` output shape

```json
{
  "derived_at": "2026-08-17T17:53:38.409934-03:00",
  "degraded": false,
  "segments": [
    {
      "roadmap": "engine-orchestration",
      "lane": "derive",
      "segment": 0,
      "repo": "mev",
      "head": "mev:MV.13.C",
      "availability": "held-repo-busy",
      "reason": "repo mev is live on carryover-improvements",
      "leverage": {
        "lanes_freed": 1,
        "lanes": ["engine-orchestration/wire"]
      }
    }
  ]
}
```

`derived_at` is an RFC 3339 timestamp of this run, same rationale as `mev frontier`'s
field of the same name. `degraded` is `true` when the fleet-lock read that feeds
`held-slot` could not run (`.fleet-locks` missing or unreadable) — "unknown", never a
hold; a consumer can use it to tell a corpus with zero live `held-slot` holds apart
from one that could not check. Each segment's `leverage.lanes_freed` counts distinct
`(roadmap, lane)` pairs downstream of this segment — a lane-scoped metric, distinct
from the block-graph export's block-scoped `dependent_count`.

#### Exit codes

| Code | Meaning |
|---|---|
| `0` | Availability computed and printed |
| `1` | `brain.toml` not found/unreadable, or the in-process graph reported `truncated: true` (should not happen at `max_nodes: usize::MAX`, but this command refuses rather than degrading if it ever does) |

**Examples:**

```bash
# Text availability from the current directory
mev lanes

# JSON availability from an explicit brain root
mev lanes --json ~/Dev/agentic-portfolio

# Just the startable segments with nonzero leverage
mev lanes --json | jq '.segments[] | select(.availability == "startable" and .leverage.lanes_freed > 0)'
```

---

### `blocks [flags] [path]`

Filtered block queries, the transitive leverage cone, and the same-repo chain —
`MV.ticket.query-verb-leverage-chain-and-filters`. Answers the ad-hoc questions an operator
actually asks (what is open in this repo, in this roadmap, startable, above this priority) plus
two derivations no other verb computes: the **transitive** downstream cone of a block (what
closing it frees, live vs. parked) and the longest run of blocks reachable **without leaving one
repo**. Read-only; writes nothing.

```bash
mev blocks [--repo <SLUG>] [--roadmap <SLUG>] [--startable] [--blocked]
           [--max-priority <N>] [--leverage] [--chain] [--limit <N>] [--json] [path]
```

| Argument / Flag | Default | Description |
|---|---|---|
| `path` | `.` | Path to search from when locating `brain.toml` (walks up to find it) |
| `--repo <SLUG>` | unset | Narrow to one repo slug. **Filters on its own** — see the callout below |
| `--roadmap <SLUG>` | unset | Narrow to one roadmap slug — see the attribution rules below |
| `--startable` | off | Narrow to blocks that are currently startable (no unmet block/gate deps). Mutually exclusive with `--blocked` |
| `--blocked` | off | Narrow to blocks that are currently blocked — the inverse of `--startable`, not a status filter (`"blocked"` is a derived lane, never an authored status). Mutually exclusive with `--startable` |
| `--max-priority <N>` | unset | Narrow to blocks whose effective priority is `<= N` (inclusive). A block with no resolvable priority never matches |
| `--leverage` | off | Report each selected startable block's transitive downstream cone (live/parked), sorted by live cone size descending. Mutually exclusive with `--chain` |
| `--chain` | off | Report each selected startable block's longest same-repo run. Mutually exclusive with `--leverage` |
| `--limit <N>` | unset | Cap the number of blocks printed/serialized, applied after any `--leverage` sort |
| `--json` | off | Emit this verb's own `QueryReport` shape instead of one text line per block |

#### `--repo` filters on its own

**Unlike `emit-block-graph`**, where a bare `--repo` without `--scope repo` is silently ignored
and the whole corpus comes back looking like a filtered result (see
[`emit-block-graph`](#emit-block-graph-flags-path) above), `--repo` on `blocks` always narrows the
result set — there is no `--scope` flag to forget. If you are used to `emit-block-graph`'s habit
of needing a second flag, that habit does not apply here: `mev blocks --repo mev` alone returns
only `mev`'s blocks.

#### Roadmap attribution

`--roadmap` is resolved via `brain::lane_segments`, which carries D57's roadmap-membership rules,
and matches on exactly one of two attributions per block:

1. **`origin_roadmap`** — the roadmap that created the block. This is the default: when a block
   has an `origin_roadmap`, `--roadmap` matches against it.
2. **The scheduled roadmap** — the roadmap a block is currently scheduled under. Used only as a
   fallback, for blocks with no declared `origin_roadmap`.

A `--roadmap` filter never falls back to "match everything" when the membership index has no
entry for a slug — an unrecognized or empty roadmap matches nothing, not the whole corpus (the
same "silence must mean zero, not 'couldn't check'" discipline as `lanes`' `degraded` flag).

#### The leverage cone: live vs. parked

`--leverage` walks the **transitive** downstream closure of each selected startable block —
everything that (directly or indirectly) depends on it, however many hops away — and splits the
result into `live` and `parked` members. Parked statuses (`deferred`, `wontfix`, `closed`) are
reported but **never counted**: the ordering ranks by live cone size only.

This distinction is load-bearing, not cosmetic. A cone of 11 blocks that is entirely parked frees
nothing pickup-able right now — ranking it above a smaller, all-live cone would send an operator
at exactly the wrong block. (Measured against the live corpus 2026-08-29: `dependent_count` on
`emit-block-graph` counts direct dependents only and cannot answer this at all — a block with a
`dependent_count` of 2 had a real transitive cone of 11 blocks across three repos.)

`--chain` computes a different derivation: the longest run of blocks reachable from a startable
head **without ever crossing a repo boundary** — a same-repo dependent extends the chain, a
cross-repo dependent does not, and a parked block never extends it either. Both the cone walk and
the chain walk terminate on a dependency cycle rather than hanging.

`--leverage` and `--chain` are mutually exclusive — pick one derivation per invocation.

#### Text output shape

One line per selected block, plus (with `--leverage` or `--chain`) the derivation's result on the
following indented line:

```
mev:MV.ticket.some-block
  leverage: 3 live, 8 parked
```

or, with `--chain`:

```
mev:MV.ticket.some-block
  chain: mev:MV.ticket.some-block -> mev:MV.ticket.next -> mev:MV.ticket.next-next
```

#### `--json` output shape

This verb's own report type — `QueryReport` — not `BlockGraphNode`/`BlockGraphExport`; neither of
those shared types gained a field for this verb:

```json
{
  "blocks": ["mev:MV.ticket.some-block"],
  "cones": {
    "mev:MV.ticket.some-block": {
      "live": ["mev:MV.ticket.downstream-a", "mev:MV.ticket.downstream-b"],
      "parked": ["mev:MV.ticket.parked-c"]
    }
  },
  "chains": {}
}
```

`cones` is populated only under `--leverage`; `chains` only under `--chain`. Both are empty
objects when their flag is not given.

#### Exit codes

| Code | Meaning |
|---|---|
| `0` | Query computed and printed |
| `1` | `brain.toml` not found/unreadable, `--startable` combined with `--blocked`, or `--leverage` combined with `--chain` |

**Examples:**

```bash
# What's open in mev?
mev blocks --repo mev

# What's startable in mev, ranked by what closing each one frees?
mev blocks --repo mev --startable --leverage

# How deep can I go in mev without switching repos?
mev blocks --repo mev --startable --chain

# Everything startable at priority 0 or 1, corpus-wide
mev blocks --startable --max-priority 1 --json
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
      "last_touched": null,
      "reconcile_failed": null
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
| `reconcile_failed` | Derived from the **same winning state file** as `last_touched` (never a different file or folder for the same block) — `true` when that file's run-state `status` is `"reconcile_failed"`, `false` when a run was found and its status was something else, and `null`/absent when no run state exists at all for the block (same absence-means-"never worked" honesty rule as `last_touched`; the field is `#[serde(skip_serializing_if)]`, so it does not appear in the JSON for a block with no run). This is the **run-state** `status` field inside `sdlc-task-state.json` (and its `-flow-`/`-run-`/plain sibling kinds) — a completely different vocabulary from the block's own **authored** `status` in `state.json` (`open`/`in_progress`/`deferred`/`closed`/`wontfix`). A `reconcile_failed` run never changes the authored status and never changes `lane` derivation (lane reads only the authored `state.json` status). The terminal run-state vocabulary itself — what `reconcile_failed` and its siblings mean, and what a consumer must not fold them into — is pinned at base-template's `docs/data-contract.md` (`doc_id: sdlc-run-state-data-contract`); this field does not re-derive that vocabulary, only surfaces it. The human-readable sibling of this JSON field is [`src/brain/emit.rs`](../../src/brain/emit.rs), which annotates a `BlockGraphExport`'s per-block text lines with `" (reconcile_failed)"` when this field is `true`, and renders byte-identical output to before the annotation existed when no block has it set |

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

### `check-consumers [--consumer <slug>] [--json] [path]`

Compiles every path-dependent consumer's **test targets** against the working mev and reports
the true outcome per consumer (`ticket-consumer-compile-gate`). This is the expensive, rare
counterpart to `mev conformance --check consumer-dependency-parity` (below) — that one catches a
stale lockfile cheaply and constantly; this one catches a genuine type/API break expensively and
rarely. **Neither covers the other's failure class**; do not assume a clean run of one implies
the other.

Consumers are discovered the same way `consumer-dependency-parity` discovers them
(`discover_mev_consumers` — path dependencies on mev declared under `[dependencies]`,
`[dev-dependencies]`, `[build-dependencies]`, or `[workspace.dependencies]` in each repo listed
under `brain.toml`'s `[[repos]]`). There is deliberately only one discovery implementation; a
second one would fail this ticket's acceptance criteria outright.

```bash
mev check-consumers
mev check-consumers --consumer bastion
mev check-consumers --json
```

| Argument / Flag | Default | Description |
|---|---|---|
| `path` | `.` | Path to search from when locating `brain.toml` (walks up to find it) |
| `--consumer <slug>` | unset | Run exactly one discovered consumer by slug instead of every consumer. An unknown slug is a hard error |
| `--json` | off | Emit the per-consumer `ConsumerResult` list as compact JSON instead of a human, per-consumer summary |

#### The command, and why every flag on it is load-bearing

For each discovered consumer, `check-consumers` spawns exactly:

```bash
CARGO_TARGET_DIR=<fresh temp dir> CARGO_TERM_COLOR=never cargo nextest run --no-run --locked --manifest-path <consumer>/Cargo.toml
```

- **`--no-run`** compiles the test targets without executing them, and is the entire reason this
  command exists as a separate, expensive check rather than folding into `cargo build`. The break
  class it exists to catch lives only in test-fixture code — struct literals and call sites that
  only test code constructs. A compile-only build of the consumer's binary sails straight past
  them; `cargo build` cannot see this class of break at all.
- **`--locked`** refuses to let cargo silently rewrite the consumer's `Cargo.lock`. mev does not
  own that repo's lockfile, and a tool that mutates a repo it's only checking is a much worse
  failure than a false negative. This has been observed to happen for real during manual
  verification with a raw (non-`--locked`) invocation — see the ticket's own Notes.
- **A fresh `CARGO_TARGET_DIR`** (a new temp dir per run) avoids `target/` lock contention and
  incremental-cache churn against a consumer repo that may have its own build or CI lane running
  concurrently. It costs a cold compile every time; that's the accepted price of never
  interfering with another lane's build.
- **`CARGO_TERM_COLOR=never`** forces plain output. A CI runner that presents a pseudo-tty to
  spawned subprocesses makes rustc auto-detect color support and wrap `error[E....]` diagnostics
  in ANSI escapes, which the stderr-signature match in `extract_compiler_errors` cannot see
  through — observed 2026-08-15 as a genuinely `Broken` consumer classified `NotEvaluable` on
  mev's own hosted CI while passing locally. `extract_compiler_errors` also strips any ANSI it
  does receive as defense in depth, but forcing color off at the source is the real fix.

A future "simplification" that drops any one of these three restores exactly the failure mode it
exists to prevent — this section exists so that trade-off is written down, not just implied by a
command flag.

#### The five outcomes

| Outcome | Meaning | Fails the run? | Operator action |
|---|---|---|---|
| `pass` | The consumer's test targets compiled clean against the working mev | No | Nothing to do |
| `broken` | A genuine type/API break — compiler diagnostics with their site (e.g. `E0063 at src/serve/handlers/board.rs:660:9`) | **Yes — the only outcome that fails the run** | Fix the named sites in that consumer repo. mev never fixes another repo; a break is that repo's to repair |
| `lockfile-stale` | The consumer's `Cargo.lock` is out of date relative to its `Cargo.toml` (cargo's `cannot update the lock file` signature under `--locked`) — bookkeeping, not a code break | No | Refresh that consumer's lockfile (its change, not mev's) |
| `skipped-dirty` | `git status --porcelain` was non-empty for that consumer — its compile result is not evidence about mev's change either way | No | Commit or stash there, then re-run |
| `not-evaluable` | The failure didn't match a known signature, or an input couldn't be gathered at all (e.g. the lockfile moved despite `--locked`) | No | Reported with a `reason`; investigate manually rather than trusting an automatic verdict |

**`broken` and `lockfile-stale` are deliberately distinct outcomes with distinct exit
behaviour.** Collapsing a stale lockfile into `broken` is exactly the failure mode that made an
earlier, naive version of this gate untrustworthy: engine-rs's lockfile-stale exit code (102) has
nothing to do with mev's own compile correctness, and treating it as a red build trains everyone
to ignore red builds.

The consumer's `Cargo.lock` is verified byte-identical before and after every run — `mev
check-consumers` reports on a consumer, it never mutates one.

#### Exit codes

| Code | Meaning |
|---|---|
| `0` | Every consumer reported `pass`, `lockfile-stale`, `skipped-dirty`, or `not-evaluable` |
| `1` | At least one consumer reported `broken`, `brain.toml` was not found/unreadable, or `--consumer` named a slug that is not a discovered consumer |

#### Why this is a post-merge gate, not a per-task check

`check-consumers` is deliberately **not** wired into `planning/harness.json`'s
`validation.checks[]` — a cold consumer compile (bastion alone measured at ~1 minute) is too
expensive to pay at every task and every review inside the SDLC loop. It is instead wired as
stage 3 of the HQ-level `hooks/pre-push` (mev-repo-scoped, blocking only on `broken`,
skipping — never blocking — when the installed `mev` predates this subcommand or no
`brain.toml` is discoverable), which runs once per push after the work in a branch is done. See
`ticket-consumer-compile-gate`'s spec Notes for the full wiring rationale.

**The three historical breaks that motivate this check** — every one invisible to a plain
`cargo build`, because the break lived only in test-fixture code:

| Change | Damage |
|---|---|
| `okf-core:OK.3.B` added a non-`Option` field to six shared structs | 101 sites broke in mev, 31 in bastion |
| mev's D58 removed a public constant | broke engine-rs's workspace compile |
| `MV.ticket.reconcile-failed-consumer` changed a public return type + added a field | 2 sites broke in bastion (`board.rs:660`, `block_graph.rs:414`), both in test fixtures |

```bash
# Human, per-consumer summary of the whole fleet
mev check-consumers

# Just one consumer
mev check-consumers --consumer bastion

# Machine-readable JSON envelope for CI/tooling
mev check-consumers --json
```

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
compiled into `mev`, per `core/docs/decisions/D58-pipeline-stage-vocabulary-home.md`
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

