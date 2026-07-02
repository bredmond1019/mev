---
type: Decision
title: D5 — heterogeneous-format ingest (extractor seam + discovery/enrichment)
description: Forward-looking sketch for extending mev beyond authored Markdown to foreign client formats (.txt/.docx/.pdf/.yaml) via a normalized document model, sidecar metadata, and a deterministic discover step — keeping the graph authored-only.
doc_id: D5-heterogeneous-format-ingest
layer: [factory, brain, engine]
project: mev
status: draft
keywords: [ingest, file formats, extractor seam, sidecar metadata, discovery, enrichment, client corpus]
related: [D4-corpus-engine-and-knowledge-graph]
---

# D5 — Heterogeneous-format ingest (extractor seam + discovery/enrichment)

**Status: draft / exploratory — deferred backlog, not Block J work.** Captured 2026-06-29 from a
design discussion so the reasoning isn't lost and so the in-flight Block J work honors the cheap
forward-compat guardrails. This decision sketches the *end-state*; it commits us only to the two
guardrails in "What we honor now."

## Why this matters

Today mev validates an **authored** corpus: Markdown we wrote, with OKF YAML frontmatter we authored
(`doc_id`, `related`, `layer`, …). When the Brain architecture (D4) is reused as the knowledge-base
substrate for **client engagements**, the source corpus is the opposite: arbitrary formats
(`.txt`, `.docx`, `.pdf`, `.yaml`), **no frontmatter, no `doc_id`, no authored edges**, and files we
**must not mutate** (a client won't let us rewrite their `.docx`).

That is a different verb. mev today **validates** authored metadata; a client corpus needs
**extraction + enrichment** — pull the text, derive what metadata is cheap, and *propose* the rest.

## The end-state shape (sketch — not yet committed)

1. **Normalized document model in the middle.** Format-specific extraction sits *below* a common
   `Document { text, metadata, scope, source_ref }`; everything above (validation, manifest, graph,
   embedding) is format-agnostic. D4's **manifest is already this normalized layer** — formats feed it.

2. **Extractor seam.** One trait/function decides "where does this file's metadata + text come from."
   Markdown-inline-frontmatter is one impl; `.txt`/`.yaml`/`.docx`/sidecar are future impls. Mirrors
   the existing `ContentValidator` trait pattern.

3. **Sidecar metadata for formats with no frontmatter slot.** `report.docx` + `report.docx.meta.yaml`
   (or a `.mev/` sidecar dir). Source stays pristine; metadata is one uniform OKF-shaped YAML
   regardless of source format, so the whole downstream is unchanged.

4. **`mev discover` (subcommand, not a flag) — deterministic, free, Rust.** Walks a foreign corpus and
   emits **proposed sidecar stubs**: `doc_id` from stem, `title` from first heading / filename / native
   doc property, `scope` from the registry, `format` from extension, dates from mtime. No edges (can't
   infer for free). "Rust owns the deterministic and free" (D4).

5. **AI enrichment — paid, Python, orchestrator.** Embeddings feed back into the graph: propose
   `related` from semantic nearest-neighbors, generate `description`/`keywords`, classify `layer`/`type`.
   The semantic layer bootstraps the structural layer for a cold corpus. "Python owns the embedding/AI
   layer" (D4).

6. **Format/Rust-Python boundary.** Easy deterministic formats (`.txt`, `.yaml`) → mev extractor impls
   directly. Messy/fragile formats (`.pdf`, and `.docx` to a degree) → normalized **upstream** (an
   ingest stage, likely Python) into markdown-text + sidecar that mev then processes — keeping mev a
   pure, dependency-light, CI-friendly compiler. (Lean, not locked.)

## What we honor NOW (the only binding part of D5)

Two guardrails, both cheap now and expensive to retrofit, applied to **`MV.3.J`**
since `MV.3.J-crawl` already shipped to review:

1. **Metadata through a single extractor seam.** `MV.3.J` reads `doc_id`/`related` via one
   `read_doc_metadata` helper — the sole site that knows metadata is inline Markdown frontmatter — so a
   future foreign-format/sidecar extractor is a one-function swap, not a scattered refactor.
2. **The graph is authored-only — never inferred.** Nodes/edges come solely from authored/confirmed
   metadata. Discovery and AI enrichment (above) produce **reviewable** proposals; nothing enters the
   graph until a human confirms it into authored frontmatter/sidecars. This is the same property that
   made us reject the inferred-edge Dgraph service in D4.

## Deferred (explicitly NOT now)

The normalized `Document` model, the extractor trait + per-format impls, sidecar reading, `mev discover`,
and AI edge-proposal are all **backlog**. Revisit when a real heterogeneous (client) corpus is on the table.

The corpus-model refactor to fully realize the seam (corpus-crawl shipped with
`CorpusEntry { path, rel, stem, scope }` and no metadata field, so the OKF pass and `MV.3.J`'s graph build
each re-parse frontmatter) is **folded into `MV.3B.Q` (manifest emit)** — that block needs per-entry
metadata to emit the file-list + metadata JSON, so it is the point where `CorpusEntry { …, metadata }`
becomes load-bearing and the extract-once optimization has a real consumer. Do **not** add a speculative
follow-on block before `MV.3.J` for this: the `read_doc_metadata` seam (Decision 7 in
`2.J-graph-integrity/tasks.md`) is already entry-keyed and I/O-internal, so when the field lands the seam
body becomes `entry.metadata` with no call-site changes. The double-read until then is negligible at brain
scale (hundreds of files).

## What this does not change

- D4 (corpus engine; pure compiler; graph in Postgres; two retrieval modes) — D5 extends it.
- The `scope:doc_id` scheme and corpus rules (`block-j-namespacing-decision.md`).
- Any shipped Phase 2 / `MV.3.J-crawl` behaviour.
