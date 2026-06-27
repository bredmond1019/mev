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
  are interim. Sequenced after Phase 3.

<!-- Add a row per decision as they are made. Record new ones with /log-decision-style atomic
     files (D4, D5, …). -->
