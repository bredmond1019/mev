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

## What this page is for

**OKF frontmatter** is the YAML block every markdown file in the Brain must open with. It is what
makes the corpus searchable and gives the structural graph its edges. This page is the field-level
reference: what each field means, what values are legal, and which diagnostic fires when it is
wrong.

Writing a new doc? The procedure — including the four YAML traps that fail every gate at once —
is in the `write-okf-markdown` skill. This page is the schema behind it.

## Quickstart

Run these in a **terminal**:

```bash
# Does my file parse and resolve?
bastion validate-brain --structure     # is it in the corpus, does frontmatter parse
bastion validate-brain --graph         # do its `related:` edges resolve
bastion validate-brain --links         # do its markdown links resolve
```

**One flag per run** — they do not compose. The minimum legal block is three fields:

```yaml
---
type: Reference
title: What this is
description: One line a searcher would recognise.
---
```

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

### `created` / `updated` — Optional, not validated

Authorship dates: when the document was first written, and when it was last meaningfully revised.
Modelled by `okf_core::OkfFrontmatter` (added 2026-08-29, okf-core block
`OK.ticket.add-created-updated-frontmatter`) so they survive a parse/serialize round-trip instead of
being dropped — but **`mev` applies no rule to either**: no required check, no format check, no
freshness check. A wrong date, a stale `updated`, or a `created` later than `updated` all pass.

| | |
|---|---|
| Required | no |
| Type | `string` (convention: `YYYY-MM-DD`; no format is enforced) |
| Violation | none — absent from the diagnostic summary above by design |

Not to be confused with two fields `mev` *does* act on: `timestamp` (the Log/ProjectStatus
freshness stamp) and `synced_from` (below, which drives `E_SYNC_DRIFT` under `--sync`).

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

## Cross-tree links: the `${BRAIN_ROOT}` prefix

`mev validate-brain --links` checks every markdown link and `file://` URI in the corpus against
the filesystem (`E_LINK_DEAD_MARKDOWN` / `E_LINK_DEAD_FILE_URI`). A link that stays inside one
document's own subtree resolves fine as an ordinary relative path. A link that has to reach
**across** the tree — say, from a `mev` doc into `core/celia/planning/blocks/` — has never had a
good shape. All three shapes people reach for are wrong:

- **A deep relative chain** (`../../../../../core/celia/planning/blocks/`) resolves, but it
  encodes the *citing file's* depth in the corpus. Move the citing file one directory and every
  such link silently breaks — nothing about the chain says what it was reaching for.
- **A machine-absolute `file://` URI** (`file:///Users/brandon/Dev/agentic-portfolio/...`)
  resolves on the machine that authored it and nowhere else. It is wrong in any clone whose brain
  root differs — the sandbox-v2 builder included, which constructs a corpus at a different root by
  design and has to carry a bespoke rewriter (`rewrite_decision_links` in
  `scripts/sandbox/build-v2.sh`) purely to work around this.
- **A literal `${BRAIN_ROOT}` written with no support in the checker** used to just fail: the
  resolver treated `${BRAIN_ROOT}` as an ordinary path segment relative to the citing file, so the
  link reported `E_LINK_DEAD_MARKDOWN` even when the target existed.

**The supported shape** is a leading `${BRAIN_ROOT}/` (or the bare `$BRAIN_ROOT/`, without braces)
at the very start of the link target. The checker expands it to the brain root already resolved
for the run — the same root `validate-brain` prints in its summary line — before checking the
path on disk:

```markdown
[the celia blocks dir](${BRAIN_ROOT}/core/celia/planning/blocks/)
[same thing, bare form]($BRAIN_ROOT/core/celia/planning/blocks/)
```

Both lines above resolve identically. The same expansion applies to a `file://` URI carrying the
prefix (`file://${BRAIN_ROOT}/core/celia/...`).

This is a resolution-time expansion, not an extraction-time rewrite: the reported `raw`/`target`
values, and every diagnostic message, still show `${BRAIN_ROOT}/...` exactly as authored — never an
expanded machine path. It does **not** weaken the check: a `${BRAIN_ROOT}`-prefixed link whose
expanded path does not exist still reports a dead link, exactly like any other broken link. The
prefix is recognized only at the start of a target — `${BRAIN_ROOT}` appearing mid-path is not a
template language, and a partial or malformed token (`${BRAIN_ROOT` unterminated, `${BRAIN_ROOTX}/`)
does not expand and is not silently accepted; it resolves (and fails) exactly as an unrecognized
path does today.

**Until the binary that ships this expansion is built and installed, do not write a literal
`${BRAIN_ROOT}` link into any corpus document as a live link** — an older checker reports it dead,
and one dead link red-gates the whole corpus's push gate for every concurrent lane, not just the
file that carries it. Show the shape in a fenced code block (as above) when documenting it before
then; the link extractor does not scan fenced code.

---

## Unknown fields

Unknown frontmatter keys are tolerated — `mev` does not reject files for having extra fields. This allows the live corpus to carry fields defined by future schema versions without failing validation.
