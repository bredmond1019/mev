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

### `validate-brain [--sync] [--graph] [--state] [--links] [path]`

Validate the Bastion Brain repo for OKF frontmatter compliance, and optionally check cross-repo sync watermark integrity, global knowledge-graph integrity, state.json schema and block-dependency graph integrity, or link integrity.

```bash
mev validate-brain [--sync] [--graph] [--state] [--links] [path]
```

| Argument / Flag | Default | Description |
|---|---|---|
| `path` | `..` | Path to the company-brain repo root |
| `--sync` | off | Also run the cross-repo sync watermark check (see below) |
| `--graph` | off | Also run the global `scope:doc_id` knowledge-graph integrity check (see below). Takes precedence over `--sync` when both flags are present — `--graph` is a superset. |
| `--state` | off | Also run the `planning/state.json` schema and cross-repo block-dependency graph integrity check (see below). Takes precedence over `--graph` and `--sync` in the dispatch chain. |
| `--links` | off | Also run the link-integrity pass (see below) — flag dead markdown links, broken `file://` URIs, dangling `[[wikilinks]]`, and references to moved/deleted paths. Takes precedence over `--state`, `--graph`, and `--sync` in the dispatch chain. |

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
3. **Schema ring** — checks field validity within each file (kind membership, status enum values, `blocked_by` well-formedness, kind-appropriate sections). In v2 schema files: validates `depends_on[]` entry well-formedness on track blocks, rejects authored `status:"blocked"` (derived, not authored), and validates `backlog[].status` membership.
4. **Graph** — builds the cross-repo block-dependency graph from all loaded files (v2: DAG edges sourced from `tracks[].blocks[].depends_on[]`) and checks it for integrity violations, including cycle detection over the `depends_on` DAG and backlog-node integrity.
5. **Status consistency** — checks that a `closed` block does not depend (via `depends_on`) on a block that is not yet `closed`.
6. **Rollup** — checks that brain `repos[]` headline entries (now/next) match their children's actual `focus` values.
7. **Focus drift** — recomputes the expected `focus` from authored `tracks[]` and warns when the stored `focus` disagrees (warning-only; exit code is unchanged).

`--state` takes precedence over `--graph` and `--sync` in the dispatch chain — when `--state` is present, neither `--graph` nor `--sync` are separately invoked. `--links` takes the highest precedence overall; when `--links` is present, `--state`, `--graph`, and `--sync` are not separately invoked.

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
| `E_STATE_CYCLE` | Error | A `depends_on` edge forms a cycle; the cycle path is named in the message |
| `E_STATE_AUTHORED_BLOCKED` | Error | A `tracks[].blocks[].status` is `"blocked"` — `blocked` is derived, not authored |
| `E_STATE_STATUS_INCONSISTENT` | Error | A `closed` block has a `type:block` `depends_on` target that is not `closed` |
| `E_STATE_DANGLING_PROMOTION` | Error | A `status:"promoted"` backlog node's `block` pointer resolves to no `tracks[]` node |
| `W_STATE_FOCUS_DRIFT` | Warning | Stored `focus` disagrees with the derivation from `tracks[]`; exit code is unchanged |

#### `--links` — link-integrity pass

When `--links` is passed, `mev` runs the full OKF schema pass first, then appends a link-integrity pass:

1. **Extract** — parses every corpus file for markdown `[text](path)` inline links, `file://` URIs, and `[[wikilink]]` references. External links (`http://`, `https://`, `mailto:`, `tel:`, protocol-relative `//`) and pure in-page anchors (`#section`) are unconditionally skipped.
2. **Resolve** — checks each local reference on disk:
   - Relative markdown links are resolved against the referring file's directory.
   - `file://` URIs are resolved to absolute paths.
   - `[[wikilinks]]` are matched against the set of authored `doc_id`s in the corpus.
3. **Moved-reference re-check** — reads `.brain-moves-pending` from the brain root (optional/ephemeral; if missing, no diagnostics are added). Each line is `<ISO-date> <path...>`; the pass flags any corpus reference that still targets a moved or deleted path.

The pass is **read-only** — it never mutates the corpus (D25).

| Locator | Severity | Condition |
|---|---|---|
| `E_LINK_DEAD_MARKDOWN` | Error | A markdown `[text](path)` link's resolved path does not exist on disk |
| `E_LINK_DEAD_FILE_URI` | Error | A `file://` URI's resolved path does not exist on disk |
| `E_LINK_DANGLING_WIKILINK` | Error | A `[[wikilink]]` target slug is not present in the corpus `doc_id` set |
| `E_LINK_MOVED_REFERENCE` | Error | A markdown or `file://` reference still points at a path listed in `.brain-moves-pending` |

