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

### `validate [--blog] [--lint] [path]`

Validate the learn-ai content tree — the learn modules by default, or, with `--blog`, the
learn-ai **blog** tree instead (Phase 12, Block A). These are **content** checks, not corpus
checks: they surface here, through `mev validate`, and never through `validate-brain`.

```bash
mev validate [--blog] [--lint] [path]
```

| Argument / Flag | Default | Description |
|---|---|---|
| `path` | `../learn-ai/content/learn`, or `../learn-ai/content/blog/published` when `--blog` is given | Path to the content root. The positional's default is resolved after parsing, not conditionally inside the derive, so the existing learn-tree default is untouched when `--blog` is absent. |
| `--blog` | off | Validate the learn-ai blog tree instead of the learn module tree: EN posts under `blog/published/*.mdx` and pt-BR posts under `blog/published/pt-BR/*.mdx` (`BlogValidator`). Changes the positional path's default and the `--json` consumer label to `"blog"`. Runs the shared content-lint passes (untagged code blocks, dead local links/assets) on by default, alongside the blog-specific frontmatter and pt-BR parity checks. |
| `--lint` | off | Run the shared content-lint passes (`W_LINT_UNTAGGED_CODE_BLOCK`, `E_LINT_DEAD_LOCAL_LINK`, `E_LINT_DEAD_ASSET`) over learn modules, in addition to the existing frontmatter/JSON checks. Only `ModuleMdx` items get the lint pass — a markdown fence scan over `.json` metadata is meaningless. A no-op when combined with `--blog`, since lint already runs there by default. Without this flag, bare `mev validate` is byte-identical to its pre-Block-A output. |

Checks each file in the tree against the learn-ai frontmatter schema and JSON struct constraints
(`LearnAiValidator`). With `--blog`, checks each post's frontmatter (`title`/`date`/`excerpt`
required), pt-BR filename parity, and the shared lint passes (`BlogValidator`).

**Examples:**

```bash
# Default path — learn modules, existing behaviour unchanged
mev validate

# Explicit path
mev validate ~/Dev/learn-ai/content/learn

# Learn modules, with the shared lint passes turned on
mev validate --lint

# Blog tree — frontmatter, pt-BR parity, and lint, all on by default
mev validate --blog

# Machine-readable output
mev --json validate
mev --json validate --blog
```

#### Diagnostic codes — `--blog` / `--lint`

