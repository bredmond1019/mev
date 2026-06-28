---
type: Index
title: mev — Planning Docs
description: Navigation index for the mev planning folder.
doc_id: planning-index
layer: [factory]
project: mev
status: active
keywords: [planning navigation, context, master plan, SDLC, concept folders]
related: [context, status, master-plan]
---

# mev — Planning Docs

The strategy, state, and decision record for mev. Code lives elsewhere; this
folder is the map.

## Files

| File | What it is | Open it when… |
|---|---|---|
| `context.md` | Orientation + governing principles (read first) | You need to understand the project |
| `status.md` | Current progress tracker | You need to know what's done / next |
| `knowledge.md` | Distilled, durable knowledge — how it works, conventions, architecture digest | You need to understand how the system works |
| `memory.md` | Repo-scoped durable memory — episodic notes, preferences, superseded facts | You need project facts that survive a handoff |
| `master-plan.md` | Strategy + phase specifications | You need the sequence of work |
| `artifacts/` | Working outputs / scratch artifacts produced during runs | You need a place for generated artifacts |
| `harness.json` | Validation/UI-test config the SDLC engines read | You're adapting the pipeline to this stack |
| `decisions/` | Atomic, append-only architectural decisions | You want to check a prior choice |
| `<concept>/` | Per-spec planning folders (task specs + pipeline state) | You're running the SDLC pipeline |

## The concept-folder model

Each unit of work gets its own **concept folder** under `planning/<concept>/` (e.g.
`planning/auth-rework/`). Human-authored planning content sits at the concept top level; the
SDLC pipeline keeps its machine state in a reserved `sdlc/` subfolder:

```
planning/<concept>/
├── tasks.md          ← the spec (Goal / Context / Tasks / Acceptance / Validation Commands)
├── breakdown.md      ← optional human decomposition notes
└── sdlc/             ← pipeline state (machine-managed — don't hand-edit)
    ├── execution-plan.json
    └── reports/      ← task{N}-implement|test|review|document|ui-test|log.md, block-workflow.md
```

The engines resolve every path off `planning/<concept>/` — `tasks.md` and `breakdown.md` stay
at the top; only pipeline state lives under `sdlc/`.

## Read Order for a Newcomer

1. `context.md` — what this is and the rules of the road
2. `status.md` — where things stand right now
3. The relevant phase section of `master-plan.md`

## Active Concept Folders

| Folder | What | Status |
|---|---|---|
| `2.M-brain-toml-reader/` | HQ-R Block M — mev reads `brain.toml` for vocab + crawl + project slugs | Done |
| `block-n-sync-watermark/` | HQ-R Block N (mev Phase 3 Block M) — `mev validate-brain --sync` watermark check | Not started |

## What's NOT Here

- Application code (lives in the source tree, not `planning/`)
- Generated task specs (those live under `planning/<concept>/`)

---

*The map, not the territory. For the chronological narrative, see the root `log.md`.*