Any error-severity diagnostic causes exit 1.

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

# OKF pass + link-integrity check
mev validate-brain --links

# Explicit path with link-integrity check
mev validate-brain --links ~/Dev/agentic-portfolio

# Machine-readable output including link diagnostics
mev --json validate-brain --links ~/Dev/agentic-portfolio
```

---

### `generate-graph [--out] [path]`

Generate an interactive HTML visualization of the Bastion Brain knowledge graph.

```bash
mev generate-graph [--out <dir>] [path]
```

| Argument / Flag | Default | Description |
|---|---|---|
| `path` | `.` | Path to search from when locating `brain.toml` (walks up to find it) |
| `--out` | `<brain_root>/planning/doc-graph` | The output directory to write the graph files (`graph.md` and `graph.html`) to |

Resolves `brain.toml` by walking up from `path`. If no `brain.toml` is found, the process exits 1.

The output is an interactive `vis.js` physics simulation that visualizes all `scope:doc_id` nodes and their `related:` edges across the entire portfolio. It includes color coding by repository scope, node sizing based on connectivity (hub nodes), hover tooltips, and a dynamic search and filtering UI.

**Examples:**

```bash
# Generate the graph in the default location (planning/doc-graph)
mev generate-graph

# Generate the graph from an explicit brain root
mev generate-graph ~/Dev/agentic-portfolio

# Generate the graph to a custom output directory
mev generate-graph --out /tmp/my-graph
```

---

### `manifest [--pretty] [path]`

Emit a JSON manifest of every file in the Brain corpus.

```bash
mev manifest [--pretty] [path]
```

| Argument / Flag | Default | Description |
|---|---|---|
| `path` | `.` | Path to search from when locating `brain.toml` (walks up to find it) |
| `--pretty` | off | Emit pretty-printed (indented) JSON instead of compact JSON |

Resolves `brain.toml` by walking up from `path`. If no `brain.toml` is found, the process
exits 1 with an error message on stderr.

The output is the manifest JSON written directly to stdout — there is no `--json` envelope
wrapper; the output *is* JSON.

#### Output shape

```json
{
  "version": "1",
  "root": "/path/to/brain",
  "entries": [
    {
      "rel": "planning/status.md",
      "scope": "brain",
      "doc_id": "mev-status",
      "doc_type": "ProjectStatus",
      "title": "MEV Status",
      "description": "Current project state for the mev validator.",
      "layer": ["factory"],
      "project": "mev",
      "status": "active",
      "keywords": ["mev", "status", "validator"]
    },
    {
      "rel": "README.md",
      "scope": "brain",
      "doc_id": null,
      "doc_type": null,
      "title": null,
      "description": null,
      "layer": null,
      "project": null,
      "status": null,
      "keywords": null
    }
  ]
}
```

| Field | Type | Description |
|---|---|---|
| `version` | string | Schema version — currently `"1"` |
| `root` | string | Display path of the HQ root used for the crawl |
| `entries` | array | All corpus files, in walk order |
| `entries[].rel` | string | Path relative to the HQ crawl root (forward-slash separated) |
| `entries[].scope` | string | Stable scope slug of the owning registry unit (e.g. `"brain"`, `"mev"`) |
| `entries[].doc_id` | string \| null | OKF `doc_id` field; `null` when not present in frontmatter |
| `entries[].doc_type` | string \| null | OKF `type` field (serialized as `doc_type`); `null` when absent |
| `entries[].title` | string \| null | OKF `title` field; `null` when absent |
| `entries[].description` | string \| null | OKF `description` field; `null` when absent |
| `entries[].layer` | array \| null | OKF `layer` field (closed-set list); `null` when absent |
| `entries[].project` | string \| null | OKF `project` field; `null` when absent |
| `entries[].status` | string \| null | OKF `status` field; `null` when absent |
| `entries[].keywords` | array \| null | OKF `keywords` field (3–7 free-form terms); `null` when absent |

Files without parseable frontmatter appear in the manifest with all metadata fields set to
`null` (graceful degradation — the OKF validator reports the error separately).

#### Exit codes

| Code | Meaning |
|---|---|
| `0` | Manifest emitted successfully |
| `1` | `brain.toml` not found, or a runtime error prevented crawl completion |

**Examples:**

```bash
# Compact JSON from the current directory
mev manifest

# Compact JSON from an explicit brain root
mev manifest ~/Dev/agentic-portfolio

# Pretty-printed JSON
mev manifest --pretty

# Pretty-printed JSON from an explicit brain root
mev manifest --pretty ~/Dev/agentic-portfolio

