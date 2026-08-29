---
type: Reference
title: mev CLI Reference
description: Full reference for the mev command-line interface — subcommands, flags, defaults, exit codes, and output formats
doc_id: cli-reference
layer: [factory]
project: mev
status: active
keywords: [cli, validate, validate-brain, json, exit-codes, mev]
related: [architecture, brain-toml-config, okf-schema]
---
# mev CLI Reference

`mev` is a Rust CLI that validates the Bastion Brain corpus and derives every generated surface
from it. **This page is the catalogue** — every command, one line each, and where its detail lives.

## What this page is for

You want to know what `mev` can do, or you know what you want and need the command for it. Start
here, then follow the link into the domain page for flags, diagnostics and examples.

If you are new to the system: the Brain is a set of markdown and `state.json` files across ~16
repos. `mev` reads them, tells you what is malformed, and regenerates the boards, caches and
rollups that are computed from them. See [Architecture](architecture.md) for how it is built.

## Quickstart

Run these in a **terminal** (not Claude Code). Every command walks up to find `brain.toml` itself,
so you can run them from anywhere inside the brain.

```bash
# Is the corpus healthy? One flag per run -- they do NOT compose.
bastion validate-brain --state

# What is open across the fleet?
mev carryover

# What could I start right now?
mev frontier

# Regenerate every derived surface (writes fleet-wide -- see the warning below)
mev emit-state --write
```

| Before you start | If it is missing |
|---|---|
| Rust 1.95+ | `rustup show` |
| `mev` on `PATH` | `cargo install --path .` from `core/mev` |
| A `brain.toml` above you | You are outside the brain; `cd` into it |

**Three traps that produce a confident wrong answer**, all of them measured here:

- **`validate-brain`'s flags do not compose.** It is an if/else-if chain; the first flag wins and
  the rest are silently ignored. One flag per invocation.
