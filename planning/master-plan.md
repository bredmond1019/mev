---
type: Plan
title: mev Master Plan
description: Strategic roadmap and phase specifications for mev — a general Markdown validator with two consumers (learn-ai content and Bastion Brain OKF frontmatter).
doc_id: master-plan
layer: [factory, brain, meta]
project: mev
status: active
keywords: [roadmap, phases, ContentValidator, Brain OKF, learn-ai content, mev]
related: [D2-scope-and-sequence, D1-initial-okf, D4-corpus-engine-and-knowledge-graph, status, context]
---

# mev — Master Plan

*Living document. Created 2026-06-18. Reframed 2026-06-26 from a learn-ai-only validator to a
general Markdown validator with two consumers (see "The Reframe" below).*

## The Goal, Stated Plainly

A Rust CLI tool (`mev`) that parses and validates Markdown/MDX against a pluggable set of content
schemas. It has **two consumers today**, behind one shared validation core:

1. **learn-ai content** — frontmatter validation, JSON struct validation, link checking, and (later)
   code-block linting and watch-mode hot-reload for learn-agentic-ai.com lessons.
2. **Bastion Brain OKF** — validate the OKF YAML frontmatter on every `.md` file across the company-brain
   repo before it lands in the RAG indexer.

"Ready" for each consumer is its own checkpoint: for learn-ai, a pass that is a *superset of*
`learn-ai/scripts/validate-content.ts` plus the cross-file integrity checks the TS script can't do; for
the Brain, a `mev validate-brain` that runs green as a pre-flight gate before a `--rebuild` of the Brain
RAG index.

## The Reframe (2026-06-26)

`mev` was originally scoped to validate learn-ai content only. The Bastion Brain now requires that every
`.md` file carry correct **OKF frontmatter** (governed by brain decision D27; schema in the brain's
`docs/okf-frontmatter.md`) before it is indexed by the Python orchestrator's `index_brain.py`. Docs with
missing or malformed frontmatter index poorly or not at all, so we want a pre-index gate.

The decision (after reviewing the codebase) was **extend, not rebuild**: the repo and binary are already
generically named, the generic core (`Diagnostic`/`Report`/`Severity`, `extract_frontmatter`,
`is_kebab_case`, the `clap` shell, the temp-dir fixture harness) is cleanly isolated, and the learn-ai
coupling is quarantined to `crawl.rs::classify()` and the `ModuleMeta`/`PathMeta`/`MdxFrontmatter`
validators. A rebuild would re-derive tested infrastructure to escape coupling that is already isolated.
So the Brain validator slots in as a **parallel crawl + validator behind a new `ContentValidator` trait**
— not a retrofit of the learn-ai classifier. The concrete trigger is **Block H of the brain's
`brain-rag-improvements` plan** — the first live RAG `--rebuild` against Mac Mini Postgres — which wants
`mev validate-brain ~/Dev/agentic-portfolio` runnable as its pre-flight check.

## The Destination

`mev` as the single validation entry point for the Bastion practice's Markdown corpus, and a portfolio
artifact demonstrating idiomatic Rust CLI design. Long-term it can promote to a subcommand of the personal
Rust ops CLI (`bastion validate ...`). The load-bearing idea: **one `Diagnostic` currency, many content
schemas** — adding a third consumer (blog, another repo) is a new `ContentValidator` impl, not a fork.

**The bigger destination (D4).** Beyond validation, `mev` is the **single corpus engine** for the Bastion
Brain: one Rust crawl produces three outputs — **diagnostics** (the validation gate), a **manifest** (the
canonical file-list + metadata the embedder consumes instead of re-crawling), and the **knowledge graph**
(the global `scope:doc_id` node index + edges, emitted as a first-class artifact). `mev` stays a **pure,
side-effect-free compiler** (files in → JSON out; no DB, no network), so it runs in CI with no credentials
and drops into a client KB with zero infra. Persistence is the orchestrator's job: the graph lives in
**Postgres beside the embeddings**, enabling **two retrieval modes over one store** — *semantic* (vector
search, fuzzy, costs tokens) and *structural* (graph/SQL, exact, free) — that fuse into graph-aware RAG.
This is the division of labor: **Rust owns the deterministic and free; Python owns the embedding/AI layer.**
See [D4](./decisions/D4-corpus-engine-and-knowledge-graph.md).