# Pipe compact JSON into jq
mev manifest | jq '.entries | length'
```

---

### `emit-state [--write] [path]`

Regenerate all derived views in the Brain corpus from the authored `tracks[]` DAG and write them in place (with `--write`) or report what would change (dry-run, without `--write`).

`mev emit-state` is the **single derivation engine** that `/log-work` shells out to for regenerating leaf `focus` fields, the brain `repos[]` / `cross_repo[]` rollup, brain `focus`, and the master-plan wave/dependency tables. Because the validator's `check_focus_drift` and `check_rollup` share the same `derive_focus` / `derive_rollup` functions, running `mev emit-state --write` followed by `mev validate-brain --state` on the same corpus will report zero `W_STATE_FOCUS_DRIFT` and zero `W_STATE_ROLLUP_DRIFT` — the emit is, by construction, the fixed point of the drift check.

```bash
mev emit-state [--write] [path]
```

| Argument / Flag | Default | Description |
|---|---|---|
| `path` | `.` | Path to search from when locating `brain.toml` (walks up to find it) |
| `--write` | off | Write the derived views in place. Without this flag the command is a dry-run |

#### Derived views updated

- **Leaf `state.json`** (`kind == "project"`): regenerates `focus` — `now` = blocks with `status: in_progress`; `next` = ready open blocks in `wave` order; `blocked` = open blocks with an unmet `depends_on`, each carrying the unmet `blocked_by[]` subset. Authored `tracks[]` and all other fields survive the round-trip unchanged.
- **Brain `state.json`** (`kind == "brain"`): regenerates `repos[]`, `cross_repo[]` (cross-repo `depends_on` edges), and the brain file's own `focus`. Authored `tracks[]`, `backlog[]`, and `tiers[]` are left untouched.
  - `repos[]` is **tier-scoped**: a brain file whose `repo` slug matches a `tier` value in `brain.toml` (e.g. `core`) scopes to only that tier's `[[repos]]`; a brain file whose `repo` matches no tier (the HQ root) scopes to every repo. See `tier_scope_for`.
  - `repos[]` is **non-destructive**: for each in-scope repo, if a loadable child `state.json` exists, its headline is derived as before (`RepoRollup.tier` populated from config); if not, but the brain file already carries a `repos[]` entry for that slug, the entry is **preserved verbatim** (with `tier` backfilled); only when neither exists is a tier-tagged empty stub emitted. A malformed or not-yet-authored child `state.json` can therefore never silently drop a repo out of the rollup.
  - `focus.now/next/blocked` is derived as the **repo-tagged union** of the in-scope children's own derived `focus` (each block carries its source `repo`), in config-repo order then within-child order, deduplicated by `(repo, id)`. Repos with no loadable child contribute nothing to `focus` (they still surface in `repos[]` via the preserve/stub branch).
- **`master-plan.md` wave tables**: splices a rendered wave/dependency Markdown table between the `<!-- BEGIN generated:wave-table -->` and `<!-- END generated:wave-table -->` sentinels. All narrative lines outside the sentinels are preserved verbatim. Re-running the emit is idempotent — if the splice produces no change, no `EmitAction` is recorded.

#### Sentinel contract

The sentinel pair format is:

```markdown
<!-- BEGIN generated:wave-table -->
| Wave | Block | Title | Status | Depends on |
| --- | --- | --- | --- | --- |
... (generated rows) ...
<!-- END generated:wave-table -->
```

- Both `BEGIN` and `END` sentinels must be present and balanced; a missing or unbalanced pair causes a `W_EMIT_NO_SENTINEL` warning and the file is skipped — sentinels are never invented into arbitrary prose.
- Re-splicing an already-emitted table is idempotent.

#### Diagnostic codes

| Locator | Severity | Condition |
|---|---|---|
| `W_EMIT_DRY_RUN` | Warning | Planned action (dry-run only; no file written) |
| `I_EMIT_WROTE` | Warning | File written (`--write` mode) |
| `W_EMIT_NO_SENTINEL` | Warning | `master-plan.md` is missing the `wave-table` sentinel pair; file skipped |
| `E_EMIT_WRITE_FAILED` | Error | IO error writing a file; causes exit 1 |
| `E_CONFIG_NOT_FOUND` | Error | `brain.toml` could not be located by walking up from `path`; causes exit 1 |

**Examples:**

```bash
# Dry-run from the current directory (reports planned changes, writes nothing)
mev emit-state

# Dry-run from an explicit brain root
mev emit-state ~/Dev/agentic-portfolio

# Write derived views in place
mev emit-state --write

# Write derived views from an explicit brain root
mev emit-state --write ~/Dev/agentic-portfolio

# Machine-readable dry-run output
mev --json emit-state

# Machine-readable write output
mev --json emit-state --write ~/Dev/agentic-portfolio
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
