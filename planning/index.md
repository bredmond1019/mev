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
| [`context.md`](context.md) | Orientation + governing principles (read first) | You need to understand the project |
| [`status.md`](status.md) | Current progress tracker | You need to know what's done / next |
| [`knowledge.md`](knowledge.md) | Distilled, durable knowledge — how it works, conventions, architecture digest | You need to understand how the system works |
| [`memory.md`](memory.md) | Repo-scoped durable memory — episodic notes, preferences, superseded facts | You need project facts that survive a handoff |
| [`master-plan.md`](master-plan.md) | Strategy + phase specifications | You need the sequence of work |
| `artifacts/` | Working outputs / scratch artifacts produced during runs | You need a place for generated artifacts |
| `harness.json` | Validation/UI-test config the SDLC engines read | You're adapting the pipeline to this stack |
| `decisions/` | Atomic, append-only architectural decisions | You want to check a prior choice |
| `archive/` | Retired concept folders — residue distilled before moving | You're reviewing completed work |
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
| `herdr-mev-patterns/` | Research notes — herdr patterns applicable to mev graph/crawl/validation (Block J done; patterns Q/R/S/watch deferred) | Active |
| `emit-graph-resolved-edges/` | `MV.3B.V` context seed — export `check_graph`'s edge resolution in `emit-graph` (v2: `target_node_id`/`target_doc_id`), killing the Rust/Python resolution divergence OR.G exposed; gates the embed pass | Active — spec decomposed; ready to run (`/sdlc-flow emit-graph-resolved-edges`) |

## Archived Concept Folders

Completed blocks are in `archive/` — see [`archive/index.md`](./archive/index.md) for the full registry.
Recent additions (distilled 2026-07-02): `3.K-link-integrity`, `3.L-structural-coverage`,
`3.P-state-integrity`, `3.P2-state-graph-validation`, `3B.Q-manifest-emit`, `3B.R-graph-emit`,
`3B.T-state-table-rollup-emit`, `3B.U-brain-rollup-tier-scoping`, `ticket-review-frontmatter`.
Earlier (distilled 2026-06-29): `2.F-content-validator-trait`, `2.G-brain-crawl`,
`2.H-brain-okf-validator`, `2.I-validate-brain-subcommand`, `2.J-corpus-crawl`,
`2.J-graph-integrity`, `2.M-brain-toml-reader`, `block-n-sync-watermark`.

## What's NOT Here

- Application code (lives in the source tree, not `planning/`)
- Generated task specs (those live under `planning/<concept>/`)

---

*The map, not the territory. For the chronological narrative, see the root `log.md`.*


<!--
Validator links:
[harness.examples.md](./harness.examples.md)
-->

<div style="display:none;">

[harness.examples.md](./harness.examples.md)

</div>