- **A piped exit code is the pipe's.** `mev conformance | tail` prints success while `mev
  conformance` exits 1. Redirect to a file, then read `$?`.
- **Every `--write` verb rewrites the whole corpus**, not just your repo, and re-runs the full
  derivation internally. A stale installed binary rewrites surfaces in an old format. Install
  first; commit immediately after.

## The 27 commands

Derived from the CLI dispatch in [`src/main.rs`](../src/main.rs), not from these docs — if a
command exists and is missing here, this table is wrong.

### Check something

| Command | What it does | Detail |
|---|---|---|
| `validate` | Validate the learn-ai content tree | [validate](cli/validate.md) |
| `validate-brain` | Validate the whole Brain corpus for OKF compliance | [validate](cli/validate.md) |
| `validate-state` | Validate a single `state.json` | [validate](cli/validate.md) |
| `conformance` | Check facts kept in two places still agree | [validate](cli/validate.md) |

`validate-state`'s diagnostic codes (including `W_STATE_SDLC_WORKFLOW_MISSING`, a warning for a
block with no `sdlc_workflow` field — never blocking, paired asymmetrically with the error
`E_STATE_SDLC_WORKFLOW_ENUM`) are catalogued on [validate](cli/validate.md), not here.
| `check-consumers` | Compile every repo that path-depends on this working tree | [lanes](cli/lanes.md) |

### Derive and write state

| Command | What it does | Detail |
|---|---|---|
| `emit-state` | Regenerate every derived surface from authored state | [state](cli/state.md) |
| `state-history` | List or restore the revisions a write recorded | [state](cli/state.md) |
| `set-block-status` | Flip one block's authored status, then re-derive | [state](cli/state.md) |
| `manifest` | Emit a JSON manifest of every file in the corpus | [state](cli/state.md) |

### Move an initiative, or clear a human gate

| Command | What it does | Detail |
|---|---|---|
| `defer-epic` | Park an epic and cascade `deferred` to its blocks | [epics](cli/epics.md) |
| `resume-epic` | Un-park an epic and return its blocks | [epics](cli/epics.md) |
| `complete-epic` | Declare an initiative finished | [epics](cli/epics.md) |
| `sync-epics` | Reconcile epic registry against blocks, both directions | [epics](cli/epics.md) |
| `close-operator-gate` | Clear a human-work gate fleet-wide | [epics](cli/epics.md) |
| `approve` | Approve a pending decision gate on a fixed payload | [epics](cli/epics.md) |
| `reject` | Reject a pending decision gate | [epics](cli/epics.md) |
| `normalize-op-slugs` | Fix stuttering operator/approval slugs fleet-wide | [epics](cli/epics.md) |

### Findings and attention

| Command | What it does | Detail |
|---|---|---|
| `carryover` | Sweep every repo's open findings; audit, dispose, archive | [carryover](cli/carryover.md) |
| `graph-findings` | Detect findings mechanically instead of by noticing | [carryover](cli/carryover.md) |
| `attention-queue` | Emit the stale items that need the operator | [carryover](cli/carryover.md) |

### Graphs and scheduling

| Command | What it does | Detail |
|---|---|---|
| `frontier` | Print every block that is startable right now | [lanes](cli/lanes.md) |
| `lanes` | Lane-segment availability plus unblock leverage | [lanes](cli/lanes.md) |
| `blocks` | Filtered block queries, the transitive leverage cone, the same-repo chain | [lanes](cli/lanes.md) |
| `emit-block-graph` | Emit the block-dependency graph as JSON | [lanes](cli/lanes.md) |
| `emit-graph` | Emit the `scope:doc_id` knowledge graph as JSON | [lanes](cli/lanes.md) |
| `generate-graph` | Render that graph as a browsable HTML page | [lanes](cli/lanes.md) |
| `doc` | Materialize brain documents; manage Opportunity records | [lanes](cli/lanes.md) |

## Detail pages

| Page | Covers |
|---|---|
| [validate](cli/validate.md) | `validate` · `validate-brain` · `validate-state` · `conformance` |
| [state](cli/state.md) | `emit-state` · `state-history` · `set-block-status` · `manifest` |
| [epics](cli/epics.md) | the four `*-epic` verbs · `close-operator-gate` · `approve`/`reject` · `normalize-op-slugs` |
| [carryover](cli/carryover.md) | `carryover` · `graph-findings` · `attention-queue` |
| [lanes](cli/lanes.md) | `frontier` · `lanes` · `blocks` · `emit-block-graph` · `emit-graph` · `generate-graph` · `check-consumers` · `doc` |


## Synopsis

```
mev [--json] <subcommand> [args]
```

## Global flags

| Flag | Description |
|---|---|
| `--json` | Emit a machine-readable JSON envelope to stdout instead of the human summary. Exit code behaviour is unchanged — exit 1 on any error-severity diagnostic. |
| `--build-stamp` | Print this binary's build provenance — `{"git_sha": <string>, "dirty": <bool \| "unknown">, "source_dir": <string>}` — as one JSON line to stdout, then exit immediately, before tracing init and before any subcommand runs. `dirty` is a JSON boolean when the compiled-in stamp is `"0"`/`"1"`, or the literal string `"unknown"` otherwise — never guessed. This is the cross-binary contract `toolchain-freshness` uses to query every registered writer named in `toolchain::CROSS_BINARY_WRITERS` (today: `bastion`, whose `src/buildstamp.rs` implements the same shape) — do not add, rename, or drop a key from this output. |

```bash
$ mev --build-stamp
{"git_sha":"a1b2c3d","dirty":false,"source_dir":"/Users/brandon/Dev/agentic-portfolio/core/mev"}
```

## Exit codes

| Code | Meaning |
|---|---|
| `0` | All checks passed (zero error-severity diagnostics) |
| `1` | One or more error-severity diagnostics found, or an unrecoverable runtime error |

Warning-severity diagnostics are reported but do not change the exit code.

---

## Human output format

Without `--json`, `mev` prints a single summary line:

```
validated <path>: <N> error(s), <M> warning(s)
```

Diagnostics are not individually printed in human mode yet — use `--json` to get the full list.

---

## JSON output format (`--json`)

The `--json` flag emits a `JsonReport` envelope:

```json
{
  "validator": "brain",
  "root": "/path/to/repo",
  "errors": 2,
  "warnings": 1,
  "diagnostics": [
    {
      "severity": "error",
      "file": "docs/projects/foo.md",
      "locator": "type",
      "message": "required field 'type' is missing or empty"
    }
  ]
}
```

| Field | Type | Description |
|---|---|---|
| `validator` | string | `"brain"` or `"learn-ai"` |
| `root` | string | Display path of the validated root |
| `errors` | number | Count of error-severity diagnostics |
| `warnings` | number | Count of warning-severity diagnostics |
| `diagnostics` | array | All diagnostics emitted during the run |
| `diagnostics[].severity` | `"error"` \| `"warning"` | Severity level |
| `diagnostics[].file` | string | File path (relative to root where possible) |
| `diagnostics[].locator` | string | In-file locator (e.g. `"type"`, `"layer[0]"`) or `""` for whole-file findings |
| `diagnostics[].message` | string | Human-readable description of the finding |
