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

<!-- Add a row per decision as they are made. Record new ones with /log-decision-style atomic
     files (D5, D6, …). -->
