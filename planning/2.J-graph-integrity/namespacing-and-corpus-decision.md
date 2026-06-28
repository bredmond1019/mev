---
type: Decision
title: Block J — doc_id Namespacing & Brain Corpus Decision
description: Canonical graph-node id scheme (scope:doc_id) and the widened-corpus rules that Block J graph-integrity checks must enforce.
doc_id: block-j-namespacing-decision
layer: [brain, console]
project: mev
status: active
keywords: [doc_id, namespacing, graph integrity, related edges, brain corpus, scope]
related: [context]
---

# Block J — doc_id Namespacing & Corpus Decision

Reference for the graph-integrity work. Settled 2026-06-28 from an empirical pass over
the live 503-file widened Brain corpus. **This doc is the source of truth for the id
scheme Block J validates.**

## Why this matters to Block J

Block J adds graph-integrity checks (dangling/duplicate `related:` edges). Those checks
need a **stable, unambiguous node-id space**. The Brain corpus is being widened to include
sub-repo docs (orchestrator, bastion, mev, …), which introduces cross-repo `doc_id`
clashes (every repo has `planning/knowledge.md` → `doc_id: knowledge`, etc.). The id
scheme below resolves that.

## Decision: canonical node id = `scope:doc_id`

- **`scope`** comes from `brain.toml` (authoritative; never hand-typed). It is the repo
  `slug` for sub-repo files, the tier name (`core`/`portfolio`/`side`/`client`) for tier
  files, and `brain` for HQ-root files.
- **`doc_id`** stays **authored** (frontmatter) and **location-independent** — it must NOT
  encode the path. Moving a file (e.g. `planning/plans/x.md` → `planning/x/plan.md`) must
  not change its id or break edges pointing at it.
- **Storage/retrieval never depends on `doc_id`.** Rows key on `file_path` (always unique);
  `doc_id` exists only for the `related:` graph.

### Why not 3-part (`scope:area:doc_id`) or full-path

Measured against the live corpus:

| Scheme | Collisions (authored graph nodes) | Breaks existing `related:` graph | Stable under file moves |
|---|---|---|---|
| **`scope:doc_id`** (chosen) | **0** | No | ✅ |
| `scope:topdir:doc_id` | n/a — still 11 leaf clashes | Yes (358 files) | ❌ |
| `scope:parentdir:doc_id` | n/a — still 11 leaf clashes | Yes | ❌ |
| `scope:relpath` (full path) | 0 | Yes | ❌ |

3-part is **insufficient** (the real differentiator is a grandparent dir like
`2.G-brain-crawl`, which no single middle segment captures) **and** location-coupled
(breaks edges on the very file moves we expect). 2-part is already collision-free for the
graph. Human-readable breadcrumbs (`amistad · planning · <title>`) are rendered at the
retrieval layer from the `project` + `file_path` columns — not baked into the key.

## Leaves vs nodes

- A file **with** an authored `doc_id` is a **graph node** — its `scope:doc_id` must be
  globally unique, and it is a legal `related:` target.
- A file **without** a `doc_id` is a **leaf** — embedded for retrieval, but **not** a graph
  node and **not** a legal `related:` target. Stem clashes among leaves are harmless
  (storage keys on `file_path`; nothing references them by id).

Today: 384 authored nodes, 119 leaves (tasks.md, sdlc working files, a few bare
`index.md`). The `sdlc/` working dirs are excluded from the corpus entirely (see below),
so most leaves disappear at the source.

## Block J checks to implement

1. **Uniqueness** — authored `scope:doc_id` is globally unique. *Baseline: 0 violations.*
2. **Edge integrity** — every `related:` entry resolves to an authored node:
   **bare `doc_id`** resolves within the *same scope*; **`scope:doc_id`** resolves across
   scopes. Unresolved/typo'd edge = error. (Existing edges are all intra-scope bare → keep
   working unchanged; cross-scope edges are new and opt-in — currently none.)
