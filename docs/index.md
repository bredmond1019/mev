---
type: Index
title: mev Docs
description: Navigation index for mev's reference documentation, grouped by what you are trying to do.
doc_id: docs-index
layer: [meta]
project: mev
status: active
keywords: [documentation, index, navigation, mev, reference]
related: [cli-reference, architecture, brain-toml-config, okf-schema, carryover-contract]
---

# mev — Documentation Index

`mev` validates the Bastion Brain corpus and derives every generated surface from it.
**New here? Start with the [CLI reference](cli.md)** — it catalogues all 26 commands and opens with
a Quickstart.

## I want to run something

| Doc | What it covers |
|---|---|
| [CLI reference](cli.md) | All 26 commands, one line each, with links into detail |
| [Validation](cli/validate.md) | Checking content, the corpus, one file, or cross-source drift |
| [State and derivation](cli/state.md) | Regenerating derived surfaces; revision history |
| [Epics and operator gates](cli/epics.md) | Moving initiatives; clearing gates that need a human |
| [Carryover and attention](cli/carryover.md) | Sweeping open findings; what needs the operator |
| [Graphs, lanes and artifacts](cli/lanes.md) | What is startable; graph exports; the consumer gate |

## I want to configure something

| Doc | What it covers |
|---|---|
| [brain.toml config](brain-toml.md) | Every config section and what it controls |
| [OKF schema](okf-schema.md) | Frontmatter fields, required vs optional, validation rules |

## I want to understand how it works

| Doc | What it covers |
|---|---|
| [Architecture](architecture.md) | Module map, the `ContentValidator` trait, core types |
| [Carryover contract](carryover-contract.md) | The ranking API and wire shape bastion consumes |
| [SDLC workflows](workflows/index.md) | The Claude Code pipeline commands and engines |

For project strategy and current focus, see the planning vault at `core/mev/planning/index.md`.
<!-- freshness proof 1788440499 -->
