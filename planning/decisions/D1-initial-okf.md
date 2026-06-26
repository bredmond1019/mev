---
type: Decision
title: "D1: Initial OKF Scaffold"
description: Project initialized on the standard OKF documentation structure.
doc_id: D1-initial-okf
layer: [factory, meta]
project: markdown-engine-validator
status: active
keywords: [OKF scaffold, planning structure, SDLC harness, base-template, initialization]
related: [decisions-index, context]
---

# D1 — Initial OKF Scaffold

**Decided:** 2026-06-18
**Status:** Accepted

## Decision

markdown-engine-validator is initialized on the standard **OKF (Open Knowledge Format)** documentation
structure: a `planning/` folder with `context.md`, `status.md`, `master-plan.md`, a
`harness.json` pipeline config, an atomic `decisions/` registry, and per-spec concept folders
`planning/<concept>/` (with pipeline state under a reserved `<concept>/sdlc/`); OKF YAML
frontmatter on every markdown file; and the curated SDLC harness (`.claude/`) for
the implement → test → review → document pipeline.

## Why

Consistency with the company brain and the other projects in the practice. The structure is
load-bearing for the SDLC workflows (they read `status.md`, `master-plan.md`, and
`planning/<concept>/`), so adopting it from day one means the pipeline runs without path
fixes.

## Rejected Alternatives

- **Ad-hoc docs (a single README + scattered notes):** rejected — the workflows depend on the
  named files, and the brain's navigation assumes the OKF layout.

## Provenance

Generated from `base-template` commit `00ad2834e232d3243a3578132b02db01a7be40ab`.
