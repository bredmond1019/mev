---
type: Reference
title: OKF Frontmatter Schema
description: Field-by-field reference for the OKF YAML frontmatter schema validated by mev validate-brain
doc_id: okf-schema
layer: [brain, factory]
project: mev
status: active
keywords: [OKF, frontmatter, schema, validation, YAML, brain, mev]
related: [brain-toml-config, cli-reference, architecture]
---

# OKF Frontmatter Schema

Every `.md` file in the Bastion Brain repo must open with a YAML frontmatter block validated by `mev validate-brain`. This document describes every field, its constraints, and the diagnostic each violation produces.

The governing decision is **D27** (company-brain `docs/decisions/`).

---

## Frontmatter block format

```yaml
---
type: Decision
title: My Decision Title
description: One-line summary written for a searcher
doc_id: my-decision-id
layer: [brain, meta]
project: mev
status: active
keywords: [okf, validation, mev, frontmatter, decisions]
related: [context, master-plan]
---
```

The block must be delimited by `---` on its own line at the top of the file. Any file missing the opening or closing delimiter gets a single `Error` diagnostic at locator `frontmatter`.

---

## Fields

### `type` — Required

Free-form string. Describes the document kind.

| | |
|---|---|
| Required | yes |
| Violation | `Error` at locator `type` |

Common values: `Decision`, `Index`, `Plan`, `Reference`, `Log`, `ProjectStatus`, `LocalContext`, `Strategy`, `Guideline`.

---

### `title` — Required

Human-readable title.

| | |
|---|---|
| Required | yes |
| Violation | `Error` at locator `title` |

---

### `description` — Required

One-line summary written for a searcher. Should answer "what will I find in this file?"

| | |
|---|---|
| Required | yes |
| Violation | `Error` at locator `description` |

---

### `doc_id` — Optional

Stable kebab-case identifier. Defaults to the filename stem if absent (absence is not an error).

| | |
|---|---|
| Required | no |
| Format | kebab-case (`my-stable-id`) **or** decision-id format (`D<N>` or `D<N>-kebab-suffix`) |
| Violation | `Error` at locator `doc_id` if present but format is invalid |

Decision-id examples: `D7`, `D29`, `D15-okf-lowercase-doc-names`.

---

### `layer` — Optional, controlled vocab

Closed-set list. Each value must be in the `vocab.layer` list from `brain.toml`.

| | |
|---|---|
| Required | no |
| Type | `string[]` (YAML list) |
| Valid values | Defined in `brain.toml` `[vocab].layer`; canonical set: `brain · engine · factory · console · surface · infra · business · content · meta` |
| Violation | `Error` at locator `layer[N]` for each invalid value |

A file may omit `layer` entirely (cross-cutting docs). Using a bare scalar instead of a list (`layer: brain`) is a YAML parse error at `frontmatter`.

---

### `project` — Optional, controlled vocab

Closed-set scalar. Must match one of the `slug` values in `brain.toml`'s `[[repos]]` entries. Omit for genuinely cross-cutting docs.

| | |
|---|---|
| Required | no |
| Valid values | Any `slug` in `[[repos]]` in `brain.toml` |
| Violation | `Error` at locator `project` if present but value is not a known slug |

---

### `status` — Optional, controlled vocab

Closed-set scalar. Must be in the `vocab.status` list from `brain.toml`.

| | |
|---|---|
| Required | no |
| Valid values | Defined in `brain.toml` `[vocab].status`; canonical set: `active · draft · deprecated · superseded · archived` |
| Violation | `Error` at locator `status` if present but value is not in the vocab |

---

### `keywords` — Optional, count-checked

Free-form topic terms. Not validated for content, only for count.

| | |
|---|---|
| Required | no |
| Type | `string[]` |
| Count range | 3–7 (inclusive) |
| Violation | `Warning` at locator `keywords` if present and count is < 3 or > 7 |

Absence is not flagged. A list of 3–7 terms is clean.

---

### `related` — Optional, tolerated

List of `doc_id` values this file depends on or cross-references. Tolerated but not validated (no check for referential integrity).

| | |
|---|---|
| Required | no |
| Type | `string[]` |
| Violation | none |

---

## Diagnostic summary

| Field | Severity | Locator | Condition |
|---|---|---|---|
| frontmatter block | Error | `frontmatter` | Missing or unterminated `---` delimiters |
| frontmatter block | Error | `frontmatter` | Malformed YAML inside the block |
| file | Error | `""` | File could not be read |
| `type` | Error | `type` | Absent or empty |
| `title` | Error | `title` | Absent or empty |
| `description` | Error | `description` | Absent or empty |
| `doc_id` | Error | `doc_id` | Present but not kebab-case or decision-id format |
| `layer[N]` | Error | `layer[N]` | Value not in `vocab.layer` |
| `project` | Error | `project` | Present but not a known repo slug |
| `status` | Error | `status` | Present but not in `vocab.status` |
| `keywords` | Warning | `keywords` | Present but count outside 3–7 |

Errors drive exit 1. Warnings are reported but do not fail the run.

---

### `synced_from` — Tolerated (not OKF-validated)

The `synced_from` field is a cross-repo sync watermark written by the auto-sync pipeline into brain cache docs (`docs/projects/<project>.md`). It records the `timestamp` value from the sub-repo's `planning/status.md` at the time the cache was last synced.

| | |
|---|---|
| Required | no |
| Format | RFC3339 datetime string (e.g. `"2026-06-27T12:00:00+00:00"`) |
| OKF violation | none — the OKF schema tolerates this field; format enforcement is done by `mev validate-brain --sync` |

The `--sync` flag reads `synced_from` and compares it against the sub-repo's current `timestamp`; a mismatch emits `E_SYNC_DRIFT`. See the [CLI reference](cli.md) for the full locator table.

---

## Unknown fields

Unknown frontmatter keys are tolerated — `mev` does not reject files for having extra fields. This allows the live corpus to carry fields defined by future schema versions without failing validation.
