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

## What this page is for

`brain.toml` is the one config file for the whole corpus. It says which repos exist, which words
are legal in which frontmatter field, what to skip while crawling, and when a stale item should
start nagging. If a validation result surprises you, the answer is usually here.

## Quickstart

Run these in a **terminal**:

```bash
# Which brain.toml is actually in force? The first one found walking up wins.
ls brain.toml || (cd .. && ls brain.toml)

# Change a threshold, then see what moved
mev attention-queue
```

There is no `mev config` command — you edit the file by hand and re-run whichever command reads it.
`validate-brain` resolves it by walking up from the target root.

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

## `[attention]`

Two different things share this table: the per-`kind` **staleness thresholds** that decide when an
item is surfaced on the Attention board at all, and the **notification policy** that decides which
of the surfaced items may interrupt a human. They are separate axes — an item can be stale (and so
on the board) without being interruptible.

### Staleness thresholds

A `carryover[]` or `backlog[]` item is stale — surfaced on the Attention board and via the
`W_STATE_*_STALE` warnings — once its age exceeds the threshold for its kind, unless it is
snoozed. Both the validator and the emit planner read this one struct, so the board shows exactly
what the warnings fire on.

| Key | Type | Default | Description |
|---|---|---|---|
| `env_days` | integer | `3` | `carryover` kind `env` — transient environmental caveats. |
| `deferred_days` | integer | `5` | `carryover` kind `deferred` — unticketed follow-ons. |
| `defect_days` | integer | `10` | `carryover` kind `defect`. |
| `drift_days` | integer | `10` | `carryover` kind `drift`. |
| `known_issue_days` | integer | `10` | Retired kind (D72); still read so legacy entries round-trip. |
| `constraint_days` | integer | `10` | Retired kind (D72); same. |
| `backlog_days` | integer | `7` | `backlog[]` rows. |

An unrecognised kind falls back to the longest carryover threshold, so a novel kind surfaces but
not eagerly.

### Notification policy (`MV.ticket.attention-notify-policy`)

Decides which Attention items may **interrupt** the operator; everything else waits for a digest.
Read by `mev attention-queue --notify-only`, and by any consumer that would otherwise re-derive the
cut — `bastion`'s operator queue calls it rather than re-implementing it, because a re-derived cut
diverges the phone from `/attention`.

| Key | Type | Default | Description |
|---|---|---|---|
| `notify_lanes` | list | `["blocking", "hot"]` | Triage lanes eligible to interrupt. |
| `notify_priority_floor` | integer | `0` | Within `hot` only, the highest priority number still eligible. `0` is hottest, so the default is P0-only. |
| `notify_blocking_any_priority` | bool | `true` | `blocking` interrupts regardless of priority — including an unset one. |
| `digest_everything_else` | bool | `true` | Everything not selected above goes to the daily digest. |

```toml
[attention]
env_days      = 3
deferred_days = 5
backlog_days  = 7

notify_lanes                 = ["blocking", "hot"]
notify_priority_floor        = 0
notify_blocking_any_priority = true
digest_everything_else       = true
```

**The defaults fail closed.** An absent `[attention]` table, or one carrying no policy keys at all,
yields the documented rule — *blocking at any priority, plus hot at P0* — never notify-everything.
That direction matters: measured 2026-08-25 the board held **415** items and the interrupt set was
**7**, so a policy that failed open would deliver hundreds of notifications on its first run and be
muted the same day.

**It is not a priority floor applied across all lanes.** Two cases distinguish the rule, and both
are pinned by tests: a `blocking` item at **P3 is included**, and a `hot` item at **P1 is
excluded**. A single `priority <= floor` test over every lane gets both wrong.

**Sizing note:** size this policy off the *arrival rate*, not the standing count. `hot` membership
is authored priority 0 or 1 with no age component, so it is a stock that never drains into `aging` —
the standing total is a number that structurally cannot go down. Between 2026-08-24 and 2026-08-25
the board grew 395 -> 415 while the interrupt set stayed at 7.

**Unknown keys in this table are silently ignored** — `AttentionThresholds` does not set
`serde(deny_unknown_fields)`. A misspelled key therefore does nothing and reports nothing; check
the spelling against the tables above rather than trusting that a key took effect.

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
