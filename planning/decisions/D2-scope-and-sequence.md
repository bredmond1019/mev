---
type: Decision
title: "D2: Validator Scope & Sequence"
description: Learn-first, validator-first, and superset-of-the-TS-script as the project's scope and sequencing decisions.
doc_id: D2-scope-and-sequence
layer: [factory, surface]
project: mev
status: active
keywords: [validator scope, learn-ai, MDX validation, anchor-slice, TS script superset]
related: [decisions-index, master-plan]
---

# D2 — Validator Scope & Sequence

**Decided:** 2026-06-18
**Status:** Accepted

## Decision

`mev` targets the learn-agentic-ai.com (`learn-ai/`) content tree. Three scoping/sequencing
choices, made with the owner:

1. **Learn modules first.** Phase 1 validates `content/learn` (paired `.json` + `.mdx` modules,
   path `metadata.json`). Blog (`content/blog`, single `.mdx`) is Phase 2 behind the same engine.
2. **Validator first, compile later.** The site loads content directly and needs no `manifest.json`
   for correctness today, so the `compile` step is deferred to Phase 3+ alongside `watch` mode.
   Phase 1–2 ship validation only (exit codes + human/`--json` reports).
3. **Aim to replace `npm run validate:content`.** `mev` targets a strict *superset* of the site's
   existing `scripts/validate-content.ts` (which is learn-only and checks frontmatter by substring),
   adding the cross-file integrity the TS script lacks — most importantly the **section anchor-slice
   contract** (`lib/content/learning/modules.server.ts` slices each section with
   `(## .*\{#<anchor>\}[\s\S]*?)(?=\n## |$)`; a missing anchor renders "Content for section X not
   found" at runtime with no build error). The Rust binary stays standalone in *this* repo; `learn-ai`
   adopts it as a pre-build gate only once it is proven.

## Why

The corpus is ~260 files; Rust parsing is fast enough for watch-mode hot-reload later. Learn
content is the highest-value, hardest case and the one with a silent runtime failure mode the
current tooling misses, so it earns the engine's first pass. Validation is the load-bearing
deliverable; the manifest is speculative until the site chooses to consume it.

## Out of Scope

- `content/summaries/` (loose prose, no schema, not published) and `content/youtube-transcripts/`
  (raw `.json`/`.txt` import artifacts) — source material, not in the build pipeline.
- Any edits to `learn-ai` until the binary is proven and the owner chooses to wire it in.

## Rejected Alternatives

- **Blog first:** simpler, but lower value and no silent failure mode to catch first.
- **Validate + compile as peers:** rejected — the site consumes no manifest yet, so compile is
  speculative; building it now risks rework.
- **Complement, not replace, the TS validator:** rejected — two coexisting validators duplicate
  coverage and drift; targeting a superset gives a clean future retirement of the TS script.

## Provenance

Decided during the initial `/plan` session; supersedes nothing. See `planning/master-plan.md`
Phase 1 blocks B–E for the implementation sequence.
