---
type: Handoff
created: 2026-06-28
---

# Handoff — Block J reshaped into a global knowledge graph (corpus-crawl → graph); specs ready

> **For the next agent:** Read this after `/prime`. Delete this file once consumed.

---

## What we're doing and why

`mev` is the validation engine for the Bastion Brain. Phase 2 (OKF schema) + Block N (`--sync`
watermark) are **done and merged**. The active work is **Phase 3 graph integrity** — and this session
substantially reshaped it through a design discussion with Brandon.

The headline: **Block J is no longer a small dangling-link checker. It is the integrity engine for a
global, cross-repo knowledge graph** that has to stay foolproof as the Brain scales and gets reused for
client knowledge bases. The settled architecture lives in
`planning/2.J-graph-integrity/namespacing-and-corpus-decision.md` (read it first — esp. the **2026-06-28
Update** section) and in the two specs below.

### Settled design decisions (do not relitigate — confirmed by Brandon this session)

1. **Global graph, not per-repo.** One cross-repo `related:` graph. Brandon explicitly wants rich
   knowledge graphs, built "as sophisticated as we can / with room to grow," reusable for clients.
2. **Canonical node id = `scope:doc_id`.** `scope` = a **registry-driven stable slug** from `brain.toml`
   (HQ, each tier sub-brain, each repo), resolved by **longest-prefix** over the registry — **never**
   inferred from tier/path position (so reorgs/renames don't break edges). `doc_id` stays authored +
   location-independent.
3. **mev is a multi-root validator** — walks every registered unit from HQ via `brain.toml`.
4. **Edge model built to grow** — generic `Edge { from, to_ref, kind }`; Block J validates `related:`
   (kind `Related`) only; typed edges (`supersedes`/`depends-on`/`parent`) extend it later.
5. **Corpus = `planning/**` + `docs/**` + root `README`/`CLAUDE`**, across all registered repos, minus
   bloat (`sdlc`/`archive`/`archived`/`trees`/`target`/…) and ephemeral (`handoff.md`, `_`-prefixed).
6. **Root files: OKF frontmatter is *optional*.** `CLAUDE.md`/`README.md` are always embedded; without a
   `doc_id` they are searchable **leaves**, with one they are **nodes**. No backfill. (We considered
   "require OKF" and withdrew it — see decision doc point 4.)
7. **Single corpus definition** — mev owns the canonical corpus crawl; `index_brain.py` (embedder)
   should *consume* mev's file-list rather than re-implement it. (Orchestrator alignment, tracked
   separately.)

---

## Completed this session