## Architecture / Design Overview

```
                         ┌─────────────────────────────┐
   mev validate  ───────▶│ LearnAiValidator            │──┐
                         │  crawl: paths/<id>/modules/ │  │
                         │  validate: Module/Path/Mdx  │  │
                         └─────────────────────────────┘  │   ┌──────────────┐
                                                          ├──▶│  Report      │
                         ┌─────────────────────────────┐  │   │  Vec<Diag>   │──▶ human / --json
   mev validate-brain ──▶│ BrainValidator              │──┘   └──────────────┘
                         │  crawl: all *.md (skip git) │
                         │  validate: OKF frontmatter  │
                         └─────────────────────────────┘
        shared:  Severity · Diagnostic · Report · extract_frontmatter · is_kebab_case · non_empty
```

`ContentValidator` is an associated-type trait (`type Item; crawl(); validate_item(); run()` driver) —
`main.rs` selects the concrete validator per subcommand, so static dispatch suffices. The generic core
stays free of any consumer's domain types; each consumer owns its own crawl, item type, and validators.

---

## Phase 0 — Foundation

### MV.0.A — Foundation setup — **Done**
- **What:** Configure the environment, scaffold the project skeleton, and verify the toolchain.
- **Why:** Establish a clean, reproducible starting point before any feature work.
- **Status:** `mev` binary scaffolded; `clap` CLI + `Diagnostic`/`Report` lib; smoke tests; all harness
  gates green.

---

## Phase 1 — Core: learn-module validation

First shippable feature set. Every block ships with tests against real fixtures (good modules + deliberately
broken ones). The bar is a *superset of* `learn-ai/scripts/validate-content.ts` (see D2). Universal currency
is the `Diagnostic` (`error` → exit 1, `warning` → exit 0); only the reporter prints.

### MV.1.B — Crawl & classify — **Done**
- **What:** `walkdir` the content root; classify each file as `learn-module-json`, `path-metadata-json`,
  or `module-mdx`. Build a `Corpus` grouped by path-id / module-id. Filename-convention checks (no spaces,
  lowercase, modules match `^\d{2}-[a-z0-9-]+\.(json|mdx)$`).
- **Status:** `walkdir` + `Corpus`; filename conventions; tests green.

### MV.1.C — Frontmatter & JSON struct validation — **Done**
- **What:** deserialize module `.json` into a strict `ModuleMeta`; validate enums (`difficulty`, section
  `type`, `level`), `duration` format, kebab-case `id`. Path `metadata.json` → `PathMeta`. MDX frontmatter
  parsed as real YAML (`title, description, duration, difficulty, lastUpdated`).
- **Status:** all tasks (1–7) complete; serde deserialization, required-field/enum/format checks,
  fixture-driven good+broken tests; all gates green.

### MV.1.D — Cross-file integrity (learn-ai differentiator) — **Not started**
- **What:** pair existence (`.json` ↔ `.mdx`); the **anchor-slice contract** (each
  `content.source = "<file>.mdx#<anchor>"` resolves to a file containing `## …{#<anchor>}`); ID coherence;
  callout types ∈ `info|warning|success|error`.
- **Acceptance:** a renamed anchor in a fixture is flagged here while the TS script stays silent.
- **Priority note:** deeply learn-ai-specific. **Deprioritized** below Phase 2 now that the Brain OKF use
  case is the immediate driver.

### MV.1.E — pt-BR parity & reporter polish — **Not started**
- **What:** each EN module requires a `pt-BR/` mirror with the identical filename; flag orphans. Finalize the
  reporter: grouped-by-file ANSI human output + `--json` for CI; correct exit codes.
- **Acceptance:** `mev validate ../learn-ai/content/learn` is green on the current corpus and reproduces every
  TS-script error plus anchor/pair/parity findings.
- **Note:** the `--json` reporter is pulled forward into **`MV.2.I`** (the Brain RAG indexer needs it); `MV.1.E`
  then only adds the ANSI grouping + pt-BR parity.

---

