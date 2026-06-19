---
type: Plan
title: markdown-engine-validator Master Plan
description: Strategic roadmap and phase specifications for markdown-engine-validator.
---

# markdown-engine-validator — Master Plan

*Living document. Created 2026-06-18.*

## The Goal, Stated Plainly

A Rust CLI tool that parses, validates, and compiles MDX/Markdown lessons for learn-agentic-ai.com — frontmatter validation, link checking, code block linting, and watch-mode hot-reload

<!-- 2–3 paragraphs: what the project is, why it matters, and what "ready" means — the
     competence or delivery checkpoint that signals Phase completion. -->

## The Destination

<!-- The named product or outcome. If commercial: the buyer, the differentiator, and the
     through-line connecting the builder to the product. -->

## Architecture / Design Overview

<!-- The key structural design: how the system is organized, its layers, an ASCII diagram if
     useful, and the load-bearing design decisions. Keep deployment specifics out — those are
     injected via config. -->

---

## Phase 0 — Foundation

### Block A — Foundation setup
- **What:** Configure the environment, scaffold the project skeleton, and verify the toolchain.
- **Why:** Establish a clean, reproducible starting point before any feature work.
- **Build notes:** <!-- specific tasks, tools, conventions -->
- **Acceptance criteria:** Codebase builds; the run/test commands in `CLAUDE.md` succeed; the
  planning infrastructure is in place.

---

## Phase 1 — Core: learn-module validation

First shippable feature set. Every block ships with tests against real fixtures copied from
`learn-ai/content/learn` (good modules + deliberately broken ones). The bar is a *superset of*
`learn-ai/scripts/validate-content.ts` (see D2). Universal currency is the `Diagnostic`
(`error` → exit 1, `warning` → exit 0); only the reporter prints.

### Block B — Crawl & classify
- **What:** `walkdir` the content root; classify each file as `learn-module-json`,
  `path-metadata-json`, or `module-mdx`. Build a `Corpus` grouped by path-id / module-id.
  Filename-convention checks (port `validateFileName`): no spaces, lowercase, modules match
  `^\d{2}-[a-z0-9-]+\.(json|mdx)$`.
- **Acceptance:** corpus enumerates the live tree; filename violations surface as diagnostics.

### Block C — Frontmatter & JSON struct validation
- **What:** deserialize module `.json` into a strict `ModuleMeta` (require `id, pathId, title,
  description, duration, type, difficulty, order, objectives, tags, version, lastUpdated` +
  non-empty `sections[]` with `id/type/order`); validate enums (`difficulty`, section `type`),
  `duration` format `^\d+\s+(minutes?|hours?)$`, kebab-case `id`. Path `metadata.json` requires
  `id, title, description, level, duration, version, lastUpdated, topics, modules`. MDX frontmatter
  parsed as real YAML (not substring): require `title, description, duration, difficulty,
  lastUpdated`. Mirror `content/learn/schemas/module-schema.json` enums where practical.
- **Acceptance:** every required-field / format violation in fixtures emits the expected diagnostic.

### Block D — Cross-file integrity (the differentiator)
- **What:** pair existence (every module `.json` ↔ sibling `.mdx`); the **anchor-slice contract**
  (each `content.source = "<file>.mdx#<anchor>"` resolves to an existing file containing
  `## …{#<anchor>}`, replicating the site regex so a pass guarantees a render); ID coherence
  (`metadata.id` has no numeric prefix while the filename does; `metadata.json.modules[]` map to
  real files; `sections[].id` == anchor == source anchor); callout types ∈ `info|warning|success|error`.
- **Acceptance:** a renamed anchor in a fixture is flagged here while the TS script stays silent.

### Block E — pt-BR parity & reporter polish
- **What:** each EN module requires a `pt-BR/` mirror with the identical filename; flag orphaned
  `.json`/`.mdx` in either locale. Finalize the reporter: grouped-by-file ANSI human output +
  `--json` for CI; correct exit codes.
- **Acceptance:** `mev validate ../learn-ai/content/learn` is green on the current corpus and
  reproduces every TS-script error plus anchor/pair/parity findings.

---

## Phase 2 — Depth / Hardening: blog + linting

`BlogValidator` behind the `ContentValidator` trait (additive, no rewrite): blog frontmatter
(`title, date, excerpt` per `content/blog/CLAUDE.md`), pt-BR filename parity, code-block
language-tag linting, and local link/asset existence — applied across both content types.

---

## Phase 3+ — Differentiating Build

- `mev watch` — hot-reload via `notify`, re-validate changed files in milliseconds.
- `mev compile` — emit `manifest.json` (path → module → section index) the site *could* adopt to
  replace runtime file walking.

---

## Quick Reference Sequence Table

| Phase | Block | What | Why | Role in destination |
|---|---|---|---|---|
| 0 | A | Foundation setup (Rust scaffold, harness gates green) | Clean starting point | Enables everything downstream |
| 1 | B | Crawl & classify content tree | Know every file and its kind | Input to all checks |
| 1 | C | Frontmatter & JSON struct validation | Catch missing/malformed fields | Superset of TS validator |
| 1 | D | Cross-file integrity (anchor-slice, pairs, ids) | Catch the silent runtime failures | The differentiator |
| 1 | E | pt-BR parity & reporter polish | Locale parity + CI-ready output | Phase 1 shippable |
| 2 | — | Blog validation + code-block/link linting | Cover the second content type | Whole-tree coverage |
| 3+ | — | `watch` (hot-reload) + `compile` (manifest.json) | Speed + precompiled index | Differentiating build |

---

*Sequenced by dependency and competence, not calendar. When life gets in the way, pick up
where you left off.*