- **Block N confirmed done/merged** (it had already shipped in `74a1c05` / PR #2). Cleaned up a stale
  handoff and a redundant regenerated spec; restored Block N's real "Done" status.
- **Block J split into two specs + reshaped**, all committed on `main`:
  - `planning/2.J-corpus-crawl/tasks.md` — **NEW foundation block** (runs first): scope-unit registry +
    `scope_for` resolver (`src/brain/scope.rs`), multi-root `crawl_corpus` (`src/brain/crawl.rs`),
    wire into `BrainValidator` + OKF-exemption for root files. 5 tasks.
  - `planning/2.J-graph-integrity/tasks.md` — **REWORKED**: global `scope:doc_id` node index +
    extensible edge model + uniqueness + `related:` resolution (bare = same scope, qualified =
    cross-scope) + leaf-as-target lint, surfaced via `--graph`. 5 tasks. Depends on 2.J-corpus-crawl.
  - `namespacing-and-corpus-decision.md` — appended the 2026-06-28 refinements (append-only).
  - `planning/master-plan.md` — inserted Block J-crawl; reshaped Block J. `planning/index.md` updated.
- The previously-running `/sdlc-flow 2.J-graph-integrity` worktree was removed (it had only the init
  commit; nothing lost). **The old worktree is gone — start fresh.**

---

## Remaining work (in order)

1. **REVIEW `portfolio/workflow-engine-rs/services/knowledge_graph/` first** (see next section) — it may
   change how we build the graph layer. Do this *before* running the specs.
2. **`/sdlc-flow 2.J-corpus-crawl`** — the foundation (registry + scope + multi-root crawl). Must land
   before the graph block.
3. **`/sdlc-flow 2.J-graph-integrity`** — the global `scope:doc_id` graph checks on top.
4. **Companion work (not mev code, flag to Brandon):**
   - brain repo: register tier sub-brains as scope units in `brain.toml`; switch `skip_dirs` to the
     bare-component bloat list (`archive`/`archived`/`trees`/`sdlc`/…).
   - orchestrator: have `index_brain.py` consume mev's corpus file-list (single source of truth).
   - per-repo: optionally add OKF frontmatter to any `CLAUDE.md`/`README.md` you want addressable as
     graph nodes (optional — leaves are fine).

---

## ⭐ Priority review item — the existing `knowledge_graph` service

**Before building Block J, review `/Users/brandon/Dev/agentic-portfolio/portfolio/workflow-engine-rs/services/knowledge_graph/`
and figure out how it fits what we're doing here.** It is an existing, real Rust knowledge-graph
service and may overlap with — or become the home of — the graph layer we're specifying.

What it is (from its README): a standalone Rust service managing concept relationships + learning paths
over a **Dgraph** backend, with:
- **Graph algorithms** (`src/algorithms/`): A*, Dijkstra (min-heap), topological sort, ranking, traversal.
- **Dgraph client** (`src/client/`): connection pooling, query/mutation parsing.
- **APIs**: async-graphql `/graphql` + REST (`/api/v1/search`, `/concept/:id`, `/learning-path`,
  `/related/:id`).
- `dgraph/schema.graphql`, `test-data/`, integration tests.

**Questions to resolve:**
- Is this the **query/retrieval layer** over the same graph mev *validates*? (mev = integrity gate that
  the graph is well-formed; knowledge_graph = stores/queries/traverses it.) If so, do the **node-id and
  edge models need to agree** — does `knowledge_graph` key nodes the same way our `scope:doc_id` scheme
  does? Should mev's `Edge { from, to_ref, kind }` map onto its Dgraph schema?
- Does it change **where** the graph lives — Dgraph vs deriving the graph from frontmatter at validate
  time? mev's Block J only needs the in-memory graph to *validate*; persistence/traversal could be this
  service's job. Avoid building a second, divergent graph model.
- Is it portfolio-only demo code, or substrate to harvest (like `workflow-engine-rs`/`claude-sdk-rs`
  are harvested into Bastion)? That determines whether we align to it or just borrow ideas.

Net: confirm the **division of labor** (mev validates; knowledge_graph stores/queries) and the **shared
contract** (node id + edge shape) before committing Block J's edge model, so the two don't diverge.

---

## Context the next agent needs

- **Branch:** `main` (clean). Remote `origin` (`bredmond1019/mev`, private). One worktree (main only).
- **Tests:** ~196 green pre-Block-J. Harness: `cargo fmt --check && cargo clippy -- -D warnings &&
  cargo test && cargo build --release`.
- **Key source:** `src/brain/{config.rs (registry: RepoEntry slug+repo_path), okf.rs, crawl.rs,
  sync.rs, mod.rs}`, `src/lib.rs` (`validate_brain`, `validate_brain_sync`; add `validate_brain_graph`),
  `src/main.rs` (`--sync` is the sibling pattern for `--graph`).
- **brain.toml** lives at HQ root (`~/Dev/agentic-portfolio/brain.toml`) — currently lists 9 repos, no
  tier units yet (that registration is companion work). mev reads whatever the registry declares.

---

## First command after `/prime`

```
# 1. Review the existing graph service and report how it fits (do this first):
#    portfolio/workflow-engine-rs/services/knowledge_graph/
# 2. Then run the foundation block:
/sdlc-flow 2.J-corpus-crawl
# 3. Then the graph block:
/sdlc-flow 2.J-graph-integrity
```