| Locator | Severity | Condition |
|---|---|---|
| `E_BLOG_MALFORMED_FRONTMATTER` | Error | A blog post's leading `---` YAML frontmatter block is absent or unparseable; causes exit 1 |
| `E_BLOG_MISSING_FIELD` | Error | A required blog frontmatter field (`title`, `date`, or `excerpt`) is missing or empty; the field name is in the message; causes exit 1 |
| `W_BLOG_PTBR_MISSING` | Warning | An EN post under `blog/published/*.mdx` has no `pt-BR/<slug>.mdx` counterpart. Warning, not error: real parity gaps exist in the live tree (1 as of 2026-08-09), and erroring here would make `--blog` red on arrival and unusable as a gate for `MV.12.B`; exit code unchanged |
| `W_LINT_UNTAGGED_CODE_BLOCK` | Warning | A fenced code block (` ``` ` or `~~~`) opens with no language tag. Presentation, not correctness; exit code unchanged |
| `E_LINT_DEAD_LOCAL_LINK` | Error | A markdown link `[text](target)` resolves to a path that does not exist. Absolute URLs (`http://`, `https://`, `mailto:`, protocol-relative `//`) and in-page anchors (`#...`) are skipped, never reported. A genuinely relative target (`./x.md`, `../assets/y.png`) resolves against the file's parent directory. A **site-absolute** target (a single leading `/`, e.g. `/en/blog/x`) is a Next.js route, not a filesystem path — see "Site-absolute route resolution" below; causes exit 1 |
| `E_LINT_DEAD_ASSET` | Error | An image reference `![alt](target)` resolves to a path that does not exist on disk; causes exit 1 |
| `E_FUNNEL_CTA_UNRESOLVED` | Error | A blog post's `cta` frontmatter value is present but outside the accepted vocabulary (`data/cta-vocabulary.toml`: `newsletter` / `booking` / `bastiel` / `module`), or `cta: module` names a `ctaTarget` that is missing, not shaped `"<path-slug>/<module-id>"` with both segments non-empty, or names a module that does not exist under the sibling learn tree. A post with **no `cta` key at all** is the legitimate newsletter default and is never flagged; causes exit 1 |
| `E_FUNNEL_MISSING_UTM` | Error | An `http(s)` URL in content whose host is `bastiel.com.br` or `bastielai.com` (including subdomains) is missing any of `utm_source`, `utm_medium`, `utm_campaign` from its query string. **Presence and shape only** — a UTM value is never checked against a campaign registry, and no network call is ever made. `mailto:` references (e.g. `mailto:brandon@bastiel.com.br`) are skipped unconditionally, never reported — they are not outbound links and matching the bare domain would false-positive on every one of them; causes exit 1 |
| `E_FUNNEL_BARE_CAL_LINK` | Error | Any `cal.com` URL appears directly in blog or module content. The booking CTA renders through its own component; a hand-written Cal.com link bypasses it; causes exit 1 |
| `E_FUNNEL_RAW_ANALYTICS_ATTR` | Error | A `data-umami-*` attribute is written directly into content instead of going through learn-ai's analytics module; causes exit 1 |
| `W_VOICE_TELL` | Warning | A phrase from the data-driven banned-phrase list (`data/voice-tells.toml`) appears in prose, outside fenced code blocks, inline code spans, blockquotes, and frontmatter. **Warning-level by design, always** — there is no configuration under which this diagnostic becomes an error, and it never affects the exit code. One code covers every phrase; the matched phrase and the 1-indexed line are in the message, so the operator reads a single ranked list instead of reconciling a code per phrase |

`--blog` and `--lint` are content checks over the learn-ai repo, surfaced only through `mev
validate` — they have no `validate-brain` equivalent and never will, per the Phase 12 boundary
between content linting and brain-corpus validation.

#### Funnel conformance (`E_FUNNEL_*`, Phase 12 Block B)

The four `E_FUNNEL_*` codes gate that published content actually participates in the funnel: every
post resolves a real CTA, every outbound `bastiel` link carries UTM params, no bare Cal.com link
survives in content, and no CTA emits through a raw `data-umami-*` attribute instead of learn-ai's
analytics module. They run under the existing `--blog` flag — there is no separate flag for this
block, matching `MV.12.A`'s "one flag runs every content check" convention.

- **UTM boundary:** the check is presence-and-shape only — it confirms `utm_source`, `utm_medium`,
  and `utm_campaign` are present in the query string, never that their *values* are correct against
  a campaign registry. And, like every check in this module, it makes no network call.
- **The `mailto:` exclusion:** `E_FUNNEL_MISSING_UTM` matches only `http://`/`https://` URLs. A
  `mailto:brandon@bastiel.com.br` reference is not an outbound link and is skipped unconditionally,
  regardless of query string. Without this exclusion the check false-positives on every `mailto:`
  occurrence in the corpus — this is the non-obvious rule a future reader would otherwise
  "simplify" away by matching on the bare `bastiel.com.br` / `bastielai.com` host.
- **Cross-repo contract:** learn-ai adopts these four codes as a gated check in its own
  `planning/harness.json`, and `bastion-web:BW.11.A` renders the per-post verdict from the
  `--json` envelope. The codes are therefore a contract across repos — renaming one is a breaking
  change for both consumers.

#### Voice tripwire (`W_VOICE_TELL`, Phase 12 Block C)

A warning-level lint for the banned-phrase list `learn-ai/CLAUDE.md`'s "Voice and tone" bullet
already states — the concrete, testable half of "doesn't sound like AI slop." It runs under the
existing `--blog` flag, alongside the funnel-conformance checks above; there is no separate flag
for this block.

- **The data file:** `data/voice-tells.toml`, embedded into the `mev` binary at compile time via
  `include_str!` (same pattern as `data/cta-vocabulary.toml`), so the tripwire works from any
  working directory. It is a flat list of `[[tells]]` tables, each with:
  - `phrase` — the literal string to match (case-insensitive, word-boundary-respecting; never a
    regex, so a bad edit can never panic or hang the validator, and can never match across a
    line break).
  - `note` — one line naming where the rule comes from. If a phrase is ever added beyond the
    seed, its `note` must record the phrase's measured hit count against the live corpus at
    authoring time (see below).
- **Adding a tell is a one-step, Rust-free edit:** append a `[[tells]]` entry to
  `data/voice-tells.toml` with a `phrase` and a sourcing `note`. No code change and no release
  are required — the loader is data-driven, which is itself pinned by a test that loads a phrase
  absent from the shipped list, via an override, and confirms it fires.
- **Exemptions:** fenced code blocks (` ``` `/`~~~`), inline code spans, blockquote lines, and
  the YAML frontmatter block are all exempt from matching, each pinned by its own test. The
  fenced-code exemption reuses the same fence-tracking logic `E_LINT_DEAD_LOCAL_LINK` uses,
  rather than a second, independently-drifting scanner.
- **Warning-level, never error, by design.** A false positive from this scanner must never be
  able to block a push — there is no configuration that promotes `W_VOICE_TELL` to an error, and
  a corpus whose only findings are voice tells exits 0. This is pinned by a test.
- **Two deliberate boundaries.** This tripwire does not judge voice, and it makes no model call.
  It catches the phrases `learn-ai/CLAUDE.md` writes down, and only those — it does not attempt
  to score tone, reading level, or "AI-ness" statistically. The judgement of whether a post
  actually reads as AI slop stays with the human operator, on the surface
  `bastion-web:BW.11.A` provides.
- **Why the seed list is only three phrases.** `learn-ai/CLAUDE.md` names exactly
  `production-ready`, `game-changing`, and `actually bites you` as marketing-language and
  colloquialism examples, and the seed list is deliberately scoped to just those. Broader
  marketing vocabulary was measured against the live corpus
  (`content/blog/published/`, EN, 2026-08-06) and excluded because it reads as ordinary
  technical English as often as it reads as hype: `robust` hits 7 files, `leverage` 4, `unlock`
  2, `seamless` 1. Seeding those would turn the report into a triage queue — the exact outcome
  the block's acceptance criterion ("a report the operator can act on without triage") rules
  out. Any term added beyond the three explicit ones must carry its own measured hit count in
  its `note`, and should be excluded when the hits are mostly legitimate usage.

#### Site-absolute route resolution (`E_LINT_DEAD_LOCAL_LINK`)

A link target with a single leading `/` (not `//`, which is protocol-relative and skipped) is a
Next.js route, not a filesystem path, and resolves through seven mapping rules against a
`content_root` — the directory containing both `blog/` and `learn/` — derived from the
validator's root (`<content>/blog/published` and `<content>/learn` both strip back to
`<content>`; when the root doesn't match either shape, `content_root` is `None` and **every**
absolute link is skipped, never reported, rather than guessed at):

| Route shape | Resolves to |
|---|---|
| `/<locale>/blog/<slug>` | `blog/published/<slug>.mdx` for `en`; `blog/published/pt-BR/<slug>.mdx` for `pt-BR` |
| `/blog/<slug>` (no locale segment) | `blog/published/<slug>.mdx` |
| `/<locale>/learn/paths/<slug>` | `learn/paths/<slug>` (`en` and `pt-BR`) |
| `/<locale>/learn/concepts/<slug>` | `learn/concepts/<slug>` (`en` and `pt-BR`) |
| `/learn/paths/<slug>` | `learn/paths/<slug>` |
| `/learn/concepts/<slug>` | `learn/concepts/<slug>` |
| `/learn/<slug>` | **nothing — reported as `E_LINT_DEAD_LOCAL_LINK`.** See below |
| anything else | skipped silently, never reported — mev does not assert on routes it does not model |

**Two ways a route can fail to resolve, and they are not the same.** An *unmodelled* shape
(`/services`, `/about`) is skipped silently: mev does not assert on routes it does not model.
A *known-invalid* shape is reported. Today there is exactly one known-invalid shape,
`/learn/<slug>`: learn-ai's App Router has no `[slug]` segment directly under `learn/` — only
`learn/paths/[slug]` and `learn/concepts/[slug]` — so `/learn/<slug>` 404s in production. It is
matched before route resolution runs, and the diagnostic names `/learn/paths/<slug>` as the
correct route so the reader can fix the link without re-deriving the route table.

Until 2026-08-09 this table aliased `/learn/<slug>` onto `learn/paths/<slug>`, which made the
link resolve to a file that exists and therefore *never* fire — a false negative that hid two
live dead links. Collapsing known-invalid back into the silent-skip branch would re-hide them,
which is why the distinction above is load-bearing rather than stylistic
(`MV.ticket.learn-link-mapping-masks-dead-links`).

This was added after the block's first full pass found `E_LINT_DEAD_LOCAL_LINK` firing 29 times
over the live corpus (18 blog, 11 learn) with zero true positives — every target was a real
site route resolved as a filesystem path. All 29 are fixed by the mapping above (one additional
learn-tree false positive, unrelated to routing — link-shaped text inside code embedded via a
JSX component prop, which carries no ` ``` ` fence — is fixed by making the link scanner
fence-aware and by skipping any `[` immediately glued to a preceding identifier character, e.g.
a Python indexing expression like `results` immediately followed by an open bracket, an index, a
close bracket, and a call — the array-indexing-then-call shape that reads as markdown link syntax
if you don't special-case it).

As of 2026-08-09, `mev validate --lint` reports zero `E_LINT_DEAD_LOCAL_LINK` over the live learn
tree, and `mev validate --blog` reports **two** — the same `/learn/12-factor-agent-development`
target in the EN and pt-BR copies of the 12-factor post, surfaced by the known-invalid rule above.
Those are content bugs owned by learn-ai's `LA.ticket.content-lint-cleanup`, not mev bugs. Both
figures are pinned by live-tree tests; the blog-side test allowlists that one tracked target and
still fails on any other dead link, so the guard against new regressions is intact.

---

### `validate-brain [--sync] [--graph] [--state] [--links] [--structure] [path]`

Validate the Bastion Brain repo for OKF frontmatter compliance, and optionally check cross-repo sync watermark integrity, global knowledge-graph integrity, state.json schema and block-dependency graph integrity, link integrity, or structural `index.md` coverage.

```bash
mev validate-brain [--sync] [--graph] [--state] [--links] [--structure] [path]
```

| Argument / Flag | Default | Description |
|---|---|---|
| `path` | `..` | Path to the company-brain repo root |
| `--sync` | off | Also run the cross-repo sync watermark check (see below) |
| `--graph` | off | Also run the global `scope:doc_id` knowledge-graph integrity check (see below). Takes precedence over `--sync` when both flags are present — `--graph` is a superset. |
| `--state` | off | Also run the `planning/state.json` schema and cross-repo block-dependency graph integrity check (see below). Takes precedence over `--graph` and `--sync` in the dispatch chain. |
| `--structure` | off | Also run the bidirectional `index.md` ↔ directory structural coverage check (see below) — flag corpus files not referenced by their directory's `index.md`, and `index.md` rows pointing at a nonexistent target. Takes precedence over `--state`, `--graph`, and `--sync` in the dispatch chain. |
| `--links` | off | Also run the link-integrity pass (see below) — flag dead markdown links, broken `file://` URIs, dangling `[[wikilinks]]`, and references to moved/deleted paths. Takes the highest precedence overall — over `--structure`, `--state`, `--graph`, and `--sync` in the dispatch chain. |

Resolves `brain.toml` by walking up from `path`. If no `brain.toml` is found, a fatal `Error`-severity diagnostic with locator `E_CONFIG_NOT_FOUND` is emitted and the process exits 1.

See [`brain.toml` config](brain-toml.md) for the configuration format and [OKF schema](okf-schema.md) for what is validated.

#### `--sync` — cross-repo watermark check

When `--sync` is passed, `mev` runs the full OKF schema pass first, then appends a second pass that compares watermarks for every `[[repos]]` entry in `brain.toml`:

- Reads `timestamp` from `<repo_path>/<status_file>` (the sub-repo's status file).
- Reads `synced_from` from `<cache_doc>` (the brain cache doc for that repo).
- Both values must be present and valid RFC3339 datetimes; they are compared as explicit UTC
  instants (each side normalized via `.to_utc()`), not as raw strings — a `-03:00` watermark and
  a `Z` watermark denoting the same moment are in sync.

A mismatch or missing watermark emits an `Error`-severity diagnostic and causes exit 1.

| Locator | Condition |
|---|---|
| `E_SYNC_FILE_MISSING` | `status_file` or `cache_doc` does not exist, or cannot be read |
| `E_SYNC_WATERMARK_MISSING` | `timestamp` or `synced_from` field is absent from the frontmatter |
| `E_SYNC_WATERMARK_MALFORMED` | A watermark is present but is not a valid RFC3339 datetime |
| `E_SYNC_DRIFT` | Both watermarks parse successfully but denote different instants |

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

1. **Discovery** — finds all `planning/state.json` files: the HQ brain state, each tier sub-brain state (via `tiers[].rollup` in the HQ state), and each leaf project state (via `[[repos]]` in `brain.toml`). A leaf repo whose `brain.toml` `tier` is `"portfolio"` is expected as `kind:"portfolio"` instead of `kind:"project"` — these are terminal repos (published to GitHub, no further planning state), expected to carry a non-empty `note` instead of `tracks[]`, and are skipped entirely by `emit-state`'s wave-table splice (no `master-plan.md` expected). Missing files emit `W_STATE_FILE_MISSING`. If the HQ root's own `state.json` exists but fails to load (parse error), tier sub-brain paths are recovered directly from `brain.toml`'s `[[repos]]` tier config (rather than from the unloadable HQ `tiers[]`) and registered as `expected_kind:"brain"` stubs — this prevents them from falling through to the leaf `[[repos]]` loop and being misclassified as `expected_kind:"project"`. A single `E_STATE_ROOT_LOAD_FAILED` diagnostic names the degraded classification; the root's own detailed `E_STATE_MALFORMED_JSON` remains the actionable error, instead of a cascade of spurious `E_STATE_SCHEMA_BAD_KIND` on every tier.
2. **Load** — deserializes each discovered file. Unparseable files emit `E_STATE_MALFORMED_JSON`, which now includes the underlying `serde_json::Error` detail (offending field/type and line:column), not just the generic message.
3. **Schema ring** — checks field validity within each file (kind membership, status enum values, `blocked_by` well-formedness, kind-appropriate sections). In v2 schema files: validates `depends_on[]` entry well-formedness on track blocks (including `Operator`'s required `exit` and `Approval`'s `<algorithm>:<hex>`-shaped `digest`, see `E_STATE_OPERATOR_MISSING_EXIT`/`E_STATE_APPROVAL_DIGEST_SHAPE` below), rejects authored `status:"blocked"` (derived, not authored), and validates `backlog[].status` membership. A separate pass, `check_operator_staleness`, emits `W_STATE_OPERATOR_STALE` for `Operator` edges whose owning file has aged past the `[attention]` `operator_days` threshold.
4. **Graph** — builds the cross-repo block-dependency graph from all loaded files (v2: DAG edges sourced from `tracks[].blocks[].depends_on[]`) and checks it for integrity violations, including cycle detection over the `depends_on` DAG and backlog-node integrity.
5. **Status consistency** — checks that a `closed` block does not depend (via `depends_on`) on a block that is not yet `closed`.
6. **Rollup** — checks that brain `repos[]` headline entries (now/next) match their children's actual `focus` values.
7. **Focus drift** — recomputes the expected `focus` from authored `tracks[]` and warns when the stored `focus` disagrees (warning-only; exit code is unchanged).
8. **`reference[]` validation** (`MV.ticket.reference-container-validation`) — checks the shape of every `reference[]` entry: `class` must be one of `trap`/`invariant`/`lesson`/`deliberate`; `scope` must set exactly one of `repo`/`tier`/`cross_repo` (`cross_repo: false` counts as set, same rule as `carryover[]` scope); `created`/`reviewed` must parse as a date; and a `slug` must not appear in both `reference[]` and `carryover[]` within the same file. `reference[]` entries are permanently-true material (traps, invariants, lessons, deliberate-choice markers) with no clock by design (D72 §5) — they are checked for shape only, never for staleness, and are never evaluated by `check_carryover_staleness` or emitted onto any triage surface (see [Carryover triage lanes](#carryover-triage-lanes), [`attention-queue`](#attention-queue---out-path-path), and `mev carryover` below — none of them read `reference[]`).

**Narrowed carryover vocabulary (D72).** `carryover[].kind` is now exactly `defect`, `deferred`, `drift`, or `env`. The pre-D72 values `constraint` and `known_issue` still deserialize but are legacy: they produce a warning-severity `W_STATE_LEGACY_KIND` naming the entry's slug, citing D72, and naming `HQ.ticket.reference-container-migration` (Block G) as the migration that clears them — they do **not** fail the run. A `kind` in neither set is a hard error, `E_STATE_SCHEMA_BAD_KIND`, whose message enumerates only the four current values (the two legacy ones are deliberately never listed, so they don't read as authorable). Flipping `W_STATE_LEGACY_KIND` from warning to error is a follow-up, gated on Block G reporting zero remaining legacy entries — not yet done.

`--state` takes precedence over `--graph` and `--sync` in the dispatch chain — when `--state` is present, neither `--graph` nor `--sync` are separately invoked. `--structure` takes precedence over `--state`, `--graph`, and `--sync`. `--links` takes the highest precedence overall; when `--links` is present, `--structure`, `--state`, `--graph`, and `--sync` are not separately invoked.

| Locator | Severity | Condition |
|---|---|---|
| `W_STATE_FILE_MISSING` | Warning | A registered repo has no `planning/state.json` |
| `E_STATE_MALFORMED_JSON` | Error | A state.json file is not valid JSON or does not match the expected schema; message includes the underlying serde error (field/type + line:column) |
| `E_STATE_ROOT_LOAD_FAILED` | Error | The HQ root `state.json` exists but failed to load; tier sub-brain classification is degraded (recovered from `brain.toml` instead of the unloadable root) — see the root's own `E_STATE_MALFORMED_JSON` for the actual cause |
| `E_STATE_SCHEMA_BAD_KIND` | Error | `kind` is not one of `project`, `brain`, or `portfolio`. Reused for two other bad-vocabulary checks: a `carryover[].kind` outside `defect`/`deferred`/`drift`/`env` (message lists only those four), and a `reference[].class` outside `trap`/`invariant`/`lesson`/`deliberate` (message lists all four) |
| `W_STATE_LEGACY_KIND` | Warning | A `carryover[].kind` is `constraint` or `known_issue` — pre-D72 vocabulary. Names the slug, cites D72, and names Block G (`HQ.ticket.reference-container-migration`) as the migration that clears it; exit code is unchanged |
| `E_STATE_SCHEMA_MALFORMED_SCOPE` | Error | A `carryover[]` or `reference[]` entry's `scope` does not set exactly one of `repo`/`tier`/`cross_repo` (`cross_repo: false` counts as set) |
| `E_STATE_REFERENCE_CARRYOVER_COLLISION` | Error | A `slug` appears in both `reference[]` and `carryover[]` in the same file, naming both containers |
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
| `E_STATE_AUTHORED_BLOCKED` | Error | A `tracks[].blocks[].status` is `"blocked"` — `blocked` is derived, not authored. (`"deferred"`, by contrast, **is** a legal authored status.) |
| `E_STATE_STATUS_INCONSISTENT` | Error | A `closed` block has a `type:block` `depends_on` target that is not `closed` |
| `E_STATE_DANGLING_PROMOTION` | Error | A `status:"promoted"` backlog node's `block` pointer resolves to no `tracks[]` node |
| `E_STATE_PRIORITY_RANGE` | Error | A `priority` value is not in 0..=3 |
| `E_STATE_DUE_FORMAT` | Error | A `due` value is not a valid YYYY-MM-DD date |
| `E_STATE_SDLC_WORKFLOW_ENUM` | Error | An `sdlc_workflow` value is not in `{none, patch, task, run, flow}` |
| `E_STATE_MODEL_ENUM` | Error | A `model` value is not in `{sonnet, gemini-pro, gemini-flash, either}` |
| `E_STATE_DATE_FORMAT` | Error | A carryover/backlog `created` / `reviewed` / `snoozed_until` value is not a valid `YYYY-MM-DD` (or RFC3339) date |
| `W_STATE_FOCUS_DRIFT` | Warning | Stored `focus` disagrees with the derivation from `tracks[]`; exit code is unchanged |
| `W_STATE_CARRYOVER_STALE` | Warning | A `carryover[]` entry has aged past its per-`kind` `[attention]` threshold and is not snoozed; exit code is unchanged |
| `W_STATE_BACKLOG_STALE` | Warning | An HQ `backlog[]` `idea`/`ready` node has aged past the `[attention]` backlog threshold and is not snoozed; exit code is unchanged |
| `W_DISTILL_STALE` | Warning | A D35-distilled `knowledge.md`/`memory.md` entry has aged past its `[attention]` `knowledge_days`/`memory_days` threshold (`check_distill_staleness`); exit code is unchanged |
| `E_STATE_OPERATOR_MISSING_EXIT` | Error | A `depends_on[]` `Operator` entry has an empty `exit` field |
| `E_STATE_APPROVAL_DIGEST_SHAPE` | Error | A `depends_on[]` `Approval` entry's `digest` is missing or not shaped `<algorithm>:<hex>` |
| `W_STATE_OPERATOR_STALE` | Warning | An `Operator` `depends_on` edge's owning `state.json` has an `updated` date older than the `[attention]` `operator_days` threshold (default 7); exit code is unchanged |

#### `--structure` — structural `index.md` coverage check

When `--structure` is passed, `mev` runs the full OKF schema pass first, then appends the structural coverage pass (D17 / CLAUDE.md Standing Rule 7):

1. Crawls the corpus (same registry-driven walk as the OKF pass).
2. Locates every directory's `index.md` corpus member and its direct-child corpus entries (siblings of that `index.md`; subdirectories are excluded — they are covered by their own `index.md`).
3. Extracts every markdown `[text](path)` link and `file://` URI from each `index.md` and resolves it against that `index.md`'s directory.
4. Checks both directions: every direct-child file must be referenced by the `index.md` (orphan detection), and every resolved `index.md` link that lands inside the corpus root must exist on disk (dangling-row detection).

Directories with no `index.md` corpus member are skipped entirely — no coverage obligation, so no orphan diagnostics. `[[wikilink]]` targets, external (`http(s)://`, `mailto:`, etc.) links, and links that resolve outside the corpus root are ignored (owned elsewhere / out of scope for this check).

| Locator | Severity | Condition |
|---|---|---|
| `E_STRUCT_ORPHAN_FILE` | Error | A corpus file in a directory is not referenced by that directory's `index.md`. Located at the orphan file. |
| `E_STRUCT_DANGLING_ROW` | Error | An `index.md` row (markdown or `file://` link) resolves to a target inside the corpus root that does not exist on disk. Located at the `index.md`. |

Any error-severity diagnostic causes exit 1.

`--structure` takes precedence over `--state`, `--graph`, and `--sync` in the dispatch chain — when `--structure` is present, none of those are separately invoked. `--links` takes precedence over `--structure`.

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

# OKF pass + structural index.md coverage check
mev validate-brain --structure

# Explicit path with structural coverage check
mev validate-brain --structure ~/Dev/agentic-portfolio

# Machine-readable output including structural diagnostics
mev --json validate-brain --structure ~/Dev/agentic-portfolio
```

---

### `validate-state <path>`

Validate a single `planning/state.json` file — the single-file sibling of
`validate-brain --state` (`MV.ticket.reference-container-validation` task 5).

```bash
mev validate-state <PATH>
```

| Argument | Description |
|---|---|
| `<PATH>` | Path to the `state.json` file to validate. Required — there is no default and no directory walk |

Loads exactly the one named file and runs only the **per-file ring** `validate-brain --state`
already runs against every discovered file — `load_state`, `check_schema`,
`check_field_policy` — so it catches the same schema errors (bad `kind`, bad `status`,
malformed `blocked_by`, the narrowed carryover vocabulary, `reference[]` shape) with the same
diagnostic codes documented under [`--state`](#--state--statejson-schema-and-block-dependency-graph-check)
above, and it does so on a malformed file whose `scope`/`related` shape prevents it from
loading cleanly at all: unparseable JSON or a schema mismatch is field-diagnosed (naming the
offending slug and field) rather than surfacing as an opaque parse error.

**Deliberately excludes every corpus-level check** — the cross-repo block-dependency graph,
cycle detection, rollup drift, focus drift, and status consistency. Those checks need sibling
repos loaded to evaluate at all and structurally cannot run from one file in isolation; running
them here would either silently no-op or require secretly walking the corpus anyway, which
defeats the point of a single-file command.

This is the check meant to run after every manual `state.json` edit — cheap enough that nothing
excuses skipping it. It would have caught the live 2026-08-13 incident before it cascaded to 50
errors across 7 files: `scope` had been hand-authored as a plain string instead of a
`CarryoverScope` object, and `related` as bare slug strings instead of `BlockedBy` objects.

#### Exit codes

| Code | Meaning |
|---|---|
| `0` | The file loaded cleanly and no error-severity diagnostic was raised (warnings — e.g. `W_STATE_LEGACY_KIND`, `W_STATE_CARRYOVER_STALE` — do not fail the run) |
| `1` | The file is missing or unreadable, is not valid JSON, fails the typed schema, or an error-severity diagnostic was raised |

**Examples:**

```bash
# Validate one repo's state.json before committing a manual edit
mev validate-state planning/state.json

# Validate an arbitrary file by absolute path
mev validate-state ~/Dev/agentic-portfolio/core/mev/planning/state.json
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

### `emit-graph [--pretty] [path]`

Emit the `scope:doc_id` knowledge graph — authored nodes, `related:` edges, and marked leaves — as a canonical JSON artifact.

```bash
mev emit-graph [--pretty] [path]
```

| Argument / Flag | Default | Description |
|---|---|---|
| `path` | `.` | Path to search from when locating `brain.toml` (walks up to find it) |
| `--pretty` | off | Emit pretty-printed (indented) JSON instead of compact JSON |

Resolves `brain.toml` by walking up from `path`. If no `brain.toml` is found, the process
exits 1 with an error message on stderr (mentioning `brain.toml`).

The output is the graph-export JSON written directly to stdout — there is no `--json` envelope
wrapper; the output *is* JSON. `mev emit-graph` is a **pure emit**: it does not write to any
file or database, and it does not re-derive or re-walk the graph — it reuses the same
`build_graph` pass that backs `mev validate-brain --graph`.

This is distinct from `generate-graph` (above), which renders an interactive HTML/`vis.js`
visualization for humans. `emit-graph` produces a machine-readable JSON artifact intended for
the orchestrator to load into a Postgres edges table beside `brain_documents` (D4).

#### Output shape

```json
{
  "version": "2",
  "root": "/path/to/brain",
  "nodes": [
    {
      "id": "brain:alpha",
      "scope": "brain",
      "doc_id": "alpha",
      "rel": "docs/alpha.md"
    }
  ],
  "edges": [
    {
      "from": "brain:alpha",
      "to_ref": "beta",
      "kind": "related",
      "target_node_id": "brain:beta",
      "target_doc_id": "beta"
    },
    {
      "from": "brain:alpha",
      "to_ref": "missing",
      "kind": "related",
      "target_node_id": null,
      "target_doc_id": null
    }
  ],
  "leaves": ["brain:a-leaf", "brain:z-leaf"]
}
```

| Field | Type | Description |
|---|---|---|
| `version` | string | Schema version — currently `"2"` |
| `root` | string | Display path of the HQ root used for the crawl |
| `nodes` | array | Every corpus file with an authored `doc_id`, in walk order — one node per `scope:doc_id` |
| `nodes[].id` | string | Canonical node id: `scope:doc_id` |
| `nodes[].scope` | string | Owning scope slug (from the corpus registry) |
| `nodes[].doc_id` | string | Authored `doc_id` (location-independent frontmatter field) |
| `nodes[].rel` | string | Path of the file relative to the HQ crawl root |
| `edges` | array | Every authored `related:` entry, in walk order |
| `edges[].from` | string | Canonical id of the source node (`scope:doc_id`) |
| `edges[].to_ref` | string | The raw `related:` entry as authored (bare or `scope:doc_id`) — not yet resolved/normalized |
| `edges[].kind` | string | Edge type; currently only `"related"` |
| `edges[].target_node_id` | string \| null | Qualified `scope:doc_id` of the resolved target node; non-null when the edge resolves to a real node, `null` when it is dangling or targets a leaf (doc-id-less file) |
| `edges[].target_doc_id` | string \| null | Authored `doc_id` of the resolved target node; non-null exactly when `target_node_id` is non-null |
| `leaves` | array | `scope:stem` for every corpus file with **no** authored `doc_id`, sorted for deterministic output |

`leaves` is always sorted, so repeated runs over an unchanged corpus emit byte-identical
output.

#### Exit codes

| Code | Meaning |
|---|---|
| `0` | Graph emitted successfully |
| `1` | `brain.toml` not found, or a runtime error prevented crawl completion |

**Examples:**

```bash
# Compact JSON from the current directory
mev emit-graph

# Compact JSON from an explicit brain root
mev emit-graph ~/Dev/agentic-portfolio

# Pretty-printed JSON
mev emit-graph --pretty

# Pipe compact JSON into jq for summary counts
mev emit-graph | jq '{nodes: (.nodes|length), edges: (.edges|length), leaves: (.leaves|length)}'
```

---

### `emit-block-graph [flags] [path]`

Emit the corpus-wide block-dependency graph — every discovered `planning/state.json` block,
enriched with derived attention/priority/topology fields, filtered by an optional scope — as a
JSON artifact.

```bash
mev emit-block-graph [--scope <hq|tier|repo|epic>] [--tier <NAME>] [--epic <SLUG>]
                      [--repo <SLUG>] [--include-closed] [--include-boundary]
                      [--max-nodes <N>] [--pretty] [path]
```

| Argument / Flag | Default | Description |
|---|---|---|
| `path` | `.` | Path to search from when locating `brain.toml` (walks up to find it) |
| `--scope <hq\|tier\|repo\|epic>` | `hq` | Scope mode — see the table below |
| `--tier <NAME>` | `core` | Tier name to scope to; consulted only when `--scope tier` is given |
| `--epic <SLUG>` | unset | Epic slug to project onto; required when `--scope epic` is given. Overrides `--tier`/`--repo` rather than intersecting with them |
| `--repo <SLUG>` | unset | Repo slug to intersect against; required when `--scope repo` is given |
| `--include-closed` | off | Include `closed`-lane blocks in the exported node set |
| `--include-boundary` | off | Re-add direct neighbours of the in-scope set as boundary nodes (`in_scope: false`); retains edges that cross the scope boundary |
| `--max-nodes <N>` | unset (no truncation) | Cap the exported node list at `N` nodes (topo-ordered); sets `truncated: true` when the pre-truncation node count exceeds `N` |
| `--pretty` | off | Emit pretty-printed (indented) JSON instead of compact JSON |

`--scope` selects the mode:

| `--scope` | Meaning | Companion flag |
|---|---|---|
| `hq` (default) | Every repo (`TierScope::All`) | — |
| `tier` | Repos in `--tier` | `--tier <NAME>` (default `core`) |
| `repo` | `TierScope::All` intersected with a single repo | `--repo <SLUG>` (required) |
| `epic` | Epic projection; overrides `--tier`/`--repo` | `--epic <SLUG>` (required) |

Resolves `brain.toml` by walking up from `path`. If no `brain.toml` is found, the process
exits 1 with an error message on stderr (mentioning `brain.toml`).

`mev emit-block-graph` is a **pure emit**: nothing is ever written to disk, no cache, no
side effects. It does not re-derive the block graph — it is a serializer over
`mev::block_graph_brain` / `build_block_graph_export`, and the output is emitted **verbatim**:
no post-processing, no field reordering, no added or dropped keys. This is the CLI companion
to bastion's `GET /api/blocks/graph` (`BA.17.A`) — node counts for a given scope must match
that endpoint's.

#### Output shape

```json
{
  "version": "1",
  "root": "/path/to/brain",
  "scope": {
    "tier": null,
    "epic": null,
    "repo": null,
    "include_closed": false,
    "include_boundary": false
  },
  "nodes": [
    {
      "key": "repo:BLOCK-ID",
      "repo": "repo",
      "id": "BLOCK-ID",
      "title": "...",
      "status": "open",
      "lane": "next",
      "track": "Phase 1",
      "wave": 1,
      "priority": 2,
      "effective_priority": 2,
      "due": null,
      "epics": [],
      "layer": 0,
      "topo_index": 0,
      "ready": true,
      "in_cycle": false,
      "in_scope": true,
      "external_deps": [],
      "unmet_count": 0,
      "dependent_count": 0,
      "last_touched": null,
      "reconcile_failed": null
    }
  ],
  "edges": [
    {
      "from": "repo:BLOCK-ID",
      "to_ref": "repo:OTHER-ID",
      "kind": "blocked_by",
      "target_node_id": "repo:OTHER-ID",
      "blocking": true
    }
  ],
  "cycles": [],
  "total_nodes": 1,
  "truncated": false
}
```

#### Field guide

`version`, `root`, `scope` (an echo of the resolved scope request), `nodes`, `edges`, `cycles`
(over the **full corpus**, never the scoped subgraph), `total_nodes` (node count before any
`--max-nodes` truncation), and `truncated`.

Every node carries the full-corpus derivations that back the state-graph views:

| Field | Description |
|---|---|
| `lane` | Derived attention lane: `now` / `next` / `blocked` / `deferred` / `closed` / `other` |
| `layer` | Longest path over resolved `depends_on` edges (`0` = no resolved prerequisites) |
| `topo_index` | Position in the full-corpus topological order |
| `effective_priority` | Effective priority; absent when it never lands in the real `0..=3` range |
| `in_scope` | `true` for a scope survivor, `false` for a node re-added only as an `--include-boundary` neighbour |
| `unmet_count` | Count of unmet dependencies for a `blocked` node; `0` for every other lane |
| `dependent_count` | Corpus-wide count of in-corpus blocks whose `BlockedBy` edges point at this node (`CrossRepo` edges excluded). Computed over the **full corpus before scope filtering**, exactly like `layer`, `topo_index`, and `effective_priority` — so it is **identical for a given node across a scoped and an unscoped export**, and reports `0` (never absent, never a sentinel) for a node nothing depends on |
| `last_touched` | Derived — never authored in any `state.json` — from the block's own on-disk SDLC run artifacts: `<spec-folder>/sdlc/sdlc-{flow,task,run,}-state.json` (all four kinds are read). The **newest** `updated_at` wins across every matched spec folder and every state-file kind, including folders under `planning/archive/`. Computed over the **full corpus before scope filtering**, exactly like `dependent_count` — so it is **identical for a given node across a scoped and an unscoped export**. `null` means the block has **never been worked**, not that it was worked long ago — no sentinel date and no `state.json.updated` fallback is ever substituted for a missing run |
| `reconcile_failed` | Derived from the **same winning state file** as `last_touched` (never a different file or folder for the same block) — `true` when that file's run-state `status` is `"reconcile_failed"`, `false` when a run was found and its status was something else, and `null`/absent when no run state exists at all for the block (same absence-means-"never worked" honesty rule as `last_touched`; the field is `#[serde(skip_serializing_if)]`, so it does not appear in the JSON for a block with no run). This is the **run-state** `status` field inside `sdlc-task-state.json` (and its `-flow-`/`-run-`/plain sibling kinds) — a completely different vocabulary from the block's own **authored** `status` in `state.json` (`open`/`in_progress`/`deferred`/`closed`/`wontfix`). A `reconcile_failed` run never changes the authored status and never changes `lane` derivation (lane reads only the authored `state.json` status). The terminal run-state vocabulary itself — what `reconcile_failed` and its siblings mean, and what a consumer must not fold them into — is pinned at base-template's `docs/data-contract.md` (`doc_id: sdlc-run-state-data-contract`); this field does not re-derive that vocabulary, only surfaces it. The human-readable sibling of this JSON field is [`render_block_graph_reconcile_failed`](../src/brain/emit.rs), which annotates a `BlockGraphExport`'s per-block text lines with `" (reconcile_failed)"` when this field is `true`, and renders byte-identical output to before the annotation existed when no block has it set |

#### Exit codes

| Code | Meaning |
|---|---|
| `0` | Graph emitted successfully |
| `1` | `brain.toml` not found, `--scope epic` given without `--epic`, `--scope repo` given without `--repo`, an unknown or blank `--epic` slug, or a serialization/runtime error |

**Examples:**

```bash
# Compact JSON, whole corpus, from the current directory
mev emit-block-graph

# Pretty-printed JSON from an explicit brain root
mev emit-block-graph --pretty ~/Dev/agentic-portfolio

# Scope to one tier
mev emit-block-graph --scope tier --tier core

# Scope to one repo
mev emit-block-graph --scope repo --repo mev

# Project onto one epic
mev emit-block-graph --scope epic --epic bastion-tui

# Include closed blocks and boundary neighbours
mev emit-block-graph --include-closed --include-boundary

# Cap the node list and check whether it truncated
mev emit-block-graph --max-nodes 50 | jq '.truncated'

# Summary counts via jq (the program-plan smoke check)
mev emit-block-graph --pretty ~/Dev/agentic-portfolio | jq '{v:.version, n:(.nodes|length), e:(.edges|length), cycles:(.cycles|length), truncated}'
```

---

### `frontier [--json] [path]`

Print the corpus-wide lane frontier: one entry per active `(roadmap, lane, segment)`
naming its startable-or-blocked head block — `MV.13.B`, Task 4. Read-only; never writes
`planning/lane-frontier.json` (that write happens only via `mev emit-state --write`,
which runs this same derivation as one of its planners).

```bash
mev frontier [path]
mev frontier --json [path]
```

| Argument / Flag | Default | Description |
|---|---|---|
| `path` | `.` | Path to search from when locating `brain.toml` (walks up to find it) |
| `--json` | off | Emit the `lane-frontier.json` artifact shape (`derived_at`, `entries`, `gate_ranks`) instead of one text line per entry |

#### The consumer contract for HTTP-side closure

**Closure over the block graph MUST run in mev itself, over the untruncated corpus.**
`mev frontier` always builds the in-process graph with `max_nodes: usize::MAX` — never
the HTTP export's truncated default. `mev emit-block-graph` (and bastion's `GET
/api/blocks/graph`, `BA.17.A`) default to `max_nodes=400` against a corpus of ~756
blocks: a client that runs its own closure over that default silently drops gates from
the frontier it computes.

**Any HTTP-side closure — bastion's `/lanes` and concurrency-slot endpoints (`BA.19.C`,
`BA.19.D`) included — MUST send `max_nodes=2000` and hard-fail on `truncated: true`
rather than degrade.** mev cannot gate that half of the contract itself — the evidence
that bastion honours it lives in bastion's own repo, not here. mev's own obligations are
(1) the `ensure_untruncated` refusal, which guarantees `mev frontier`/`mev emit-state`
never computes a frontier over a truncated node set, and (2) this written contract for
every downstream consumer to build against.

#### Text output shape

One line per frontier entry:

```
{roadmap}/{lane}#{segment} {repo}:{id} — startable
{roadmap}/{lane}#{segment} {repo}:{id} — blocked by <reason>[, <reason>...]
```

`<reason>` is each unmet `blocked_by` dependency: `repo:id` for a block dep,
`operator:<slug>` / `approval:<slug>` for a gate, `external:<what>` for an external dep.

#### `--json` output shape

```json
{
  "derived_at": "2026-08-17T13:19:58.661626-03:00",
  "entries": [
    {
      "roadmap": "engine-orchestration",
      "lane": "derive",
      "segment": 0,
      "repo": "mev",
      "key": "mev:MV.13.B",
      "id": "MV.13.B",
      "title": "Frontier computation + gate_rank",
      "status": "in_progress",
      "unmet_blocks": [],
      "unmet_gates": [],
      "startable": true
    }
  ],
  "gate_ranks": [
    {
      "kind": "operator",
      "slug": "operator-fleet-concurrency-live-smoke-test",
      "rank": 1,
      "gates": ["base-template:BT.ticket.heavy-command-signals-rust-build"]
    }
  ]
}
```

`derived_at` is an RFC 3339 timestamp of this run, not of the last `state.json` commit —
lane progress lands live between `/log-work` runs, so a consumer needs this field to
tell how stale the frontier is relative to the corpus it read. `gate_ranks` derives a
rank for operator/approval gates, which are targetless (they gate a block but have no
dependents of their own) and so never receive an `effective_priority` directly: each
gate's rank is the minimum effective priority across every block it gates.

#### Exit codes

| Code | Meaning |
|---|---|
| `0` | Frontier computed and printed |
| `1` | `brain.toml` not found/unreadable, or the in-process graph reported `truncated: true` (should not happen at `max_nodes: usize::MAX`, but this command refuses rather than degrading if it ever does) |

**Examples:**

```bash
# Text frontier from the current directory
mev frontier

# JSON frontier from an explicit brain root
mev frontier --json ~/Dev/agentic-portfolio

# Just the startable heads
mev frontier --json | jq '.entries[] | select(.startable)'
```

---

### `lanes [--json] [path]`

Print six-state lane-segment availability plus lane-level unblock leverage, computed
over the corpus-wide lane frontier — `MV.13.C`, Task 5. Read-only; never writes
`planning/lane-availability.json` (that write happens only via `mev emit-state
--write`, which runs this same derivation as one of its planners).

```bash
mev lanes [path]
mev lanes --json [path]
```

| Argument / Flag | Default | Description |
|---|---|---|
| `path` | `.` | Path to search from when locating `brain.toml` (walks up to find it) |
| `--json` | off | Emit the `lane-availability.json` artifact shape (`derived_at`, `degraded`, `segments`) instead of one text line per segment |

#### The six states

Every lane segment resolves to exactly one of six states, in this fixed precedence
(highest first) — see `docs/architecture.md` for the full rationale:

`done` > `held-block` > `held-operator` > `held-repo-busy` > `held-slot` > `startable`

`held-repo-busy` is derived from exactly one source of lane-liveness truth: the
per-`(repo, roadmap)` orchestration-run record's `lifecycle:` frontmatter
(`planning/orchestration-run/<roadmap>/notes.md`) — never `lane-log.jsonl` or the
`.fleet-locks` fleet-lock registry. `docs/architecture.md` names both rejected
candidates and why.

#### Text output shape

One line per segment:

```
{roadmap}/{lane}#{segment} {repo}:{id} — {availability} (<reason>) frees N lane(s)
```

`{id}` renders as `-` for `done` segments (no live head). The `(<reason>)` clause is
omitted entirely for `startable`/`done`, which need no explanation. `frees N lane(s)`
is always present, including `frees 0 lanes` — the zero case is a real answer, not an
absence.

> **Known caveat — `frees N lane(s)` on a `done` segment is historical, not actionable.**
> A closed segment still reports the lanes it *used to* gate, which are already free, so a
> board that sorts or ranks by `lanes_freed` will float finished work to the top. Filter
> `done` out rather than trusting the number. Tracked as a mev carryover
> (`lanes-freed-nonzero-on-done-segments`); deliberately not folded into
> `MV.ticket.done-segment-discovery`, whose scope was the missing state itself.

#### `--json` output shape

```json
{
  "derived_at": "2026-08-17T17:53:38.409934-03:00",
  "degraded": false,
  "segments": [
    {
      "roadmap": "engine-orchestration",
      "lane": "derive",
      "segment": 0,
      "repo": "mev",
      "head": "mev:MV.13.C",
      "availability": "held-repo-busy",
      "reason": "repo mev is live on carryover-improvements",
      "leverage": {
        "lanes_freed": 1,
        "lanes": ["engine-orchestration/wire"]
      }
    }
  ]
}
```

`derived_at` is an RFC 3339 timestamp of this run, same rationale as `mev frontier`'s
field of the same name. `degraded` is `true` when the fleet-lock read that feeds
`held-slot` could not run (`.fleet-locks` missing or unreadable) — "unknown", never a
hold; a consumer can use it to tell a corpus with zero live `held-slot` holds apart
from one that could not check. Each segment's `leverage.lanes_freed` counts distinct
`(roadmap, lane)` pairs downstream of this segment — a lane-scoped metric, distinct
from the block-graph export's block-scoped `dependent_count`.

#### Exit codes

| Code | Meaning |
|---|---|
| `0` | Availability computed and printed |
| `1` | `brain.toml` not found/unreadable, or the in-process graph reported `truncated: true` (should not happen at `max_nodes: usize::MAX`, but this command refuses rather than degrading if it ever does) |

**Examples:**

```bash
# Text availability from the current directory
mev lanes

# JSON availability from an explicit brain root
mev lanes --json ~/Dev/agentic-portfolio

# Just the startable segments with nonzero leverage
mev lanes --json | jq '.segments[] | select(.availability == "startable" and .leverage.lanes_freed > 0)'
```

---

### `emit-state [--write] [path]`

Regenerate all derived views in the Brain corpus from the authored `tracks[]` DAG and write them in place (with `--write`) or report what would change (dry-run, without `--write`).

`mev emit-state` is the **single derivation engine** that `/log-work` shells out to for regenerating leaf `focus` fields, the brain `repos[]` / `cross_repo[]` rollup, brain `focus`, the master-plan wave/dependency tables, the per-project cache docs (focus line + `synced_from` watermark), the tier sub-brain rollup tables, the HQ Operating Board, the HQ unified priority board, and (MV.13.A) the cross-repo `planning/lane-segments.json` artifact — every live `lane-*.txt` file's blocks segmented into `{roadmap, lane, segment, position}` runs by `state.json` ownership, with double-claimed cross-roadmap blocks resolved via `# ORIGIN:` annotations. Because the validator's `check_focus_drift` and `check_rollup` share the same `derive_focus` / `derive_rollup` functions, running `mev emit-state --write` followed by `mev validate-brain --state` on the same corpus will report zero `W_STATE_FOCUS_DRIFT` and zero `W_STATE_ROLLUP_DRIFT` — the emit is, by construction, the fixed point of the drift check across every generated surface.

```bash
mev emit-state [--write] [--scope <repo>] [path]
```

| Argument / Flag | Default | Description |
|---|---|---|
| `path` | `.` | Path to search from when locating `brain.toml` (walks up to find it) |
| `--write` | off | Write the derived views in place. Without this flag the command is a dry-run |
| `--scope <repo>` | unset (whole corpus) | Limit regeneration to one repo's own derived surfaces plus the rollups it feeds — nothing else. Omit for today's default full-corpus behaviour, byte-for-byte unchanged. |

#### `--scope <repo>` — per-repo regeneration

Unscoped, `emit-state --write` regenerates the derived surfaces of **every** registered repo on
every run — a single `/log-work` in one sub-repo dirties dozens of files across the whole corpus.
`--scope <repo>` narrows a `--write` to exactly the surfaces that one repo feeds, derived
mechanically from the `[[repos]]` registry in `brain.toml` (never hardcoded):

- the repo's own leaf `planning/state.json`,
- its `cache_doc` (e.g. `docs/projects/<slug>.md`),
- its tier container's rollup `status.md` (the `[[repos]]` entry whose `slug` matches this repo's
  `tier`), when that tier resolves to a distinct registered entry, and
- the HQ root's `status.md` (the Operating Board) — every repo feeds this one.

Every other repo's files are left byte-identical. A scoped run never blanks or truncates a repo it
did not visit — rollups preserve every row it didn't touch. An unknown `--scope` slug fails fast
with `E_EMIT_UNKNOWN_SCOPE`, naming every valid slug, before any planner runs or any file is
touched.

**Operating guidance:** reach for `--scope <repo>` from a single sub-repo's own workflow commands
(`/log-work`, `/start-block`, `/blocked`, …) where only that repo's state changed — it keeps the
diff local and avoids cross-repo churn when several agents are working concurrently. Leave the
periodic full reconciliation (cron-bound `routine.sh`, or any run meant to catch drift across the
whole corpus) unscoped — only a full run recomputes every cross-repo edge (`cross_repo[]`, the
unified board's cross-repo priority sort, epic relationships) that a single repo's scope cannot see.

```bash
# Regenerate only mev's own derived surfaces (its state.json, cache doc, tier rollup, HQ board)
mev emit-state --scope mev --write

# Unknown slug: fails fast, names the valid slugs, writes nothing
mev emit-state --scope not-a-repo --write
```

#### Advisory lock on `--write`

`--write` (scoped or not) takes an exclusive advisory lockfile at `<root>/.mev-emit.lock` for the
duration of the run, recording the owning pid. This guards against concurrent `emit-state --write`
invocations interleaving writes to the same derived file — a real risk given how many workflow
commands shell out to `emit-state --write`, and one `E_EMIT_LINKED_WORKTREE` does not cover: that
guard only catches a linked git *worktree*, not the symlinked `planning/` vaults (D46) two agents
in different sub-repos can both be writing through at once.

A second concurrent `--write` polls briefly for the lock to free up, then fails with
`E_EMIT_LOCK_HELD` (naming the holder's pid) and writes nothing rather than interleaving. A
lockfile whose owning process is no longer alive is reclaimed automatically instead of blocking
forever. Dry-run (no `--write`) never takes the lock and is unaffected by contention.

#### Derived views updated

- **Leaf `state.json`** (`kind == "project"`): regenerates `focus` — `now` = blocks with `status: in_progress`; `next` = ready open blocks in `wave` order; `blocked` = open blocks with an unmet `depends_on`, each carrying the unmet `blocked_by[]` subset. Authored `tracks[]` and all other fields survive the round-trip unchanged.
- **Brain `state.json`** (`kind == "brain"`): regenerates `repos[]`, `cross_repo[]` (cross-repo `depends_on` edges), and the brain file's own `focus`. Authored `tracks[]`, `backlog[]`, and `tiers[]` are left untouched.
  - `repos[]` is **tier-scoped**: a brain file whose `repo` slug matches a `tier` value in `brain.toml` (e.g. `core`) scopes to only that tier's `[[repos]]`; a brain file whose `repo` matches no tier (the HQ root) scopes to every repo. See `tier_scope_for`.
  - `repos[]` is **non-destructive**: for each in-scope repo, if a loadable child `state.json` exists, its headline is derived as before (`RepoRollup.tier` populated from config); if not, but the brain file already carries a `repos[]` entry for that slug, the entry is **preserved verbatim** (with `tier` backfilled); only when neither exists is a tier-tagged empty stub emitted. A malformed or not-yet-authored child `state.json` can therefore never silently drop a repo out of the rollup.
  - `focus.now/next/blocked` is derived as the **repo-tagged union** of the in-scope children's own derived `focus` (each block carries its source `repo`), in config-repo order then within-child order, deduplicated by `(repo, id)`. Repos with no loadable child contribute nothing to `focus` (they still surface in `repos[]` via the preserve/stub branch).
- **`master-plan.md` wave tables**: splices a rendered wave/dependency Markdown table between the `<!-- BEGIN generated:wave-table -->` and `<!-- END generated:wave-table -->` sentinels. All narrative lines outside the sentinels are preserved verbatim. Re-running the emit is idempotent — if the splice produces no change, no `EmitAction` is recorded.
- **`master-plan.md` body** (`MV.ticket.master-plan-generator`, same file as the wave table but a separate sentinel region): splices an initiative index plus per-phase block sections — each block's title, description, status (`open` when absent), wave (`—` when absent) and dependency edges — between `<!-- BEGIN generated:master-plan-body -->` and `<!-- END generated:master-plan-body -->`. Initiative labels are read from optional `planning/blocks/<ID>.json` records; malformed records warn and are skipped. A repo whose `master-plan.md` is missing, carries no sentinel pair, or has no blocks is skipped (`W_EMIT_NO_SENTINEL`, or silently when there is simply nothing to render) — the sentinel pair is never created for you, so the generator stays inert until a repo opts in by adding it.
- **Project-cache docs** (`docs/projects/<slug>.md`, one per leaf project repo): splices the derived focus headline into the `<!-- BEGIN generated:project-cache -->` / `<!-- END generated:project-cache -->` sentinels and reconciles the doc's OKF frontmatter `synced_from` field to the child `state.json`'s `updated` watermark. A repo with no matching `[[repos]]` entry, or whose entry has a blank `cache_doc`, is silently skipped (nothing to target).
- **Tier rollup tables** (each tier sub-brain's sibling `status.md`): splices a rendered per-repo now/next/blocked rollup table into the `<!-- BEGIN generated:tier-rollup -->` / `<!-- END generated:tier-rollup -->` sentinels. Only brain files scoped to a single tier (`tier_scope_for` resolves to `TierScope::Tier`) are targeted — the HQ root (`TierScope::All`) is skipped by this planner.
- **HQ Operating Board** (the HQ brain's `status.md`): splices a rendered NOW/NEXT/BLOCKED board across every registered repo into the `<!-- BEGIN generated:hq-board -->` / `<!-- END generated:hq-board -->` sentinels.
- **HQ unified priority board** (the same HQ brain's `status.md`, independent sentinel region): splices a priority-ranked NOW/NEXT/BLOCKED/DUE-SOON board into the `<!-- BEGIN generated:unified-board -->` / `<!-- END generated:unified-board -->` sentinels. Rows are tagged `[BIZ]`/`[ENG]` by the source repo's configured tier; `NEXT` is stably re-sorted by `(effective priority asc, due asc)` (absent values last, wave order as the implicit tiebreak). Effective priority (MV.7.A) is computed by `effective_priorities` via reverse-topological `min`-propagation over the `depends_on` DAG, so a block with no own priority that gates a hotter dependent inherits that dependent's priority and floats to the top instead of sorting last; it falls back to the block's own raw `priority` when no hotter dependent exists. `DUE-SOON` lists blocks due within 14 days (overdue included and annotated) sorted by due date ascending.
- **Attention board** (every brain-level `status.md`, tier-scoped): splices the stale-item board into the `<!-- BEGIN generated:attention -->` / `<!-- END generated:attention -->` sentinels. Unlike the boards above (HQ root only), this emits for **both** scopes: the HQ root (`TierScope::All`) unions `carryover[]` from every loaded repo/tier plus the whole HQ `backlog[]`; each tier sub-brain (`TierScope::Tier`) shows its own tier's leaf-repo carryover (plus the tier brain's own) and the HQ backlog nodes whose `repo` belongs to that tier. Seven lanes total: four **carryover triage lanes** — `BLOCKING` · `HOT` · `AGING` · `STANDING` (`MV.ticket.carryover-triage-ranking`) — followed by Aging backlog · Orphaned captures · Stale distilled knowledge, each row `[<repo>]`-tagged. See [Carryover triage lanes](#carryover-triage-lanes) below for how the first four are populated and ordered; the latter three are unchanged — sorted oldest-first, showing only items past their `[attention]` threshold (the visible twin of `W_STATE_BACKLOG_STALE`/`W_DISTILL_STALE`). The fourth lane (distill-freshness-lane) reads each repo's `knowledge.md`/`memory.md` once (cached across boards) and lists D35-distilled entries whose `distill_stale_age` exceeds the `[attention]` `knowledge_days`/`memory_days` threshold, capped at 10 rows per board with an "…and N more" tail — the same predicate `check_distill_staleness` fires `W_DISTILL_STALE` on, so the board never shows an entry the warning didn't also flag.

#### Carryover triage lanes

**Board membership no longer gates on staleness alone.** Before `MV.ticket.carryover-triage-ranking`,
the carryover lane was a single age-sorted "Stale carryover" list gated by `carryover_stale_age`
(the visible twin of `W_STATE_CARRYOVER_STALE`) — measured against the live corpus, only **6 of 142**
`carryover[]` entries were stale, so the board hid the other **136**, including every P0 filed that
day. Every non-snoozed `carryover[]` entry is now ranked via the public `rank_carryover` function
(re-exported from `src/lib.rs`; see [`docs/carryover-contract.md`](carryover-contract.md) for the
full, versioned, producer-owned contract) and lands in exactly one of four lanes, assigned in this
order so membership is total and mutually exclusive:

| Lane | Membership | Within-lane order |
|---|---|---|
| `BLOCKING` | at least one unmet `blocks[]` edge | effective priority of what it blocks, ascending (0 hottest); then age descending |
| `HOT` | authored `priority` 0 or 1, not already `BLOCKING` | priority ascending, then age descending |
| `AGING` | stale (per `carryover_stale_age`), and `priority` 2/3 or absent | age descending |
| `STANDING` | no `priority` and no `blocks[]` | age descending |

`carryover_stale_age` remains the single source of the `stale` flag and feeds only `AGING`
membership plus every row's displayed age — it is never reimplemented for this pass.
`effective_priority` propagates across a carryover's `blocks[]` edges by the same cycle-safe
reverse-topological min-propagation the block dependency graph already uses, so a low-priority
carryover gating a hot block inherits that hotness; ties and cycles resolve deterministically and
never hang.

`STANDING` is a **low-frequency re-affirm lane**, not a backlog: it exists so permanent
constraints (e.g. "`planning/` is a symlink, pass `-L`") stop competing for attention with
actionable P0/P1 work and blocking edges.

Each triage lane is capped at `CARRYOVER_LANE_CAP` (20) rows, oldest/lowest-ranked dropped first
within the lane's own order, with an explicit `- …and N more` line stating the true hidden count
when the lane is over cap — never a silent truncation, matching the existing convention on the
distilled-knowledge lane's cap. `mev carryover --json` remains the uncapped, full-fidelity export
for a consumer that needs every entry.
- **Portfolio `state.json`** (`kind == "portfolio"`): not regenerated at all (no `focus` to derive — these are terminal repos), and skipped entirely by the wave-table splice pass — no `master-plan.md` is expected, so no `W_EMIT_NO_SENTINEL` is raised for these repos.

All of the project-cache, tier-rollup, HQ-board, and unified-board planners share the same fixed-point and sentinel-safety guarantees as the wave-table splice: a target document missing its sentinel pair produces a `W_EMIT_NO_SENTINEL` warning and is left untouched, and re-running the emit over already-emitted content produces no further `EmitAction`/`I_EMIT_WROTE`.

#### Sentinel contract

Every planner splices into its own named marker, using the same `<!-- BEGIN generated:<marker> --> ... <!-- END generated:<marker> -->` pair format. The wave-table example:

```markdown
<!-- BEGIN generated:wave-table -->
| Wave | Block | Title | Status | Depends on |
| --- | --- | --- | --- | --- |
... (generated rows) ...
<!-- END generated:wave-table -->
```

The other planners use their own markers in the same document types: `project-cache` (leaf `docs/projects/<slug>.md`), `tier-rollup` (tier sub-brain `status.md`), `hq-board` (HQ `status.md`), `unified-board` (the same HQ `status.md`, an independent sentinel region alongside `hq-board`), and `attention` (every brain-level `status.md` — HQ and each tier sub-brain — an independent sentinel region).

- Both `BEGIN` and `END` sentinels must be present and balanced; a missing or unbalanced pair causes a `W_EMIT_NO_SENTINEL` warning and the file is skipped — sentinels are never invented into arbitrary prose.
- Re-splicing an already-emitted table is idempotent.

#### Diagnostic codes

| Locator | Severity | Condition |
|---|---|---|
| `W_EMIT_DRY_RUN` | Warning | Planned action (dry-run only; no file written) |
| `I_EMIT_WROTE` | Warning | File written (`--write` mode) |
| `W_EMIT_NO_SENTINEL` | Warning | A target document is missing its marker's sentinel pair (`wave-table`, `project-cache`, `tier-rollup`, `hq-board`, `unified-board`, or `attention`); file skipped |
| `E_EMIT_WRITE_FAILED` | Error | IO error writing a file; causes exit 1 |
| `E_CONFIG_NOT_FOUND` | Error | `brain.toml` could not be located by walking up from `path`; causes exit 1 |
| `E_EMIT_LINKED_WORKTREE` | Error | `--write` invoked from inside a linked git worktree; causes exit 1 |
| `E_EMIT_INCOMPLETE_CORPUS` | Error | `--write` refused because one or more discovered `state.json` files failed to load; causes exit 1 |
| `E_EMIT_UNKNOWN_SCOPE` | Error | `--scope` names a slug with no matching `[[repos]]` entry in `brain.toml`; the message names every valid slug; causes exit 1 |
| `E_EMIT_LOCK_HELD` | Error | `--write` could not acquire the advisory lock at `<root>/.mev-emit.lock` within the timeout because another live process already holds it (names the holder pid); causes exit 1. A lockfile whose owning process is no longer alive is reclaimed automatically instead of blocking forever. Dry-run never takes the lock. |

`--write` refuses to run when `path` resolves to a linked git worktree (e.g. `trees/<slug>/` under a
repo that already has its own main working tree) — `emit-state` resolves every repo's derived-file
paths from `brain.toml`, not from CWD, so writing from a worktree would silently regenerate the
**main checkout's** files instead of the worktree's own copy. The command prints an error naming
the worktree path and exits non-zero (`E_EMIT_LINKED_WORKTREE`) without writing anything. Dry-run
(no `--write`) is read-only and is never gated — it still succeeds from inside a worktree. Run
`--write` from the repo's main working tree instead.

`--write` also refuses to run when the corpus is incomplete: if any discovered `state.json` fails
to load (an `E_STATE_MALFORMED_JSON` diagnostic), every derived view is a cross-repo union
(`repos[]`/`cross_repo[]`, tier rollups, HQ/unified/epic boards, master-plan and epic sequence
tables) — regenerating them from a partial corpus would silently erase the missing repo(s) from
every surface, and rewriting `cross_repo[]` would delete the dangling references that are the only
evidence of the failure. The command pushes `E_EMIT_INCOMPLETE_CORPUS` alongside the underlying
`E_STATE_MALFORMED_JSON` cause, writes nothing, and exits non-zero. Dry-run is unaffected — it is
the diagnostic tool for exactly this situation, and still runs every planner and reports the
`W_EMIT_DRY_RUN` actions that would have been taken. Fix the load failure named by
`E_STATE_MALFORMED_JSON`, then re-run `--write`.

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

# Write only mev's own derived surfaces (leaf state.json, cache doc, tier rollup, HQ board)
mev emit-state --scope mev --write

# Machine-readable dry-run output
mev --json emit-state

# Machine-readable write output
mev --json emit-state --write ~/Dev/agentic-portfolio
```

#### Revision history

Every `--write` overwrite of an existing file goes through `apply_plan()`'s append-only writer
(see `mev state-history` below): before the new content lands, the file's **prior** content is
snapshotted to `<dir>/.mev-history/<filename>/<seq>__<timestamp>`, then the write itself lands
atomically (temp file in the destination's own directory, then `fs::rename`). Creating a
brand-new file records no revision — there is no prior content to lose. A snapshot/prune failure
emits `W_HISTORY_FAILED` and does not block the primary write; history is a safety net, never a
new way for `emit-state` to fail. Snapshotting is controlled by the `[history]` table in
`brain.toml` (`enabled`, `keep`) — see `docs/brain-toml.md`. Dry-run remains fully side-effect-free:
no history directory is created and nothing is written.

---

### `state-history <path> [--restore <seq>]`

List (or restore) the append-only revision history `apply_plan()` records for one file every time
it overwrites existing content. This is the read-back half of the "Revision history" note on
`emit-state` above — a snapshot nobody can retrieve is inert, so `state-history` is what makes a
bad derived write recoverable.

```bash
mev state-history <path> [--restore <seq>]
```

| Argument / Flag | Default | Description |
|---|---|---|
| `path` | required | The file whose revision history to list or restore (e.g. `planning/state.json`), not a brain root to search from — every other subcommand's `path` walks up looking for `brain.toml`; this one already knows exactly which file's history it wants. |
| `--restore <seq>` | unset (list mode) | Restore revision `<seq>`'s content back to `path` instead of listing. |

#### List mode (default)

Read-only; never takes the advisory lock. Prints that file's revisions **newest first** — seq,
UTC timestamp, byte size:

```
     2  20260804T120501Z  842 bytes
     1  20260804T091203Z  798 bytes
```

A file with no recorded revisions prints an explicit `no revisions recorded for <path>` message
and exits successfully — an empty history is a normal state, not an error. `--json` emits a
compact/pretty JSON array of `{seq, recorded_at, bytes}` records, newest first.

#### `--restore <seq>`

Reads revision `<seq>`, first records the file's **current** on-disk content as a new revision
(so a wrong restore is itself undoable via a second restore), then writes revision `<seq>`'s
content back to `<path>` atomically via the same temp-file + rename helper `apply_plan()` uses.
Prints what was restored and what the pre-restore content was saved as (or the JSON equivalent
under `--json`: `restored_seq`, `path`, `pre_restore_revision`).

Because it mutates the file, `--restore` takes the same advisory lock at `<root>/.mev-emit.lock`
that `emit-state --write` takes (resolved from `path`'s own parent directory, walking up to find
`brain.toml`), and the same linked-worktree guard — refusing to run from inside a linked git
worktree with the same shape of message `emit-state --write` gives. List mode is read-only and
skips both checks, exactly like emit-state's dry-run.

An unknown `--restore <seq>` fails, naming the valid seq range. A path with no revisions and
`--restore` given still exits successfully with the "no revisions recorded" message — there is
nothing to restore.

#### Diagnostic codes

| Locator | Severity | Condition |
|---|---|---|
| `W_HISTORY_FAILED` | Warning | The pre-restore snapshot could not be recorded; the restore itself still proceeds (history is a safety net, never a new way for restore to fail) |
| `E_EMIT_LINKED_WORKTREE` | Error | `--restore` invoked from inside a linked git worktree; refused before the lock is taken |
| `E_EMIT_LOCK_HELD` | Error | `--restore` could not acquire the advisory lock because another live write process already holds it (names the holder pid); a stale lock (owning process no longer alive) is reclaimed automatically instead |

#### Exit codes

| Code | Meaning |
|---|---|
| `0` | Revisions listed, "no revisions recorded", or restore applied |
| `1` | No revision `<seq>` on disk (names the valid seq range), `E_EMIT_LOCK_HELD`, a linked-worktree refusal, or an IO failure reading/writing the file |

**Examples:**

```bash
# List a file's revision history, newest first
mev state-history planning/state.json

# Machine-readable listing
mev --json state-history planning/state.json

# Restore revision 1 (also snapshots the current content first)
mev state-history planning/state.json --restore 1
```

---

### `defer-epic <slug> [--write] [path]` · `resume-epic <slug> [--write] [path]` · `complete-epic <slug> [--write] [path]` · `sync-epics [--write] [path]`

Park and un-park a whole initiative, keeping the HQ `epics[]` registry status and
its member blocks' authored statuses in agreement.

An epic is "parked" when its registry `status` is `paused` **and** its unfinished
member blocks are `deferred`. Those two can drift — a `paused` epic whose blocks
are still `open` keeps flooding `focus.next` even though you consider the
initiative shelved. These commands move both together.

| Command | Registry | Member blocks |
|---|---|---|
| `defer-epic <slug>` | → `paused` | `open` → `deferred` |
| `resume-epic <slug>` | → `active` | `deferred` → `open` |
| `complete-epic <slug>` | → `complete` | untouched |
| `sync-epics` | fully-deferred epics → `paused` | stragglers in a `paused` epic → `deferred` |

**`complete-epic` is the odd one out: it never cascades.** `defer-epic` and
`resume-epic` move member blocks along with the registry; `complete-epic` sets
*only* the registry epic's status to `complete` and touches zero member blocks.
It is an **operator declaration** that an initiative is finished, not something
mev infers — `W_STATE_EPIC_ALL_CLOSED` (all members closed) stays warn-only by
design, precisely because the last block closing is not the same as the goal
being met. A `complete` epic drops off the board entirely. There is no
`reopen-epic`; if that turns out to be wrong, undo it by hand in the registry.

**`in_progress` blocks are never touched**, in either direction. Parking work you
are mid-block on is far more likely to be a mistake than an intent, so it is left
alone and reported as `W_EPIC_SKIPPED_IN_PROGRESS`. `closed` blocks are likewise
never reopened.

**`sync-epics` never un-defers anything.** An `active` epic with *some* deferred
blocks is a perfectly normal state (you parked two of nine). Un-parking is always
explicit, via `resume-epic`.

**Dry-run by default**, exactly like `emit-state`: without `--write` the proposed
edits print as `W_EMIT_DRY_RUN` and nothing is touched. A successful `--write`
additionally runs `emit-state --write`, so `focus`, the boards and the rollups are
regenerated in the same invocation instead of being left drifted.

**`--write` takes the same advisory lock `emit-state --write` and
`set-block-status --write` take**, at `<root>/.mev-emit.lock`, before any file is
touched — `defer-epic`, `resume-epic`, `complete-epic`, and `sync-epics` all
share one dispatch function, so one lock acquisition covers all four. If another live process
already holds it, the command fails with `E_EMIT_LOCK_HELD` (naming the holder's
pid) and writes nothing; a lockfile whose owning process is no longer alive is
reclaimed automatically instead of blocking forever. Dry-run (no `--write`) never
takes the lock and is unaffected by contention.

> **These, plus [`set-block-status`](#set-block-status-repoid-status---write-path),
> are the only commands that write *authored* state.** Everything else mev writes is
> derived. The cascade lives behind an explicit command precisely so `emit-state`
> stays safe to run unattended — see `src/brain/epics.rs` for the full rationale.

```bash
# What would parking the TUI initiative change?
mev defer-epic bastion-tui

# Park it (and regenerate every derived view)
mev defer-epic bastion-tui --write

# Bring it back
mev resume-epic bastion-tui --write

# You deferred blocks by hand; make the registry agree
mev sync-epics --write

# Declare the initiative finished (registry only — no member block is touched)
mev complete-epic bastion-tui --write
```

Exit codes: `0` planned/applied successfully · `1` unknown epic slug
(`E_EPIC_UNKNOWN`), no HQ registry (`E_EPIC_NO_REGISTRY`), an unreadable
state.json (`E_EPIC_INCOMPLETE_CORPUS` on `--write`), the advisory lock already
held (`E_EMIT_LOCK_HELD` on `--write`), or a write failure.

---

### `set-block-status <repo:id> <status> [path] [--write] [--force-operator-gate]`

Set **one** block's authored `status` in its repo's `planning/state.json`. The
block-level counterpart to the epic commands above: those move a whole initiative,
this moves exactly one block and nothing else.

**Status only.** Not `priority`, not `due`, not a generic `set-block-field`. The
narrow surface keeps the caller's contract precise; a generic setter would push
per-field validation to runtime.

**The key is always `repo:id`** — e.g. `mev:MV.10.A` — the same
`"{repo_slug}:{block_id}"` form `global_status_map` and `effective_priorities` use.
Block ids are only unique *within* a repo, so an unqualified id is **rejected**
rather than guessed at.

| Status | Meaning |
|---|---|
| `open` | not started, a candidate for `next` |
| `in_progress` | actively being worked (derives into `focus.now`) |
| `deferred` | parked on the back burner (derives into `focus.deferred`) |
| `closed` | done |
| `wontfix` | terminal, but distinct from `closed` — satisfies a `{type:block}` dependency exactly like `closed`, and is tallied in its own `EpicProgress.wontfix` count so it never inflates the `closed` count in the epic progress line |

> **`blocked` is not authorable, and this command rejects it.** `blocked` is a
> *derived* lane: `emit-state` computes it from a block's unmet `depends_on` edges
> and stamps it onto `focus.blocked[]` entries. Writing it onto a `tracks[]` block
> is exactly what `validate-brain`'s `E_STATE_AUTHORED_BLOCKED` exists to catch, so
> input is validated against `VALID_TRACK_BLOCK_STATUSES` (the four above) and not
> against the wider `VALID_STATUSES`. Passing `blocked` fails with
> `E_BLOCK_BAD_STATUS` and writes nothing.

**Setting a block to the status it already has is a no-op success** — zero actions,
zero diagnostics, exit `0`, nothing written. Running the same `--write` twice leaves
the corpus byte-identical.

**Dry-run by default**, exactly like the epic commands: without `--write` the
proposed edit prints as `W_EMIT_DRY_RUN` and not a byte is touched. A successful
`--write` takes the same advisory lock `emit-state --write` takes, refuses to run
against an incomplete corpus (`E_EMIT_INCOMPLETE_CORPUS`), and then runs
`emit-state --write` so `focus`, the boards and the rollups agree with the new
authored value in the same invocation.

**Who calls this.** The intended caller is an **engine-rs workflow node** invoking
the CLI on bastion-web's behalf — "mark this done", "park this" from the web UI.
`bastion serve` is **read-only by decision (D25)** and stays that way, so the write
lands here in mev, the deterministic writer for the brain corpus. The workflow node
itself is engine-rs work and is not part of this command's contract.

**Starting a block that is operator-gated is refused.** Moving a block to
`in_progress` with `--write` while it carries an unmet `Operator` `depends_on`
edge fails with `E_BLOCK_OPERATOR_GATED` unless `--force-operator-gate` is also
passed. The override itself is refused with `E_FORCE_OPERATOR_GATE_NOT_TTY`
whenever stdin is not a TTY — there is no other bypass, and no priority
threshold exempts a block from the gate. The gate only guards *starting*; moving
an operator-gated block to any other status needs no override.

```bash
# What would closing MV.10.A change? (dry run — writes nothing)
mev set-block-status mev:MV.10.A closed ~/Dev/agentic-portfolio

# Apply it, and regenerate every derived view
mev set-block-status mev:MV.10.A closed ~/Dev/agentic-portfolio --write

# Park a single block without touching its epic
mev set-block-status bella:BE.2.C deferred --write

# Mark a block as intentionally not being done (terminal, distinct from closed)
mev set-block-status mev:MV.10.B wontfix ~/Dev/agentic-portfolio --write

# Start a block despite an unmet operator gate (interactive shells only)
mev set-block-status mev:MV.10.C in_progress ~/Dev/agentic-portfolio --write --force-operator-gate

# Machine-readable
mev --json set-block-status mev:MV.10.A in_progress --write
```

Exit codes: `0` planned (dry-run), applied, or already at the target status · `1`
any error-severity diagnostic or a write failure.

| Diagnostic | Cause |
|---|---|
| `E_BLOCK_BAD_KEY` | the key is not `repo:id` (a bare block id, or an empty half) |
| `E_BLOCK_BAD_STATUS` | the status is not one of the five authorable values — this is what rejects `blocked` |
| `E_BLOCK_NOT_FOUND` | no loaded `state.json` owns that `repo:id`; the message lists the known repo slugs when the repo half is the problem |
| `E_EMIT_INCOMPLETE_CORPUS` | `--write` attempted while at least one `state.json` failed to load |
| `E_EMIT_LOCK_HELD` | another mev write holds the brain-root advisory lock |
| `E_BLOCK_OPERATOR_GATED` | `--write`ing a block to `in_progress` while it carries an unmet `Operator` `depends_on` edge, without `--force-operator-gate` |
| `E_FORCE_OPERATOR_GATE_NOT_TTY` | `--force-operator-gate` was passed but stdin is not a TTY |

---

### `close-operator-gate <slug> --exit-verified [path] [--write]`

Removes every `Operator` `depends_on` edge carrying `slug`, fleet-wide, under the
same advisory lock `emit-state --write` takes. This is a **verified-or-refused**
command, not a dry-run/`--write`-shaped planner like the epic commands: it refuses
outright, before any file is read, unless `--exit-verified` is passed — passing
the flag is the caller asserting the gate's exit condition has actually been
checked, not a formality. An unknown slug (no loaded file has a matching edge)
is also refused. A successful `--write` re-runs `emit-state --write` so `focus`
and the boards drop the closed gate in the same invocation.

```bash
mev close-operator-gate deploy-approval-2 --exit-verified ~/Dev/agentic-portfolio --write
```

| Diagnostic | Cause |
|---|---|
| `E_OPERATOR_GATE_NOT_VERIFIED` | `--exit-verified` was not passed |
| `E_OPERATOR_GATE_UNKNOWN` | no loaded `state.json` has an `Operator` edge matching `slug` |
| `E_EMIT_LOCK_HELD` | another mev write holds the brain-root advisory lock |

Exit codes: `0` applied · `1` refused or a write failure.

---

### `approve <slug> --digest <digest> [path] [--write]` · `reject <slug> [path] [--write]`

Remove every `Approval` `depends_on` edge carrying `slug`, fleet-wide, under the
same advisory lock. `approve` additionally requires `--digest` to match every
matching edge's stored `digest`; a mismatch on *any* matching edge refuses the
whole call and changes nothing (`E_APPROVAL_DIGEST_MISMATCH`) rather than
silently re-queuing the block — a shared slug is meant to carry one reviewed
payload. `reject` takes no digest and always clears matching edges; the
rejection is recorded via a non-suppressible `I_EMIT_WROTE` diagnostic (the same
pattern `close-operator-gate` uses), not a separate log file. Both re-run
`emit-state --write` on a successful `--write`.

```bash
mev approve ship-decision-1 --digest sha256:9f2c... ~/Dev/agentic-portfolio --write
mev reject ship-decision-1 ~/Dev/agentic-portfolio --write
```

| Diagnostic | Cause |
|---|---|
| `E_APPROVAL_DIGEST_MISMATCH` | (`approve` only) `--digest` does not match a matching edge's stored digest |
| `E_OPERATOR_GATE_UNKNOWN` | no loaded `state.json` has an `Approval` edge matching `slug` |
| `E_EMIT_LOCK_HELD` | another mev write holds the brain-root advisory lock |

Exit codes: `0` applied · `1` refused or a write failure.

---

### `doc materialize` · `doc opportunity ingest|set-stage|add-action|merge-contacts`

The generic brain-document materializer (Phase 9, Block MV.9.A) — plans (and, with `--write`,
applies) any of okf-core's three `BrainDocModel` implementors (`Opportunity`,
`LearningArtifact`, `Proposal`) from a raw JSON payload, plus the Opportunity-specific
command family for the `business/docs/opportunities/` corpus.

```bash
mev doc materialize --model <opportunity|learning-artifact|proposal> --input <path.json> [path] [--write]
mev doc opportunity ingest --input <path.json> [--kind company|prospecting-sweep|job-posting] [path] [--write]
mev doc opportunity set-stage <slug> <stage> [path] [--write]
mev doc opportunity add-action <slug> --kind <k> --note <n> [--at <ISO date>] [path] [--write]
mev doc opportunity merge-contacts <slug> --input <path.json> [path] [--write]
```

Every verb resolves its target-corpus root via `find_brain_root` from the optional trailing
`path` argument (default `.`), exactly like `emit-state`. **Dry-run is the default on every
verb** — without `--write`, nothing is touched on disk and every planned action is still
reported; `--write` applies the plan.

#### `doc materialize`

| Flag | Default | Description |
|---|---|---|
| `--model` | *(required)* | Which okf-core model to build: `opportunity` \| `learning-artifact` \| `proposal` |
| `--input` | *(required)* | Path to the JSON payload the model is built from |
| `path` | `.` | Path to search from when locating `brain.toml` |
| `--write` | off | Apply the write; without this the command is a dry-run |

`--model opportunity` dispatches through the same shape auto-detection as `doc opportunity
ingest` (`--kind` is not exposed on this generic verb — use `doc opportunity ingest` when you
need to name it explicitly). `--model learning-artifact` builds via
`LearningArtifact::from_payload(input)`; `--model proposal` reads `company_name` and `roadmap`
off `input` and builds via `Proposal::from_automation_roadmap`. Any other `--model` value pushes
`E_DOC_UNKNOWN_MODEL` and plans nothing.

#### `doc opportunity ingest`

| Flag | Default | Description |
|---|---|---|
| `--input` | *(required)* | Path to a `CompanyBrief` / `ProspectingResult` / job-posting JSON payload |
| `--kind` | auto-detect | `company` \| `prospecting-sweep` \| `job-posting`. Omit to auto-detect from the input's shape (`company_name` present → company; `prospects`/`vertical` present → prospecting-sweep; neither → `E_DOC_UNKNOWN_INPUT_SHAPE`, pass `--kind` explicitly) |
| `path` | `.` | Path to search from when locating `brain.toml` |
| `--write` | off | Apply the write; without this the command is a dry-run |

Creates or updates the target Opportunity document (path derived from its `IndexIntent`, under
`business/docs/opportunities/`) and reconciles that directory's `index.md` table in the same
plan. The raw ingested payload is embedded as the first fenced `json` block in the body.

#### `doc opportunity set-stage <slug> <stage>`

Sets an existing Opportunity's `stage` field. `stage` must be one of the values authored in
`business/docs/pipeline.md`'s `## Stages` line — the vocabulary is **read from that file**, not
compiled into `mev`, per [D58](../../../docs/decisions/D58-pipeline-stage-vocabulary-home.md)
(the file's `## Stages` line is the single source both `mev` and bastion's `parse_stages` read).
As documented today that line resolves to the seven values `identified | researching | contacted
| conversation | proposal-sent | closed-won | closed-lost`, but that list can change without a
`mev` release — only `pipeline.md` needs to change. `pipeline.md` is resolved from the brain root
(walked upward from the target document's path), never from CWD. Any `stage` value outside the
resolved vocabulary pushes `E_DOC_BAD_STAGE` and plans nothing. Re-running with the same stage is
a zero-action no-op (`W_DOC_UNCHANGED`).

Resolving the vocabulary itself can fail independently of the `stage` argument's validity — see
`E_DOC_PIPELINE_ROOT_NOT_FOUND`, `E_DOC_PIPELINE_MD_MISSING`, and
`E_DOC_PIPELINE_STAGES_UNPARSEABLE` in the diagnostics table below. Each names the file (or the
search root) and plans nothing; none panics, and none degrades into flagging every stage as
invalid.

#### `doc opportunity add-action <slug>`

| Flag | Default | Description |
|---|---|---|
| `--kind` | *(required)* | The action's kind (e.g. `email`, `call`, `meeting`) |
| `--note` | *(required)* | A free-form note describing the action |
| `--at` | today | The action's ISO date |

Appends one `{at, kind, note}` entry to the opportunity's `actions[]`. An identical triple
already present is not re-appended — a repeat call is a zero-action no-op.

#### `doc opportunity merge-contacts <slug>`

| Flag | Default | Description |
|---|---|---|
| `--input` | *(required)* | Path to a JSON contact object, or a JSON array of contact objects |

Merges contacts into the opportunity's `contacts[]`, matched on `name`: `emails` / `whatsapp` /
`phones` / `links` are unioned (deduped, order-stable), and `role`/`note` are filled only when
the existing value is empty. An already-merged contact is a zero-action no-op.

#### Shared behaviour across every `doc` verb

- **Linked-worktree write guard:** `--write` from inside a linked git worktree is refused with
  the same guard message `emit-state` uses (`doc` resolves derived-file paths from `brain.toml`,
  not CWD, so a worktree write would silently regenerate the main checkout's files instead).
- Every mutator (`set-stage` / `add-action` / `merge-contacts`) requires the target document to
  already exist; a missing target pushes `E_DOC_NOT_FOUND` and plans nothing.
- `--json` wraps the report in the standard `JsonReport` envelope (see below), labelled
  `doc-materialize`, `doc-opportunity-ingest`, `doc-opportunity-set-stage`,
  `doc-opportunity-add-action`, or `doc-opportunity-merge-contacts`.

#### Diagnostic codes

| Locator | Severity | Condition |
|---|---|---|
| `W_DOC_UNCHANGED` | Warning | Computed content already matches the existing file; no action planned |
| `W_DOC_MISSING_SENTINEL` | Warning | A `BodySection::Generated` section's sentinel pair is absent; that section is left untouched rather than clobbered |
| `W_DOC_INDEX_MISSING` | Warning | The target `index.md` is absent; no index action planned (never creates one) |
| `W_DOC_INDEX_NO_TABLE` | Warning | `index.md` has no parsable table; no index action planned |
| `W_DOC_INDEX_COLUMN_MISMATCH` | Warning | The model's `row_cells` count doesn't match the table's column count; no index action planned |
| `E_DOC_BAD_INDEX_PATH` | Error | The model's `IndexIntent.index_path` has no parent directory component |
| `E_DOC_UNKNOWN_INPUT_SHAPE` | Error | `ingest` input matches neither the company nor the prospecting-sweep shape and `--kind` was not given |
| `E_DOC_UNKNOWN_MODEL` | Error | `materialize --model` is not one of `opportunity` \| `learning-artifact` \| `proposal` |
| `E_DOC_BAD_STAGE` | Error | `set-stage`'s `stage` argument is not in the vocabulary parsed from `business/docs/pipeline.md`'s `## Stages` line (D58) |
| `E_DOC_PIPELINE_ROOT_NOT_FOUND` | Error | No brain root (`brain.toml`) could be located above the target document's path, so `business/docs/pipeline.md` cannot be resolved to validate `stage` |
| `E_DOC_PIPELINE_MD_MISSING` | Error | The brain root was found but `business/docs/pipeline.md` does not exist (or cannot be read) there |
| `E_DOC_PIPELINE_STAGES_UNPARSEABLE` | Error | `business/docs/pipeline.md` exists but has no parseable `## Stages` section (missing heading, or no backtick-delimited tokens before the next heading) |
| `E_DOC_NOT_FOUND` | Error | A mutator's target file is absent or unparsable |
| `W_EMIT_DRY_RUN` / `I_EMIT_WROTE` | Warning | Reused unchanged from `apply_plan`'s write half — see `emit-state` above |

Exit codes: `0` planned (dry-run) or applied successfully with no errors · `1` a
resolution/parse/write failure, a linked-worktree write refusal, or any error-severity
diagnostic (`E_DOC_*` / `E_CONFIG_NOT_FOUND`).

**Examples:**

```bash
# Dry-run: what would ingesting this brief produce?
mev doc opportunity ingest --input company-brief.json

# Apply it
mev doc opportunity ingest --input company-brief.json --write

# Explicit kind
mev doc opportunity ingest --input posting.json --kind job-posting --write

# Move an opportunity forward
mev doc opportunity set-stage acme-co contacted --write

# Log an action
mev doc opportunity add-action acme-co --kind email --note "sent intro" --write

# Merge in a new contact
mev doc opportunity merge-contacts acme-co --input contact.json --write

# Materialize a learning-artifact document from a payload
mev doc materialize --model learning-artifact --input lesson-payload.json --write

# Machine-readable dry-run output
mev --json doc opportunity ingest --input company-brief.json
```

---

### `carryover [--repo <slug>] [--json] [--allow-exec] [--audit] [--window <days>] [path]`

Fleet-wide, **read-only** sweep of every discovered `planning/state.json`'s `carryover[]`
array. Evaluates each entry's `clears_when` predicate where it is machine-checkable and sorts
the fleet into three lanes. `--audit` switches to a census over both triage containers instead
(see [`--audit` — the `carryover[]`/`reference[]` census](#--audit--the-carryover-reference-census)
below).

```bash
mev carryover [--repo <SLUG>] [--json] [--allow-exec] [--audit] [--window <DAYS>] [path]
```

| Argument / Flag | Default | Description |
|---|---|---|
| `path` | `.` | Path to search from when locating `brain.toml` (walks up to find it) |
| `--repo <SLUG>` | unset | Restrict the sweep to one repo's `carryover[]` entries. An unknown slug is a hard error naming the valid slugs |
| `--json` | off | Emit the `CarryoverReport` (or, under `--audit`, the `CarryoverAudit`) as compact JSON instead of the human summary |
| `--allow-exec` | off | Opt in to running `command_exits_zero` predicates. Without it, every such entry reports `not-evaluable` (reason `execution-not-allowed`) and **no command is ever run** |
| `--audit` | off | Report a fleet-wide `carryover[]`/`reference[]` census instead of the per-entry sweep — total, per-container and per-kind/per-class counts, typed-predicate coverage, and inflow/outflow over `--window` days. Composed entirely from the same loaded corpus and `CarryoverReport` the ordinary sweep already produces — no second corpus walk |
| `--window <DAYS>` | `30` | Window, in days, `--audit`'s inflow/outflow figures are measured over. Ignored without `--audit` |

Resolves `brain.toml` by walking up from `path`, discovers and loads every repo's
`planning/state.json` (individual load failures are skipped, not fatal), and evaluates every
`carryover[]` entry against the corpus.

**`mev carryover` never writes anything.** No `--write` flag exists for this subcommand. The
`cleared` lane is a recommendation for a human (or a later, separately specced mutation
command) to act on — it is never an automatic deletion.

#### The three lanes

| Lane | Meaning |
|---|---|
| `cleared` | At least one reference was extracted from the entry and **every** extracted reference is currently satisfied — a recommendation to delete the entry |
| `actionable` | At least one reference was extracted, but **at least one** is unsatisfied — the specific unmet reference(s) are named so a reader can act without re-reading the predicate |
| `not-evaluable` | No reference could be extracted. Reason `prose` (`clears_when` is present but is pure prose), `no-closure-verb` (it names a block but never says the block must close), `ambiguous-reference` (a bare block ID matched more than one repo and was dropped), `execution-not-allowed` (a `command_exits_zero` predicate was present but `--allow-exec` was not passed), `gate-mention-not-checkable` (it names a validator/gate/CI concept but nothing checkable — no path, no block — could be extracted, flagged as a candidate for a typed `command_exits_zero` predicate), or `no-predicate` (`clears_when` is `None`) |

#### Typed `clears_when` predicates

Alongside prose, `clears_when` may be a typed predicate object (`{"type": "block_closed", ...}`
etc.). All four typed predicate kinds are evaluated:

- **`block_closed { repo, id }`** — satisfied when `"{repo}:{id}"`'s authored status in the
  loaded corpus is exactly `closed`. A `{repo, id}` pair with **no matching node at all** in the
  loaded corpus (a typo'd repo slug or ID) is never satisfied and is reported distinctly from an
  ordinary unmet reference — `unresolvable: {repo}:{id} (not found in loaded corpus)` in the
  human summary, `{"type": "unresolved_block", "key": "..."}` in `--json` — so a data problem
  doesn't read the same as "the block just hasn't closed yet". Unlike the prose grammar, the
  typed form needs no [`CLOSURE_VERBS`](#the-two-evaluable-predicate-classes) gate: it is
  unambiguous by construction.
- **`file_exists { path }`** — satisfied under the same two-root resolution as the prose Class B
  reference below (brain root, then the owning repo's path).
- **`file_contains { path, pattern }`** — satisfied when `path` resolves under that same
  two-root strategy **and** its contents contain `pattern` as a literal substring (never a
  regex). Every failure mode — missing file, unreadable file, non-UTF8 contents, or a file
  larger than 5 MiB (never read into memory) — is `satisfied: false`, never a panic and never
  `satisfied: true`.
- **`command_exits_zero { command }`** — satisfied only when running `sh -c <command>` (cwd: the
  owning repo's path if known, else the brain root) exits with status `0` **and** `--allow-exec`
  was passed. This is the one predicate that executes arbitrary shell from a data file, so it
  carries three deliberate safety properties:
  1. **Opt-in, off by default.** Without `--allow-exec`, `command_exits_zero` entries are never
     run — they report `not-evaluable` with reason `execution-not-allowed` instead. An unrun
     command is unknown, and unknown must never read as `cleared`.
  2. **In-process wall-clock timeout.** `timeout(1)` does not exist on macOS, so the ~2s bound is
     enforced by polling `try_wait` and killing the child on expiry — a bad predicate cannot hang
     a fleet-wide sweep.
  3. **Failure is never success.** Spawn failure, non-zero exit, signal death, and timeout are
     all `satisfied: false`; only a clean exit status of `0` satisfies.

#### The two evaluable prose predicate classes

Only two classes of prose `clears_when` are ever machine-evaluated; anything else falls into
`not-evaluable` rather than being guessed at:

- **Block references — from `clears_when` only.** Block IDs matched in the prose by a strict
  grammar (`[A-Z]{2,3}\.(?:\d+\.[A-Z0-9]+|ticket\.[a-z0-9][a-z0-9-]*|chore\.[a-z0-9][a-z0-9-]*)`).
  A match is kept only when **both** hold:
  1. The predicate contains a word-bounded **closure verb** — one of `land` · `lands` ·
     `landed` · `landing` · `ship` · `ships` · `shipped` · `shipping` · `merge` · `merges` ·
     `merged` · `closes` · `closed`. A predicate that names a block without one is reported
     `not-evaluable` with reason `no-closure-verb`.
  2. The token resolves to exactly one node in the loaded corpus (preferring the carryover's
     own scope repo when the bare ID is ambiguous); an ID resolving to nodes in more than one
     repo is dropped and the entry reported `not-evaluable` with reason `ambiguous-reference`
     rather than guessed at. An unresolvable token is simply not a block reference and is
     discarded silently.

  A block reference is satisfied when its node's authored status is `closed`.

  **`related[]` is not consulted.** The schema documents it as *optional related edges* — a
  "see also", not a clearing condition. A carryover merely related to block X does not clear
  when X closes.

> **Why both gates exist.** Verified against the live corpus 2026-08-03:
> `core:ba-0-a-id-collision` reads *"one of the two `BA.0.A` blocks is renamed and Phase 0 is
> backfilled"*, and `BA.0.A` **is** `closed`. Without the closure-verb gate the sweep
> recommended deleting a live, unresolved `known_issue`. A false `cleared` is the only verdict
> here that destroys durable knowledge.
- **Path assertion references** — extracted only when the `clears_when` text contains a
  word-bounded entry from a bounded, documented verb vocabulary — the path analogue of the
  closure-verb gate above. A path named with no assertion verb at all is a *subject*, not a
  *condition*, and nothing is extracted for it (`not-evaluable`, reason `prose`).
  - **Presence verbs** — `exists` · `created` · `added` · `written` · `present` · `corrected` ·
    `fixed`. Whitespace-delimited tokens containing `/` and ending in one of
    `.md .rs .py .sh .ts .tsx .json .toml` are resolved against the brain root and against the
    owning repo's `repo_path`; satisfied when either resolves to an existing file. The
    `corrected`/`fixed` verbs pair with an already-checkable file this way rather than attempting
    to parse what "corrected" means — a predicate like *"X is corrected"* with no named file/block
    stays `not-evaluable`.
  - **Absence verbs** — `removed` · `deleted` · `gone`. Same path resolution, but satisfied when
    the path does **not** resolve to an existing file — the inverse polarity, reported as a
    distinct `path_absent` reference (`{"type": "path_absent", "path": "...", "satisfied": ...}`
    in `--json`) so "the path exists" is never conflated with "the path is gone".
  - When a predicate names a validator/gate/CI concept (`validator` · `gate` · `lint` · `linter` ·
    `harness` · `pipeline` · `suite` · `ci`) but nothing checkable was extracted from it, it is
    reported `not-evaluable` with reason `gate-mention-not-checkable` rather than plain `prose` —
    a hint that it is a candidate for a typed `command_exits_zero` predicate. Nothing derives a
    command from this prose and runs it automatically; that stays explicitly out of scope.

**All extracted references are combined conjunctively (AND), even when the prose says "or".**
This is a deliberate, safe-failure-direction bias: it can mis-report a genuinely-cleared
`or`-predicate as `actionable`, but it can never mis-report an unmet dependency as `cleared`.
Disjunction parsing is out of scope.

Every reported entry also carries its repo, slug, kind, `age_days`, and a `stale` flag derived
from the existing `carryover_stale_age` helper (honouring `reviewed` / `snoozed_until`) — no
staleness logic is reimplemented here. As of `MV.ticket.carryover-triage-ranking`, each entry
additionally passes through its authored `priority` (0..=3, absent when not set), `finding_id`,
and `blocks[]` (the `BlockedBy` edges the entry gates) verbatim — the same fields the Attention
board's carryover triage lanes are ranked on (see [Carryover triage lanes](#carryover-triage-lanes)
above and [`docs/carryover-contract.md`](carryover-contract.md) for the full, versioned wire
shape). `mev carryover`/`mev carryover --json` itself still sorts and reports the three
`clears_when` lanes (`cleared`/`actionable`/`not-evaluable`) below — those are an orthogonal
question from the four triage lanes and are unaffected by this block.

#### Cross-repo dedup: clusters, suggestions, and the typo guard

`MV.ticket.carryover-dedup-clusters` adds a second, orthogonal pass over the same loaded
`entries` — no new file reads, no second discovery walk. It answers "is this the same finding
filed more than once?" using the free-form, authored `finding_id: Option<String>` field.

- **`clusters`** (`CarryoverReport.clusters`, human section `CLUSTERS`) — every entry sharing a
  non-empty `finding_id`, grouped exactly one cluster per distinct id string. Grouping is exact
  (no case-folding, no fuzzy join): `finding_id` is hand-written by a human, so the human is the
  identity authority, not the tool. Two or more entries in the *same* repo may legitimately share
  one `finding_id` (many-to-one) — they still appear as distinct members, never collapsed.
  **Per-repo priority divergence is shown side by side and is never reconciled.** A claim can be
  genuinely P0 in one repo and genuinely P2 in another (the measured case: a `nextest` claim is
  P0 in `okf-core`, where the hook does not fire, and P2 in `mev`, where it works as documented) —
  dedup merges the *claim*, never the *priority*. No merged/effective/max/min priority field
  exists anywhere in the shape, and no diagnostic is emitted merely because priorities diverge.
- **`suggestions`** (`CarryoverReport.suggestions`, human section `SUGGESTED DUPLICATES —
  UNCONFIRMED`) — candidate duplicate pairs among entries that carry **no** `finding_id`, from a
  crude token-overlap pass over `slug` + `text` (stopwords removed, tokens under 3 chars
  dropped). A pair is suggested when `jaccard >= 0.18` **or** `overlap_coefficient >= 0.34` —
  both operator-measured against the live corpus, in `DEDUP_JACCARD_MIN` /
  `DEDUP_OVERLAP_MIN`. **Suggestions are never auto-applied.** They do not mutate `finding_id`
  and are not written to any file; a human confirms a suggested pair by hand-authoring the same
  `finding_id` string onto both entries' `planning/state.json`. The heading itself carries
  UNCONFIRMED, not only a trailing note, since a heading is what survives a skim.
- **`single_repo_finding_ids`** (human section `SINGLE-REPO finding_id WARNINGS`) — the sorted
  list of `finding_id` values whose cluster spans exactly one repo. A `finding_id` is meant to
  link the *same* finding **across** repos; one that never left a single repo usually means the
  id was mistyped somewhere and silently failed to group with its intended match — the same
  "field nothing validates" defect class this feature exists to close.

All three sections are omitted from the human summary when empty, matching the existing lane
behaviour, and none of them affects the exit code.

#### `--audit` — the `carryover[]`/`reference[]` census

`mev carryover --audit` (`MV.ticket.reference-container-validation` task 4) answers "what does
the fleet's triage material actually look like", as opposed to the per-entry sweep above, which
answers "what should a human act on right now". It is composed entirely from the same loaded
corpus (`files`) and `CarryoverReport` the ordinary sweep already produced — no new filesystem
read, no second discovery walk — and, like every other `carryover` invocation, it is
**read-only**: the audit recommends, a human disposes; nothing is ever deleted or rewritten.

| Figure | Meaning |
|---|---|
| `total` / `carryover_count` / `reference_count` | Fleet-wide entry count, and the split across the two containers |
| per-kind (`carryover[]`) | `carryover[]` entries grouped by `kind` — includes legacy `constraint`/`known_issue` wherever they still appear, since D72's narrowing didn't rewrite any data |
| per-class (`reference[]`) | `reference[]` entries grouped by `class` (`trap`/`invariant`/`lesson`/`deliberate`, plus any not-yet-valid value present in the corpus) |
| typed-predicate coverage | How many `carryover[]` entries carry a typed `clears_when` predicate (`block_closed`/`file_exists`/`file_contains`/`command_exits_zero`) rather than free prose or no predicate at all |
| clear rate | `cleared_total / clearable_total` — **scoped to `carryover[]` only.** `reference[]` entries have no `clears_when` and are structurally never clearable, so they are excluded from the denominator by construction, not by a filter: a raw per-repo rate would punish reference-heavy repos for behaving correctly (measured on the live corpus: `bastiel` 11%, `okf-core` 0/14 — composition, not discipline) |
| inflow / outflow | Entries whose `created` date falls within `--window` days of today (inflow), and `Cleared`-lane entries whose staleness anchor (`max(created, reviewed)`) falls within the window (outflow) — a proxy for "recently became safe to delete", since no container records an actual deletion timestamp |

#### Exit codes

| Code | Meaning |
|---|---|
| `0` | Sweep (or, under `--audit`, the census) completed successfully, regardless of how many entries land in any lane |
| `1` | `brain.toml` not found/unreadable, an unknown `--repo` slug, or a serialization error under `--json` |

**Examples:**

```bash
# Human, lane-grouped summary of the whole fleet
mev carryover

# Machine-readable JSON envelope
mev carryover --json

# Restrict to one repo
mev carryover --repo mev

# From an explicit brain root
mev carryover ~/Dev/agentic-portfolio

# Opt in to running command_exits_zero predicates
mev carryover --allow-exec

# Fleet-wide carryover[]/reference[] census, default 30-day window
mev carryover --audit

# Census over a 90-day inflow/outflow window, as JSON
mev carryover --audit --window 90 --json
```

---

### `attention-queue [--out <path>] [path]`

Emits every Attention-board item — across all four lanes (stale carryover's `Blocking`/`Hot`/
`Aging`/`Standing` sub-lanes, aging backlog, orphaned captures, and stale distilled knowledge) —
as a JSON array of `EN.8.A`-compatible operator payloads (`MV.ticket.attention-queue-delivery`).
This is how the Attention board stops being a surface somebody has to remember to open: instead of
running `/attention` and triaging the whole list in one sitting, `engine-rs`'s operator queue
(`EN.8.B`) can deliver one item at a time, in priority order.

`attention-queue` reuses the exact same corpus load and `effective_priorities` derivation that
`emit-state`'s attention-board planner (`plan_attention_board`) uses internally — the same
`collect_attention_rows` call, the same carryover union, the same backlog/distilled staleness
thresholds. There is only one board-derivation path in this codebase; the queue can never show a
different item, or a different order, than `/attention` itself would show.

| Argument / Flag | Default | Description |
|---|---|---|
| `path` | `.` | Path to search from when locating `brain.toml` (walks up to find it). |
| `--out <path>` | stdout | Write the JSON array to this file instead of printing it. |

#### Payload shape

Each array element is an `AttentionQueuePayload`:

| Field | Description |
|---|---|
| `item_id` | Stable identifier hashed from the item's IDENTITY only (repo, lane, slug) — never from mutable content. Re-running on an unchanged corpus reproduces it byte-for-byte; an item whose text/age/priority changed keeps its `item_id` and gets a new `digest` instead — a re-queue, not a new item, per `EN.8.A`. |
| `gate_id` | `"attention:<item_id>"` — excluded from the digest, per `OperatorPayload`'s own contract. |
| `rendered_summary` | Self-contained decision text: repo, lane, kind, slug, age, the item's text, and effective priority where present. An operator reading only the notification can decide without opening the repo. |
| `options` | 2–3 named `{key, label}` response options (see "Per-lane option sets" below). |
| `digest` | SHA-256 over `rendered_summary` + `options`, byte-identical to what `engine-core`'s `OperatorPayload::digest_of` computes for the same inputs — pinned by a hard-coded expected hex digest in this repo's test suite, so a future drift in either side's algorithm fails loudly instead of producing payloads the queue silently re-queues forever. |
| `effective_priority` | The post-propagation priority the board ranked this item at, supplied by mev rather than recomputed by the queue — `EN.8.B`'s `OperatorQueueItem::effective_priority` is enqueuer-supplied, never queue-computed. |
| `lane` | The carryover triage lane (`Blocking`/`Hot`/`Aging`/`Standing`), or absent for backlog/capture/distilled rows. |
| `repo` | The repo slug this item belongs to. |
| `source` | Provenance tag; `"attention-board"` for every item this command emits. |

The `gate_id`/`rendered_summary`/`options`/`digest` subset deserializes unchanged into
`engine-core`'s `OperatorPayload` — `item_id`, `effective_priority`, `lane`, `repo`, and `source`
ride alongside it as mev-owned fields the queue uses for ordering and provenance.

#### Per-lane option sets

`engine-core`'s `limits.rs` caps a payload at **3 response buttons**, **2 minimum**, and a
**20-character label limit** (confirmed against Meta's WhatsApp Cloud API docs 2026-08-12) — so
the board's five triage actions (promote · keep · snooze · resolve · archive) cannot ship as one
tap set. `attention-queue` resolves this by assigning each lane ≤3 options chosen for what that
lane can actually do, with every lane's set including a **session channel** option that routes the
operator to the full triage surface for any action that did not fit:

| Lane | Options | Why |
|---|---|---|
| Distilled (`knowledge.md`/`memory.md`) | Re-affirm, Open session | Never offers Snooze — HQ `CLAUDE.md`'s Attention rule: the distilled lane is re-affirmed by bumping `freshness:`, never snoozed. |
| Standing (carryover) | Keep, Open session | Never offers Promote or Resolve — `Standing` entries are permanently-true constraints, not items that graduate or close. |
| Blocking / Hot / Aging (carryover), Backlog, Capture | Promote, Snooze, Open session | The three actions that make sense for a genuinely time-bound item. |

A set outside 2..=3 options, or any label over 20 characters (measured in characters, not bytes —
labels may contain non-ASCII), is rejected in code at construction time rather than silently
truncated or dropped.

#### Ordering and stability

Items are sorted hottest-first by `effective_priority` (a lower number is hotter; an absent
priority sorts last), tie-broken by age descending then `item_id` ascending — a fully deterministic
order, so re-running on an unchanged corpus reproduces byte-identical output and a depth-1 queue
delivers the hottest item first.

#### Two boundaries this command does not cross

1. **mev derives; it does not project.** This command reads the corpus and emits an artifact —
   nothing more. It never enqueues into `engine-core`'s operator queue, opens a notification
   channel, or writes into `BA.18.A`'s sink. `engine-core`'s `queue/item.rs` module header states
   the crate does not read mev's state or shell out to `mev`; wiring this artifact into the queue
   (giving `ItemSource` a fourth, attention-sourced variant) is `engine-rs`'s change, made in a
   separate decision, not mev's.
2. **Depth limiting belongs to the queue, not mev.** `attention-queue` emits the *full* ordered
   set every run. `EN.8.B` holds items pending and releases one at a time — emission is not
   delivery. What mev owes is *correct ordering*; if `effective_priority` were wrong, depth-1
   delivery would faithfully deliver the wrong item first.

`attention-queue` is **read-only**: it never writes `state.json`, `BA.18.A`'s sink, or any
notification channel — only stdout, or the file named by `--out`. An empty board prints `[]` and
exits `0`; an empty queue is not an error.

**Examples:**

```bash
# JSON array to stdout, sibling brain repo at ..
mev attention-queue

# Explicit brain root
mev attention-queue ~/Dev/agentic-portfolio

# Write to a file instead of stdout
mev attention-queue --out /tmp/attention-queue.json
```

---

### `conformance [--check <name>] [--json] [path]`

A **registry of named drift checks** over facts kept in two places. Each registered check
canonicalizes both sides of a duplicated fact into a sorted item list, digests each side with an
in-house FNV-1a 64-bit hex digest (no new crate dependency — this is an equality/display aid, not
a security primitive), and compares. Equal digests pass; unequal digests report drift with the
concrete set difference in **both** directions (`only in <left>: ...` / `only in <right>: ...`),
so the operator sees what to fix rather than "these differ". Modelled on qm's
`conformance.ts` (`canonicalize → digest → compare → report`) — this is a gate, not an
auto-repair tool; no check ever writes anything.

```bash
mev conformance
mev conformance --check backlog-parity
mev conformance --json
mev conformance ~/Dev/agentic-portfolio
```

| Argument / Flag | Default | Description |
|---|---|---|
| `path` | `.` | Path to search from when locating `brain.toml` (walks up to find it) |
| `--check <name>` | unset | Run exactly one named check instead of the full registry. An unknown name is a hard error listing the valid check names |
| `--json` | off | Emit the `ConformanceReport` as compact JSON instead of the human, per-check summary |

#### The registered checks

| Name | Left side | Right side | What drift means |
|---|---|---|---|
| `backlog-parity` | HQ `planning/backlog.md` — ticket titles under `## Active` + `## Promoted` only (`## Superseded` and `## Shipped` are record-only and excluded), trimmed/whitespace-collapsed, sorted | `planning/state.json` `backlog[].title`, same canonicalization, sorted | A ticket title present in the markdown but missing from `backlog[]`, or vice versa — join key is the exact title |
| `epics-index-parity` | `core/planning/epics/index.md` rows as `(slug, status)`, where `slug` is resolved from the row's link target against the registry's own `epics[].plan` pointers (falling back to the link stem only when no `plan` value matches) | HQ `epics[]` registry as `(slug, status)` pairs, `status` being the registry's status field | A `(slug, status)` pair present on only one side — including an epic in `epics[]` with no index row, or an index row whose status disagrees with the registry. The check also asserts every registry epic's `plan` target exists on disk |
| `project-cache-watermark` | Each `docs/projects/<project>.md` frontmatter `synced_from` | The sub-repo's real `planning/status.md` frontmatter `timestamp` | Delegates entirely to `crate::brain::sync::check_sync` (the same logic behind `mev validate-brain --sync`) — this check is an **adapter**, not a reimplementation, and surfaces that function's `E_SYNC_DRIFT` / `E_SYNC_WATERMARK_MISSING` / `E_SYNC_WATERMARK_MALFORMED` / `E_SYNC_FILE_MISSING` diagnostics verbatim rather than re-parsing RFC3339 timestamps itself |
| `toolchain-freshness` | The running `mev` binary's compiled-in build stamp (`MEV_BUILD_GIT_SHA`, `MEV_BUILD_DIRTY`, `MEV_BUILD_SOURCE_DIR` — stamped into the binary by `build.rs` via `cargo:rustc-env` at compile time) | `git rev-parse HEAD` run in `MEV_BUILD_SOURCE_DIR` right now | A different live SHA than the stamped one ("the running binary is behind its source; rebuild"), or the same SHA but the build was dirty (a distinct drift message) |
| `sibling-rule-coverage` | The declared `SIBLING_RULES` table (`src/brain/conformance/sibling.rs`) — each rule's name + members | Source-text analysis of every `.rs` file under `<MEV_BUILD_SOURCE_DIR>/src` and `/tests` | Any of four findings on any rule: a member function that no longer routes through its `shared_helper`, re-inlines a `forbidden` predicate, has gone missing outright, or has lost its `covering_test`'s coverage of every member — see below |

#### `sibling-rule-coverage` — a rule taught to one function must be taught to its sibling

`derive_brain_focus` and `derive_rollup` both resolve a repo's state file and both must honour
the **dual-role rule** (a registered repo is either a leaf `kind: "project"` or a tier sub-brain
root `kind: "brain"` carrying its own authored `tracks[]`). The first learned the rule; the
second kept hard-filtering `kind == "project"` and stayed silently wrong for months — blind to
`business`'s 22 open blocks and `hq`'s 9 — because nothing checked that a rule taught to one
sibling was taught to the other. `sibling-rule-coverage` is that check, generalized: it declares
a table of sibling-function pairs (or triples) that must all route through one shared helper,
must never re-inline the predicate that helper replaced, and must all be exercised together by
one covering test.

Unlike the other checks (which compare two independently-maintained facts), this one is
**source-text analysis over the crate's own source** — the defect class is always "one call site
was edited and its sibling was not," which is directly visible in the text. No `syn`/AST parsing;
brace-depth counting over the crate's `.rs` files is sufficient and dependency-free.

**The `SiblingRule` fields** (`src/brain/conformance/sibling.rs`):

| Field | Meaning |
|---|---|
| `name` | Stable rule name, e.g. `"dual-role-repo-resolution"` |
| `invariant` | One sentence describing the shared rule, quoted verbatim in every finding |
| `members` | The function names that must all agree on the invariant |
| `shared_helper` | The function every member's body must call |
| `forbidden` | Inline substrings that must never reappear in a member's body — the pattern the shared helper was extracted to eliminate |
| `covering_test` | The name of a test whose body must mention every member — the "asserted against BOTH" proof |

**The four failure modes**, each a distinct named finding:

| Finding | Meaning |
|---|---|
| `missing-member` | A declared member function no longer exists in the source — the rule is stale; fix the rule or the rename |
| `helper-not-called` | A member's body does not mention `shared_helper` — the exact regression: someone re-implemented the rule locally |
| `forbidden-inlined` | A member's body contains one of the rule's `forbidden` substrings |
| `test-not-covering` | `covering_test` is absent, or present but does not mention every member |

Every finding's message names the rule, the member, and quotes the invariant verbatim.

**Not-evaluable, not drift, on a missing source tree:** the check locates the source via the
`MEV_BUILD_GIT_SHA`/`MEV_BUILD_SOURCE_DIR` build stamp (the same one `toolchain-freshness` uses).
If that variable is unset, `unknown`, or points at a missing directory, the whole check reports
`not-evaluable` — it never guesses a `pass`. Within a rule that *is* evaluable, a member whose
body cannot be located (function not found, or unbalanced braces) reports `missing-member`
rather than passing silently — the check never fails open.

**The two registered rules:**

1. **`dual-role-repo-resolution`** — *"A registered repo resolves to its state file whether that
   file is `kind: \"project\"` (leaf) or `kind: \"brain\"` (tier sub-brain root carrying its own
   authored `tracks[]`)."* Members: `derive_rollup`, `derive_brain_focus`. Shared helper:
   `resolve_repo_state_file`. Forbidden: `f.kind == "project"`. Covering test:
   `dual_role_rule_holds_for_both_resolvers` (`tests/sibling_rules.rs`).
2. **`block-status-map-construction`** — *"Authored block status is looked up through one
   `\"{repo}:{id}\"` map built by `block_status_map`; no call site rebuilds it inline."* Members:
   `check_status_consistency`, `ready_order`, `derive_focus`. Shared helper: `block_status_map`.
   Forbidden: `status_map.insert(`. Covering test: `all_status_consumers_agree_on_one_fixture`
   (`tests/sibling_rules.rs`).

**Registering a new sibling rule** — add one `SiblingRule` literal to `SIBLING_RULES` in
`src/brain/conformance/sibling.rs`, nothing else:

1. Name the invariant in one sentence — it will be quoted verbatim in every finding.
2. List the `members` that must agree on it.
3. Name (or extract, if it doesn't exist yet) the `shared_helper` every member must call.
4. Name the `forbidden` inline substring(s) the shared helper was extracted to eliminate.
5. Write (or reuse) a `covering_test` whose body mentions every member literally, and point
   `covering_test` at its name.

The registry is declarative — the CLI wiring, `--check` dispatch, and the four-finding-mode scan
logic all iterate `SIBLING_RULES` generically.

```bash
mev conformance --check sibling-rule-coverage
```

#### Pass / drift / not-evaluable

Every check reports exactly one of three statuses:

- **`pass`** — both sides canonicalized to the same digest (or, for `toolchain-freshness`, the
  stamped SHA matches the live HEAD and the build was clean).
- **`drift`** — the two sides genuinely diverge. Only a real two-sided comparison can produce
  this status.
- **`not-evaluable`** — the check's inputs were absent (the backlog file missing from this
  checkout, the epics index not present, the source dir gone, git unavailable, or a stamped
  value of `unknown`). **Absent inputs always yield `not-evaluable`, never `drift`** — a check
  that cannot compare both sides has nothing to report divergence about.

#### Exit codes

| Code | Meaning |
|---|---|
| `0` | Every check reported `pass` or `not-evaluable` |
| `1` | At least one check reported `drift`, **or** `brain.toml` was not found/unreadable, `--check` named an unknown check, or JSON serialization failed |

#### Registering a fifth check

The registry is a real registry — adding a check touches exactly two things:

1. Add one file under `src/brain/conformance/` (e.g. `my_check.rs`) exposing a
   `pub fn run(ctx: &ConformanceCtx) -> CheckOutcome`. Build both sides as sorted
   `Vec<String>`, wrap each in a `FactSide`, and hand them to the shared `compare_sides`
   helper (or return `CheckOutcome { status: CheckStatus::NotEvaluable, .. }` directly when an
   input is absent).
2. Add one `ConformanceCheck { name, description, run }` entry to `all_checks()` in
   `src/brain/conformance/mod.rs`.

Nothing else changes — the CLI wiring, `--check` dispatch, `--json` envelope, and exit-code
logic all iterate `all_checks()` generically. A test in `mod.rs` asserts every registered check
has a unique, non-empty name, so a name collision fails the suite rather than silently shadowing
a check.

**Examples:**

```bash
# Human, per-check summary of the whole registry
mev conformance

# Run just one check
mev conformance --check toolchain-freshness

# Machine-readable JSON envelope
mev conformance --json

# From an explicit brain root
mev conformance ~/Dev/agentic-portfolio
```

---

### `check-consumers [--consumer <slug>] [--json] [path]`

Compiles every path-dependent consumer's **test targets** against the working mev and reports
the true outcome per consumer (`ticket-consumer-compile-gate`). This is the expensive, rare
counterpart to `mev conformance --check consumer-dependency-parity` (below) — that one catches a
stale lockfile cheaply and constantly; this one catches a genuine type/API break expensively and
rarely. **Neither covers the other's failure class**; do not assume a clean run of one implies
the other.

Consumers are discovered the same way `consumer-dependency-parity` discovers them
(`discover_mev_consumers` — path dependencies on mev declared under `[dependencies]`,
`[dev-dependencies]`, `[build-dependencies]`, or `[workspace.dependencies]` in each repo listed
under `brain.toml`'s `[[repos]]`). There is deliberately only one discovery implementation; a
second one would fail this ticket's acceptance criteria outright.

```bash
mev check-consumers
mev check-consumers --consumer bastion
mev check-consumers --json
```

| Argument / Flag | Default | Description |
|---|---|---|
| `path` | `.` | Path to search from when locating `brain.toml` (walks up to find it) |
| `--consumer <slug>` | unset | Run exactly one discovered consumer by slug instead of every consumer. An unknown slug is a hard error |
| `--json` | off | Emit the per-consumer `ConsumerResult` list as compact JSON instead of a human, per-consumer summary |

#### The command, and why every flag on it is load-bearing

For each discovered consumer, `check-consumers` spawns exactly:

```bash
CARGO_TARGET_DIR=<fresh temp dir> CARGO_TERM_COLOR=never cargo nextest run --no-run --locked --manifest-path <consumer>/Cargo.toml
```

- **`--no-run`** compiles the test targets without executing them, and is the entire reason this
  command exists as a separate, expensive check rather than folding into `cargo build`. The break
  class it exists to catch lives only in test-fixture code — struct literals and call sites that
  only test code constructs. A compile-only build of the consumer's binary sails straight past
  them; `cargo build` cannot see this class of break at all.
- **`--locked`** refuses to let cargo silently rewrite the consumer's `Cargo.lock`. mev does not
  own that repo's lockfile, and a tool that mutates a repo it's only checking is a much worse
  failure than a false negative. This has been observed to happen for real during manual
  verification with a raw (non-`--locked`) invocation — see the ticket's own Notes.
- **A fresh `CARGO_TARGET_DIR`** (a new temp dir per run) avoids `target/` lock contention and
  incremental-cache churn against a consumer repo that may have its own build or CI lane running
  concurrently. It costs a cold compile every time; that's the accepted price of never
  interfering with another lane's build.
- **`CARGO_TERM_COLOR=never`** forces plain output. A CI runner that presents a pseudo-tty to
  spawned subprocesses makes rustc auto-detect color support and wrap `error[E....]` diagnostics
  in ANSI escapes, which the stderr-signature match in `extract_compiler_errors` cannot see
  through — observed 2026-08-15 as a genuinely `Broken` consumer classified `NotEvaluable` on
  mev's own hosted CI while passing locally. `extract_compiler_errors` also strips any ANSI it
  does receive as defense in depth, but forcing color off at the source is the real fix.

A future "simplification" that drops any one of these three restores exactly the failure mode it
exists to prevent — this section exists so that trade-off is written down, not just implied by a
command flag.

#### The five outcomes

| Outcome | Meaning | Fails the run? | Operator action |
|---|---|---|---|
| `pass` | The consumer's test targets compiled clean against the working mev | No | Nothing to do |
| `broken` | A genuine type/API break — compiler diagnostics with their site (e.g. `E0063 at src/serve/handlers/board.rs:660:9`) | **Yes — the only outcome that fails the run** | Fix the named sites in that consumer repo. mev never fixes another repo; a break is that repo's to repair |
| `lockfile-stale` | The consumer's `Cargo.lock` is out of date relative to its `Cargo.toml` (cargo's `cannot update the lock file` signature under `--locked`) — bookkeeping, not a code break | No | Refresh that consumer's lockfile (its change, not mev's) |
| `skipped-dirty` | `git status --porcelain` was non-empty for that consumer — its compile result is not evidence about mev's change either way | No | Commit or stash there, then re-run |
| `not-evaluable` | The failure didn't match a known signature, or an input couldn't be gathered at all (e.g. the lockfile moved despite `--locked`) | No | Reported with a `reason`; investigate manually rather than trusting an automatic verdict |

**`broken` and `lockfile-stale` are deliberately distinct outcomes with distinct exit
behaviour.** Collapsing a stale lockfile into `broken` is exactly the failure mode that made an
earlier, naive version of this gate untrustworthy: engine-rs's lockfile-stale exit code (102) has
nothing to do with mev's own compile correctness, and treating it as a red build trains everyone
to ignore red builds.

The consumer's `Cargo.lock` is verified byte-identical before and after every run — `mev
check-consumers` reports on a consumer, it never mutates one.

#### Exit codes

| Code | Meaning |
|---|---|
| `0` | Every consumer reported `pass`, `lockfile-stale`, `skipped-dirty`, or `not-evaluable` |
| `1` | At least one consumer reported `broken`, `brain.toml` was not found/unreadable, or `--consumer` named a slug that is not a discovered consumer |

#### Why this is a post-merge gate, not a per-task check

`check-consumers` is deliberately **not** wired into `planning/harness.json`'s
`validation.checks[]` — a cold consumer compile (bastion alone measured at ~1 minute) is too
expensive to pay at every task and every review inside the SDLC loop. It is instead wired as
stage 3 of the HQ-level `hooks/pre-push` (mev-repo-scoped, blocking only on `broken`,
skipping — never blocking — when the installed `mev` predates this subcommand or no
`brain.toml` is discoverable), which runs once per push after the work in a branch is done. See
`ticket-consumer-compile-gate`'s spec Notes for the full wiring rationale.

**The three historical breaks that motivate this check** — every one invisible to a plain
`cargo build`, because the break lived only in test-fixture code:

| Change | Damage |
|---|---|
| `okf-core:OK.3.B` added a non-`Option` field to six shared structs | 101 sites broke in mev, 31 in bastion |
| mev's D58 removed a public constant | broke engine-rs's workspace compile |
| `MV.ticket.reconcile-failed-consumer` changed a public return type + added a field | 2 sites broke in bastion (`board.rs:660`, `block_graph.rs:414`), both in test fixtures |

```bash
# Human, per-consumer summary of the whole fleet
mev check-consumers

# Just one consumer
mev check-consumers --consumer bastion

# Machine-readable JSON envelope for CI/tooling
mev check-consumers --json
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
