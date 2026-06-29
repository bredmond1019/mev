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

## Synopsis

```
mev [--json] <subcommand> [args]
```

## Global flags

| Flag | Description |
|---|---|
| `--json` | Emit a machine-readable JSON envelope to stdout instead of the human summary. Exit code behaviour is unchanged — exit 1 on any error-severity diagnostic. |

## Subcommands

### `validate [path]`

Validate the learn-ai content tree.

```bash
mev validate [path]
```

| Argument | Default | Description |
|---|---|---|
| `path` | `../learn-ai/content/learn` | Path to the content root |

Checks each file in the tree against the learn-ai frontmatter schema and JSON struct constraints (`LearnAiValidator`).

**Examples:**

```bash
# Default path
mev validate

# Explicit path
mev validate ~/Dev/learn-ai/content/learn

# Machine-readable output
mev --json validate
```

---

### `validate-brain [--sync] [--graph] [--state] [path]`

Validate the Bastion Brain repo for OKF frontmatter compliance, and optionally check cross-repo sync watermark integrity, global knowledge-graph integrity, or state.json schema and block-dependency graph integrity.

```bash
mev validate-brain [--sync] [--graph] [--state] [path]
```

| Argument / Flag | Default | Description |
|---|---|---|
| `path` | `..` | Path to the company-brain repo root |
| `--sync` | off | Also run the cross-repo sync watermark check (see below) |
| `--graph` | off | Also run the global `scope:doc_id` knowledge-graph integrity check (see below). Takes precedence over `--sync` when both flags are present — `--graph` is a superset. |
| `--state` | off | Also run the `planning/state.json` schema and cross-repo block-dependency graph integrity check (see below). Takes precedence over `--graph` and `--sync` in the dispatch chain. |

Resolves `brain.toml` by walking up from `path`. If no `brain.toml` is found, a fatal `Error`-severity diagnostic with locator `E_CONFIG_NOT_FOUND` is emitted and the process exits 1.

See [`brain.toml` config](brain-toml.md) for the configuration format and [OKF schema](okf-schema.md) for what is validated.

#### `--sync` — cross-repo watermark check

When `--sync` is passed, `mev` runs the full OKF schema pass first, then appends a second pass that compares watermarks for every `[[repos]]` entry in `brain.toml`:

- Reads `timestamp` from `<repo_path>/<status_file>` (the sub-repo's status file).
- Reads `synced_from` from `<cache_doc>` (the brain cache doc for that repo).
- Both values must be present and valid RFC3339 datetimes; they must be identical.

A mismatch or missing watermark emits an `Error`-severity diagnostic and causes exit 1.

| Locator | Condition |
|---|---|
| `E_SYNC_FILE_MISSING` | `status_file` or `cache_doc` does not exist, or cannot be read |
| `E_SYNC_WATERMARK_MISSING` | `timestamp` or `synced_from` field is absent from the frontmatter |
| `E_SYNC_WATERMARK_MALFORMED` | A watermark is present but is not a valid RFC3339 datetime |
| `E_SYNC_DRIFT` | Both watermarks parse successfully but their values differ |

#### `--graph` — knowledge-graph integrity check

When `--graph` is passed, `mev` runs the full OKF schema pass first, then appends a graph integrity pass:

1. Crawls the corpus (same registry-driven walk as the OKF pass).
2. Builds the global `scope:doc_id` knowledge graph from frontmatter (`doc_id` + `related` fields).
3. Runs integrity checks over the built graph.

Files with a `doc_id` become graph nodes; files without one are leaves. All `related:` entries are resolved as either bare `doc_id` refs (resolved within the from-node's scope first) or qualified `scope:doc_id` refs.

| Locator | Severity | Condition |
|---|---|---|
| `E_GRAPH_DUPLICATE_DOC_ID` | Error | Two corpus files in the same scope share a `doc_id`. |
| `related` | Error | A `related:` entry resolves to no node and no leaf (dangling). |
| `related` | Warning | A `related:` entry resolves to a corpus file that has no `doc_id` (leaf target). |

Graph errors (`E_GRAPH_DUPLICATE_DOC_ID`, dangling `related:`) cause exit 1. The leaf-target warning alone does not change the exit code.

`--graph` takes precedence over `--sync` when both flags are given — it is a superset (runs the OKF schema pass that `--sync` also runs, plus the graph pass).

#### `--state` — state.json schema and block-dependency graph check

When `--state` is passed, `mev` runs the full OKF schema pass first, then appends the state-validation pipeline:

1. **Discovery** — finds all `planning/state.json` files: the HQ brain state, each tier sub-brain state (via `tiers[].rollup` in the HQ state), and each leaf project state (via `[[repos]]` in `brain.toml`). Missing files emit `W_STATE_FILE_MISSING`.
2. **Load** — deserializes each discovered file. Unparseable files emit `E_STATE_MALFORMED_JSON`.
3. **Schema ring** — checks field validity within each file (kind membership, status enum values, blocked_by well-formedness, kind-appropriate sections).
4. **Graph** — builds the cross-repo block-dependency graph from all loaded files and checks it for integrity violations.
5. **Rollup** — checks that brain `repos[]` headline entries (now/next) match their children's actual `focus` values.

`--state` takes the highest precedence in the dispatch chain — when `--state` is present, `--graph` and `--sync` are not separately invoked.

| Locator | Severity | Condition |
|---|---|---|
| `W_STATE_FILE_MISSING` | Warning | A registered repo has no `planning/state.json` |
| `E_STATE_MALFORMED_JSON` | Error | A state.json file is not valid JSON or does not match the expected schema |
| `E_STATE_SCHEMA_BAD_KIND` | Error | `kind` is not one of `project` or `brain` |
| `E_STATE_SCHEMA_MISSING_FIELD` | Error/Warning | A required field is absent or a kind-appropriate section is missing |
| `E_STATE_SCHEMA_BAD_STATUS` | Error | A `status` value is not in the allowed enum |
| `E_STATE_SCHEMA_BAD_BLOCKED_BY` | Error | A `blocked_by[]` entry has an unknown or malformed `type` |
| `E_STATE_DUPLICATE_BLOCK_ID` | Error | Two `tracks[]` blocks in the same repo share an `id` |
| `E_STATE_DANGLING_FOCUS` | Error | A leaf repo focus entry's `block` is absent from `tracks[]` |
| `E_STATE_UNKNOWN_REPO` | Error | A `blocked_by` or `cross_repo` edge names an unknown repo |
| `E_STATE_DANGLING_BLOCKED_BY` | Error | A cross-repo block dependency's block does not exist in the named repo |
| `E_STATE_DANGLING_CROSS_REPO` | Error | A brain `cross_repo[]` edge's endpoint does not resolve to a known block |
| `W_STATE_ROLLUP_DRIFT` | Warning | Brain `repos[]` headline differs from the child repo's actual `focus` |

**Examples:**

```bash
# Default: validates OKF frontmatter in the sibling brain repo at ..
mev validate-brain

# OKF pass + sync watermark check
mev validate-brain --sync

# Explicit path with sync check
mev validate-brain --sync ~/Dev/agentic-portfolio

# OKF pass + knowledge-graph integrity check
mev validate-brain --graph

# Explicit path with graph check
mev validate-brain --graph ~/Dev/agentic-portfolio

# OKF pass + state.json schema and block-dependency graph check
mev validate-brain --state

# Explicit path with state check
mev validate-brain --state ~/Dev/agentic-portfolio

# Machine-readable output (consumed by the Brain RAG indexer)
mev --json validate-brain ~/Dev/agentic-portfolio

# Machine-readable output including sync diagnostics
mev --json validate-brain --sync ~/Dev/agentic-portfolio

# Machine-readable output including graph diagnostics
mev --json validate-brain --graph ~/Dev/agentic-portfolio

# Machine-readable output including state diagnostics
mev --json validate-brain --state ~/Dev/agentic-portfolio
```

---

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
