---
type: Index
title: mev Docs
description: Navigation index for all mev reference documentation
doc_id: docs-index
layer: [meta]
project: mev
status: active
keywords: [documentation, index, navigation, mev, reference]
related: [mev-readme, cli-reference, architecture, brain-toml-config, okf-schema, carryover-contract]
---

# mev — Documentation Index

| Doc | What it covers |
|---|---|
| [CLI reference](cli.md) | Subcommands (including `state-history` list/restore, `attention-queue` operator payload delivery, `check-consumers` post-merge consumer compile gate, `frontier` corpus-wide startable-block frontier + gate_rank, `lanes` six-state segment availability + lane-level unblock leverage), flags, defaults, exit codes, examples |
| [Architecture](architecture.md) | Module map, `ContentValidator` trait, core types (`Diagnostic`, `Report`, `JsonReport`) |
| [brain.toml config](brain-toml.md) | Full `brain.toml` schema — `[vocab]`, `[crawl]`, `[[repos]]` |
| [OKF schema](okf-schema.md) | OKF frontmatter fields, required vs optional, validation rules |
| [Carryover triage ranking contract](carryover-contract.md) | Canonical, producer-owned contract for `rank_carryover` — the four-lane ranking API and wire shape bastion consumes |
| [SDLC workflows](workflows/index.md) | Claude Code SDLC pipeline commands and workflow engines |

For project strategy and current focus, see [`planning/`](../planning/index.md).
