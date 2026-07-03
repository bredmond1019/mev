---
type: Index
title: mev Decisions Registry
description: Index of atomic, append-only architectural decision records for mev.
doc_id: decisions-index
layer: [factory]
project: mev
status: active
keywords: [decision records, ADR, architectural decisions, OKF, append-only]
related: [D1-initial-okf, D2-scope-and-sequence]
---

# Decisions Registry

Architectural decision records (ADRs) for mev. Each decision is **one atomic
file**, append-only — never edit a settled decision; supersede it with a new one and link back.

## Decisions

- [D1: Initial OKF Scaffold](./D1-initial-okf.md) — Project initialized on the standard OKF
  documentation structure.
- [D2: Validator Scope & Sequence](./D2-scope-and-sequence.md) — Learn-first, validator-first,
  superset of the site's `validate-content.ts`; summaries/transcripts out of scope.
- [D3: Corpus-level config file](./D3-corpus-config-system.md) — Plan to move hardcoded
  skip-lists, doc_id patterns, and vocab sets into a per-corpus `.mev.toml`; current hardcodes
  are interim. Sequenced after Phase 3. **Superseded by `brain.toml`.**
- [D4: Corpus engine & knowledge graph](./D4-corpus-engine-and-knowledge-graph.md) —
  Destination architecture: mev is the single corpus engine (one crawl → diagnostics +
  manifest + graph), a pure side-effect-free compiler; the brain graph is an emitted product
  stored in Postgres beside the embeddings; two retrieval modes (semantic + structural) fuse
  at retrieval. Reject the Dgraph `knowledge_graph` service for the brain.
- [D5: Heterogeneous-format ingest](./D5-heterogeneous-format-ingest.md) — *Draft/deferred.*
  Forward-looking sketch for extending mev to foreign client formats (`.txt`/`.docx`/`.pdf`/`.yaml`)
  via a normalized document model, sidecar metadata, and a deterministic `mev discover` step. Binds
  only two guardrails now (honored in Block J): metadata behind a single extractor seam, and the graph
  stays authored-only (never inferred).
- [D6: scope:doc_id canonical id scheme & corpus rules](./D6-scope-doc-id-namespacing.md) —
  Registry-driven stable `scope:doc_id` canonical node ids; corpus membership rules (docs/ +
  planning/ + root README/CLAUDE, minus skip_dirs, minus ephemeral); nodes vs leaves; extensible
  edge model; authored-only graph guarantee. Settled 2026-06-28 from live 503-file corpus pass.
- [D7: Brain rollup tier-scoping, preserve rule, and brain-focus union](./D7-brain-rollup-tier-scoping-and-preserve.md) —
  Brain `repos[]` rollup scopes by tier (not global) via `brain.toml`; a tier repo with no loadable
  child `state.json` is preserved verbatim instead of silently dropped (fixes the live
  `core`/HQ rollup corruption incident); `RepoRollup.tier` is always populated; brain `focus` is
  derived as a repo-tagged union of in-scope children's focus.
- [D8: portfolio kind — terminal repos with no planning state](./D8-portfolio-kind-terminal-repos.md) —
  New `state.json` `kind:"portfolio"` for repos published to GitHub with no further planning
  state (`rag-engine-rs`, `workflow-engine-rs`, `claude-sdk-rs`); requires a `note` instead of
  `tracks[]`; exempt from the `master-plan.md` sentinel warning in `emit-state`.
- [D9: BA.15.12 (okf-core format convergence) — mev-side mirror](./D9-ba15-12-okf-core-convergence-mirror.md) —
  Mirrors bastion's D15/D16: a pending cross-repo dependency where bastion's `okf-core` crate
  eventually becomes the single implementation of `brain/okf.rs`, `brain/state.rs`, `brain/graph.rs`,
  and `brain/graph_emit.rs`, deleted here once `okf-core` gains the matching models. Not yet scheduled
  as a block in this repo — corrects `master-plan.md`'s stale "out of scope" note on `MV.3B.V`.

<!-- Add a row per decision as they are made. Record new ones with /log-decision-style atomic
     files (D10, D11, …). -->