3. **Leaf-as-target lint (optional)** — a `related:` edge pointing at a file that has no
   `doc_id` is flagged ("referenced but not addressable; author a `doc_id`").
4. **Optional polish** — 5 `index.md` files in amistad/price-scout lack `doc_id` (fall back
   to stem `index`). Author ids if they should be addressable nodes.

## Corpus rules Block J validates against

The Brain corpus (what `index_brain.py` embeds and what graph checks crawl) is uniform
across every scope: **`docs/` + `planning/` subtrees + root `README.md`/`CLAUDE.md`**,
minus `skip_dirs`, minus ephemeral (`handoff.md`, `_`-prefixed).

`skip_dirs` (brain.toml) now excludes as **bare components at any depth**:
`target, node_modules, .git, .claude, .agent, .agents, .repo-backups, archive, archived,
trees, sdlc, venv, .venv`.

- **`trees/`** — git worktrees: in-flight, unmerged. Never embed.
- **`archive/` + `archived/`** — `/archive` harvests durable residue into
  `knowledge.md`/`memory.md` first, then the archive is invisible to the corpus.
- **`sdlc/`** — per-block SDLC working files (worklog, `reports/*`). Transient process
  output; `/archive` harvests anything durable. `plan.md` + `tasks.md` are kept (they are
  "planned work").

Net live corpus: ~483 files (was 130 — HQ + tiers only).

---

## Update — 2026-06-28 (architecture refinements)

Settled in a design pass focused on making the graph **foolproof and reusable for larger
client knowledge bases**. These refine (and in two places supersede) the rules above; the
`scope:doc_id` core is unchanged.

1. **`scope` = a registry-driven *stable slug*, never inferred from path/tier position
   (supersedes "tier name for tier files").** Every scope-owning unit — HQ, each *tier
   sub-brain*, and each *repo* — is registered in `brain.toml` with an immutable `slug`. A
   file's scope is the slug of its owning unit, found by **longest-prefix match** of the file
   path against the registry (root unit `.` = `brain` is the fallback). Moving a repo between
   tiers or renaming a tier therefore does **not** change node ids; renames become explicit,
   validated migrations. *Why:* tier-position-derived scope silently re-ids docs on any
   reorg and breaks inbound edges — unacceptable for a graph meant to scale and ship to clients.

2. **mev is a *multi-root* validator.** To validate one global cross-repo graph, mev walks
   **every registered unit** from the HQ root via the `brain.toml` registry (not the old
   single-root, nested-git-pruned walk). This is the pre-commit/CI integrity gate.

3. **Edge model is built to grow.** Block J validates `related:` first, but the graph module
   represents an edge generically (`from-node`, `to-ref`, `kind`) so typed edges
   (`supersedes`, `depends-on`, `parent`, …) drop in as a later block with no refactor.

4. **Root instruction files require OKF frontmatter (supersedes "CLAUDE.md is a leaf").**
   `CLAUDE.md` / `README.md` are embedded **and** carry OKF frontmatter, so they validate as
   full nodes. mev stops file-blocklisting `CLAUDE.md`. `handoff.md` and `_`-prefixed files
   stay ephemeral/excluded. *Consequence:* `validate-brain` reports errors until frontmatter
   is backfilled onto every repo's `CLAUDE.md`/`README.md` (companion content work).

5. **Single corpus definition.** mev owns the canonical corpus crawl (the file-list); the
   embedder (`index_brain.py`) should *consume* that list rather than re-implement the rules,
   so "what's validated" == "what's embedded" by construction. (Alignment with orchestrator —
   tracked separately, not built in Block J.)

6. **Block split.** The work is delivered as two blocks: **`2.J-corpus-crawl`** (registry +
   scope resolver + multi-root corpus crawl — the shared foundation) lands first, then
   **`2.J-graph-integrity`** (global `scope:doc_id` node index + edge integrity) builds on it.
