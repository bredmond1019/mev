---
type: Decision
title: D3 — Corpus-level config file for validator rules
description: Plan to move hardcoded corpus rules (skip-lists, doc_id patterns, vocab sets) into a per-corpus config file, making mev a generic engine driven by config rather than baked-in assumptions.
doc_id: D3-corpus-config-system
layer: [factory]
project: mev
status: draft
keywords: [config, corpus, skip-list, vocab, doc_id, extensibility]
related: [D2-scope-and-sequence, decisions-index]
---

# D3 — Corpus-level config file for validator rules

**Status:** draft — planned for a future block; current hardcodes are interim.

## Context

Several corpus-specific rules are currently hardcoded in Rust source:

- **Crawl skip-list** (`src/brain/crawl.rs`): `.claude`, `.repo-backups`, `.agent`, `target`,
  `node_modules`, `.git` — Brain-specific dirs that are not OKF docs.
- **doc_id patterns** (`src/brain/okf.rs`): the `D<n>-…` decision-file format, added in the
  fix that accompanied this decision. Standard kebab-case plus a Brain-specific extension.
- **Controlled vocab** (`src/brain/okf.rs`): `layer`, `project`, `status` closed sets — any
  new project or layer value requires a Rust edit and a release.

As `mev` gains a second consumer (learn-ai), the divergence between corpus rules will grow.
Hardcoding rules per-consumer doesn't scale.

## Decision

Introduce a **per-corpus config file** (tentative: `.mev.toml` at the corpus root) that
`mev` auto-discovers when validating. The Rust code becomes a pure validation engine; all
corpus-specific rules live in config.

Minimum viable fields:

```toml
[crawl]
skip_dirs = ["target", "node_modules", ".git", ".claude", ".repo-backups", ".agent"]

[doc_id]
# Patterns accepted in addition to standard kebab-case (applied as prefix-match rules)
extra_patterns = ["D\\d+(-[a-z0-9]+)*"]

[vocab]
layer   = ["brain", "engine", "factory", "console", "surface", "infra", "business", "content", "meta"]
project = ["bastion", "bastion-ui", "python-orchestration", "learn-ai", "rag-engine-rs",
           "claude-sdk-rs", "workflow-engine-rs", "mev",
           "bella", "price-scout", "amistad", "base-template", "brain"]
status  = ["active", "draft", "deprecated", "superseded", "archived"]
```

## Discovery model

`mev` walks up from the validated corpus root to find `.mev.toml`. If none is found,
built-in defaults apply (current hardcodes become the defaults). This mirrors how
`.eslintrc` and `pyproject.toml` travel with the corpus they govern.

Each consumer (Brain, learn-ai) carries its own `.mev.toml`. The Brain's `.mev.toml`
would live at `agentic-portfolio/` root.

## What changes vs. today

The `is_decision_id` / `is_valid_doc_id` Rust functions written as part of the adjacent
fix are **not thrown away** — they become the pattern-matching engine that config-supplied
patterns drive. Similarly, the crawl pruning and vocab-check logic stays in Rust; only
the *values* move to config.

## Interim state

Until this block ships, corpus rules remain hardcoded. Each hardcode should be marked
with a `// TODO(D3): move to config` comment so they're easy to migrate.

## Sequencing

Implement after Phase 3 (graph + link + structural integrity checks) so the config
surface area is known before the schema is locked. Tentative slot: Phase 4 or a
standalone block after Block L.
