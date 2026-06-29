---
type: Decision
title: D6 — scope:doc_id Canonical Id Scheme & Corpus Rules
description: Canonical graph-node id scheme (scope:doc_id) via registry-driven stable slugs, corpus membership rules, leaf vs node split, and the two-block delivery plan for Phase 3 Block J.
doc_id: D6-scope-doc-id-namespacing
layer: [brain, factory]
project: mev
status: active
keywords: [doc_id, namespacing, graph integrity, related edges, brain corpus, scope, registry]
related: [D4-corpus-engine-and-knowledge-graph, D5-heterogeneous-format-ingest, knowledge, context]
---

# D6 — `scope:doc_id` Canonical Id Scheme & Corpus Rules

Settled 2026-06-28 from an empirical pass over the live 503-file widened Brain corpus.
This document is the source of truth for the id scheme that `mev validate-brain --graph` validates.
Distilled from `planning/archive/2.J-graph-integrity/namespacing-and-corpus-decision.md`.

---

## Decision: canonical node id = `scope:doc_id`

- **`scope`** = a **registry-driven stable slug** (from `brain.toml` `[[repos]]`). A file's scope
  is the slug of its owning unit, found by **longest-prefix match** of the file path against the
  registry (root unit `repo_path = "."`, slug `brain`, is the fallback). Moving a repo between
  tiers or renaming a tier does **not** change node ids — renames are explicit, validated migrations.
  *Supersedes the earlier "tier-position-derived scope" stance (withdrawn 2026-06-28).*

- **`doc_id`** stays **authored** (frontmatter) and **location-independent**. It must NOT encode
  the path. Moving a file must not change its id or break edges pointing at it.

- **Storage/retrieval never depends on `doc_id`.** Rows key on `file_path` (always unique);
  `doc_id` exists only for the `related:` graph.

### Why `scope:doc_id` (2-part) over alternatives

Measured against the live corpus:

| Scheme | Collisions | Breaks existing `related:` graph | Stable under file moves |
|---|---|---|---|
| **`scope:doc_id`** (chosen) | **0** | No | ✓ |
| `scope:topdir:doc_id` | 11 leaf clashes | Yes (358 files) | ✗ |
| `scope:parentdir:doc_id` | 11 leaf clashes | Yes | ✗ |
| `scope:relpath` (full path) | 0 | Yes | ✗ |

3-part is insufficient (the real differentiator is a grandparent dir, which no single middle
segment captures) **and** location-coupled. 2-part is already collision-free for the graph.

---

## Nodes vs Leaves

- A file **with** an authored `doc_id` is a **graph node**: its `scope:doc_id` must be globally
  unique; it is a legal `related:` target.
- A file **without** a `doc_id` is a **leaf**: embedded for retrieval, but not a graph node and
  not a legal `related:` target. Stem clashes among leaves are harmless (storage keys on
  `file_path`; nothing references them by id).

Corpus baseline at decision time: 384 authored nodes, 119 leaves.

---

## Block J graph checks

1. **Uniqueness** — authored `scope:doc_id` is globally unique (baseline: 0 violations).
2. **Edge integrity** — every `related:` entry resolves to an authored node: bare `doc_id`
   resolves within the *same scope*; `scope:doc_id` resolves cross-scope. Unresolved = error.
3. **Leaf-as-target lint** — a `related:` edge pointing at a file with no `doc_id` is a warning
   ("referenced but not addressable; author a `doc_id`").

---

## Corpus membership rules (what graph checks validate against)

The Brain corpus is uniform across every scope:
**`docs/**` + `planning/**` + root `README.md`/`CLAUDE.md`**, minus `skip_dirs`, minus ephemeral
(`handoff.md`, `_`-prefixed files).

`skip_dirs` (from `brain.toml`, matched as bare components at any depth):
`target, node_modules, .git, .claude, .agent, .agents, .repo-backups, archive, archived,
trees, sdlc, venv, .venv`.

- **`trees/`** — git worktrees: in-flight, unmerged. Never embed.
- **`archive/` + `archived/`** — `/archive` harvests durable residue before exclusion.
- **`sdlc/`** — per-block SDLC working files. Transient. `tasks.md` stays (planned work).

---

## Architecture: mev is a multi-root validator

To validate one global cross-repo graph, mev walks **every registered unit** from the HQ root
via the `brain.toml` registry — not the old single-root, nested-git-pruned walk. This is the
pre-commit/CI integrity gate.

Root instruction files (`CLAUDE.md`, `README.md`) are **always in the corpus**; OKF frontmatter
is **optional** on them. A root file without frontmatter is a searchable leaf; one with a
`doc_id` is a full node. `handoff.md` and `_`-prefixed files remain ephemeral/excluded.

---

## Delivery: two-block split

Work was delivered as:
1. **`2.J-corpus-crawl`** — registry + scope resolver + multi-root corpus crawl (shared foundation)
2. **`2.J-graph-integrity`** — global `scope:doc_id` node index + edge integrity (`--graph` flag)

The split happened because D5 guardrails were added after `2.J-corpus-crawl` had shipped to review.

---

## Edge model (extensible)

Edges are represented as `{ from: canonical_id, to_ref: String, kind }` with a `kind` enum
starting at `Related`. Typed edges (`supersedes`, `depends-on`, `parent`, …) extend this schema
with no reshape. Block J validates `related:` only; further edge types land as later blocks.

---

## Authored-only guarantee (D5 binding)

The graph is built solely from authored/confirmed metadata (frontmatter today; reviewed sidecars
later). mev does **not** infer, propose, or auto-apply nodes or edges. Proposed metadata
(future `mev discover` / AI enrichment, per D5) lands as reviewable artifacts and only becomes
graph input once a human confirms it into authored frontmatter/sidecars. This preserves the
"authored, not inferred" property.