## Phase 2 — Generalize: `ContentValidator` trait + Brain OKF validation *(the current priority)*

Introduce the shared abstraction and the second consumer. This is where `mev` stops being "the learn-ai
validator" and becomes a general Markdown validator. Each block ends with `cargo fmt --check`,
`cargo clippy -- -D warnings`, and `cargo test` green, and **must keep the existing learn-ai tests passing**.

### MV.2.F — `ContentValidator` trait + shared core
- **What:** Add the associated-type `ContentValidator` trait (`crawl` + `validate_item` + default `run`
  driver). Extract `extract_frontmatter`, `is_kebab_case`, `non_empty` (with their unit tests) into a
  `shared` module. Relocate the learn-ai code behind a `LearnAiValidator` (move `crawl.rs`/`meta.rs` into a
  `learn_ai/` module verbatim) and rewrite `validate()` as a thin wrapper. Public API (`ContentFile` fields,
  `validate`, `crawl`, `validate_file`) preserved via `pub use` so all existing tests pass unchanged.
- **Acceptance:** module layout refactored; the full existing test suite is green with no signature changes.

### MV.2.G — Brain crawl
- **What:** A parallel crawl entry point: `MdFile { path, rel, stem }` + `crawl_brain(root)` that walks all
  `.md` under a root with a two-layer skip-list — a name blocklist (`target/`, `node_modules/`, `.git/`,
  ignored non-git dirs) and a **nested-git rule** that prunes any non-root directory containing its own
  `.git` (generically excludes every sub-project). The `depth() > 0` guard exempts the brain root, which is
  itself a git repo.
- **Acceptance:** unit tests prove nested-git dirs and `target/` are pruned, root-level `.md` is still found,
  and non-`.md` files are skipped.

### MV.2.H — Brain OKF frontmatter validator
- **What:** `OkfFrontmatter` serde struct (all fields `Option`, extras tolerated). Validate: required-field
  presence (`type`, `title`, `description` — each absence its own diagnostic); controlled-vocab membership for
  `layer` (list), `project`, `status`; kebab-case `doc_id` if present; `keywords` count 3–7 (warning); missing
  frontmatter entirely → single error. `type` is presence-only (open vocab). Mirror the schema in the brain's
  `docs/okf-frontmatter.md`. Settle the `layer` scalar-vs-list question empirically against the live corpus.
- **Acceptance:** fixtures for good doc, each missing required field, bad vocab value, non-kebab `doc_id`,
  keywords out of range, and missing frontmatter emit the expected diagnostics.

### MV.2.I — `validate-brain` subcommand + JSON reporter
- **What:** `mev validate-brain <brain-root>` (default `..`) wired to `BrainValidator`. A global `--json` flag
  emitting a machine-readable envelope (`{ validator, root, errors, warnings, diagnostics[] }`) the RAG indexer
  can consume; requires `Serialize` on `Diagnostic`/`Severity`. Update the CLI `about` text.
- **Acceptance:** `mev validate-brain ~/Dev/agentic-portfolio` reports real OKF violations, skips all
  nested-git sub-projects and `target/`; `--json` emits valid JSON usable as a pre-`--rebuild` gate.

---

## Phase 3 — Brain integrity: graph + sync *(serves the Brain; invoked by `bastion`)*

