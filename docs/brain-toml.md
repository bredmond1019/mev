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

The remaining fields are consumed by `mev validate-brain --sync` to check cross-repo sync watermarks.

| Key | Type | Required | Description |
|---|---|---|---|
| `slug` | string | yes | Short identifier; drives the `project` vocabulary |
| `tier` | string | no | Classification, e.g. `"primary"`, `"secondary"` |
| `repo_path` | string | no | Path relative to the brain root |
| `status_file` | string | no | Path (relative to brain root) to the sub-repo's status file; must contain a `timestamp` RFC3339 scalar in its frontmatter — consumed by `--sync` |
| `cache_doc` | string | no | Path (relative to brain root) to the brain cache doc for this repo; must contain a `synced_from` RFC3339 scalar in its frontmatter — consumed by `--sync` |
| `heading` | string | no | Heading used in the brain README quick-status table |
| `prefix` | string | no | Short block-ID prefix for this repo (e.g. `"MV"`, `"HQ"`) — used to resolve bare/prefix-stripped spec-folder names against this entry's blocks |

---

## `[history]`

Controls the append-only revision-history writer that `apply_plan()` (the single write point behind `emit-state --write`, the doc materializer, the Opportunity family, and the index reconciler) runs before every overwrite. See `mev state-history` for the read-back CLI.

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `true` | Whether a file's prior content is snapshotted before it is overwritten. |
| `keep` | integer | `10` | Maximum revisions retained per file; the oldest are pruned once this cap is exceeded. |

```toml
[history]
enabled = true
keep    = 10
```

An absent `[history]` table is equivalent to the example above — history is on, capped at 10 revisions per file.

Setting `enabled = false` disables snapshotting entirely. The write itself stays atomic (temp file + rename) either way; what you lose is the recovery path — a bad derived write (a rollup that silently drops a repo, a materializer regression, etc.) becomes unrecoverable instead of restorable via `mev state-history --restore`.

---

## `[carryover]`

Block-level startability enforcement knob for `carryover[].blocks[]` edges (`MV.16.C`). When on, a
block named by a carryover entry's `blocks[]` is held out of `next`/the frontier even if its own
`depends_on` is fully met — see `docs/cli.md`'s `[carryover]` subsection for the full mechanism
(the three escape hatches, the cap-exceeded reporting, and `--would-block`'s enforcement header).

| Key | Type | Default | Description |
|---|---|---|---|
| `enforce_blocks` | bool | `false` | Whether `carryover[].blocks[]` edges actually hold their target block out of readiness. Off by default; flipping it on for the real corpus is a separate operator decision (HQ.7.C), not this section landing. |
| `max_gates_per_repo` | integer | `10` | Cap on how many carryover-sourced gates apply per target repo per derivation pass; excess candidates are reported (`cap_exceeded`), never silently applied. |

```toml
[carryover]
enforce_blocks     = false
max_gates_per_repo = 10
```

An absent `[carryover]` table is equivalent to the example above — enforcement off, cap of 10.

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
