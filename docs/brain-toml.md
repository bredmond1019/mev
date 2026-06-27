---
type: Reference
title: brain.toml Config Reference
description: Full schema reference for brain.toml — the corpus config file that drives the Brain OKF validator
doc_id: brain-toml-config
layer: [brain, factory]
project: mev
status: active
keywords: [brain.toml, config, corpus, vocab, crawl, repos, OKF]
related: [cli-reference, okf-schema, architecture]
---

# `brain.toml` Config Reference

`brain.toml` is the corpus config file for the Bastion Brain repo. `mev validate-brain` resolves it by walking up from the target root — the first `brain.toml` found wins.

It controls three things:
- **Controlled vocabularies** (`[vocab]`) — the closed sets for `layer` and `status` OKF fields
- **Crawl skip list** (`[crawl]`) — directories pruned during the walk
- **Project registry** (`[[repos]]`) — the valid `project` slug values + metadata for future sync

---

## Full example

```toml
[vocab]
layer  = ["brain", "engine", "factory", "console", "surface", "infra", "business", "content", "meta"]
status = ["active", "draft", "deprecated", "superseded", "archived"]

[crawl]
skip_dirs = ["target", "node_modules", ".git", ".claude", ".repo-backups", ".agent"]

[[repos]]
slug        = "brain"
tier        = "primary"
repo_path   = "."
status_file = "planning/status.md"
cache_doc   = "docs/projects/brain.md"
heading     = "Brain"

[[repos]]
slug        = "mev"
tier        = "primary"
repo_path   = "core/mev"
status_file = "planning/status.md"
cache_doc   = "docs/projects/mev.md"
heading     = "mev"
```

---

## `[vocab]`

Defines the closed sets used by the OKF validator. If a field is absent, the corresponding vocabulary defaults to empty — every value will fail validation.

| Key | Type | Description |
|---|---|---|
| `layer` | `string[]` | Valid values for the OKF `layer` field. Each `.md` file's `layer` list is checked against this set. |
| `status` | `string[]` | Valid values for the OKF `status` field. |

Both keys default to `[]` if the `[vocab]` section is absent.

---

## `[crawl]`

Controls which directories are pruned during the Markdown walk.

| Key | Type | Default | Description |
|---|---|---|---|
| `skip_dirs` | `string[]` | `[]` | Directory names (or relative paths) to skip entirely. |

### Skip entry formats

Two formats are supported:

**Name match** — prunes any directory with that leaf name, anywhere in the tree:
```toml
skip_dirs = ["target", "node_modules", ".git"]
```

**Path match** — prunes a directory only at a specific relative path from the root:
```toml
skip_dirs = ["planning/archive"]
```
`planning/archive` is skipped but `docs/archive` is not.

### Built-in skip rules (not configurable)

In addition to `skip_dirs`, the crawler always applies these rules regardless of config:

- **Nested-git rule** — any directory at depth > 0 containing its own `.git` entry is pruned (skips sub-project repos nested inside the brain root)
- **File blocklist** — individual files named `CLAUDE.md`, `CLAUDE.local.md`, `GEMINI.md`, or `handoff.md` are never validated (tool config and transient session artifacts)

---

## `[[repos]]`

An array of repo entries. The `slug` field of each entry becomes a valid value for the OKF `project` field — any `.md` file with `project: foo` where `foo` is not a known slug gets an error diagnostic.

The remaining fields are metadata reserved for future `--sync` functionality (Block N).

| Key | Type | Required | Description |
|---|---|---|---|
| `slug` | string | yes | Short identifier; drives the `project` vocabulary |
| `tier` | string | no | Classification, e.g. `"primary"`, `"secondary"` |
| `repo_path` | string | no | Path relative to the brain root |
| `status_file` | string | no | Path to the status file within the repo |
| `cache_doc` | string | no | Path to the brain cache doc for this repo |
| `heading` | string | no | Heading used in the brain README quick-status table |

---

## Lookup order

`find_brain_config(root)` walks up from `root`, checking for `brain.toml` at each level:

```
<root>/brain.toml          ← checked first
<root>/../brain.toml
<root>/../../brain.toml
…
```

The first file found is parsed and returned. If no `brain.toml` is found before the filesystem root, a fatal `E_CONFIG_NOT_FOUND` diagnostic is emitted (exit 1).

---

## Defaults when sections are absent

All three top-level sections are optional. An empty `brain.toml` (or no file at all once found) is valid TOML but produces empty vocabularies, meaning every controlled-vocab field will fail OKF validation. This is intentional: a misconfigured or missing vocab is a configuration error surfaced as diagnostics rather than a silent pass.