The deterministic checks that keep the Brain corpus *correct as a whole*, beyond per-file schema. Governed
by **brain decision D29** (mev is the single validation engine; `bastion validate` is a front door over it,
not a reimplementation). These are the three outer rings of validation after the schema ring (Phase 2):
graph + link integrity, structural coverage, and cross-repo sync. Each is read-only diagnostics (never
mutates the corpus — upholds D25) and `--json`-able so an agent can act on the findings. This is the mev
implementation of the brain-program's integrity/freshness work (the `hooks/README.md` `bastion validate
--integrity`, "Block K"), surfaced through `bastion`.

> **`MV.3.J` reshaped 2026-06-28** into a global cross-repo knowledge graph (see
> `planning/2.J-graph-integrity/namespacing-and-corpus-decision.md`). It now splits into a
> corpus-crawl foundation (`MV.3.J-crawl`) and the graph checks (`MV.3.J`). Canonical node id =
> `scope:doc_id`, where `scope` is a registry-driven stable slug from `brain.toml`.
>
> **D4 (2026-06-28) reframes the back half of Phase 3.** The crawl and graph mev builds here are
> not validation-only throwaways — they are the **corpus engine's outputs** (manifest + graph)
> that the embedder and a structural-query surface consume. Two forward-compat constraints apply
> to `MV.3.J-crawl` and `MV.3.J` (below); three additive blocks (Q–S) deliver the emitted products.
> See [D4](./decisions/D4-corpus-engine-and-knowledge-graph.md).

### MV.3.J-crawl — Multi-root corpus crawl + scope registry  *(foundation; lands first)*
- **What:** Make mev a **multi-root validator**. (1) A scope-unit **registry** read from `brain.toml`
  (HQ, each tier sub-brain, each repo → immutable `slug` + path) with a longest-prefix `scope_for(path)`
  resolver. (2) A canonical **corpus crawl** that walks every registered unit from the HQ root and includes
  only `planning/**` + `docs/**` + root `README.md`/`CLAUDE.md`, minus `skip_dirs` bloat (`target`,
  `node_modules`, `.git`, `archive`, `archived`, `trees`, `sdlc`, `.venv`, …) and ephemeral
  (`handoff.md`, `_`-prefixed). Replaces the old single-root, nested-git-pruned walk; `CLAUDE.md` is no
  longer file-blocklisted (root files now carry OKF frontmatter). This file-list is the **single corpus
  definition** the embedder should consume.
- **Acceptance:** the crawl returns each registered unit's `planning/`+`docs/`+root files with correct
  scope; bloat/ephemeral/unregistered dirs are excluded; `scope_for` is stable under simulated tier/path
  moves (slug-keyed). Companion (out of repo): `brain.toml` registers tier units; root-file frontmatter
  backfill.
- **Forward-compat (D4):** the crawl returns a **clean, owned data structure** (not state buried inside a
  validation pass) — it is about to feed both the manifest emit (Block Q) and the embedder. Build it as a
  reusable result, not a side effect.

### MV.3.J — Graph integrity (`scope:doc_id` `related:` edges)
- **What:** Over the corpus crawl, build a global **`scope:doc_id`** node index (node = a file with an
  authored `doc_id`; files without one are leaves). Flag duplicate canonical ids, and every `related:`
  edge that resolves to neither a node nor a leaf (**bare** id resolves within the referrer's scope;
  qualified **`scope:doc_id`** resolves cross-scope); a `related:` pointing at a leaf is a warning. The
  edge representation is generic (`from`, `to-ref`, `kind`) so typed edges (`supersedes`/`depends-on`/…)
  extend it later. Generalizes the learn-ai **anchor-slice contract** (Block D): a reference must resolve.
- **Acceptance:** a `related:` ref to a renamed/deleted node is flagged; duplicate `scope:doc_id`s are
  flagged; the same `doc_id` under different scopes is **not** flagged; a clean corpus passes.
- **Forward-compat (D4):** graph construction is a **reusable module** and its node/edge structs are
  **`Serialize`-able** — the same graph mev *validates* here is the graph mev *emits* in Block R. Do not
  bury it in a build-check-discard function.

### MV.3.K — Link integrity (markdown / `file://` / `[[wikilink]]`)
- **What:** Check that markdown `[text](path)` and `file:///…` links resolve to files that exist on disk,
  and that `[[wikilinks]]` (where used — e.g. memory docs) resolve to a known `doc_id`. Consume
  `.brain-moves-pending` (the post-commit delete/rename log) so a rename surfaces every now-broken reference
  to the old path.
- **Acceptance:** a link to a moved/deleted file is flagged; a `.brain-moves-pending` entry drives a
  targeted re-check of references to that path.

### MV.3.L — Structural coverage (`index.md` ↔ directory, D17)
- **What:** Enforce CLAUDE.md Standing Rule 7 / D17: every file in a directory appears in that directory's
  `index.md` (orphan detection), and every `index.md` row points at a file that exists (dangling-row
  detection). Bidirectional.
- **Acceptance:** a new file not listed in its `index.md` is flagged; an `index.md` row for a deleted file is
  flagged.

### MV.3.P — State integrity (`planning/state.json` schema + cross-repo block graph)
- **What:** `mev validate-brain --state`. Discover every repo's `planning/state.json` (HQ + each tier
  sub-brain + each `brain.toml` `[[repos]]` leaf — via the **cross-repo read mode** of `MV.3.M`, since
  the leaf files live in gitignored nested-git sub-repos invisible to the corpus walk), validate each
  against the canonical schema (`core/planning/state-schema.md`), and check the **work-block dependency
  graph** for referential integrity. This is the work-block analogue of `MV.3.J`: where `MV.3.J`
  validates the *document* graph (`scope:doc_id` nodes, `related:` edges), this validates the *block*
  graph (block-ID nodes, `blocked_by` / `cross_repo` edges). The marquee check
  (`E_STATE_DANGLING_BLOCKED_BY`) is the direct port of `E_GRAPH_DANGLING_RELATED` from docs to blocks.
  Rings: (1) JSON+schema (struct validation, not an external JSON-Schema file — consistent with OKF);
  (2) intra-repo (`focus` ↔ `tracks`, duplicate ids); (3) cross-repo (`blocked_by` / `cross_repo` edges
  resolve to real blocks); (4) brain rollup drift (`repos[]` vs child's actual `focus` — a **warning**,
  since the rollup lags by design between `/log-work` runs).
- **Acceptance:** a `blocked_by` pointing at a nonexistent target block is flagged
  (`E_STATE_DANGLING_BLOCKED_BY`); a malformed/ bad-enum state file is flagged; a drifted brain rollup
  is a warning (exit 0); a registered repo with no state file is a warning; the five live state.json
  files validate clean. Governed by **D29**.
- **Forward-compat (D4):** build a **`Serialize`-able** state graph (`StateNode` = block,
  `StateEdge { from, to_ref, kind }` with `kind` ∈ `BlockedBy` | `CrossRepo`) in a reusable
  `build_state_graph`, separate from `check_state_graph` — the graph mev *validates* here is the graph a
  future block *emits* (the state-graph parallel to `MV.3B.R`) and the input to mev *generating* the
  brain `repos[]`/`cross_repo[]` rollup instead of `/log-work` hand-writing it. Spec:
  `planning/3.P-state-integrity/tasks.md`.

### MV.3.M — Sync integrity (brain cache ↔ sub-repo canonical)
- **What:** The `synced_from` watermark check (D29). For each project in a small **projects manifest**
  (`{project, path, status_file}` — the machine form of the CLAUDE.md sub-projects table), read the
  sub-repo's *live* `planning/status.md` "Last updated" and compare against the `synced_from` frontmatter on
  the brain cache doc `docs/projects/<project>.md`. `live > synced_from` ⇒ a `Sync` error ("brain cache
  stale — run `/log-work` / `/sync-status`"). Program-tracker wave tables are **excluded** (they lag by
  design). Requires a **cross-repo read mode** distinct from the corpus walk, since Block G's nested-git
  pruning makes sub-repos invisible to the normal walk.
- **Acceptance:** a sub-repo advanced past its brain cache's `synced_from` is flagged; an in-sync project
  passes; snapshot trackers are never flagged.
- **v2 hardening (additive):** content-hash watermark (catches edits that don't bump the date); structured
  `current_block` comparison (catches an exact in-place mismatch).

### MV.3.P2 — State-graph expansion validation (cycles + status consistency + derivation drift)
- **What:** Extend `MV.3.P`'s `--state` validator to guard the **v2 schema** — the full work-block DAG
  (`core/planning/state-schema.md`, settled in `core/planning/state-graph-design-decisions/notes.md`).
  Beyond P's dangling check it adds: (1) **DAG acyclicity** over `type:block` `depends_on` edges —
  `E_STATE_CYCLE` reporting the cycle path (NEW; the doc graph needs no equivalent); (2) **status
  consistency** — reject the now-illegal authored `status: "blocked"` (`E_STATE_AUTHORED_BLOCKED`; blocked
  is derived, never stored) and flag a `closed` block whose `depends_on` target is not `closed`;
  (3) `depends_on` resolution extended to the **full** DAG (every block carries edges, not only
  active-blocked ones); (4) **backlog-node checks** — a `promoted` backlog node whose `block` resolves to
  nothing, and a backlog `depends_on` that dangles (same `E_STATE_DANGLING_BLOCKED_BY` family, only the
  source node type differs); (5) **derivation-drift warnings** — recompute `focus` / `repos[]` /
  `cross_repo[]` from the authored graph and emit `W_STATE_FOCUS_DRIFT` / `W_STATE_ROLLUP_DRIFT` on
  mismatch. Reuses `MV.3.M`'s cross-repo read mode and `MV.3.P`'s `Serialize`-able `StateGraph`.
- **Sequencing:** depends on the brain-side `state-schema.md` v2 edit + the 5-file `depends_on` re-seed
  landing first. Drift stays **warning-only** here; the warn→error flip is a follow-on gated on the
  `/log-work` derived-view writer existing (so a red build always has a tool that can fix it).
- **Acceptance:** a cyclic `depends_on` chain is flagged with its path; an authored `status:"blocked"` is
  rejected; a `closed`-depends-on-non-`closed` pair is flagged; a dangling backlog `depends_on` / orphan
  `promoted` node is flagged; `focus`/rollup drift is a warning (exit 0); the re-seeded five files
  validate clean. Governed by **D29 / D36**.
- **Forward-compat (D4):** the readiness + topological "what's next" ordering computed here is the same
  logic the emit block (`MV.3B.T`) serializes — build it as a reusable function, not buried in the check.

---

## Phase 3B — The Brain as a queryable product (corpus engine outputs, D4)

Where Phase 3 makes the corpus *correct*, Phase 3B makes the corpus engine's work *consumable*. These
blocks turn mev's crawl and graph into emitted artifacts and wire the two retrieval modes. They are
**additive** — they build on Blocks J-crawl/J without changing them. mev stays a **pure compiler** (emits
JSON; never touches a DB); persistence and the AI layer are the orchestrator's. Governed by
[D4](./decisions/D4-corpus-engine-and-knowledge-graph.md).

### MV.3B.Q — Manifest emit (kill the double crawl)
- **What:** `mev validate-brain --emit-manifest` (or a `mev manifest` subcommand) emits the canonical
  **file-list + per-file OKF metadata** as JSON, straight from the `MV.3.J-crawl` result. Companion (out of
  repo, orchestrator): refactor `index_brain.py` to **consume mev's manifest** instead of re-implementing
  `_collect_files`/`_corpus_roots`/`_classify_doc_type`/`normalize_metadata`. After this, "what's
  validated == what's embedded" holds by construction.
- **Carries the D5 extract-once refactor:** this block adds the metadata field to `CorpusEntry` and parses
  frontmatter **once during the crawl** (the OKF pass and `MV.3.J`'s graph build currently each re-read it).
  Manifest emit is the first consumer that genuinely needs per-entry metadata, so the corpus-model refactor
  deferred in D5 lands here, not as a speculative block before `MV.3.J`. `MV.3.J`'s `read_doc_metadata` seam
  collapses to `entry.metadata` with no call-site changes.
- **Acceptance:** the emitted manifest lists exactly the corpus crawl's files with correct scope/doc_id/
  metadata; an orchestrator dry-run driven by the manifest indexes the same file set the Python crawl did
  (parity check); mev itself still writes nothing to any DB.

### MV.3B.R — Graph emit + structural query surface
- **What:** mev emits the **graph JSON** (nodes = `scope:doc_id` + metadata; edges = `{from, to_ref, kind}`)
  from the `MV.3.J` graph module. Companion (out of repo): the orchestrator loads it into a **Postgres edges
  table beside `brain_documents`**, and a thin **structural-query surface** (`bastion` subcommand and/or an
  MCP tool) answers "where does X live / what is connected to Y / what's the status of Z" by SQL/recursive
  CTE — **free, instant, no tokens**. (Algorithms like centrality/shortest-path are a later, optional graft —
  ideas borrowable from `workflow-engine-rs/services/knowledge_graph`, not its Dgraph backend.)
- **Acceptance:** the emitted graph round-trips (every authored node + `related:` edge present, leaves
  marked); a structural query returns a doc's neighbors with zero embedding calls.

### MV.3B.S — Graph-aware RAG *(orchestrator; mev provides the edges)*
- **What:** the orchestrator's retrieval path uses the graph to **expand/rerank** semantic hits — traverse
  `related:` (later `supersedes`/`parent`/`depends-on`) from the top vector matches to feed the LLM
  *connected* context, not isolated chunks. A query **router** sends structural questions to the graph
  (free/exact) and semantic ones to vector+LLM, with the hybrid path fusing both. The `related` column
  already exists per-row in `brain_documents` — this is the block that finally traverses it.
- **Acceptance:** for a query whose answer spans linked docs, graph-expanded retrieval surfaces the
  neighbors a pure vector search misses; a purely structural query is answered without an LLM call.
- **Note:** this block is orchestrator-side work, tracked here only because mev's emitted edge model is its
  contract. It does not change mev.

### MV.3B.T — State-graph derived-view emit (the `MV.3B.R` parallel)
- **What:** Make mev the **single derivation engine** (`mev emit-state`) for every derived view the v2
  state schema declares **generated**: the leaf **`focus`** snapshot (now/next/blocked), the brain
  `repos[]` / `cross_repo[]` rollup, and the master-plan **wave/dependency tables** (written into
  `master-plan.md` between sentinel comments `<!-- BEGIN generated:wave-table -->` … `<!-- END -->` so
  narrative is never clobbered). Reads the union of all repos' `tracks[]`, builds the work-block DAG
  (reusing `MV.3.P2`'s graph + topo ordering and the **same `derive_focus` the drift check uses**), and
  emits — same **pure compiler** model as `MV.3B.R` (files in → artifact out; no DB, no network).
  **`/log-work` regenerates state by shelling out to `mev emit-state --write`** rather than re-deriving
  in a brain command, so the validator and the writer share one derivation. Settles D3 (Option B): mev
  owns derived-view generation, not a conversational `/log-work` agent.
- **Acceptance:** the emitted tables match the authored DAG (wave order + dependency columns);
  regeneration preserves every line of narrative outside the sentinels; the brain rollup matches the
  children's `tracks[]`; mev writes nothing to any DB.
- **Note:** this is the block that lets `MV.3.P2` flip derivation drift from warning → error — once the
  tables and rollup are generated, drift becomes fixable and therefore enforceable.

### MV.3B.U — Brain rollup tier-scoping + brain-focus aggregation (safe brain-kind emit)
- **What:** Make `mev emit-state --write` **safe and correct for brain-kind `state.json`** (it is not today).
  `MV.3B.T`'s `derive_rollup` rebuilds `repos[]` from a **global** scan of loadable `kind:"project"` files, so the
  rollup is not tier-scoped and any tier repo without a loadable `state.json` (unauthored, or a parse failure) is
  **silently dropped**. This corrupted `core/planning/state.json` + the HQ root file live (the `bastion` entry
  vanished on `E_STATE_MALFORMED_JSON`). Fix: **tier-scope** the rollup via `brain.toml` (HQ = full set),
  **preserve** the existing hand-authored entry for any sourceless in-scope repo (non-destructive), **populate**
  `RepoRollup.tier`, and **derive** brain `focus` as the repo-tagged union of in-scope children's focus.
- **Acceptance:** each brain file's `repos[]` reflects only its tier (HQ all); no in-scope repo dropped (derive /
  preserve / tier-tagged stub); `tier` populated; brain `focus` is the repo-tagged union; fixed point preserved
  (emit then re-emit is a no-op); a live dry-run against the brain shows zero dropped repos.
- **Note:** follow-up to `MV.3B.T` (append-only decision; do not edit MV.3B.T). Until it lands, `emit-state --write`
  must not be run against any brain-kind `state.json`. Spec: `planning/3B.U-brain-rollup-tier-scoping/tasks.md`.
  Carryover: `mev-brain-rollup-tier-scoping` (in `core/planning/state.json`).

---

## Phase 4 — Depth / Hardening: blog + linting

`BlogValidator` as a fourth `ContentValidator` impl (additive, no rewrite): blog frontmatter
(`title, date, excerpt`), pt-BR filename parity, code-block language-tag linting, and local link/asset
existence — applied across content types.

---

## Phase 5+ — Differentiating Build

- `mev watch` — hot-reload via `notify`, re-validate changed files in milliseconds.
- `mev compile` — emit `manifest.json` (path → module → section index) the site *could* adopt to replace
  runtime file walking.

---

## Quick Reference Sequence Table

| Phase | Block | What | Why | Role in destination |
|---|---|---|---|---|
| 0 | MV.0.A | Foundation setup (Rust scaffold, gates green) | Clean starting point | Enables everything downstream |
| 1 | MV.1.B | Crawl & classify content tree | Know every file and its kind | Input to all learn-ai checks |
| 1 | MV.1.C | Frontmatter & JSON struct validation | Catch missing/malformed fields | Superset of TS validator |
| 1 | MV.1.D | Cross-file integrity (anchor-slice, pairs, ids) | Catch silent runtime failures | learn-ai differentiator *(deprioritized)* |
| 1 | MV.1.E | pt-BR parity & ANSI reporter polish | Locale parity + human output | Phase 1 shippable |
| 2 | MV.2.F | `ContentValidator` trait + shared core | One core, many schemas | Makes `mev` general |
| 2 | MV.2.G | Brain crawl (all `.md`, skip nested git) | Enumerate the brain corpus | Input to OKF checks |
| 2 | MV.2.H | Brain OKF frontmatter validator | Gate docs before RAG index | The brain use case |
| 2 | MV.2.I | `validate-brain` subcommand + `--json` | Runnable pre-`--rebuild` gate | Brain shippable |
| 3 | MV.3.J | Graph integrity (`related:` edges resolve) | Catch dangling/duplicate doc_ids | Brain correctness (D29) |
| 3 | MV.3.K | Link integrity (markdown/`file://`/`[[wiki]]`) | Catch dead links + moved files | Brain correctness (D29) |
| 3 | MV.3.L | Structural coverage (`index.md` ↔ dir, D17) | Catch orphan files + dangling rows | Brain correctness (D29) |
| 3 | MV.3.M | Sync integrity (`synced_from` watermark) | Catch brain↔sub-repo status drift | Brain correctness (D29) |
| 3 | MV.3.P | State integrity (`state.json` schema + block graph) | Catch dangling `blocked_by` + rollup drift | Brain correctness (D29) |
| 3 | MV.3.P2 | State-graph expansion validation (cycles + status + drift) | Guard the v2 full DAG (acyclicity, derived blocked, drift) | Brain correctness (D29/D36) |
| 3B | MV.3B.Q | Manifest emit (file-list + metadata JSON) | Embedder consumes it; kill double crawl | Corpus engine output (D4) |
| 3B | MV.3B.R | Graph emit + structural query surface | Free/exact "where/what's connected" answers | Knowledge graph as product (D4) |
| 3B | MV.3B.S | Graph-aware RAG *(orchestrator)* | Fuse semantic + structural retrieval | The two-mode endgame (D4) |
| 3B | MV.3B.T | State-graph derived-view emit (`emit-state`) | Generate leaf `focus` + master-plan tables + brain rollup (single engine `/log-work` calls) | Corpus engine output (D3/D4) |
| 3B | MV.3B.U | Brain rollup tier-scoping + brain-focus aggregation | Make `emit-state --write` safe for brain-kind state.json (no rollup truncation) | Corpus engine output (D3/D4) |
| 4 | — | Blog validation + code-block/link linting | Cover a fourth content type | Whole-tree coverage |
| 5+ | — | `watch` (hot-reload) + `compile` (manifest.json) | Speed + precompiled index | Differentiating build |

---

*Sequenced by dependency and competence, not calendar. When life gets in the way, pick up where you left
off. Phase 2 (Brain OKF gate) is done; Phase 3 (Brain integrity) is the current priority — `MV.3.J-crawl`
then `MV.3.J`, built with the D4 forward-compat constraints; Phase 3B (corpus engine outputs,
D4) follows; `MV.1.D`–`MV.1.E` and Phase 4 resume after.*
