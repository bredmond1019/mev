---
type: Index
title: markdown-engine-validator Decisions Registry
description: Index of atomic, append-only architectural decision records for markdown-engine-validator.
doc_id: decisions-index
layer: [factory]
project: markdown-engine-validator
status: active
keywords: [decision records, ADR, architectural decisions, OKF, append-only]
related: [D1-initial-okf, D2-scope-and-sequence]
---

# Decisions Registry

Architectural decision records (ADRs) for markdown-engine-validator. Each decision is **one atomic
file**, append-only — never edit a settled decision; supersede it with a new one and link back.

## Decisions

- [D1: Initial OKF Scaffold](./D1-initial-okf.md) — Project initialized on the standard OKF
  documentation structure.
- [D2: Validator Scope & Sequence](./D2-scope-and-sequence.md) — Learn-first, validator-first,
  superset of the site's `validate-content.ts`; summaries/transcripts out of scope.

<!-- Add a row per decision as they are made. Record new ones with /log-decision-style atomic
     files (D3, D4, …). -->
